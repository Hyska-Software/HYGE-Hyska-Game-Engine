# Goal: R-084 Editor Command Model

Execute the approved R-084 plan. Keep `EditorSessionRuntime` and its live ECS
world as the sole edit/history owner, preserve the versioned IPC boundary, and
do not introduce frontend or duplicate scene state. Completion requires the
mixed undo-five/redo-three integration evidence, stale-revision non-mutation,
redo invalidation, and history continuity across save.
