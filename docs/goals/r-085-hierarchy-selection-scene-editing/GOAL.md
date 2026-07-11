# Goal: R-085 Hierarchy, Selection and Persistent Scene Editing

Execute `docs/goals/r-085-hierarchy-selection-scene-editing/PLAN.md`.

The Rust editor service and `hyge-scene::WorldDocument` are the only sources
of truth. Preserve stable scene IDs, reciprocal `Parent`/`Children` links,
backend-owned Shift selection, undo/redo and save/reopen evidence. Do not add
a parallel frontend scene model. Say `implemented but unproven` if real
save/reopen or protocol evidence cannot be captured.
