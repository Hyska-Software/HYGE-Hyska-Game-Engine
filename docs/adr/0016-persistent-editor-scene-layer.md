# ADR-0016: Persistent Editor Scene Layer

- **Status:** Accepted
- **Date:** 2026-07-11
- **Deciders:** Hyge core team
- **Related:** ADR-0007, ADR-0015

## Context

`.hyge-world` previously stored only prefab instances. That format cannot
represent arbitrary editor operations such as reparenting a prefab child,
duplicating a subtree or destroying an entity without modifying the source
prefab or creating a second editor database.

## Decision

`WorldDocument` gains an optional versioned `SceneEditLayer`. The layer stores
materialized scene nodes, persistent scene IDs, parent IDs, sibling order,
names, reflected component overrides and tombstones. Legacy documents keep
their existing prefab loading path and receive deterministic IDs derived from
the root instance and hierarchy path. Once saved by the editor, the edit
layer becomes the authoritative scene representation while prefab assets stay
unchanged.

The active ECS world remains the editor's mutation truth. Save captures the
scene-managed ECS world into the layer; load reconstructs it through the
canonical `AppTypeRegistry` and reflection APIs. Bevy entity bits remain
process-local transport metadata and are never persisted as scene identity.

## Consequences

- Reparent, duplicate, instantiate, destroy and undo can survive save/reopen.
- Old `.hyge-world` files remain readable without migration tooling.
- The materialized layer is larger than a prefab-instance-only document.
- Unsupported reflected component serialization must fail the save rather than
  silently dropping editor state.
