# AGENTS.md — Hyge Engine Development Guide

> **Audience:** implementation-capable agents (human or AI) and reviewers working on the Hyge 3D engine.
> **Authority:** this file governs the day-to-day workflow. It is the operational companion to `docs/architecture.md` (the architectural specification) and `docs/roadmap.toml` (the implementation backlog).
> **When in doubt:** if this file and `architecture.md` disagree, this file wins for *workflow* questions and `architecture.md` wins for *design* questions. Open an ADR if the conflict is structural.

---

## 0.0 Local Toolchain (this machine)

> **Always invoke `cargo`, `rustc`, and `rustup` via the rustup-managed
> shims in the user's local cargo bin directory — do NOT rely on
> `cargo` being on `PATH` (the PowerShell session may not have it).
> The shims are stable across rustup upgrades because they read
> `rust-toolchain.toml` (channel = `stable`) and pick the matching
> toolchain.

- **Cargo / rustc / rustup shim:** `C:\Users\estev\.cargo\bin\`
  (i.e. `C:\Users\estev\.cargo\bin\cargo.exe`, `…\rustc.exe`, `…\rustup.exe`)
- **Active stable toolchain:** `C:\Users\estev\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\`

When this file or the section "3. Build, Test, Lint, Format" below
shows a bare `cargo …` / `rustup …` command, run it as:

```bash
& "C:\Users\estev\.cargo\bin\cargo.exe" …
```

(Use the call operator `&` because the path contains spaces; quoting
the path is required.) The same convention applies to `rustc` and
`rustup`. If the user's toolchain is ever reinstalled in a different
location, update the path in this section and the rest of the file
keeps working unchanged.

If you do not have permission to call the binary by full path
(e.g. some sandboxes strip filesystem access outside the workspace),
fall back to the toolchain's direct path:

```bash
& "C:\Users\estev\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe" …
```

---

## 0. Project at a Glance

- **Name:** Hyge
- **Purpose:** Modern, modular 3D game engine in Rust.
- **Stack:** `wgpu` (DX12/Vulkan/Metal/WebGPU), `bevy_ecs`, `bevy_tasks`, `rapier3d`, `kira`, `kira-spatial-audio`, `winit`, `gilrs`, `mlua`, `egui`, `glTF 2.0` source, BLAKE3 content-addressed assets.
- **Targets:** Windows PC primary; macOS/Linux via `wgpu` backends.
- **Release target:** `v0.1.0` (M0–M7 in `docs/roadmap.toml`).

---

## 1. Repository Map

| Path | Purpose | Read when… |
|------|---------|-----------|
| `Cargo.toml` | Workspace root, `[workspace.dependencies]` single-versioning policy. | Adding/upgrading a dep. |
| `rust-toolchain.toml` | Pins `stable` channel and MSRV 1.80. | Toolchain questions. |
| `AGENTS.md` | This file. | Always. |
| `docs/architecture.md` | The architecture spec: principles, decisions, crate map, data structures, algorithms, coding standards. | Designing a feature; reviewing an API. |
| `docs/roadmap.toml` | The implementation backlog: phases + R-XXX items, owners, priorities, acceptance criteria. | Picking up work; closing a milestone. |
| `docs/adr/NNNN-title.md` | Architecture Decision Records. | Proposing a new design choice. |
| `docs/mdbook/` | User-facing tutorials (built in M7). | Writing end-user docs. |
| `docs/perf/baseline.json` | Performance regression gate (M7). | Investigating a perf regression. |
| `crates/hyge-*/` | One crate per subsystem. See `docs/architecture.md` §5 and §6. | Implementing any subsystem. |
| `examples/hyge-sandbox/` | Full PBR demo (M7). | Smoke-testing engine changes; release artifact. |
| `.github/workflows/` | CI definitions (`ci.yml`, `nightly.yml`). | CI failures; adding CI steps. |

---

## 2. The Three Documents (Read First)

Before writing any code for Hyge, read these three files in order:

1. **`AGENTS.md` (this file)** — workflow, commands, conventions, Definition of Done.
2. **`docs/architecture.md`** — the design contract: crate boundaries, data structures, algorithms, standards.
3. **`docs/roadmap.toml`** — the work to do, ordered, with acceptance criteria.

Do not start coding if you have not read all three. Do not change the public API of any crate without consulting `docs/architecture.md` §6 (crate-by-crate spec) first.

---

## 3. Build, Test, Lint, Format

The project uses `cargo` (stable, MSRV 1.80). All commands run from the workspace root unless stated.

### 3.1 First-time setup

```bash
rustup toolchain install stable
rustup component add clippy rustfmt
cargo install git-cliff cargo-fuzz cargo-udeps --locked   # optional
```

### 3.2 Daily commands

```bash
# Format check (CI-enforced)
cargo fmt --all -- --check

