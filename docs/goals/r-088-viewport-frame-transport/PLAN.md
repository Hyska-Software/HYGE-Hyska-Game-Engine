# R-088 Viewport Frame Transport and Input Bridge Implementation Plan

**Intent:** Deliver editor viewport frames and revisioned input without moving
renderer or ECS ownership out of Rust.

**Truth Owner:** `hyge-editor` owns session state, camera state and control;
`hyge-editor-shm` owns only Windows mapping handles.

**Contract Boundary:** Version-2 authenticated control messages describe a
three-slot RGBA8 shared-memory ring; the Python client maps and verifies it.

**Cutover:** v2 clients use shared memory. Version-1 clients retain the
existing control behavior and receive no frame transport.

**Acceptance Evidence:** stress test validates header integrity, drop
detection, reconnect generation change and shutdown safety.
