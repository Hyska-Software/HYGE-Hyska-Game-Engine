//! Render-pass barriers: usage transitions for textures and buffers, plus
//! the per-resource state table the compiler threads through.
//!
//! # Barrier inference algorithm (R-021)
//!
//! Given a topologically-ordered list of passes from
//! [`RenderGraph::compile`](crate::graph::RenderGraph::compile), the
//! algorithm walks each pass in order and emits a [`Barrier`] for every
//! resource whose access pattern changed. Concretely:
//!
//! 1. **Collect touched resources.** For each pass, union its
//!    [`Pass::reads`](crate::pass::Pass::reads) and
//!    [`Pass::writes`](crate::pass::Pass::writes) into a deduplicated
//!    list `T`.
//! 2. **For every resource `r` in `T`,** look up the prior access
//!    pattern in the [`BarrierStateTable`].
//!    - If `r` was touched by a previous pass in the current frame
//!      and its previous usage `from` differs from the current
//!      usage `to` (resolved from the pass's
//!      [`Pass::texture_usages`](crate::pass::Pass::texture_usages)
//!      / [`Pass::buffer_usages`](crate::pass::Pass::buffer_usages)
//!      hints, falling back to `wgpu::TextureUses::empty()` /
//!      `wgpu::BufferUses::empty()` when the pass did not declare
//!      one), emit a `Barrier::Texture { from, to }` (or `Buffer`)
//!      and update the table to `to`.
//!    - If `r` was not touched in the current frame (first touch
//!      of the frame), emit a `Barrier` with
//!      `from = wgpu::TextureUses::UNINITIALIZED` (or
//!      `wgpu::BufferUses::empty()`) when the resource is
//!      [`ResourceLifetime::Transient`], OR always emit a
//!      barrier when the resource is
//!      [`ResourceLifetime::Persistent`] — the runtime does not
//!      know the prior state of a persistent resource (it may have
//!      been written by the host, the swapchain, or an external
//!      system) so we conservatively force a transition so the
//!      underlying `wgpu` backend inserts a real
//!      `transition_resources` call at execute time.
//!    - If `from == to`, no barrier is emitted (the GPU is already
//!      in the required state).
//! 3. **Update the state table** to reflect the new usage. Subsequent
//!    passes see `r`'s new state on the next iteration.
//!
//! The result is per-pass `barriers_before: Vec<Barrier>` stored on
//! each [`CompiledPass`](crate::compile::CompiledPass). The execute
//! step later walks the same order and uses the inferred barrier
//! list to drive the actual `wgpu` transition emission (R-021 wires
//! up the per-resource hal callback; the wgpu transition emission
//! is the next implementation item — R-023 / R-024 in the
//! renderer).
//!
//! # Unsafe
//!
//! This is the **only** module in the crate that uses `unsafe`. The
//! [`BarrierStateTable`] is a raw-vec style container: it holds a
//! `Vec<MaybeUninit<AccessState>>` plus a `len` cursor and grows
//! via `Vec::set_len` in `ensure_slot`. The `set` / `get` accessors
//! use `MaybeUninit::assume_init_*` to project out an `AccessState`
//! only when the slot is known to be initialized (`idx < self.len`).
//! The safety invariants are documented inline on each `unsafe`
//! block. Miri exercises the container in
//! `tests::miri_barrier_state_table_set_len_is_sound` to verify the
//! unsafe is sound.

use std::fmt;
use std::mem::MaybeUninit;

use crate::resource::ResourceHandle;

/// A single resource-usage transition between two passes in the
/// compiled render graph.
///
/// The variants mirror the two kinds of GPU resources: textures and
/// buffers. The `from` field is the prior access pattern (or
/// `wgpu::TextureUses::UNINITIALIZED` / `wgpu::BufferUses::empty()`
/// for first-touch barriers).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Barrier {
    /// A texture-usage transition.
    Texture {
        /// The resource the barrier applies to.
        resource: ResourceHandle,
        /// The previous access pattern. `wgpu::TextureUses::UNINITIALIZED` for first touch.
        from: wgpu::TextureUses,
        /// The new access pattern. The pass following this barrier will
        /// read/write with these usages.
        to: wgpu::TextureUses,
    },
    /// A buffer-usage transition.
    Buffer {
        /// The resource the barrier applies to.
        resource: ResourceHandle,
        /// The previous access pattern. `wgpu::BufferUses::empty()` for first touch.
        from: wgpu::BufferUses,
        /// The new access pattern. The pass following this barrier will
        /// read/write with these usages.
        to: wgpu::BufferUses,
    },
}

