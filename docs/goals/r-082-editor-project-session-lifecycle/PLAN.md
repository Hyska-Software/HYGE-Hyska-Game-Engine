# R-082 — Editor Project and Session Lifecycle Implementation Plan

**Intent:** Turn the R-080/R-081 editor transport into a real, engine-owned
project and scene session that can load, save, reconnect, report readiness and
shut down without leaking a frontend process or project lock.

**Current Behavior:** `hyge-editor::EditorServer` authenticates clients and
retains reconnectable `project`/`scene` strings in `SessionRegistry`, but
`open_project` and `open_scene` only record paths. The service does not own a
`hyge_app::App`/ECS world, does not call the real scene loader, has no project
lock or revision, and does not model loading/ready/degraded/failed states.
`hyge-tools editor` validates a directory, optionally starts Python, and only
kills the child after `server.run()` returns. `hyge-scene` already provides the
real `.hyge-world` path loader and `LoadedSceneState`, but its runtime contract
is not yet connected to the editor service.

**Expected Outcome:** One authenticated editor session owns one project lock,
one engine world and at most one active scene. Requests transition a visible
state machine, use the production project/scene loading paths, save the active
world through a durable atomic replace with a monotonic persisted revision,
resume metadata and engine state on reconnect, and release every resource on
normal or error shutdown.

**Target-Perspective Output:** The PySide6 client receives explicit lifecycle
events/responses such as `loading`, `ready`, `degraded` or `failed`, can open a
real project and `.hyge-world`, save and observe the returned revision, then
reconnect or close while the service leaves no orphan process, session or lock.

**Truth Owner:** `hyge-editor` owns lifecycle state, engine host, project lock,
revision and teardown. `hyge-scene` owns world document decoding and ECS scene
instantiation. `hyge-editor-protocol` plus `protocol/editor.schema.json` own
the wire shape. `hyge-tools` owns process orchestration and frontend-child
ownership; it must not maintain a second editor session store.

**Contract Boundary:** Version-one loopback TCP envelopes from R-081. Add
stable payloads for lifecycle status, project/scene open results, save result
and failure diagnostics. Status events carry `session_id`, canonical paths,
state, scene revision and a machine-readable degraded/failed code where
applicable. The service serializes mutations per session; reconnect only
rebinds the same session generation and never creates a second ECS world.

**Cutover:** Replace the current metadata-only `update_project`/
`update_scene` handlers with calls into one `EditorSessionRuntime`. Route
`server_shutdown` through the same teardown object used by `hyge-tools`; make
the CLI pass project/frontend ownership into the server. Keep the existing
R-081 `SessionRegistry` as the identity/reconnect layer, but move live engine
state out of `EditorState` and into the canonical runtime record.

**Displaced Path:** Metadata-only open handlers, ad-hoc child cleanup after
`run()`, and any future frontend-side project/scene truth. No second loader,
lock manager, revision counter or ECS mirror may be introduced.

**Value Density:** Highest-value slice is a real fixture project over the
existing `WorldDocument`/`WorldLoader` path plus a TCP lifecycle test. This
proves the whole boundary without requiring the future snapshot, command or
viewport systems from R-083+.

**Acceptance Evidence:** Rust lifecycle tests with a checked-in or generated
fixture project, atomic-save/revision assertions, lock contention/release
assertions, child-process teardown assertions, and a reconnect test that
observes one session/world. Python IPC tests validate the new response/event
payloads. Retain a protocol trace and saved `.hyge-world` fixture as test
artifacts where practical.

**Evidence Lane:** Run focused `hyge-editor` tests first, then the real
`hyge-app`/`hyge-scene` integration test, Python tests, and finally workspace
fmt/clippy/test/doc gates. A passing unit test that never calls
`load_world_document_from_path` is insufficient for this item.

