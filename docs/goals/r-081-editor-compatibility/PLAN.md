# R-081 — Protocol Compatibility and Session Lifecycle

**Intent:** Make the editor IPC independently validatable, version-negotiated,
reconnectable and safe to evolve without duplicating ECS state.

**Current Behavior:** R-080 has strict version framing, token authentication
and project/scene metadata, but no negotiated handshake, session identity,
reconnect registry, duplicate request detection or schema validator.

**Expected Outcome:** The schema validates the complete v1 envelope contract;
the server negotiates version 1, returns explicit compatibility/auth errors,
enforces request IDs/timeouts and resumes one canonical in-process session.

**Target-Perspective Output:** A PySide6 client can connect, receive a server
session ID, disconnect, reconnect with that ID and observe the same project
metadata without creating a second session.

**Truth Owner:** `protocol/editor.schema.json` owns the wire shape;
`hyge-editor-protocol` owns framing/structural validation;
`hyge-editor` owns authentication and session state.

**Contract Boundary:** Big-endian 4-byte length-prefixed JSON over IPv4
loopback TCP. `hello` is the first frame and includes supported versions,
client identity, optional session ID and token.

**Cutover:** Replace R-080's one-shot handshake and in-memory connection state
with negotiated protocol/session handling. Keep R-082 responsible for real
project/world loading and process-restart persistence.

**Acceptance Evidence:** Draft 2020-12 schema fixtures, Rust framing/server
tests, Python client tests, and a TCP reconnect test proving identity/state
continuity and stale-connection rejection.

**Kill Criteria:** Do not add a second session store, silently accept an
unsupported version, or treat a disconnected/replaced connection as the
current session owner.

**Architecture Slice:** `protocol/editor.schema.json`,
`crates/hyge-editor-protocol`, `crates/hyge-editor`,
`tools/hyge-editor-python`, `docs/architecture.md`, and this goal package.

**Plan Review Gate:** Requires post-implementation correctness and
maintainability review before R-081 is reported complete.

## Implementation tasks

1. Add structural-vs-negotiated protocol framing, correlation IDs and explicit
   envelope validation.
2. Define handshake/ack payloads and conditional JSON Schema validation.
3. Add session registry with generation replacement, TTL and metadata resume.
4. Add server request-ID conflict handling, socket timeouts and stable errors.
5. Update the Python client with supported versions, session retention,
   reconnect and timeout behavior.
6. Run focused and workspace gates, then update roadmap evidence.

## Non-goals

Real scene loading, ECS snapshots, reflection, commands, project locks and
cross-process persistence remain R-082 through R-084.
