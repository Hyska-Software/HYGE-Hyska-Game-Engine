//! Hyge editor: the full visual editor built on `egui` + `egui_dock`.
//!
//! Panels: viewport (running the same render graph as runtime), hierarchy
//! (tree view + drag-drop reparent), inspector (reflect-driven component
//! editor), content browser (file tree + thumbnails), asset graph, console
//! (`tracing` ring buffer), profiler (frame stats sparkline), undo/redo.
//!
//! See `docs/architecture.md` §6.13 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-080..R-089.