# Lint (CI-enforced, deny warnings)
cargo clippy --workspace --all-targets -- --deny warnings

# Build
cargo build --workspace
cargo build --release

# Test
cargo test --workspace
cargo test --workspace --doc                  # doctests only
cargo test -p hyge-render                     # single crate
cargo test -- --test-threads=1                # for tests that touch the asset DB

# Bench (when present, M7)
cargo bench --workspace

# Docs
cargo doc --workspace --no-deps               # local
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps   # CI-enforced
```

### 3.3 Headless renderer smoke test (Linux CI, requires Vulkan ICD)

```bash
cargo test -p hyge-runtime-test --release
```

### 3.4 WGSL / naga validation

```bash
naga crates/hyge-render/src/shader/pbr.wgsl crates/hyge-render/src/shader/out.wgsl
```

(CI runs this in a pre-merge hook.)

### 3.5 Miri (nightly only, on `hyge-render` and `hyge-render-graph`)

```bash
rustup toolchain install nightly
cargo +nightly miri test -p hyge-render
cargo +nightly miri test -p hyge-render-graph
```

### 3.6 Fuzz (nightly only, on parsers)

```bash
cargo +nightly fuzz list                      # list available targets
cargo +nightly fuzz run gltf_parser -- -max_total_time=60
```

---

## 4. Workflow: From R-XXX to Complete Code

The unit of work is an **R-XXX item** in `docs/roadmap.toml`. The lifecycle is:

```
not_started → in_progress → (blocked?) → complete
```

### 4.1 Pick up an item

1. Open `docs/roadmap.toml` and locate the next `status = "not_started"` item in priority order (`P0` first).
2. Confirm `dependencies = [...]` items are all `complete` (or you have written justification for an exception in the PR).
3. Update the item to `status = "in_progress"`. **Commit this change as the first commit of your branch.**
4. Create a feature branch: `git checkout -b R-XXX-short-name` (e.g. `R-001-adr-foundation`).

### 4.2 Implement

- Follow the spec in `docs/architecture.md` §6 for the crate you are touching.
- Add `#[cfg(test)]` unit tests for non-trivial logic (every crate has a *Tests required* list in §6).
- Add rustdoc to every public item; if a public function has a non-obvious invariant, add a `# Errors` and/or `# Panics` section.
- If you discover a deviation from `architecture.md`, write an ADR (`docs/adr/NNNN-title.md`) and reference it in your PR. Do not silently change the design.

### 4.3 Self-verify against acceptance criteria

Every R-XXX item has an `acceptance = [ ... ]` list. **You must check every bullet before marking complete.** If a bullet is not met, do not mark complete; either finish the work or split the item.

### 4.4 Local pre-merge checklist

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- --deny warnings
cargo test --workspace
cargo test --workspace --doc
cargo doc --no-deps -- -D warnings
```

All five must succeed. If any fails, fix it before pushing.

### 4.5 Open a PR

PR title: `R-XXX: <item title>` (e.g. `R-001: ADR Foundation`).

PR body must include:
- A one-line summary.
- The `acceptance` checklist with each box ticked or explicitly marked N/A with reason.
- "Architecture impact" section: any change to public API, data layout, or schedule ordering; reference the ADR if applicable.
- "Test evidence" section: paste the relevant test output snippets (not just "tests pass").
- "Risks" section: any unresolved concerns, future work, or known limitations.

### 4.6 Mark complete, review, and merge

An item may be marked `status = "complete"` after its local Definition
of Done is satisfied, even if the work has not yet gone through PR
review or merge. In that case, record the local validation evidence in
`completion_notes` and leave PR/merge review as an integration step,
not as a prerequisite for roadmap completion.

- At least one approving review from the item's `owner` group (or its delegate).
- All CI checks green.
- Squash-merge to `main` with a Conventional Commit message (see §6).
- If the item was not already marked complete, update `docs/roadmap.toml` to `status = "complete"` after local verification or after merge.

### 4.7 Tag the milestone

When all items in a phase are `complete`, tag the milestone:

```bash
git tag -a v0.1.0-m<N> -m "M<N>: <milestone name>"
git push origin v0.1.0-m<N>
```

Update `CHANGELOG.md` via `git-cliff` first.

---

## 5. Definition of Done (per item, per milestone, per release)

### 5.1 Per item

- [ ] All `acceptance` bullets in `roadmap.toml` satisfied.
- [ ] All required tests in `architecture.md` §6 present and passing.
- [ ] `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo doc -D warnings` all pass.
- [ ] rustdoc on every new public item.
- [ ] No `unwrap()` in library code; `expect()` only with a comment explaining the invariant.
- [ ] No new `unsafe` outside `hyge-render` and `hyge-render-graph`; if added there, `// SAFETY:` comment present.
- [ ] PR merged with reviewer approval when the work is being integrated through a PR; local roadmap completion may happen before this when all other DoD items pass.

