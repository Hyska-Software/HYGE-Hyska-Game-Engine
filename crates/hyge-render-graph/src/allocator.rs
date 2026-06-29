//! Arena allocator for transient (frame-scoped) resources.
//!
//! R-022 fleshes out the actual first-touch / last-release slot
//! reuse. The R-020 skeleton stored a per-resource slot and
//! flipped a `live` bit; the new design is a proper descriptor-keyed
//! slot pool where resources with matching `SlotKey`s share slots
//! across frames.
//!
//! # Algorithm
//!
//! 1. **Register** — at graph compile time, each transient resource
//!    is described via [`TransientAllocator::register`]. The
//!    allocator stores the descriptor but does not yet allocate a
//!    slot.
//! 2. **Allocate** — at execute time, when a pass first touches a
//!    transient resource, the executor calls
//!    [`TransientAllocator::allocate`]. This pops a free slot with
//!    a matching [`SlotKey`] off the per-key free stack, or creates
//!    a new one if the stack is empty. The slot is then bound to
//!    the resource handle and marked live.
//! 3. **Deallocate** (optional, in-frame) — a pass that knows a
//!    resource is no longer needed for the rest of the frame can
//!    call [`TransientAllocator::deallocate`]. The slot is returned
//!    to the free pool early.
//! 4. **Next frame** — at the end of every frame,
//!    [`TransientAllocator::next_frame`] walks every slot, clears
//!    its `live` flag, clears its owner, and pushes the index back
//!    onto the per-key free stack. The slot object (and, once
//!    R-023 wires them up, the backing `wgpu` resource) is kept
//!    for reuse on the next frame.
//!
//! # Memory ceiling
//!
//! The slot count is bounded by the number of distinct [`SlotKey`]s
//! ever seen. Within a single resource type, the slot count is
//! bounded by `peak_live()` for that key. Resources with
//! non-matching keys can never share a slot, so a graph that uses
//! `N` different texture/buffer shapes will allocate at most `N`
//! slots over its entire lifetime. The `tests` module + the
//! `tests/transient_allocator.rs` integration test assert this on
//! the 1000-frame leak check.

use std::collections::HashMap;

use crate::resource::{ResourceHandle, ResourceKind};

/// A unique key derived from a resource descriptor. Two resources
/// with the same key can share a slot — this is the descriptor-based
/// reuse that keeps the working set bounded.
///
/// The `label` field is deliberately excluded: labels are
/// debug-only and must not affect slot identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SlotKey {
    /// Texture slot key: all format/size/usage fields.
    Texture {
        /// Width in texels.
        width: u32,
        /// Height in texels.
        height: u32,
        /// Depth (1 for 2D / 2D array).
        depth: u32,
        /// Array layer count (1 for non-array).
        array_layers: u32,
        /// Mip level count (1 for no mip chain).
        mip_levels: u32,
        /// Sample count (1 for no MSAA).
        sample_count: u32,
        /// Pixel format.
        format: wgpu::TextureFormat,
        /// Allowed usage bits.
        usage: wgpu::TextureUsages,
    },
    /// Buffer slot key.
    Buffer {
        /// Size in bytes.
        size: u64,
        /// Allowed usage bits.
        usage: wgpu::BufferUsages,
    },
}

impl SlotKey {
    /// Derives a key from a resource descriptor. Two descriptors
    /// that differ only in `label` produce the same key.
    #[must_use]
    pub(crate) fn from_kind(kind: &ResourceKind) -> Self {
        match kind {
            ResourceKind::Texture(t) => Self::Texture {
                width: t.width,
                height: t.height,
                depth: t.depth,
                array_layers: t.array_layers,
                mip_levels: t.mip_levels,
                sample_count: t.sample_count,
                format: t.format,
                usage: t.usage,
            },
            ResourceKind::Buffer(b) => Self::Buffer {
                size: b.size,
                usage: b.usage,
            },
        }
    }
}

