# R-102 — Editor Reconnect, Failure Recovery and Shutdown

**Intent:** Make backend failure, reconnect, broken project/scene input,
viewport transport death and shutdown observable and recoverable.
**Truth Owner:** `hyge-editor` owns runtime/session truth; Python/QML owns only
disposable presentation state.
**Contract Boundary:** Versioned TCP envelopes plus generation-scoped Windows
shared memory.

## Implemented slices

- Session mutation gate fences old generations before reconnect commits.
- Server shutdown wakes the non-blocking accept loop, releases runtimes and is
  idempotent.
- Protocol errors support recoverability and actionable path/operation/action
  metadata; lifecycle states include reconnecting/disconnected/closing.
- Python session retries transient failures with capped backoff, preserves
  session identity, drops pending mutations and refreshes after resume.
- Viewport producer/consumer handles close idempotently and publish ownership
  and heartbeat metadata; non-Windows shared-memory access returns errors.
- Qt shell exposes backend status, retry timing and manual retry/close actions.

## Evidence

- `cargo test -p hyge-editor --test r102_recovery -- --test-threads=1`
- `cargo test -p hyge-editor --test r088_viewport_transport -- --test-threads=1`
- `python -m pytest -q tools/hyge-editor-python/tests`
- Workspace fmt, clippy, test and documentation gates.

## Completion gate

R-102 is complete only when the real backend/session/viewport shutdown path
passes without stale-generation mutation, leaked project lock or leaked
mapping, and the roadmap completion notes contain the exact evidence.
