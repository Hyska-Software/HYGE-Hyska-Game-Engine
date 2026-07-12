# R-101 — Scene Hot-Reload and PersistOnReload Integration

**Intent:** Connect the real scene watcher and ECS reload path to the live
editor session without losing persistent scene state.

**Truth owner:** `hyge-scene` owns scene identity, loading and restoration;
`hyge-editor` owns session dirty/conflict state and protocol publication;
Python/QML owns presentation only.

**Acceptance evidence:** real temporary project, real watcher, real editor
runtime and real TCP protocol proving reload, conflict decisions, persistent
state and refreshed snapshots.

**Execution note:** keep R-101 scoped to scene reload. Crash recovery,
packaging and the integrated workflow remain R-102–R-106.

## Implementation contract

- Reload is transactional: parse/validation failure leaves the previous ECS
  world active.
- `SceneNodeId` is the persistent identity; ECS entity bits are disposable.
- `PersistOnReload` captures reflected components and hierarchy metadata before
  replacement and restores them after a successful reload.
- External edits while the materialized editor document is dirty produce an
  explicit conflict. Supported actions are `reload_discard`, `keep_editor`,
  and `save_then_reload`.
- New protocol messages must be synchronized across Rust, JSON schema and the
  Python client before they are used by the frontend.

## Verification

Run focused scene/editor/Python tests first, then serial workspace tests,
clippy, format and documentation gates. R-101 is not complete without a
target-perspective TCP/fixture trace or screenshot-equivalent evidence.
