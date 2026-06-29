# Hyge — Crate Ownership Map

> **Status:** Accepted
> **Last updated:** 2026-06-29
> **Related:** `AGENTS.md` §6.3 (PR rules), `docs/roadmap.toml` R-002

## 1. Purpose

This document is the contract for:

- **Who reviews PRs** that touch a given crate.
- **Who can self-merge** changes within a crate.
- **How to route** cross-crate changes.
- **How to escalate** decisions that cross group boundaries.

Every crate in the Hyge workspace has a **primary owner group**, a **named lead**, and **at least one named delegate** (the acceptance criterion of `roadmap.toml` R-002). This map is read at PR-routing time; the `ecosystem` group is the only group that can modify the meta-rules in §6.

## 2. Owner Groups

A **group** is a unit of responsibility that owns one or more crates. Groups are stable across the v0.x line; the `ecosystem` group is the only group that can create, rename, split, or dissolve a group.

| Group ID | Scope | Responsibilities | Reviews |
|----------|-------|------------------|---------|
| `core` | `hyge-core` | Math (`glam` wrappers), color spaces, log (`tracing`) init, asset id, error types. The foundation everyone else builds on. | Any change to public types; pedantic. |
| `ecs` | `hyge-ecs` | `bevy_ecs` re-exports, `HygePlugin` trait, `Schedule`s, `SystemSet`s. The only crate that names `bevy_ecs`. | High — API ripples across all crates. |
| `render-graph` | `hyge-render-graph` | DAG, `Pass` trait, barrier inference, transient allocator, `CompiledGraph`. Used by `hyge-render`. | The only `unsafe`-gated crate besides `hyge-render`; miri-clean. |
| `render` | `hyge-render` | `wgpu` device, `BindlessTable`, clustered forward, post-process, shaders, frame snapshot producer. The other `unsafe`-gated crate. | WGSL + Rust; passes snapshot tests. |
| `asset` | `hyge-asset` | `Asset` trait, `Handle`, `AssetId`, `AssetDb` (SQLite), `FileWatcher` (`notify`), `GpuUploadTask`. | DB round-trips + serial tests. |
| `scene` | `hyge-scene` | glTF import, meshlet bake, `.hyge-world`/`.hyge-prefab` formats, `Prefab` system, component catalog, `RenderExtract` for the renderer. | Reflect-driven; round-trips; deterministic bake. |
| `physics` | `hyge-physics` | `rapier3d` wrapper, fixed-timestep, character controller, spatial queries, collision events. Feature-flag-gated. | Determinism at fixed timestep. |
| `audio` | `hyge-audio` | `kira` backend, bus mixer, spatial 3D, HRTF (feature-flag-gated). | Bus graph; mock-only tests in CI. |
| `window` | `hyge-window` | `winit` event loop, surface, raw input (Windows), `Window` resource, `DeviceEvent`s. | Event translation; Windows-only tests behind cfg. |
| `input` | `hyge-input` | `Action<T>`, TOML binding parser, hot-reload, edge detection. | TOML schema stable; hot-reload idempotent. |
| `script` | `hyge-script` | `mlua` engine, sandbox, `bevy_reflect` bindings, hot-reload, `on_reload` hook. | Sandbox enforced; Lua FFI overhead profiled. |
| `app` | `hyge-app` | `AppBuilder`, `AppConfig`, `default_plugins`. The only crate that knows the full graph. | Wiring; minimal logic. |
| `editor` | `hyge-editor` | `egui` panels (viewport, hierarchy, inspector, content browser, asset graph, console, profiler), undo/redo, docking, theme. | Reflect-driven inspector correctness. |
| `tools` | `hyge-tools` | CLI (`import`, `cook`, `headless`, `inspect`, `serve`, `doctor`). | Golden output for known inputs. |
| `tooling` | `hyge-runtime-test` | Test harness: `create_test_renderer`, `capture_frame`, `assert_image_matches`, `hash_image`. | Used by every crate that needs GPU tests. |
| `examples` | `examples/hyge-sandbox` | The end-to-end demo. Smoke-tests the whole engine. | Builds clean; runs without warnings. |
| `docs` | `docs/`, `AGENTS.md`, `README.md`, `docs/mdbook/` | User-facing tutorials, API reference, governance docs. | mdbook builds; links valid. |
| `build` | root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.github/`, `clippy.toml`, `rustfmt.toml` | Build system, CI, MSRV, toolchain. | CI green on Windows + Linux. |
| `release` | `CHANGELOG.md`, git tags, GitHub releases | Release process, semver, changelog. | Tag format; release notes. |
| `ecosystem` | This document, `docs/architecture.md`, `docs/roadmap.toml`, `docs/adr/` | Top-level governance, ADR review, roadmap stewardship, ownership arbitration. | The tie-breaker group. |

## 3. Per-Crate Ownership

Each row is a binding: the group is the owner; the lead is the primary point of contact; the delegates are the named fallbacks. Names below are **role identifiers** (`<crate-or-group>-<role>`); real names (e.g. GitHub handles) are filled in as the team grows — see §7 "How to populate names" for the procedure.

| Crate / Area | Primary Group | Lead | Delegates | Escalation |
|--------------|---------------|------|-----------|------------|
| `hyge-core` | core | `core-lead` | `core-d1`, `core-d2` | ecosystem |
| `hyge-ecs` | ecs | `ecs-lead` | `ecs-d1`, `ecs-d2` | ecosystem |
| `hyge-render-graph` | render-graph | `render-graph-lead` | `render-graph-d1`, `render-graph-d2` | ecosystem |
| `hyge-render` | render | `render-lead` | `render-d1`, `render-d2` | ecosystem |
| `hyge-asset` | asset | `asset-lead` | `asset-d1`, `asset-d2` | ecosystem |
| `hyge-scene` | scene | `scene-lead` | `scene-d1`, `scene-d2` | ecosystem |
| `hyge-physics` | physics | `physics-lead` | `physics-d1`, `physics-d2` | ecosystem |
| `hyge-audio` | audio | `audio-lead` | `audio-d1`, `audio-d2` | ecosystem |
| `hyge-window` | window | `window-lead` | `window-d1`, `window-d2` | ecosystem |
| `hyge-input` | input | `input-lead` | `input-d1`, `input-d2` | ecosystem |
| `hyge-script` | script | `script-lead` | `script-d1`, `script-d2` | ecosystem |
| `hyge-app` | app | `app-lead` | `app-d1`, `app-d2` | ecosystem |
| `hyge-editor` | editor | `editor-lead` | `editor-d1`, `editor-d2` | ecosystem |
| `hyge-tools` | tools | `tools-lead` | `tools-d1`, `tools-d2` | ecosystem |
| `hyge-runtime-test` | tooling | `tooling-lead` | `tooling-d1`, `tooling-d2` | ecosystem |
| `examples/hyge-sandbox` | examples | `examples-lead` | `examples-d1`, `examples-d2` | ecosystem |
| `docs/`, `AGENTS.md`, `README.md` | docs | `docs-lead` | `docs-d1`, `docs-d2` | ecosystem |
| root build files, CI, toolchain | build | `build-lead` | `build-d1`, `build-d2` | ecosystem |
| CHANGELOG, tags, releases | release | `release-lead` | `release-d1`, `release-d2` | ecosystem |
| `docs/architecture.md`, `docs/roadmap.toml`, `docs/adr/`, this file | ecosystem | `ecosystem-lead` | `ecosystem-d1`, `ecosystem-d2` | (n/a — last resort) |

> **Coverage check (R-002 acceptance #2 + #3):** every one of the 15 implementation crates (14 workspace + `hyge-runtime-test`) has a primary owner group, a named lead, and at least one named delegate. ✓

## 4. PR Routing Rules

A PR is routed by the set of files it touches. The routing determines the **set of required reviewers** and the **number of approvals needed**.

### 4.1 Single-crate PR

- **Definition:** touches files only under a single `crates/<name>/` directory (or `examples/<name>/` for examples).
- **Route to:** the primary owner group's lead and delegates for that crate.
- **Required approvals:** 1.
- **Self-merge allowed?** Yes, if the author is a delegate of that crate AND the author is not the sole reviewer of their own PR (CODEOWNERS handles this).

### 4.2 Multi-crate PR

- **Definition:** touches files under 2+ `crates/<name>/` directories.
- **Route to:** the union of the affected groups' leads and delegates.
- **Required approvals:** 1 from each affected group.
- **Self-merge allowed?** No — at least one other human must approve.

### 4.3 Cross-cutting PR (root, CI, toolchain)

- **Definition:** touches root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.github/`, `clippy.toml`, `rustfmt.toml`, or any workspace-level file.
- **Route to:** the `build` group + every group whose crate is affected (transitively).
- **Required approvals:** 1 from `build` + 1 from each affected group.
- **Self-merge allowed?** No.