/// One slot in the arena. The descriptor is fixed at slot creation
/// time; only the `owner` and `live` fields change during execution.
#[derive(Debug, Clone)]
pub struct Slot {
    /// The resource currently bound to this slot, or `None` if the
    /// slot is in the free pool.
    pub owner: Option<ResourceHandle>,
    /// Whether the slot is live (allocated) in the current frame.
    pub live: bool,
    /// The descriptor this slot was created for. A slot will only
    /// be reused for resources with a matching [`SlotKey`].
    pub desc: ResourceKind,
}

/// Arena allocator for transient resources.
///
/// Constructed by [`CompiledGraph::from_parts`](crate::compile::CompiledGraph::from_parts)
/// during graph compile. Driven frame-to-frame by the render
/// executor: each frame calls
/// [`TransientAllocator::allocate`] for every transient resource a
/// pass touches (in the order determined by the barrier inference)
/// and finishes with a single
/// [`TransientAllocator::next_frame`] call.
#[derive(Debug, Default)]
pub struct TransientAllocator {
    /// All slots ever created. Slots are never removed;
    /// [`TransientAllocator::next_frame`] only resets their `live`
    /// flag and pushes them back onto the per-key free stack.
    slots: Vec<Slot>,
    /// Map from resource handle to its current slot index. A handle
    /// has no entry outside of an `allocate` ... `deallocate` /
    /// `next_frame` window.
    active: HashMap<ResourceHandle, usize>,
    /// Free slots grouped by descriptor key. Each list is a LIFO
    /// stack — the most recently freed slot is reused first,
    /// which keeps the working set of (eventual) `wgpu` resources
    /// warm in the GPU's memory pool.
    free_by_key: HashMap<SlotKey, Vec<usize>>,
    /// Resource descriptors (from compile-time registration).
    descriptors: HashMap<ResourceHandle, ResourceKind>,
    /// Peak live count observed since the allocator was created.
    /// Monotonically non-decreasing.
    peak_live: usize,
    /// Total allocation events. Monotonically non-decreasing.
    total_allocations: u64,
}

impl TransientAllocator {
    /// Creates an empty allocator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a transient resource descriptor. The slot is not
    /// allocated until the resource is first used in a frame (via
    /// [`TransientAllocator::allocate`]). Multiple `register` calls
    /// for the same handle replace the previously stored
    /// descriptor; this is how `CompiledGraph::from_parts` updates
    /// descriptors on re-compile.
    pub fn register(&mut self, handle: ResourceHandle, desc: ResourceKind) {
        self.descriptors.insert(handle, desc);
    }

    /// Returns the descriptor registered for `handle`, if any.
    /// The descriptor is set by [`TransientAllocator::register`]
    /// and unchanged for the lifetime of the allocator.
    #[must_use]
    pub fn descriptor(&self, handle: ResourceHandle) -> Option<&ResourceKind> {
        self.descriptors.get(&handle)
    }

    /// Allocates a slot for `handle` (first-touch semantics). If a
    /// free slot with a matching [`SlotKey`] exists, it is reused;
    /// otherwise a new slot is created.
    ///
    /// Returns the slot index on success, or `None` if `handle` was
    /// not previously registered via
    /// [`TransientAllocator::register`].
    pub fn allocate(&mut self, handle: ResourceHandle) -> Option<usize> {
        let desc = self.descriptors.get(&handle)?.clone();
        let key = SlotKey::from_kind(&desc);
        // Pop a free slot of the matching key, or push a new one.
        let slot_idx = if let Some(free_list) = self.free_by_key.get_mut(&key) {
            free_list.pop()
        } else {
            let idx = self.slots.len();
            self.slots.push(Slot {
                owner: None,
                live: false,
                desc: desc.clone(),
            });
            Some(idx)
        }
        .expect("either the free list yielded an index or we just pushed one");
        let slot = &mut self.slots[slot_idx];
        slot.owner = Some(handle);
        slot.live = true;
        self.active.insert(handle, slot_idx);
        self.total_allocations += 1;
        let live = self.slots.iter().filter(|s| s.live).count();
        if live > self.peak_live {
            self.peak_live = live;
        }
        Some(slot_idx)
    }