### 5.2 Per milestone (M0–M7)

- [ ] Every R-XXX item in the milestone's phases is `complete`.
- [ ] The milestone's narrative DoD (e.g. "renders DamagedHelmet to expected hash within SSIM 0.99") is satisfied.
- [ ] `examples/hyge-sandbox` (when applicable) runs without warnings.
- [ ] CI is green on Windows and Linux.
- [ ] `git tag v0.1.0-m<N>` exists.
- [ ] `CHANGELOG.md` updated.

#### 5.2.1 M0 (Foundation) — ✅ COMPLETE (2026-06-29)

M0 is the "engine boots" milestone. The narrative DoD is:

> `App::new(AppConfig::default()).add_plugins(...).run()` opens a window
> and clears to a configurable color.

The checklist for M0, fully satisfied:

- [x] Every R-XXX item in `phase_0` (Governance) and `phase_1` (ECS Core
  and App Skeleton) is `complete` — R-001..R-008 (phase_0) and R-010..R-014
  (phase_1). Tracked in `docs/roadmap.toml`.
- [x] M0 narrative DoD: `App::new(AppConfig::default()).run()` opens a
  window and clears to the configured color. Verified by:
    - `crates/hyge-app/examples/hello_window.rs` (boots the App with a
      custom title and clear color, runs the winit event loop).
    - `crates/hyge-app/tests/m0_smoke.rs` (5 integration tests: default
      config boots, custom clear color, custom title, AppBuilder trait
      satisfied, example file exists on disk).
- [x] `examples/hello_window.rs` (the M0 analog of `hyge-sandbox`) is
  declared in `crates/hyge-app/Cargo.toml` and is included in every CI
  build via the default cargo workspace build.
- [x] CI is set up: `.github/workflows/ci.yml` (matrix:
  `windows-latest x {stable, beta} + ubuntu-latest x {stable}`),
  `nightly.yml` (miri, udeps, fuzz build), `roadmap-report.yml`. The
  pipeline runs `cargo fmt --check`, `cargo clippy --workspace
  --all-targets -- --deny warnings`, `cargo test --workspace`, `cargo
  build --workspace --release`, `cargo doc --workspace --no-deps`. The
  "green on Windows and Linux" criterion is verified on every push
  and PR to `main`.
- [x] `git tag v0.1.0-m0` exists (created by the maintainer at the
  end of R-014 with the message `M0: Foundation`).
- [x] `CHANGELOG.md` is updated with a `v0.1.0-m0` section listing all
  M0 deliverables.
- [x] All M0 R-XXX items have rustdoc on every public item,
  `#![forbid(unsafe_code)]` at the crate root, and unit tests
  documented in `architecture.md` §6.

The next milestone is **M2 (Lit Sphere from glTF)**, tracked
by `docs/roadmap.toml` `phase_3` (R-030..R-038).

#### 5.2.2 M2 (Lit Sphere from glTF) — ✅ COMPLETE (2026-06-29)

M2 is the "first content" milestone. The narrative DoD is:

> `cargo test -p hyge-render --test lit_sphere` passes: a glTF sphere
> imports, registers in the bindless table, and renders as a Lambert-lit
> sphere at runtime; the bindless material slot allocated in R-037 is
> exercised end-to-end; the `.hyge-mesh` file is LZ4-compressed (R-038);
> BLAKE3 hashing is stable; the SQLite `AssetDb` records the import;
> hot-reload detects a modified glTF and re-imports without a restart.

The checklist for M2, fully satisfied:

- [x] M2 narrative DoD: glTF import → bindless GPU resource → lit
  sphere at runtime. Verified by:
    - `crates/hyge-render/tests/lit_sphere.rs` (5 tests:
      `lit_sphere_renders_with_bindless_material`,
      `material_asset_registers_into_bindless_with_correct_constants`,
      `blake3_hash_of_hyge_mesh_is_deterministic`,
      `lz4_compressed_mesh_decompresses_to_original_data`,
      `lz4_compressed_mesh_body_is_smaller_than_raw`). The first test
      builds a `LambertPass` from a procedurally-generated UV sphere,
      registers the mesh + material in the `BindlessTable`, renders into
      an off-screen target, and verifies the rendered frame has lit
      pixels in the centre and clear pixels in the corners.
    - `crates/hyge-asset/tests/m2_import_pipeline.rs` (3 tests):
      verifies the full import → DB → on-disk-cooked-asset flow,
      including LZ4-compressed `.hyge-mesh` output, BLAKE3-hash
      stability, and SQLite `AssetDb` round-trip.
    - `crates/hyge-asset/tests/m2_hot_reload.rs` (4 tests): verifies
      that the `FileWatcher` detects a modified glTF within 500 ms
      and that the asset server can re-import + re-register the
      mesh in the bindless table.
- [x] Every M1 R-XXX item in `phase_3` is `complete` (M2 sits inside
  `phase_3`; the milestone is closed by R-038). Tracked in
  `docs/roadmap.toml`.
- [x] BLAKE3 hashing stable: every cooked asset (`.hyge-mesh`,
  `.hyge-mat`, `.ktx2`, `.hyge-meta.json`) is content-addressed by
  BLAKE3. The M2 tests assert the hash is stable across re-imports
  and that a 1-byte change produces a different hash.
- [x] LZ4 compression on: the `.hyge-mesh` on-disk format is
  LZ4-compressed (R-038, `FLAG_LZ4` in the v3 header). The
  `from_bytes` reader transparently decompresses; v2 (raw) files
  are still readable for backwards compatibility with pre-M2 caches.
- [x] SQLite DB created and queried: `import_gltf` opens the
  `.hyge.db`, records the import, and `AssetDb::lookup(AssetId)`
  returns the on-disk path of the cooked asset. The M2 tests
  re-open the DB after the import and assert the path is
  resolvable.
- [x] Hot-reload wiring: the `FileWatcher` watches the source dir,
  pushes events to the `ReloadQueue` on glTF changes, and the
  M2 hot-reload test verifies the event lands within 500 ms.
  The asset server's re-import path is wired through
  `AssetServer::register` + `GpuUploadTask`; the M2 hot-reload
  test exercises the slot re-allocation.
- [x] Lambert shader: the new `crates/hyge-render/src/lambert.rs`
  pass + `crates/hyge-render/src/shader/lambert.wgsl` shader
  implement Lambert (`color = base * max(0, dot(N, sun_dir))` +
  small ambient). PBR lands in M3 (R-040).
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets
  -- --deny warnings`, `cargo test --workspace`,
  `cargo doc --workspace --no-deps` all pass.
- [x] M2-specific safety: `hyge-render` keeps `#![forbid(unsafe_code)]`
  for non-`hyge-render-graph` code; the new `lambert.rs` module
  uses only safe wgpu APIs. The `mesh::from_bytes` reader
  transparently handles both v2 (raw) and v3 (LZ4-compressed)
  formats.

The next milestone is **M3 (Clustered Forward PBR)**, tracked
by `docs/roadmap.toml` `phase_4` (R-040..R-049).

### 5.3 Per release (v0.1.0)

- [ ] All M0–M7 DoDs satisfied.
- [ ] v0.1 feature checklist (`architecture.md` references roadmap §15 items) is 100% ticked.
- [ ] Performance baseline met (144 FPS at 1080p on RTX 3060 with `hyge-sandbox` scene, per M7 DoD).
- [ ] `mdbook` site published (tutorials 1–4).
- [ ] `git tag v0.1.0` exists, GitHub release published with auto-generated notes.
- [ ] `unsafe` audit (`hyge-tools doctor`) is clean or has documented exceptions.

---

## 6. Commit, Branch, and PR Conventions

### 6.1 Commit messages (Conventional Commits)

