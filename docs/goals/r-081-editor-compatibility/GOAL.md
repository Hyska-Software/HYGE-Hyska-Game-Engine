# Goal: R-081 Protocol Compatibility and Session Lifecycle

Use Krypton Execution to execute `docs/goals/r-081-editor-compatibility/PLAN.md`.

Preserve the schema as the wire source of truth, negotiate supported protocol
versions before creating a session, retain one canonical session record across
reconnects, reject stale generations, and capture schema/TCP evidence before
marking R-081 complete.
