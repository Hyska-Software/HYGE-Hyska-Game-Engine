# ADR-0019: Editor Recovery, Generation Fencing and Shutdown

**Status:** Accepted  
**Date:** 2026-07-12  
**Scope:** `hyge-editor`, `hyge-editor-protocol`, `hyge-editor-shm`, PySide6/QML

## Decision

- Rust owns session generation, mutation serialization, runtime teardown and
  project/scene recovery truth.
- A reconnect waits for the prior session mutation gate, then replaces the
  generation. Requests from an older generation are rejected as
  `session_replaced`.
- The Python client retries transient connection failures with a capped
  `100ms, 250ms, 500ms, 1s, 2s, 5s` backoff. Mutations are never replayed.
- A backend process restart creates a new session; the frontend may reopen
  only the last known project and scene paths, without replaying unsaved edits.
- Viewport mappings are generation-scoped, explicitly closable and expose
  producer/consumer ownership and heartbeat metadata. Handle destruction is
  the final cleanup mechanism.
- Shutdown is idempotent and converges listener wakeup, session/runtime
  release, transport cleanup, project-lock release and frontend-child cleanup.

## Consequences

The frontend can remain responsive while the backend is unavailable, but a
pending mutation is deliberately abandoned and must be reissued by the user.
The protocol carries actionable diagnostics instead of relying on logs or
process termination. Shared-memory transport remains Windows-specific while
the verifier and cleanup behavior remain testable on other platforms.