### 4.4 Documentation / governance PR

- **Definition:** touches `docs/`, `AGENTS.md`, `README.md`, `docs/mdbook/`, `docs/architecture.md`, `docs/roadmap.toml`, `docs/adr/`, or this file.
- **Route to:** the group that owns the area (see §3) + `ecosystem` for any meta-rule change.
- **Required approvals:** 1 from the owning group. Meta-rule changes require 2 (one of which is `ecosystem`).
- **Self-merge allowed?** Typo fixes and broken links: yes. Anything else: no.

### 4.5 Emergency / revert / hotfix PR

- **Definition:** explicitly labeled `hotfix` or `revert`; or a CI is red on `main`.
- **Route to:** the on-call (rotating, see §5).
- **Required approvals:** 1.
- **Self-merge allowed?** Yes, with on-call approval.
- **Follow-up required:** post-mortem within 48h; fix PR within 1 week.

### 4.6 ADR PR

- **Definition:** a new file under `docs/adr/` or a modification to an `Accepted` ADR.
- **Route to:** the `ecosystem` group.
- **Required approvals:** 2.
- **Self-merge allowed?** Never.
- **Superseding an `Accepted` ADR** is allowed only via a new ADR that explicitly references the one it supersedes.

## 5. Code Review Expectations

- **Response time:** reviewers respond within **1 business day** for normal PRs, **4 hours** for `priority = P0` items in `docs/roadmap.toml`. If a reviewer is OOO, they hand off to a delegate in the PR.
- **Scope of review:** correctness, safety, design, tests, docs. **Not style** — style is enforced by `cargo fmt` and `cargo clippy`.
- **Approval semantics:** "I would be comfortable shipping this PR as-is" — not "looks plausible".
- **Blocking comments:** explicit "request changes" is required to block. Comment-only reviews do not block; they are advisory.
- **Stale PRs:** PRs with no activity for 14 days are auto-closed; the author can reopen when ready.