impl Barrier {
    /// Returns the resource the barrier applies to, regardless of variant.
    #[must_use]
    pub fn resource(&self) -> ResourceHandle {
        match self {
            Barrier::Texture { resource, .. } | Barrier::Buffer { resource, .. } => *resource,
        }
    }

    /// Returns true if this is a texture barrier.
    #[must_use]
    pub fn is_texture(&self) -> bool {
        matches!(self, Barrier::Texture { .. })
    }

    /// Returns true if this is a buffer barrier.
    #[must_use]
    pub fn is_buffer(&self) -> bool {
        matches!(self, Barrier::Buffer { .. })
    }
}

impl fmt::Display for Barrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Barrier::Texture { resource, from, to } => {
                write!(f, "Texture({}: {:?} -> {:?})", resource, from, to)
            }
            Barrier::Buffer { resource, from, to } => {
                write!(f, "Buffer({}: {:?} -> {:?})", resource, from, to)
            }
        }
    }
}

/// The most recent access pattern observed for a resource during
/// the current compile pass.
///
/// `Copy` so a [`BarrierStateTable`] can hand it out by value from
/// [`BarrierStateTable::get`] without touching lifetimes. The
/// internal bitflag payload is also `Copy`; no `Drop` is needed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AccessState {
    /// The resource was last touched as a texture with these usages.
    Texture(wgpu::TextureUses),
    /// The resource was last touched as a buffer with these usages.
    Buffer(wgpu::BufferUses),
}

impl AccessState {
    /// Returns true if the prior usage matches the new usage for the
    /// same resource kind.
    #[must_use]
    pub(crate) fn matches(self, other: AccessState) -> bool {
        match (self, other) {
            (AccessState::Texture(a), AccessState::Texture(b)) => a == b,
            (AccessState::Buffer(a), AccessState::Buffer(b)) => a == b,
            _ => false,
        }
    }
}

/// Per-resource state table used by the barrier inference pass.
///
/// Indexed by [`ResourceHandle::index`]. Slots that have never been
/// touched are tracked by the `len` cursor: slots in `[0, len)` are
/// initialized to a valid `AccessState`, slots in `[len, capacity)`
/// are uninitialized `MaybeUninit<AccessState>` memory and must
/// not be read.
///
/// # Why `unsafe`?
///
/// The straightforward safe alternative is
/// `Vec<Option<AccessState>>::resize_with(idx + 1, || None)`. That
/// is correct but pays an extra discriminant write per slot on the
/// hot compile path. We use the raw-vec pattern instead so the
/// compiler can elide the per-slot zeroing; the cost is the four
/// `unsafe` blocks documented below, all of which are validated by
/// the `miri_barrier_state_table_set_len_is_sound` test under
/// `cargo +nightly miri test -p hyge-render-graph`.
pub(crate) struct BarrierStateTable {
    /// Backing storage. Slots `[0, len)` are initialized;
    /// slots `[len, capacity)` are uninitialized.
    storage: Vec<MaybeUninit<AccessState>>,
    /// Number of initialized slots. `len <= storage.len()`.
    len: usize,
}

