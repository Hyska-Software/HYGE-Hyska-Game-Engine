# Goal: R-082 Editor Project and Session Lifecycle

Use Krypton Execution to execute
`docs/goals/r-082-editor-project-session-lifecycle/PLAN.md`.

Core rules:

- Treat `PLAN.md` as the source plan and `hyge-editor` as the lifecycle truth
  owner.
- Route project/scene operations through the real `hyge-app`/
  `hyge-scene` loaders; do not add metadata-only or fake ECS paths.
- Preserve one session identity, one engine world and one project lock across
  reconnect; reject stale generations.
- Make save atomic and report a revision only after durable replacement.
- Converge protocol shutdown, process errors and drop cleanup on one
  idempotent teardown path that closes the frontend child and releases locks.
- Capture real loader, filesystem, TCP and subprocess evidence before marking
  R-082 complete. Say “implemented but unproven” if any acceptance evidence
  cannot be captured.
