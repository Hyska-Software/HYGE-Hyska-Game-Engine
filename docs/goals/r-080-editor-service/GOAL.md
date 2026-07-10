# Goal: R-080 Editor Service and Versioned IPC

Execute the approved R-080 implementation in `docs/goals/r-080-editor-service/PLAN.md`.

Preserve the schema as the wire source of truth, require an authenticated
loopback handshake per connection, keep the Python process as a protocol-only
client, and do not claim future editor commands are implemented.
