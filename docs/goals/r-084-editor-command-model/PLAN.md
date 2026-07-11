# R-084 — Editor Command Model and Transactional History

R-084 is implemented by `hyge-editor::commands` and `hyge-editor::history`.
The live ECS world owned by `EditorSessionRuntime` is the only mutable scene
truth. Commands validate the snapshot revision before mutation, capture their
pre-state, and implement both `apply` and `revert` with structured failures.

Supported commands are component edit/add/remove, reparent, prefab
instantiation, subtree duplication and subtree destruction. Hierarchy links
are captured separately from reflected component values because `Parent` and
`Children` contain opaque ECS entity IDs.

The authenticated server dispatches command messages and exposes revision,
affected-entity, remapping, selection and history operation metadata in
`command_completed`. `undo` and `redo` preserve failed-stack state, new edits
clear redo, and scene save leaves history intact.

Evidence is provided by `r084_commands.rs`, `r084_history.rs`, protocol schema
validation, focused crate tests, and the workspace quality gates.
