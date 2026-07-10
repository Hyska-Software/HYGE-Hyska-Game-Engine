# R-080 — Rust Editor Service and Versioned IPC

**Intent:** Complete the authenticated Rust editor control service and its
versioned JSON protocol without duplicating ECS state.

**Truth owner:** `protocol/editor.schema.json` owns the wire contract;
`hyge-editor` owns authentication and service session metadata.

**Contract boundary:** Big-endian 4-byte length-prefixed JSON envelopes over
IPv4 loopback TCP. The first request on every connection is `hello` with the
session token.

**Cutover:** Replace the initial permissive server behavior that accepted
pre-auth requests and reported future commands as accepted. Unsupported editor
commands now return structured `unsupported_request` errors.

**Acceptance evidence:** Rust protocol, server, CLI and Python tests cover
round-trip framing, malformed/oversized/version-invalid messages, handshake
success/failure, TCP correlation IDs, loopback enforcement and CLI argument
handling.

## Implementation tasks

1. Keep protocol types, framing and errors modular while preserving public
   reexports.
2. Enforce loopback binding and per-connection authentication in
   `hyge-editor`.
3. Validate CLI projects, generate/override session tokens, and manage the
   optional Python child process.
4. Align the Python client and schema fixtures with the Rust wire contract.
5. Run focused tests, formatting, clippy, workspace tests and documentation
   gates before updating the roadmap.

## Non-goals

Real scene loading, ECS snapshots, reflection catalogs, transactional commands,
viewport transport and reconnect lifecycle remain R-081 through R-084.