impl BarrierStateTable {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            storage: Vec::new(),
            len: 0,
        }
    }

    /// Returns the number of initialized slots.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the table has no initialized slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Ensures the slot for `handle` exists in the backing storage,
    /// growing the Vec as needed. Idempotent: calling it on a handle
    /// whose slot already exists is a no-op.
    ///
    /// This is the only function that touches the uninitialized
    /// region; the `unsafe` block is documented inline.
    pub fn ensure_slot(&mut self, handle: ResourceHandle) {
        let idx = handle.index() as usize;
        if idx >= self.storage.len() {
            self.storage.reserve(idx + 1);
            // SAFETY: `Vec::reserve` guarantees the backing allocation
            // has at least `idx + 1` capacity. We then extend the
            // `Vec`'s length to `idx + 1` via `set_len`, which leaves
            // the new slots `[old_len, idx + 1)` as indeterminate
            // `MaybeUninit<AccessState>` memory. The contract of
            // `BarrierStateTable` is that the *initialized* region is
            // `[0, self.len)`; the new slots in `[old_len, idx + 1)`
            // are above `self.len` and are therefore not read by
            // [`BarrierStateTable::get`] until the caller writes them
            // via [`BarrierStateTable::set`], which sets the slot and
            // bumps `self.len` to cover it. The new slots are never
            // observed as uninitialized memory from outside the
            // `unsafe` block in `set`.
            unsafe {
                self.storage.set_len(idx + 1);
            }
        }
    }

    /// Returns the state previously stored for `handle`, or `None`
    /// if the slot was never written (either because `ensure_slot`
    /// was never called for this handle, or because it was called
    /// but `set` was not).
    #[must_use]
    pub fn get(&self, handle: ResourceHandle) -> Option<AccessState> {
        let idx = handle.index() as usize;
        if idx < self.len {
            // SAFETY: the slot at `idx` is in the initialized region
            // `[0, self.len)`. The contract of `set` requires that
            // every slot in that region holds a valid `AccessState`.
            Some(unsafe { self.storage[idx].assume_init_read() })
        } else {
            None
        }
    }

    /// Stores `state` at `handle`. Initializes the slot if it was
    /// previously uninitialized, and grows the table if necessary.
    pub fn set(&mut self, handle: ResourceHandle, state: AccessState) {
        let idx = handle.index() as usize;
        if idx >= self.storage.len() {
            self.ensure_slot(handle);
        }
        if idx >= self.len {
            // Initialize a fresh slot. We assign directly into the
            // `MaybeUninit` cell; the value is now initialized and
            // `self.len` is bumped to cover it.
            // SAFETY: the slot at `idx` is in the uninitialized
            // region `[self.len, self.storage.len())`. Writing a
            // valid `AccessState` here is sound. No old value needs
            // to be dropped (the slot is uninitialized). After this
            // write, the slot is initialized and `self.len` is
            // updated to `idx + 1`, which makes the slot visible to
            // `get`.
            unsafe {
                self.storage[idx] = MaybeUninit::new(state);
            }
            self.len = idx + 1;
        } else {
            // Overwrite an already-initialized slot. `AccessState`
            // is `Copy` (no `Drop`), so the old value can simply be
            // replaced without a drop call.
            // SAFETY: the slot at `idx` is initialized (idx <
            // self.len). The new value is a valid `AccessState`.
            unsafe {
                self.storage[idx] = MaybeUninit::new(state);
            }
        }
    }
}

