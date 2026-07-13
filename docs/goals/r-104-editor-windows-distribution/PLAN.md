# R-104 — PySide6/QML packaging and Windows distribution

**Intent:** Produce a reproducible standalone Windows editor package.
**Current Behavior:** Source QML is loaded from the checkout and `hyge-tools`
always launches the frontend through `python`.
**Expected Outcome:** A relocatable package contains the frozen PySide6/QML
frontend, Qt runtime, QML resources, `hyge-tools.exe` and a relative launcher.
**Target-Perspective Output:** A clean Windows directory launches the real
R-103 fixture, connects to the Rust service and retains protocol, screenshot
and scene evidence without Python or PATH assumptions.
**Truth Owner:** Rust `hyge-tools`/`hyge-editor` owns runtime state;
Python/QML remains a protocol client.
**Contract Boundary:** Package launcher → `hyge-tools.exe` → frozen PySide6
frontend → existing versioned loopback protocol.
**Cutover:** `.exe` frontends execute directly; source `.py` launch remains for
development and existing R-103 CI.
**Displaced Path:** Checkout-relative QML and literal `python` for packaged
execution.
**Acceptance Evidence:** Clean-venv build, standalone deploy output, package
manifest and clean-machine R-103 smoke evidence.
**Evidence Lane:** `target/editor-windows/` and `target/r104-evidence/`.
**Kill Criteria:** No second runtime owner, no packaged PATH lookup, no source
checkout dependency, and no completion without smoke proof.

## Ordered tasks

1. Add QRC-backed QML loading while retaining source fallback.
2. Add the checked-in PySide6 project and standalone deployment spec.
3. Launch `.exe` frontends directly and test both modes.
4. Assemble the package with the Rust binary, launcher and manifest.
5. Run the packaged smoke against the checked-in R-103 fixture.
6. Record evidence in the roadmap and Windows CI artifact.
