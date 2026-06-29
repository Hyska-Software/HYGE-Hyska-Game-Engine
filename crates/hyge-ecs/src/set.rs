//! Cross-crate system ordering sets.
//!
//! Each enum is a [`SystemSet`] that groups related systems within a
//! schedule. Systems in the same set run in parallel (subject to
//! conflict detection); ordering *between* sets is declared in
//! `hyge-app`'s schedule configuration via `.before()` / `.after()` /
//! `.chain()`.
//!
//! See `docs/architecture.md` §6.2 and §7.1 for the intended ordering.

use bevy_ecs::schedule::SystemSet;

/// Transform propagation and sync.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum TransformSet {
    /// Propagate parent transforms to children (hierarchy walk).
    Propagate,
    /// Sync local `Transform` to `GlobalTransform`.
    Sync,
    /// Flush transform-related commands before the render extract.
    Flush,
}

/// Input capture and translation.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum InputSet {
    /// Capture raw `DeviceEvent`s from the window layer.
    Capture,
    /// Translate device events into `ActionMap` state.
    Translate,
    /// Clear per-frame edge-detection state (`just_pressed`, etc.).
    Flush,
}

/// Asset loading, GPU upload, and hot-reload.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum AssetSet {
    /// Load pending assets from disk.
    Load,
    /// Upload loaded asset data to GPU (bindless table).
    Upload,
    /// Process hot-reload events from the file watcher.
    HotReload,
}

/// Physics simulation.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum PhysicsSet {
    /// Sync ECS `Transform` to physics body transforms.
    Sync,
    /// Step the physics simulation (rapier3d).
    Step,
    /// Write physics transforms back to ECS `Transform`.
    Writeback,
    /// Emit collision / contact events.
    Events,
}

/// Audio update.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum AudioSet {
    /// Update listener and emitter positions.
    Update,
    /// Process `PlaySound` / `StopSound` events.
    Events,
}

/// Script (Lua) lifecycle.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum ScriptSet {
    /// Load or reload scripts from disk.
    Load,
    /// Run `on_update(dt)` for each script entity.
    Update,
    /// Process `ScriptError` events.
    Events,
}

/// Editor panels and commands.
#[derive(SystemSet, Hash, PartialEq, Eq, Clone, Debug)]
pub enum EditorSet {
    /// Render editor UI (egui).
    UI,
    /// Update selection state.
    Selection,
    /// Process undo/redo commands.
    Command,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_set_variants_differ() {
        assert_ne!(TransformSet::Propagate, TransformSet::Sync);
        assert_ne!(TransformSet::Sync, TransformSet::Flush);
        assert_ne!(TransformSet::Propagate, TransformSet::Flush);
    }

    #[test]
    fn input_set_variants_differ() {
        assert_ne!(InputSet::Capture, InputSet::Translate);
        assert_ne!(InputSet::Translate, InputSet::Flush);
    }

    #[test]
    fn physics_set_variants_differ() {
        assert_ne!(PhysicsSet::Sync, PhysicsSet::Step);
        assert_ne!(PhysicsSet::Step, PhysicsSet::Writeback);
        assert_ne!(PhysicsSet::Writeback, PhysicsSet::Events);
    }

    #[test]
    fn all_sets_clone_and_eq() {
        let a = AudioSet::Update;
        let b = a.clone();
        assert_eq!(a, b);

        let c = ScriptSet::Load;
        let d = c.clone();
        assert_eq!(c, d);

        let e = EditorSet::UI;
        let f = e.clone();
        assert_eq!(e, f);
    }

    #[test]
    fn sets_from_different_enums_never_compare_equal() {
        // This is a compile-time property (different types), but we
        // document it here: TransformSet::Sync and PhysicsSet::Sync are
        // different types and cannot be compared with ==.
        assert_ne!(TransformSet::Sync, TransformSet::Sync); // same, should be eq
        // The following would not compile (different types):
        // assert_ne!(TransformSet::Sync, PhysicsSet::Sync);
    }
}
