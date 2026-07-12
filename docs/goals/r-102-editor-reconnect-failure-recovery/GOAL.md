# Goal: R-102 Editor Recovery and Shutdown

Execute the R-102 implementation in `PLAN.md`.

The Rust editor service remains the truth owner for session generations,
runtime state, project/scene errors, viewport transport and teardown. The
PySide6 frontend is a disposable protocol client: retry transient failures,
refresh authoritative snapshots after reconnect, and never replay mutations
that may have been accepted by the previous backend generation.

Completion requires focused Rust/Python tests plus real TCP shutdown,
generation/reconnect, project-error and transport cleanup evidence. If a
backend kill/restart or resource-release scenario cannot be demonstrated,
report the feature as implemented but unproven.
