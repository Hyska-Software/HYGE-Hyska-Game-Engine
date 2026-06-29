//! Integration tests for the [`TransientAllocator`] (R-022).
//!
//! Covers the two acceptance bullets that require *integration*
//! rather than unit-level coverage:
//! 1. **No leaks between frames** — a 1000-frame loop asserts the
//!    slot count never exceeds the registered resource count and
//!    `total_allocations` equals `frames × resources` exactly.
//! 2. **Allocator reuses free slots when sizes match** — a
//!    same-descriptor pool across 3 frames proves the slot count
//!    is bounded by the number of *distinct* descriptors, not by
//!    the number of `allocate` calls.
//!
//! The fragment-as-fraction-of-peak case is covered in
//! `src/allocator.rs::tests::fragment_as_fraction_of_peak_usage`.
//!
//! Run with:
//! ```bash
//! & "C:\Users\estev\.cargo\bin\cargo.exe" test -p hyge-render-graph \
//!     --test transient_allocator
//! ```

use hyge_render_graph::prelude::*;

const FRAMES: u64 = 1000;
const RESOURCES_PER_FRAME: u32 = 10;

fn build_n_buffer_handles(n: u32) -> Vec<(ResourceHandle, ResourceKind)> {
    (0..n)
        .map(|i| {
            let handle = ResourceHandle::from_index(i);
            let desc = ResourceKind::Buffer(BufferDesc::new(
                1024 * u64::from(i + 1),
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
            ));
            (handle, desc)
        })
        .collect()
}

#[test]
fn thousand_frames_no_leak_between_frames() {
    let handles = build_n_buffer_handles(RESOURCES_PER_FRAME);
    let mut a = TransientAllocator::new();
    for (h, d) in &handles {
        a.register(*h, d.clone());
    }

    // Run FRAMES frames, allocating every transient resource on
    // every frame. The slot count must be bounded by the number
    // of distinct descriptors — never grow past the registered
    // resource count.
    for _ in 0..FRAMES {
        for (h, _) in &handles {
            a.allocate(*h).expect("registered handle must allocate");
        }
        assert_eq!(
            a.live_count(),
            RESOURCES_PER_FRAME as usize,
            "every frame must have every resource live"
        );
        a.next_frame();
        assert_eq!(a.live_count(), 0, "next_frame must clear liveness");
    }

    // Memory ceiling: no leak. The total slot count is the
    // number of distinct `SlotKey`s ever seen. Every handle has a
    // unique size, so there are RESOURCES_PER_FRAME distinct keys.
    assert_eq!(
        a.slot_count(),
        RESOURCES_PER_FRAME as usize,
        "slot count must not grow across frames"
    );
    // Peak live: the maximum number of slots live at any point.
    // In this test every frame has every resource live, so peak ==
    // RESOURCES_PER_FRAME.
    assert_eq!(
        a.peak_live(),
        RESOURCES_PER_FRAME as usize,
        "peak_live equals the per-frame resource count"
    );
    // Total allocation events: exactly `frames × resources`.
    // If a slot were created more than once (e.g. the reuse path
    // accidentally allocated instead of popping the free stack),
    // this would be higher.
    assert_eq!(
        a.total_allocations(),
        u64::from(RESOURCES_PER_FRAME) * FRAMES,
        "total_allocations must equal frames × resources"
    );
}

#[test]
fn reuses_free_slots_when_sizes_match() {
    let mut a = TransientAllocator::new();
    // Two handles with the *same* descriptor → same SlotKey → the
    // allocator must reuse free slots across frames, while keeping
    // separate slots when both resources are live concurrently.
    let shared_desc = ResourceKind::Buffer(BufferDesc::new(256, wgpu::BufferUsages::VERTEX));
    let h0 = ResourceHandle::from_index(0);
    let h1 = ResourceHandle::from_index(1);
    a.register(h0, shared_desc.clone());
    a.register(h1, shared_desc.clone());

    // Frame 1: both live.
    a.allocate(h0).expect("alloc 0");
    a.allocate(h1).expect("alloc 1");
    assert_eq!(
        a.slot_count(),
        2,
        "concurrently-live resources need distinct slots"
    );
    a.next_frame();

    // Frame 2: both live again — should reuse the same slot.
    a.allocate(h0).expect("alloc 0 again");
    a.allocate(h1).expect("alloc 1 again");
    assert_eq!(
        a.slot_count(),
        2,
        "no new slots after reusing the free pool"
    );
    assert_eq!(a.peak_live(), 2, "peak still 2 (both live concurrently)");

    a.next_frame();

    // Frame 3: same. total_allocations should be 6 (2 per frame ×
    // 3 frames), and slot_count should still be 2.
    a.allocate(h0).expect("alloc 0 frame 3");
    a.allocate(h1).expect("alloc 1 frame 3");
    assert_eq!(a.slot_count(), 2, "still two slots after 3 frames");
    assert_eq!(a.total_allocations(), 6, "6 allocations across 3 frames");
}

#[test]
fn distinct_keys_keep_separate_slots_across_many_frames() {
    let mut a = TransientAllocator::new();
    let handles = build_n_buffer_handles(8);
    for (h, d) in &handles {
        a.register(*h, d.clone());
    }

    for _ in 0..200 {
        for (h, _) in &handles {
            a.allocate(*h).expect("alloc");
        }
        a.next_frame();
    }

    // 8 distinct sizes → 8 distinct SlotKeys → 8 slots.
    assert_eq!(a.slot_count(), 8);
    assert_eq!(a.peak_live(), 8);
    assert_eq!(a.total_allocations(), 8 * 200);
}

#[test]
fn next_frame_pops_all_slots_to_free_pool() {
    let mut a = TransientAllocator::new();
    let handles = build_n_buffer_handles(5);
    for (h, d) in &handles {
        a.register(*h, d.clone());
    }
    for (h, _) in &handles {
        a.allocate(*h).expect("alloc");
    }
    assert_eq!(a.live_count(), 5);
    assert_eq!(a.free_count(), 0);
    a.next_frame();
    assert_eq!(a.live_count(), 0);
    assert_eq!(a.free_count(), 5, "all 5 slots should be free");
}