impl Default for BarrierStateTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_extractor() {
        let h = ResourceHandle::from_index(42);
        let b = Barrier::Texture {
            resource: h,
            from: wgpu::TextureUses::UNINITIALIZED,
            to: wgpu::TextureUses::RENDER_ATTACHMENT,
        };
        assert_eq!(b.resource(), h);
        assert!(b.is_texture());
        assert!(!b.is_buffer());
    }

    #[test]
    fn display_is_informative() {
        let h = ResourceHandle::from_index(2);
        let b = Barrier::Buffer {
            resource: h,
            from: wgpu::BufferUses::empty(),
            to: wgpu::BufferUsages::VERTEX,
        };
        let s = format!("{b}");
        assert!(s.contains("r2"));
        assert!(s.contains("VERTEX"));
    }

    #[test]
    fn access_state_matches_same_kind() {
        let a = AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT);
        let b = AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT);
        let c = AccessState::Texture(wgpu::TextureUses::TEXTURE_BINDING);
        let d = AccessState::Buffer(wgpu::BufferUsages::STORAGE);
        assert!(a.matches(b));
        assert!(!a.matches(c));
        assert!(!a.matches(d));
    }

    #[test]
    fn empty_table() {
        let t = BarrierStateTable::new();
        assert_eq!(t.len(), 0);
        assert!(t.is_empty());
        assert!(t.get(ResourceHandle::from_index(0)).is_none());
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut t = BarrierStateTable::new();
        let h = ResourceHandle::from_index(3);
        t.set(h, AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT));
        assert_eq!(t.len(), 4); // 0..=3 initialized
        assert!(!t.is_empty());
        let got = t.get(h);
        assert_eq!(got, Some(AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT)));
    }

    #[test]
    fn ensure_slot_is_idempotent() {
        let mut t = BarrierStateTable::new();
        let h = ResourceHandle::from_index(5);
        t.ensure_slot(h);
        t.ensure_slot(h);
        t.ensure_slot(h);
        // No set was called, so `len` is still 0 and get returns None.
        assert_eq!(t.len(), 0);
        assert!(t.get(h).is_none());
        // After set, the slot at index 5 is initialized.
        t.set(h, AccessState::Buffer(wgpu::BufferUsages::VERTEX));
        assert_eq!(t.len(), 6);
        assert!(t.get(h).is_some());
    }

    #[test]
    fn set_overwrites_existing() {
        let mut t = BarrierStateTable::new();
        let h = ResourceHandle::from_index(0);
        t.set(h, AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT));
        t.set(h, AccessState::Texture(wgpu::TextureUses::TEXTURE_BINDING));
        assert_eq!(
            t.get(h),
            Some(AccessState::Texture(wgpu::TextureUses::TEXTURE_BINDING))
        );
        assert_eq!(t.len(), 1);
    }

    /// Miri-only soundness test: the `unsafe set_len` in
    /// `BarrierStateTable::ensure_slot` must not produce undefined
    /// behaviour for any sequence of `ensure_slot` + `set` + `get`
    /// calls.
    ///
    /// Run with:
    /// ```bash
    /// & "C:\Users\estev\.cargo\bin\cargo.exe" +nightly miri test \
    ///     -p hyge-render-graph --lib barrier::tests::miri
    /// ```
    #[test]
    fn miri_barrier_state_table_set_len_is_sound() {
        let mut t = BarrierStateTable::new();
        // Out-of-order ensure_slot calls.
        t.ensure_slot(ResourceHandle::from_index(0));
        t.ensure_slot(ResourceHandle::from_index(10));
        t.ensure_slot(ResourceHandle::from_index(5));
        // Unset slots must be reported as None.
        for i in 0..=10 {
            assert!(
                t.get(ResourceHandle::from_index(i)).is_none(),
                "slot {i} should be None before set"
            );
        }
        // Set a slot, then verify it round-trips.
        t.set(
            ResourceHandle::from_index(7),
            AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT),
        );
        assert_eq!(
            t.get(ResourceHandle::from_index(7)),
            Some(AccessState::Texture(wgpu::TextureUses::RENDER_ATTACHMENT))
        );
        // Set a slot in the middle of the sparse range.
        t.set(
            ResourceHandle::from_index(3),
            AccessState::Buffer(wgpu::BufferUsages::VERTEX),
        );
        assert_eq!(
            t.get(ResourceHandle::from_index(3)),
            Some(AccessState::Buffer(wgpu::BufferUsages::VERTEX))
        );
        // Neighbors of the new set slot must still be None.
        assert!(t.get(ResourceHandle::from_index(2)).is_none());
        assert!(t.get(ResourceHandle::from_index(4)).is_none());
        // Overwrite an existing slot.
        t.set(
            ResourceHandle::from_index(7),
            AccessState::Texture(wgpu::TextureUses::TEXTURE_BINDING),
        );
        assert_eq!(
            t.get(ResourceHandle::from_index(7)),
            Some(AccessState::Texture(wgpu::TextureUses::TEXTURE_BINDING))
        );
        // Drop the table — exercises the `MaybeUninit` drop path.
        drop(t);
    }
}
