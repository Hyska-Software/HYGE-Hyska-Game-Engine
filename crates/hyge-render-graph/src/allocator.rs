//! Arena allocator for transient (frame-scoped) resources.
//!
//! The skeleton (R-020) defines the data structure and the
//! `next_frame` reset hook. R-022 fleshes out the actual
//! first-touch / last-release slot reuse and the integration with
//! `wgpu::Device`.
//!
//! # Design
//!
//! - One [`TransientAllocator`] per [`CompiledGraph`](crate::compile::CompiledGraph).
//! - Each transient resource declares the descriptor it will be
//!   created from; the allocator decides which concrete `wgpu`
//!   object backs it on first use of the frame.
//! - At the end of each frame, [`TransientAllocator::next_frame`]
//!   drops all currently-allocated slots and resets the free list,
//!   so memory cannot leak between frames.
//!
//! The current implementation keeps the per-resource descriptor and
//! the slot id, but does not yet pool by descriptor (R-022 work).

use crate::resource::{ResourceHandle, ResourceKind};

/// Tracks one transient resource slot in the arena.
#[derive(Debug)]
struct Slot {
    /// Stable handle identifying the resource.
    handle: ResourceHandle,
    /// Descriptor the slot was created with.
    desc: ResourceKind,
    /// True if the slot is currently holding a live `wgpu` resource.
    /// Cleared by [`TransientAllocator::next_frame`].
    live: bool,
}

/// Arena allocator for transient resources.
///
/// Constructed by [`CompiledGraph`](crate::compile::CompiledGraph)
/// and advanced frame-to-frame by
/// [`TransientAllocator::next_frame`]. R-022 will replace the
/// placeholder bookkeeping with a proper first-fit slot pool that
/// reuses slots of matching descriptors across frames.
#[derive(Debug, Default)]
pub struct TransientAllocator {
    slots: Vec<Slot>,
}

impl TransientAllocator {
    /// Creates an empty allocator.
    #[must_use]
    pub fn new() -> Self {
        Self { slots: Vec::new() }
    }

    /// Registers a transient resource slot with its descriptor.
    /// Called by [`CompiledGraph`](crate::compile::CompiledGraph)
    /// during compile.
    pub(crate) fn register(&mut self, handle: ResourceHandle, desc: ResourceKind) {
        self.slots.push(Slot { handle, desc, live: false });
    }

    /// Returns the descriptor registered for `handle`, if any.
    #[must_use]
    pub fn descriptor(&self, handle: ResourceHandle) -> Option<&ResourceKind> {
        self.slots.iter().find(|s| s.handle == handle).map(|s| &s.desc)
    }

    /// Returns the number of registered slots (live or not).
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Returns the number of live slots in the current frame.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.slots.iter().filter(|s| s.live).count()
    }

    /// Marks a slot as live. The actual wgpu object is owned by the
    /// [`ResourceTable`](crate::pass::ResourceTable); the allocator
    /// only tracks the lifetime bit.
    pub(crate) fn mark_live(&mut self, handle: ResourceHandle) {
        if let Some(slot) = self.slots.iter_mut().find(|s| s.handle == handle) {
            slot.live = true;
        }
    }

    /// Ends the current frame: marks every slot as no longer live.
    /// The next frame starts from an empty pool.
    pub fn next_frame(&mut self) {
        for slot in &mut self.slots {
            slot.live = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{BufferDesc, TextureDesc};

    #[test]
    fn register_and_query() {
        let mut a = TransientAllocator::new();
        let h = ResourceHandle::from_index(3);
        let d = ResourceKind::Texture(TextureDesc::new_2d(
            1,
            1,
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        ));
        a.register(h, d.clone());
        assert_eq!(a.slot_count(), 1);
        assert_eq!(a.live_count(), 0);
        assert_eq!(a.descriptor(h), Some(&d));
    }

    #[test]
    fn next_frame_resets_liveness() {
        let mut a = TransientAllocator::new();
        let h = ResourceHandle::from_index(0);
        a.register(
            h,
            ResourceKind::Buffer(BufferDesc::new(64, wgpu::BufferUsages::VERTEX)),
        );
        a.mark_live(h);
        assert_eq!(a.live_count(), 1);
        a.next_frame();
        assert_eq!(a.live_count(), 0);
    }

    #[test]
    fn empty_allocator_is_sane() {
        let a = TransientAllocator::new();
        assert_eq!(a.slot_count(), 0);
        assert_eq!(a.live_count(), 0);
        assert!(a.descriptor(ResourceHandle::from_index(0)).is_none());
    }
}