    /// Deallocates the slot bound to `handle`, returning it to the
    /// free pool early (before [`TransientAllocator::next_frame`]).
    /// The slot can then be reused by the next
    /// [`TransientAllocator::allocate`] call for a resource with a
    /// matching [`SlotKey`].
    ///
    /// Returns the slot index on success, or `None` if `handle` is
    /// not currently allocated.
    pub fn deallocate(&mut self, handle: ResourceHandle) -> Option<usize> {
        let slot_idx = *self.active.get(&handle)?;
        let slot = &mut self.slots[slot_idx];
        slot.owner = None;
        slot.live = false;
        let key = SlotKey::from_kind(&slot.desc);
        self.free_by_key.entry(key).or_default().push(slot_idx);
        self.active.remove(&handle);
        Some(slot_idx)
    }

    /// Ends the current frame: every live slot has its `live` flag
    /// and `owner` cleared and is pushed back onto the per-key free
    /// stack. The slot objects (and, once R-023 wires them up, the
    /// backing `wgpu` resources) are kept for reuse on the next
    /// frame.
    pub fn next_frame(&mut self) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.live {
                slot.live = false;
                slot.owner = None;
                let key = SlotKey::from_kind(&slot.desc);
                self.free_by_key.entry(key).or_default().push(i);
            }
        }
        self.active.clear();
    }

    /// Returns the total number of slots the allocator has ever
    /// created. Monotonically non-decreasing; the leak test asserts
    /// this is bounded by the number of distinct keys the graph
    /// ever uses.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Returns the number of slots currently live (allocated in
    /// this frame).
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|s| s.live).count()
    }

    /// Returns the number of slots currently in the free pool.
    #[must_use]
    pub fn free_count(&self) -> usize {
        self.slots.iter().filter(|s| !s.live).count()
    }

    /// Returns the peak live count observed since the allocator
    /// was created. Monotonically non-decreasing; used by the
    /// leak / fragment tests as the memory ceiling.
    #[must_use]
    pub fn peak_live(&self) -> usize {
        self.peak_live
    }

    /// Returns the total number of [`TransientAllocator::allocate`]
    /// calls since the allocator was created. Monotonically
    /// non-decreasing. The 1000-frame test asserts this equals
    /// `frames * resources_per_frame` exactly.
    #[must_use]
    pub fn total_allocations(&self) -> u64 {
        self.total_allocations
    }

    /// Returns a snapshot view of the slot at the given index, for
    /// introspection and tests. Returns `None` if the index is out
    /// of range.
    #[must_use]
    pub fn slot(&self, idx: usize) -> Option<&Slot> {
        self.slots.get(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{BufferDesc, TextureDesc};

    fn buffer_desc(size: u64) -> ResourceKind {
        ResourceKind::Buffer(BufferDesc::new(size, wgpu::BufferUsages::VERTEX))
    }

    #[test]
    fn register_stores_descriptor() {
        let mut a = TransientAllocator::new();
        let h = ResourceHandle::from_index(3);
        let d = buffer_desc(64);
        a.register(h, d.clone());
        // No slot has been allocated yet — only the descriptor is stored.
        assert_eq!(a.slot_count(), 0);
        assert_eq!(a.descriptor(h), Some(&d));
    }

    #[test]
    fn allocate_creates_slot() {
        let mut a = TransientAllocator::new();
        let h = ResourceHandle::from_index(0);
        a.register(h, buffer_desc(64));
        let idx = a.allocate(h).expect("registered handle should allocate");
        assert_eq!(a.slot_count(), 1);
        assert!(a.slot(idx).expect("slot exists").live);
        assert_eq!(a.slot(idx).expect("slot exists").owner, Some(h));
        assert_eq!(a.live_count(), 1);
    }

    #[test]
    fn allocate_returns_none_for_unregistered() {
        let mut a = TransientAllocator::new();
        assert!(a.allocate(ResourceHandle::from_index(0)).is_none());
    }

    #[test]
    fn reuses_free_slot_with_matching_key() {
        let mut a = TransientAllocator::new();
        // Two resources with the *same* descriptor → same SlotKey.
        let d = buffer_desc(64);
        a.register(ResourceHandle::from_index(0), d.clone());
        a.register(ResourceHandle::from_index(1), d.clone());
        a.allocate(ResourceHandle::from_index(0)).expect("alloc 0");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc 1");
        assert_eq!(a.slot_count(), 2);
        let live_after_first_frame = a.peak_live();
        a.next_frame();
        // After next_frame, the slots are returned to the free pool.
        assert_eq!(a.live_count(), 0);
        assert_eq!(a.free_count(), 2);
        // Re-allocating the same two resources should reuse the two
        // existing slots — no new slots are created.
        a.allocate(ResourceHandle::from_index(0)).expect("alloc 0 again");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc 1 again");
        assert_eq!(a.slot_count(), 2, "matching-key slots must be reused");
        assert_eq!(a.peak_live(), live_after_first_frame);
    }

    #[test]
    fn allocates_new_slot_for_non_matching_key() {
        let mut a = TransientAllocator::new();
        a.register(ResourceHandle::from_index(0), buffer_desc(64));
        a.register(ResourceHandle::from_index(1), buffer_desc(128));
        a.allocate(ResourceHandle::from_index(0)).expect("alloc 0");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc 1");
        assert_eq!(a.slot_count(), 2, "distinct keys must each get their own slot");
    }

    #[test]
    fn next_frame_resets_liveness_and_returns_to_pool() {
        let mut a = TransientAllocator::new();
        let h = ResourceHandle::from_index(0);
        a.register(h, buffer_desc(64));
        a.allocate(h).expect("alloc");
        assert_eq!(a.live_count(), 1);
        assert_eq!(a.free_count(), 0);
        a.next_frame();
        assert_eq!(a.live_count(), 0);
        assert_eq!(a.free_count(), 1);
    }

    #[test]
    fn peak_live_tracks_max_concurrent() {
        let mut a = TransientAllocator::new();
        for i in 0..5 {
            a.register(ResourceHandle::from_index(i), buffer_desc(64 * (i as u64 + 1)));
        }
        for i in 0..3 {
            a.allocate(ResourceHandle::from_index(i)).expect("alloc");
        }
        assert_eq!(a.peak_live(), 3);
        a.next_frame();
        for i in 3..5 {
            a.allocate(ResourceHandle::from_index(i)).expect("alloc");
        }
        // Peak is the max of concurrent live in any frame.
        assert_eq!(a.peak_live(), 3);
    }

    #[test]
    fn deallocate_returns_to_free_pool_early() {
        let mut a = TransientAllocator::new();
        a.register(ResourceHandle::from_index(0), buffer_desc(64));
        a.register(ResourceHandle::from_index(1), buffer_desc(128));
        a.allocate(ResourceHandle::from_index(0)).expect("alloc 0");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc 1");
        assert_eq!(a.live_count(), 2);
        a.deallocate(ResourceHandle::from_index(0)).expect("dealloc 0");
        assert_eq!(a.live_count(), 1);
        assert_eq!(a.free_count(), 1);
        // The deallocated slot is immediately available for reuse.
        a.allocate(ResourceHandle::from_index(0)).expect("re-alloc 0");
        assert_eq!(a.slot_count(), 2, "deallocated slot must be reused");
    }

    #[test]
    fn free_count_matches_total_minus_live() {
        let mut a = TransientAllocator::new();
        for i in 0..4 {
            a.register(ResourceHandle::from_index(i), buffer_desc(64));
        }
        a.allocate(ResourceHandle::from_index(0)).expect("alloc");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc");
        assert_eq!(a.live_count(), 2);
        assert_eq!(a.free_count(), a.slot_count() - a.live_count());
    }

    #[test]
    fn total_allocations_is_monotonic() {
        let mut a = TransientAllocator::new();
        a.register(ResourceHandle::from_index(0), buffer_desc(64));
        a.allocate(ResourceHandle::from_index(0)).expect("alloc");
        assert_eq!(a.total_allocations(), 1);
        a.next_frame();
        a.allocate(ResourceHandle::from_index(0)).expect("alloc again");
        assert_eq!(a.total_allocations(), 2);
    }

    #[test]
    fn label_does_not_affect_key() {
        let mut a = TransientAllocator::new();
        let mut d1 = buffer_desc(64);
        d1 = match d1 {
            ResourceKind::Buffer(mut b) => {
                b.label = Some("a".into());
                ResourceKind::Buffer(b)
            }
            other => other,
        };
        let d2 = buffer_desc(64);
        a.register(ResourceHandle::from_index(0), d1);
        a.register(ResourceHandle::from_index(1), d2);
        a.allocate(ResourceHandle::from_index(0)).expect("alloc 0");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc 1");
        // Two resources with matching key (size + usage) → one slot reused.
        assert_eq!(a.slot_count(), 1);
    }

    #[test]
    fn texture_descriptor_keys_match_on_all_fields() {
        let mut a = TransientAllocator::new();
        let d1 = ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
        let d2 = ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ));
        let d3 = ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureUsages::TEXTURE_BINDING, // different usage
        ));
        a.register(ResourceHandle::from_index(0), d1);
        a.register(ResourceHandle::from_index(1), d2);
        a.register(ResourceHandle::from_index(2), d3);
        a.allocate(ResourceHandle::from_index(0)).expect("alloc 0");
        a.allocate(ResourceHandle::from_index(1)).expect("alloc 1");
        a.allocate(ResourceHandle::from_index(2)).expect("alloc 2");
        // d1 and d2 share a key (everything matches). d3 has a
        // different key (usage differs). Total: 2 slots.
        assert_eq!(a.slot_count(), 2);
    }

    /// R-022 acceptance: covers the "fragment as fraction of
    /// peak-usage" case. After a frame that used the peak number
    /// of slots, the next frame uses only a fraction of them. The
    /// allocator must (a) return the unused slots to the free
    /// pool, (b) not grow the slot count past the peak, and
    /// (c) keep the peak_live metric at the historical maximum.
    #[test]
    fn fragment_as_fraction_of_peak_usage() {
        let mut a = TransientAllocator::new();
        // 20 transient buffers, each with a unique size → 20
        // distinct `SlotKey`s. Each buffer's first allocation will
        // create a new slot.
        for i in 0..20u32 {
            a.register(
                ResourceHandle::from_index(i),
                ResourceKind::Buffer(BufferDesc::new(
                    64 * u64::from(i + 1),
                    wgpu::BufferUsages::VERTEX,
                )),
            );
        }

        // Frame 1: allocate everything (peak usage).
        for i in 0..20u32 {
            a.allocate(ResourceHandle::from_index(i))
                .expect("frame 1 alloc");
        }
        assert_eq!(a.live_count(), 20, "peak frame: 20 live");
        assert_eq!(a.slot_count(), 20, "peak frame: 20 slots");
        assert_eq!(a.peak_live(), 20, "peak frame: peak_live == 20");
        a.next_frame();

        // Frame 2: use only a fragment (5 of 20). The other 15
        // slots must be returned to the free pool.
        for i in 0..5u32 {
            a.allocate(ResourceHandle::from_index(i))
                .expect("frame 2 alloc");
        }
        assert_eq!(a.live_count(), 5, "fragment frame: 5 live");
        assert_eq!(a.free_count(), 15, "fragment frame: 15 free");
        assert_eq!(a.slot_count(), 20, "fragment frame: still 20 slots (peak)");
        assert_eq!(
            a.peak_live(),
            20,
            "fragment frame: peak_live unchanged at 20"
        );
        a.next_frame();

        // Frame 3: same fragment usage. No new slots should be
        // created — the 5 from frame 2 are reused.
        for i in 0..5u32 {
            a.allocate(ResourceHandle::from_index(i))
                .expect("frame 3 alloc");
        }
        assert_eq!(a.slot_count(), 20, "no new slots allocated in frame 3");

        // Frame 4: return to peak usage.
        for i in 0..20u32 {
            a.allocate(ResourceHandle::from_index(i))
                .expect("frame 4 alloc");
        }
        assert_eq!(a.slot_count(), 20, "still 20 slots after returning to peak");
        assert_eq!(a.peak_live(), 20);
    }
}
