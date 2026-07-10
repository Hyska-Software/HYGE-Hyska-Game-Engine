# ADR-0013: Full Visual Editor in v0.1

- **Status:** Superseded by [ADR-0015](0015-python-qt-editor.md)
- **Date:** 2026-06-29
- **Superseded:** 2026-07-10

This ADR originally selected an in-process `egui` editor. That implementation
was never started. The v0.1 editor requirement remains, but the UI technology,
process boundary and ownership model are replaced by ADR-0015:

- Rust owns the engine/editor state and command execution.
- PySide6 + Qt Quick/QML owns the desktop presentation.
- A versioned IPC protocol connects the two processes.
- The renderer remains the source of viewport frames.