## 6. Escalation Path

When reviewer and author disagree and cannot resolve:

1. The crate's primary owner group is consulted. The group's lead is the tie-breaker for that crate.
2. If still unresolved, or if the disagreement is cross-crate, the `ecosystem` group arbitrates.
3. If the disagreement is a design question (not a "is this right" question), the ADR process is invoked: write a new ADR or supersede an existing one.
4. ADRs are the final word for design questions. The `ecosystem` group does not override ADRs; it arbitrates procedural disputes.

## 7. How to Populate Names (Roles → Real People)

The names in §3 are role identifiers. To bind a real person to a role:

1. Open a PR against this file.
2. Replace the role identifier in the relevant row(s) with the person's GitHub handle (e.g. `@alice`) and a real name (e.g. `Alice Example`).
3. The PR must be approved by the `ecosystem` group AND by the person being added (consent).
4. The same person can hold multiple roles; the table makes the allocation explicit so it can be rebalanced.

**Conventions:**

- One person should be the **lead** of no more than 2 crates (so they have time to actually lead).
- One person can be a **delegate** of any number of crates.
- A group with no assigned humans is **unowned**; the `ecosystem` group treats this as a `P0` risk.

## 8. On-Call Rotation

The on-call role is the safety net for §4.5 (emergency / revert / hotfix). For v0.1:

- **On-call primary:** `release-lead` (since release issues are the most common emergency).
- **On-call delegate:** `build-lead` (since CI is the second most common emergency).
- **Rotation period:** weekly.
- **Out of hours:** the on-call is reachable via the issue tracker; non-emergency PRs wait.

The on-call rotation is the only `release` group responsibility that is not strictly "release-shaped". It exists so that a real human is always reachable when a PR needs §4.5 treatment.

## 9. Updating This Document

When ownership changes (new delegate, new crate, new group, lead swap):

- PR modifies this file.
- Reviewers: `ecosystem` group (consent of the affected group + the person being added).
- The corresponding `docs/roadmap.toml` row is updated in the same PR if the change is structural (new crate, new group, group split/merge).
- The `last updated` line at the top of this file is bumped on every merge.

**Forbidden changes** (require an ADR):

- Renaming a group.
- Changing the PR routing algorithm in §4.
- Changing the escalation path in §6.

## 10. Summary Checklist (for the `ecosystem` group)

- [ ] Every crate in the workspace has a lead and ≥1 delegate in §3.
- [ ] No group is unowned.
- [ ] No human holds >2 leads.
- [ ] On-call rotation is staffed for the next 4 weeks.
- [ ] This document was updated within the last 90 days.

If any of the above is false, the `ecosystem` group is `blocked` and the issue is raised to P0.

---

*End of `docs/ownership.md`. Implementation of governance: see `docs/roadmap.toml` `phase_0` items R-001, R-002, R-003.*
