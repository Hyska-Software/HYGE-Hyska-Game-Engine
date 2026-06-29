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
//!    pattern in the `BarrierStateTable`.
//!    - If `r` was touched by a previous pass in the current frame
//!      and its previous usage `from` differs from the current
//!      usage `to` (resolved from the pass's
//!      [`Pass::texture_usages`](crate::pass::Pass::texture_usages)
//!      / [`Pass::buffer_usages`](crate::pass::Pass::buffer_usages)
//!      hints, falling back to `wgpu::TextureUsages::empty()` /
//!      `wgpu::BufferUsages::empty()` when the pass did not declare
//!      one), emit a `Barrier::Texture { from, to }` (or `Buffer`)
//!      and update the table to `to`.
//!    - If `r` was not touched in the current frame (first touch
//!      of the frame), emit a `Barrier` with
//!      `from = wgpu::TextureUsages::empty()` (or
//!      `wgpu::BufferUsages::empty()`) when the resource is
//!      [`crate::resource::ResourceLifetime::Transient`], OR always emit a
//!      barrier when the resource is
//!      [`crate::resource::ResourceLifetime::Persistent`] — the runtime does not
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
//! # Safety
//!
//! The state table is intentionally safe: sparse handles are stored in
//! `Vec<Option<AccessState>>`, so untouched slots are represented as
//! `None` rather than uninitialized memory.

use crate::resource::ResourceHandle;
use std::fmt;

/// A single resource-usage transition between two passes in the
/// compiled render graph.
///
/// The variants mirror the two kinds of GPU resources: textures and
/// buffers. The `from` field is the prior access pattern (or
/// `wgpu::TextureUsages::empty()` / `wgpu::BufferUsages::empty()`
/// for first-touch barriers).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Barrier {
    /// A texture-usage transition.
    Texture {
        /// The resource the barrier applies to.
        resource: ResourceHandle,
        /// The previous access pattern. Empty for first touch.
        from: wgpu::TextureUsages,
        /// The new access pattern. The pass following this barrier will
        /// read/write with these usages.
        to: wgpu::TextureUsages,
    },
    /// A buffer-usage transition.
    Buffer {
        /// The resource the barrier applies to.
        resource: ResourceHandle,
        /// The previous access pattern. Empty for first touch.
        from: wgpu::BufferUsages,
        /// The new access pattern. The pass following this barrier will
        /// read/write with these usages.
        to: wgpu::BufferUsages,
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
    Texture(wgpu::TextureUsages),
    /// The resource was last touched as a buffer with these usages.
    Buffer(wgpu::BufferUsages),
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
/// touched are `None`, which keeps sparse handles safe and simple.
pub(crate) struct BarrierStateTable {
    /// Backing storage. `None` means the resource has not been touched.
    storage: Vec<Option<AccessState>>,
}

impl BarrierStateTable {
    /// Creates an empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            storage: Vec::new(),
        }
    }

    /// Returns the number of initialized slots.
    #[must_use]
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.storage.iter().filter(|slot| slot.is_some()).count()
    }

    /// Returns true if the table has no initialized slots.
    #[must_use]
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.storage.iter().all(Option::is_none)
    }

    /// Ensures the slot for `handle` exists in the backing storage,
    /// growing the Vec as needed. Idempotent: calling it on a handle
    /// whose slot already exists is a no-op.
    ///
    pub fn ensure_slot(&mut self, handle: ResourceHandle) {
        let idx = handle.index() as usize;
        if self.storage.len() <= idx {
            self.storage.resize_with(idx + 1, || None);
        }
    }

    /// Returns the state previously stored for `handle`, or `None`
    /// if the slot was never written (either because `ensure_slot`
    /// was never called for this handle, or because it was called
    /// but `set` was not).
    #[must_use]
    pub fn get(&self, handle: ResourceHandle) -> Option<AccessState> {
        let idx = handle.index() as usize;
        self.storage.get(idx).copied().flatten()
    }

    /// Stores `state` at `handle`. Initializes the slot if it was
    /// previously uninitialized, and grows the table if necessary.
    pub fn set(&mut self, handle: ResourceHandle, state: AccessState) {
        let idx = handle.index() as usize;
        self.ensure_slot(handle);
        self.storage[idx] = Some(state);
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
            from: wgpu::TextureUsages::empty(),
            to: wgpu::TextureUsages::RENDER_ATTACHMENT,
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
            from: wgpu::BufferUsages::empty(),
            to: wgpu::BufferUsages::VERTEX,
        };
        let s = format!("{b}");
        assert!(s.contains("r2"));
        assert!(s.contains("VERTEX"));
    }

    #[test]
    fn access_state_matches_same_kind() {
        let a = AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT);
        let b = AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT);
        let c = AccessState::Texture(wgpu::TextureUsages::TEXTURE_BINDING);
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
        t.set(
            h,
            AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT),
        );
        assert_eq!(t.len(), 1);
        assert!(!t.is_empty());
        let got = t.get(h);
        assert_eq!(
            got,
            Some(AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT))
        );
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
        // After set, only the slot at index 5 is initialized.
        t.set(h, AccessState::Buffer(wgpu::BufferUsages::VERTEX));
        assert_eq!(t.len(), 1);
        assert!(t.get(h).is_some());
    }

    #[test]
    fn set_overwrites_existing() {
        let mut t = BarrierStateTable::new();
        let h = ResourceHandle::from_index(0);
        t.set(
            h,
            AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT),
        );
        t.set(
            h,
            AccessState::Texture(wgpu::TextureUsages::TEXTURE_BINDING),
        );
        assert_eq!(
            t.get(h),
            Some(AccessState::Texture(wgpu::TextureUsages::TEXTURE_BINDING))
        );
        assert_eq!(t.len(), 1);
    }

    /// Sparse-table soundness test: `ensure_slot` must not make unset
    /// sparse slots appear initialized for any sequence of `ensure_slot`,
    /// `set`, and `get` calls.
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
            AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT),
        );
        assert_eq!(
            t.get(ResourceHandle::from_index(7)),
            Some(AccessState::Texture(wgpu::TextureUsages::RENDER_ATTACHMENT))
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
            AccessState::Texture(wgpu::TextureUsages::TEXTURE_BINDING),
        );
        assert_eq!(
            t.get(ResourceHandle::from_index(7)),
            Some(AccessState::Texture(wgpu::TextureUsages::TEXTURE_BINDING))
        );
        // Drop the table — exercises the `MaybeUninit` drop path.
        drop(t);
    }
}
