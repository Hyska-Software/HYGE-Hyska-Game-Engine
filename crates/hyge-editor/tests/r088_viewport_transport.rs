//! R-088 producer/consumer and input bridge evidence.

use hyge_editor::{InputBridge, ViewportInputBatch, ViewportRing};

#[test]
fn producer_consumer_stress_preserves_headers_and_shutdown_safety() {
    let mut ring = ViewportRing::new(11);
    let mut first = None;
    for id in 0..10_000_u64 {
        let pixels = vec![(id & 0xff) as u8; 16 * 16 * 4];
        let header = ring.publish(16, 16, id, id + 1, &pixels).expect("publish");
        assert_eq!(header.frame_id, id + 1);
        assert_eq!(ring.consume(&header), Some(pixels));
        first.get_or_insert(header);
    }
    assert!(ring.consume(&first.expect("first frame")).is_none());
    ring.close();
    assert!(ring.publish(16, 16, 1, 1, &[0; 16 * 16 * 4]).is_err());
}

#[test]
fn revisioned_mouse_keyboard_and_camera_batches_reject_stale_input() {
    let mut bridge = InputBridge::default();
    let batch = ViewportInputBatch {
        generation: 3,
        expected_input_revision: 0,
        input_revision: 1,
        events: Vec::new(),
    };
    assert!(bridge.accept(&batch, 3).is_ok());
    assert_eq!(bridge.accept(&batch, 3), Err("stale_input_revision"));
    assert_eq!(
        bridge.accept(
            &ViewportInputBatch {
                generation: 4,
                expected_input_revision: 1,
                input_revision: 2,
                events: Vec::new(),
            },
            3,
        ),
        Err("stale_input_revision")
    );
}