**Kill Criteria:** Stop and revise the design if (1) a request can mutate a
world without the session generation/revision check, (2) reconnect creates a
second `App`/ECS world, (3) save can expose a partial file or reports a
revision before durable replacement, (4) a lock survives a failed open or
shutdown, (5) the frontend child is owned by both CLI and server, or (6) a
new fake loader/project store is added instead of adapting the production
loader path.

**Architecture Slice:**

- Files to create: `crates/hyge-editor/src/lifecycle.rs`,
  `crates/hyge-editor/src/project.rs`,
  `crates/hyge-editor/src/lock.rs`,
  `crates/hyge-editor/tests/r082_lifecycle.rs`, and a deterministic fixture
  under `crates/hyge-editor/tests/fixtures/` (or the existing repository
  fixture convention).
- Files to modify: `crates/hyge-editor/src/lib.rs`, `state.rs`, `server.rs`,
  `crates/hyge-editor/Cargo.toml`, `crates/hyge-editor-protocol/src/envelope.rs`,
  `protocol/editor.schema.json`, `crates/hyge-tools/src/cmd/editor.rs`,
  `crates/hyge-tools/src/cli.rs` only if lifecycle ownership changes its
  arguments, `tools/hyge-editor-python/src/hyge_editor/ipc.py`, its tests,
  `docs/roadmap.toml`, and `AGENTS.md` only for the final evidence checklist.
- Files to avoid: renderer, snapshots, command/undo code, QML models, and
  `hyge-scene` serialization internals unless integration reveals a concrete
  loader defect. R-083 owns hierarchy/reflection snapshots; R-084 owns edits.
- Source of truth: canonicalized project root + active scene path in the
  lifecycle runtime; the live ECS world is the only mutable scene state;
  `LoadedSceneState.document` is the loaded document baseline; the on-disk
  file and sidecar revision marker are the persisted save truth.
- Read path: validate project root and project metadata, acquire lock, build
  an editor-safe `hyge_app::App` with the project-root asset/script config,
  install the scene registry/library, then call
  `hyge_scene::load_world_document_from_path` for the requested world.
- Write path: serialize the active `WorldDocument` from the editor runtime,
  write a uniquely named sibling temporary file, flush/sync it, atomically
  replace the target, then atomically update the revision metadata and report
  the revision only after both replacements succeed. Preserve the previous
  target until replacement succeeds; clean temporary files on every failure.
- Contract boundary: lifecycle messages/events are serialized under the v1
  schema and use existing correlation IDs; errors remain structured
  `engine_error` responses with stable codes.
- Integration points: `hyge-app::App`/`bevy_ecs::World`,
  `hyge-scene::{WorldDocument, WorldLoader, LoadedSceneState,
  load_world_document_from_path}`, `hyge-asset::AssetDb`/asset watcher as
  available from the project config, `SessionRegistry`, and the optional
  Python child process.
- Migration/cutover: keep R-081 reconnect identity behavior; on a resumed
  session reuse its runtime record if still alive, otherwise return an
  actionable `session_runtime_unavailable`/`project_not_open` error rather
  than silently recreating state. Existing metadata-only tests become real
  loader tests.
- Acceptance gate: `r082_lifecycle` must prove open → ready → save →
  disconnect/reconnect → close, including invalid project/scene, degraded
  load, atomic failure cleanup, lock contention and child termination.

**Plan Review Gate:** Requires PRE review before execution and a final
correctness/maintainability review before R-082 is marked complete.

## Contract decisions to lock before coding

1. **Lifecycle states.** Use `Loading`, `Ready`, `Degraded` and `Failed`.
   `Loading` is transitional and includes an operation identifier; `Ready`
   means project lock + engine world + requested scene are live; `Degraded`
   means the project/session is usable but a non-fatal asset/optional-service
   problem was recorded; `Failed` means the requested operation did not commit
   and the previous stable runtime remains intact where possible.
2. **Open transaction.** Validate/canonicalize paths and acquire the project
   lock before replacing the current runtime. Build/load into a candidate
   runtime; commit it only after the real loader succeeds. On failure, release
   candidate resources and preserve the old ready session.