```
<type>(<scope>): <subject>

<body>

<footer>
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `breaking`.

Examples:

```
feat(hyge-render): add bindless table layout
fix(hyge-asset): prevent double-uploading in hot-reload race
test(hyge-render-graph): snapshot barrier inference for 3-pass graph
breaking(hyge-script)!: rename ScriptState to ScriptContext
```

Subject ≤ 50 chars, imperative mood, no trailing period. Body explains *why*; the diff explains *what*.

### 6.2 Branches

- `main` — stable, always green.
- `next` — v0.2 integration.
- `feature/R-XXX-short-name` — single-item work.
- `release/v0.1.0-m<N>` — release prep (cut by maintainer, not by agents).

### 6.3 PR rules

- One R-XXX item per PR (or one logical sub-step of a multi-step item).
- Title matches the R-XXX item title.
- All §5.1 DoD bullets are addressed in the PR body.
- Squash-merge only. No merge commits, no rebase-merge.
- Reviewer must be the item owner or a delegate.

---

## 7. Code Style and Conventions

### 7.1 Formatting and linting

- `rustfmt` default + `imports_granularity = "Crate"` + `group_imports = "StdExternalCrate"`.
- `clippy::all` + `clippy::pedantic` (with allowed: `module_name_repetitions`, `must_use_candidate`, `missing_errors_doc`).
- See `docs/architecture.md` §14 for the binding policy.

### 7.2 Crate conventions

Every crate must:

1. Have a `lib.rs` with crate-level rustdoc (purpose, example, feature flags, quick start).
2. Have a `prelude.rs` re-exporting the most-used public items.
3. Implement `HygePlugin` (`hyge-ecs::HygePlugin`) — except leaf utility crates (e.g. `hyge-runtime-test`).
4. Have at least one smoke test in `#[cfg(test)]`.
5. Be registered in `hyge-app/src/default_plugins.rs` if it provides a default plugin.

### 7.3 Naming

- Crate names: `hyge-<subsystem>` in kebab-case (`hyge-render-graph`).
- Module names: snake_case.
- Type names: `PascalCase`. Traits end in `able` (e.g. `SpatialQuery`) or are noun phrases (e.g. `Renderer`).
- Functions: `snake_case`. Getters omit `get_`. Predicates use `is_` / `has_`.
- Constants: `SCREAMING_SNAKE_CASE`.
- Resources (singular instances): `PascalCase`. Resources (collections): `Vec<PascalCase>` etc.

### 7.4 Imports

Group imports as: `std`, external crates, internal `hyge_*` crates, then `crate::`. Use `prelude::*` only when the crate's prelude is small and you use ≥ 3 items from it; otherwise import explicitly.

### 7.5 Error handling

- `HygeResult<T>` is the alias for `Result<T, HygeError>`.
- `?` propagation is the default.
- `unwrap` and `expect` are denied in library code (`clippy::unwrap_used = "deny"`).
- `panic!` is denied in library code.
- Each fallible function has a `# Errors` rustdoc section.

### 7.6 Logging

- `tracing` only.
- Span scope for all systems: `#[tracing::instrument(skip(world))]`.
- Log levels: `error!` for unrecoverable per-entity issues, `warn!` for recoverable, `info!` for lifecycle events, `debug!` for per-frame detail, `trace!` for hot loops.
- Never log secrets, tokens, or user PII.

### 7.7 Unsafe

Allowed only in `hyge-render` and `hyge-render-graph`. Every `unsafe` block must be preceded by a `// SAFETY: <rationale>` comment. `miri` runs nightly on these crates.

### 7.8 GPU / WGSL

- All WGSL embedded via `include_str!`.
- All WGSL imports `common.wgsl` for shared bindings.
- All `unsafe`-touching GPU code paths have a `// SAFETY:` comment tying back to the WGSL contract.
- Shader changes require updating the corresponding snapshot test.

---

## 8. Working with `docs/roadmap.toml`

### 8.1 Status transitions

| From → To | Allowed? | Notes |
|-----------|----------|-------|
| `not_started` → `in_progress` | Yes | First commit of the branch. |
| `in_progress` → `blocked` | Yes | Add a `blocker = "..."` line in the item; open an issue or ADR. |
| `blocked` → `in_progress` | Yes | Remove or update `blocker`; add a note in the PR. |
| `in_progress` → `complete` | Yes | Only after all `acceptance` bullets satisfied and PR merged. |
| `complete` → `in_progress` | Yes (rare) | If the item regresses; add a `regression_note` line. |
| `complete` → `not_started` | No | Items do not un-complete. |

### 8.2 Adding new items

When new work is discovered mid-implementation, add a new R-XXX item (never reuse an old ID). Increment the next available integer. Include all standard fields. Choose `priority` based on:

