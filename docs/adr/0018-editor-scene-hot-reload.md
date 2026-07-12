# ADR-0018: Editor Scene Hot-Reload Ownership and Conflict Policy

**Status:** Accepted  
**Date:** 2026-07-12  
**Scope:** `hyge-scene`, `hyge-editor`, editor protocol and PySide6 frontend

## Context

The editor loads a real `.hyge-world` into ECS, but external scene changes
were not connected to the live session. Full unload/reinstantiation could
discard `PersistOnReload` state and overwrite unsaved editor work.

## Decision

- `EditorSessionRuntime` owns the live `FileWatcher` and installs its
  `ReloadQueue` into the engine world. The frontend never owns ECS truth.
- `SceneNodeId`, not Bevy entity bits, is the identity across reloads.
  `PersistOnReload` entities are captured through reflection and restored only
  after the new scene has been validated and instantiated.
- Decode and hydration failures leave the previous `LoadedSceneState` active.
- Dirty local edits produce `scene_reload_conflict`; the client explicitly
  chooses `reload_discard`, `keep_editor`, or `save_then_reload`.
- Successful reloads publish `scene_reloaded`; the Qt client then requests a
  fresh authoritative snapshot.

## Consequences

File changes are observed without restarting the service and persistent state
survives the reload boundary. Reload is serialized with editor commands, and
the current implementation refreshes the prefab library before reapplying the
active scene through the production loader.