3. **Project lock.** Use an OS-backed exclusive lock file inside the project
   (for example `.hyge/editor.lock`) held by the runtime guard. It contains
   diagnostic owner/session data, is never treated as the lock itself, and is
   removed/released by RAII teardown. A second service fails deterministically.
4. **Revision.** Start from the persisted revision if present, otherwise the
   content hash/initial revision policy chosen in the implementation review.
   Every successful save increments exactly once and returns `{revision,
   path, asset_id}`. Failed saves do not increment or replace the target.
5. **Shutdown ordering.** Stop accepting/mutating requests, emit
   `server_shutdown`/final status, save nothing implicitly, drop the engine
   runtime, release the lock, request frontend termination, wait with a bounded
   timeout, then force-kill only as a final fallback. The owner performs this
   once and is idempotent.

## Implementation tasks

### 1. Establish the lifecycle model and ownership seams

- Modify `state.rs` to retain session identity/generation but store a shared
  runtime handle/status rather than only path strings.
- Add `lifecycle.rs` with `EditorSessionRuntime`, status/error/result types,
  serialized mutation boundary and idempotent close.
- Add `project.rs` for canonical project validation, project-root engine
  construction, scene path containment checks, and project diagnostics.
- Add `lock.rs` for an OS-backed exclusive lock guard with explicit release
  and tests for contention/release-on-drop.
- Keep public APIs rustdoc-complete and return `HygeResult`/structured editor
  errors; no `unwrap`/panic in library code.

**Verify:** focused compile and unit tests for state transitions, path
containment, lock acquisition, lock contention and guard drop.

### 2. Connect open operations to the real engine and scene loaders

- Extend `hyge-editor/Cargo.toml` with only the required workspace crates
  (`hyge-app`, `hyge-scene`, `hyge-asset`, and any existing config/runtime
  dependency proven necessary).
- Build the editor runtime with a non-window/headless-safe `App` configuration
  appropriate for service use; do not call `App::run()` or create a GUI
  window from the server thread.
- Install the existing scene type registry/prefab library and route
  `open_scene` through `load_world_document_from_path`, which in turn uses
  `WorldLoader`; do not duplicate msgpack decoding or ECS instantiation.
- Define how project prefabs/assets are discovered for the fixture. If the
  current project format has no manifest, use the canonical existing asset
  directories and record the missing-manifest limitation as a degraded
  diagnostic rather than inventing a second project format.
- Commit project/scene runtime only after load succeeds; emit status updates
  for loading/ready/degraded/failed.

**Verify:** an integration test writes/loads a real `.hyge-world` and asserts
`LoadedSceneState`, root entities and canonical paths; invalid scene and
missing prefab cases produce failed/degraded diagnostics without a half-open
session.

### 3. Define and validate lifecycle wire messages

- Update `Envelope` helpers and `protocol/editor.schema.json` with the exact
  lifecycle status, open result, save result and close/shutdown payloads.
- Preserve R-081 correlation/error rules and add stable codes for lock held,
  project invalid, scene invalid, load failed, save failed and session closed.
- Make `open_project`, `open_scene` and `save_scene` return the committed
  result, while status changes are explicit events or correlated responses;
  document ordering so the frontend cannot treat `loading` as ready.
- Update the Python `EditorClient` and tests to decode status/result payloads,
  preserve session identity and surface failure without losing reconnect data.

**Verify:** schema validation fixtures for every success/failure state and
  Python round-trip tests for open/load/save/close payloads.

### 4. Implement atomic save and persisted revision

- Add a project-scoped revision record with a documented format and
  corruption behavior.
- Serialize the active `WorldDocument` through its existing `to_bytes()`;
  stage beside the target, flush/sync, atomically replace, then atomically
  persist the revision metadata.
- Ensure parent directories are created only within the canonical project
  root, reject path traversal, clean temp files, and never report a revision
  before durable commit.
- Define recovery when the revision sidecar is corrupt: preserve the scene,
  return `degraded`/`failed` explicitly, and do not reset silently.