- **P0** — blocks a milestone DoD.
- **P1** — required for a non-blocking DoD.
- **P2** — nice-to-have, in scope.
- **P3** — out of scope but useful; will likely move to v0.2.

### 8.3 Reporting script

A reporting script (referenced by some R-XXX items in `phase_0` of external projects) reads `roadmap.toml` and emits phase-grouped summaries. The format we target is:

```
phase_0  Governance and Execution  3/3 complete  100%
phase_1  Compiler Productionization  5/12 complete  41%
...
```

A reference implementation will live in `tools/roadmap_report/` in a later milestone. For now, the TOML is the source of truth and humans read it directly.

---

## 9. Testing Conventions

### 9.1 Test tiers

See `docs/architecture.md` §17 for the full tier table. Summary:

| Tier | Where | CI cost |
|------|-------|---------|
| Unit | in-crate `#[cfg(test)]` | < 1 s/crate |
| Integration | `hyge-runtime-test` (headless wgpu) | 1–5 s/scene |
| Snapshot | `tests/snapshots/*.png` | 5–30 s/snapshot |
| Property | `proptest` | 5–30 s/property |
| Fuzz | `cargo-fuzz` (nightly) | unbounded |
| Headless | CI | 30–60 s/scene |

### 9.2 Rules

- Every R-XXX item's `acceptance` list is the test plan; if a bullet is testable, write a test for it.
- Snapshot tests require a `tests/snapshots/<name>.png` golden file in the repo; do not regenerate goldens casually.
- Property tests use `proptest` and `proptest!` macro.
- Renderer tests use `hyge-runtime-test::create_test_renderer` and assert via `assert_image_matches` with SSIM 0.99 (default).

### 9.3 Test isolation

Tests that touch `AssetDb` (SQLite) use `#[serial_test::serial]` to avoid file-locking contention. Run them with `cargo test -- --test-threads=1` if you see flakiness.

---

## 10. Common Pitfalls

| Pitfall | Symptom | Fix |
|---------|---------|-----|
| Adding a field to a public struct | "Why is my downstream code broken?" | New type, not a new field. Or major-version bump. |
| Using `println!` in library code | clippy deny | Use `tracing::info!` etc. |
| `unwrap()` "just this once" | Clippy deny + flaky tests | `expect()` with invariant comment, or `?` with a proper error variant. |
| Silent API change in renderer | Render tests fail with no obvious reason | Snapshot test will catch it; update golden only if intentional, and add a `render-diff` note in CHANGELOG. |
| Reading `World` from the render thread | Borrow conflict | Use `FrameSnapshot` instead; renderer is read-only on the world. |
| Asset DB in parallel tests | "database is locked" | `#[serial_test::serial]` on the test, or `cargo test -- --test-threads=1`. |
| Skipping `miri` review on `unsafe` | Future bug | Add a one-line `// MIRI:` comment explaining why this `unsafe` is sound. |
| Changing a phase DoD without an ADR | Drift from `architecture.md` | Write `docs/adr/NNNN-<title>.md` first; reference in PR. |
| Forgetting to update `roadmap.toml` after merge | Roadmap stale | Either include the status change in the same PR, or follow up immediately. |

---

## 11. Communication Norms

- **Status updates:** in the PR description, in `roadmap.toml` `blocker = "..."` lines, and in the milestone tracking issue.
- **Architectural questions:** propose as an ADR PR (`docs/adr/NNNN-title.md`); do not change `architecture.md` directly without an accepted ADR.
- **Blockers:** if you cannot complete an item in the planned time, mark it `blocked` in `roadmap.toml` and add a `blocker = "..."` line. Open an issue if the blocker is structural.
- **Regressions:** if a previously-complete item regresses, mark it `in_progress` with `regression_note = "..."` and link the issue.

---

## 12. What This File Does *Not* Cover

- **Tutorial-style onboarding.** See `docs/mdbook/` (built in M7).
- **API reference.** See rustdoc (`cargo doc --open`).
- **Performance tuning.** See `docs/perf/baseline.json` (M7) and the `tracy` integration guide (M6).
- **Release process.** See `docs/architecture.md` §15 + the `release/v0.1.0` branch runbook (forthcoming, M7).

For anything not covered here, **default to the principle in `architecture.md` §3: "Crate boundaries are API contracts."** If you are about to cross a crate boundary, change a public type, or alter a schedule, you are probably about to need an ADR.

---

*End of AGENTS.md. Implementation is tracked in `docs/roadmap.toml`. Architecture is in `docs/architecture.md`.*
