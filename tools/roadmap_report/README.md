# hyge-roadmap-report

A standalone Rust binary that reads `docs/roadmap.toml` and emits phase-grouped progress reports.

Implements roadmap item **R-003** (see `docs/roadmap.toml`).

## Why standalone

This tool serves engine **maintainers**, not engine **users**. It lives outside the main `hyge-*` workspace to keep the engine's build graph clean. It is added to the workspace by R-004 (or any later item) when appropriate.

## Usage

```bash
# Build (from the tool directory)
cd tools/roadmap_report
cargo build --release

# Run on the workspace roadmap (default: docs/roadmap.toml)
./target/release/hyge-roadmap-report
# or, from the workspace root:
cargo run --release -p hyge-roadmap-report --manifest-path tools/roadmap_report/Cargo.toml

# JSON output for CI / dashboards
./target/release/hyge-roadmap-report --format json

# Strict: fail with non-zero exit if validation errors are found
./target/release/hyge-roadmap-report --strict

# Quiet: only emit validation issues, no progress report (CI gating)
./target/release/hyge-roadmap-report --strict --quiet
```

## Output

### Human (default)

```
Hyge Engine Implementation Roadmap
Updated: 2026-06-29T13:42Z

phase_0  Governance and Foundation                    2/8 complete   25.0%
phase_1  ECS Core and App Skeleton                    0/5 not_started 0.0%
...

Total: 2/76 complete (2.6%)

By status:
  complete:    2
  in_progress: 0
  blocked:     0
  not_started: 74
```

The phase `primary status` column reports the dominant status of items in the phase: `complete` if all are done, `blocked` if any are blocked, `in_progress` if any are in progress, otherwise `not_started`. `no_items` is reported for empty phases.

### JSON

```json
{
  "title": "Hyge Engine Implementation Roadmap",
  "updated_at": "2026-06-29T13:42Z",
  "phases": [
    {
      "id": "phase_0",
      "name": "Governance and Foundation",
      "goal": "...",
      "total": 8,
      "complete": 2,
      "in_progress": 0,
      "blocked": 0,
      "not_started": 6,
      "percent_complete": 25.0
    }
  ],
  "totals": {
    "items": 76,
    "complete": 2,
    "in_progress": 0,
    "blocked": 0,
    "not_started": 74,
    "percent_complete": 2.63
  }
}
```

## Schema validation

The tool validates the v1 schema of `roadmap.toml` and emits `ValidationIssue`s with `severity: Error | Warning`.

**Errors** (cause `--strict` to fail):

- `version != 1`
- `status_scale` missing any of `not_started`, `in_progress`, `blocked`, `complete`
- Duplicate phase IDs
- Duplicate item IDs
- Item references an unknown phase
- Item `status` / `priority` / `risk` is not in the corresponding scale
- Item has an empty `owner`
- Item depends on itself
- Item depends on an unknown item

**Warnings** (informational, never fail):

- `priority_scale` missing any of `P0`, `P1`, `P2`, `P3`
- `risk_scale` missing any of `low`, `medium`, `high`

## CI

`.github/workflows/roadmap-report.yml` runs the tool on every PR and on push to `main`, fails on validation errors, and posts the report as a comment.

- PR comment: sticky (one comment per PR, updated on each run) via `marocchino/sticky-pull-request-comment`.
- Milestone tracking issue comment: weekly via cron, posted to the issue whose number is stored in the `HYGE_ROADMAP_MILESTONE_ISSUE` repository variable (silently no-op if the variable is unset).

## Library API

The tool is a thin wrapper around the `hyge_roadmap_report` library. Reuse it for dashboards, CI, or any other roadmap-aware tool:

```rust
use hyge_roadmap_report::{parse_file, validate, Report};

let roadmap = parse_file("docs/roadmap.toml")?;
let issues = validate(&roadmap);
let report = Report::from_roadmap(&roadmap);

if issues.iter().any(|i| i.severity == hyge_roadmap_report::Severity::Error) {
    // fail CI
}
```

## Adding to the workspace

When ready to add to the main `hyge-*` workspace, add this path to `[workspace] members` in the root `Cargo.toml`:

```toml
[workspace]
members = [
    "crates/*",
    "examples/*",
    "tools/roadmap_report",
]
```

The `hyge_roadmap_report` library is forward-compatible with the v1 schema: new optional fields in `roadmap.toml` are ignored, so future ADR-driven changes to the schema do not break this tool.

## Testing

```bash
cd tools/roadmap_report
cargo test
```

Test fixtures live in `tests/data/` and cover valid roadmaps, duplicate phase IDs, bad priorities, unknown phase references, self-dependencies, and unknown dependency references.