**Verify:** bytes round-trip after save, revision increments once, target is
  unchanged on injected write/rename failure, no temp files remain, and a
  second runtime reads the persisted revision.

### 5. Make server and CLI teardown authoritative

- Refactor `EditorServer` to own an idempotent shutdown coordinator containing
  sessions, active runtime guards and optional frontend-child ownership.
- `server_shutdown`, listener errors, client disconnect policy and `Drop`
  must converge on the same close path; no blocking child wait while holding
  the session mutex.
- Change `hyge-tools/src/cmd/editor.rs` so it passes the optional child into
  the server/coordinator or otherwise uses the server's explicit shutdown
  handle. Remove the current split ownership where CLI cleanup happens only
  after `run()` returns.
- Close the frontend gracefully first, wait boundedly, then kill/wait as
  fallback; always release the project lock even if the child fails.

**Verify:** child-process fixture exits after shutdown, lock can be acquired
  by a second process after close, server shutdown is idempotent, and an
  accept-loop/frontend error does not orphan the child or lock.

### 6. Prove the end-to-end lifecycle and sync project evidence

- Add `crates/hyge-editor/tests/r082_lifecycle.rs` covering:
  open project, open real scene, status progression, save/revision,
  reconnect/resume, stale generation rejection, close, lock release and
  no-orphan assertions.
- Add the minimal Python client tests and a protocol trace fixture; use a
  real subprocess for the child/close test rather than mocking `Child`.
- Run targeted Rust/Python gates, then workspace gates with the repository's
  full-path rustup cargo shim from `AGENTS.md`.
- Update `docs/roadmap.toml` only after every acceptance bullet has evidence;
  add completion notes with exact test commands/artifact paths. Update
  `AGENTS.md` M6 checklist only if this lifecycle evidence is part of its
  documented DoD.

**Definition of done:** all five R-082 acceptance bullets are demonstrated by
  real loader/process/filesystem evidence; otherwise report the item as
  implemented but unproven and leave it `in_progress`.

## Verification commands

```powershell
& "C:\Users\estev\.cargo\bin\cargo.exe" test -p hyge-editor --test r082_lifecycle
& "C:\Users\estev\.cargo\bin\cargo.exe" test -p hyge-editor
python -m pytest -q tools/hyge-editor-python/tests
& "C:\Users\estev\.cargo\bin\cargo.exe" fmt --all -- --check
& "C:\Users\estev\.cargo\bin\cargo.exe" clippy --workspace --all-targets -- --deny warnings
& "C:\Users\estev\.cargo\bin\cargo.exe" test --workspace
& "C:\Users\estev\.cargo\bin\cargo.exe" doc --workspace --no-deps
```

## Non-goals

R-082 does not implement hierarchy/reflection snapshots (R-083), edit
commands or undo/redo (R-084), viewport rendering/transport (R-087/R-088),
QML models (R-089+), scene hot-reload conflict resolution (R-101), or full
backend crash recovery (R-102). It may expose only the lifecycle data needed
by those later tasks.

## Risks and mitigations

- **`App` is currently window-oriented:** isolate the service host from
  `App::run()` and add the smallest headless construction seam; do not create
  a second engine implementation.
- **World serialization is document-based, not an ECS serializer:** R-082
  should persist the authoritative loaded `WorldDocument` baseline and avoid
  pretending arbitrary editor mutations are already supported; R-084 must
  later define command/document synchronization.
- **Project format is underspecified:** do not silently add a manifest. Use
  existing project-root/asset conventions and record any degraded optional
  setup in the status contract; promote a manifest to an ADR only if loading
  cannot be made deterministic.
- **Windows atomic replacement/locking semantics:** implement and test the
  Windows path first, keep temp files on the same volume, and verify the lock
  can be reacquired after teardown.
- **Thread ownership races:** centralize runtime mutation and shutdown in one
  coordinator; never hold `SessionRegistry` locks while loading, saving or
  waiting for a process.

