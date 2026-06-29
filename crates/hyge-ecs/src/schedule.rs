//! Schedule labels for Hyge's fixed set of per-frame schedules.
//!
//! The `App` (in `hyge-app`) registers one
//! [`bevy_ecs::schedule::Schedule`] per [`Label`] variant and runs them in
//! the order declared in the enum, each frame. The ordering is a hard
//! contract: systems in `First` always run before systems in `PreUpdate`,
//! and so on.
//!
//! See `docs/architecture.md` §7.1 for the runtime schedule ordering.

use bevy_ecs::schedule::ScheduleLabel;

/// The canonical set of Hyge schedule labels, declared in execution order.
///
/// Each variant corresponds to one [`bevy_ecs::schedule::Schedule`] that
/// the `App` runs per frame. The enum is declared in the order the
/// schedules execute; this is also the order used by the `Display` impl
/// for diagnostic output.
#[derive(ScheduleLabel, Hash, PartialEq, Eq, Clone, Debug)]
pub enum Label {
    /// Start of frame. Input edge ingestion, asset hot-reload
    /// notifications, time delta computation.
    First,

    /// Before the fixed timestep. Accumulator bookkeeping, pre-physics
    /// setup.
    PreUpdate,

    /// Fixed-timestep schedule. Runs N times per frame (default 60 Hz,
    /// max 5 substeps) to consume the accumulator. Physics step,
    /// deterministic gameplay logic.
    FixedUpdate,

    /// Variable-timestep schedule. Runs once per frame. AI, animation,
    /// scripting, scene updates, script-driven logic.
    Update,

    /// Produces a `FrameSnapshot` from the `World`. Lock-free via dirty
    /// archetype snapshot. Runs on the main thread, just before handing
    /// the snapshot to the render thread.
    RenderExtract,

    /// Consumes a `FrameSnapshot`, builds the render graph, records
    /// commands, and presents. Runs on the dedicated render thread
    /// (ADR-0005).
    Render,

    /// End of frame. Frame present ack, profiler flush, async asset
    /// upload tail.
    Last,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_uniqueness() {
        // Every pair of distinct variants must be unequal. This is a
        // smoke test that the derive produced a sensible Eq impl.
        let all = [
            Label::First,
            Label::PreUpdate,
            Label::FixedUpdate,
            Label::Update,
            Label::RenderExtract,
            Label::Render,
            Label::Last,
        ];
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b, "same index should be equal");
                } else {
                    assert_ne!(a, b, "different indices should differ");
                }
            }
        }
    }

    #[test]
    fn label_clone_is_equal() {
        let original = Label::Update;
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn label_debug_is_informative() {
        let s = format!("{:?}", Label::RenderExtract);
        assert!(
            s.contains("RenderExtract"),
            "debug should contain the variant name: {s}"
        );
    }
}
