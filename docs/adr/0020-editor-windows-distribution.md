# ADR-0020: Standalone Windows Editor Distribution

- **Status:** Accepted
- **Date:** 2026-07-13
- **Scope:** `tools/hyge-editor-python`, `hyge-tools`, Windows packaging

## Context

The external editor is a Rust service plus a PySide6/QML client. Source
execution currently launches the client through the developer's `python`
command and resolves QML relative to the repository. That is unsuitable for
a Windows user who has only the distributed editor package.

## Decision

Distribute the editor as a relocatable standalone directory containing the
frozen `HygeEditor.exe`, Qt libraries/plugins/QML resources, a package-relative
`bin/hyge-tools.exe`, and `HygeEditor.cmd` as the entrypoint.

`hyge-tools` remains the process owner: a `.py` frontend is launched through
the source-development Python path, while a `.exe` frontend is executed
directly. The packaged frontend loads `qrc:/qml/Main.qml`; source mode keeps
the filesystem fallback for tests and development.

The package launcher resolves every executable relative to its own directory.
No Python installation, repository checkout, cargo installation or PATH entry
is required at runtime.

`HygeEditor.cmd` is the end-user entrypoint. When no project argument is
provided it uses the Windows-native folder selector, validates that the chosen
directory contains a `.hyge-world`, and starts the backend with port `0` so
parallel editor sessions do not collide. `HygeEditor.exe` remains an internal
frontend child and reports the launcher instruction if opened by itself.

## Consequences

- Packaging is reproducible through `pyside6-project`, `pyside6-deploy` and a
  checked-in deployment specification.
- The package is a directory rather than a onefile executable so Qt plugins
  and the Rust backend remain inspectable and relocatable.
- Clean-machine smoke tests use the packaged binary and R-103 fixture, not a
  fake backend or source-tree frontend.
- Updating Python, PySide6, Qt modules or package layout requires updating the
  deployment spec and package evidence.
