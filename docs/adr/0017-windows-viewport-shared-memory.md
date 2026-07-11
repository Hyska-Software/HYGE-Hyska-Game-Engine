# ADR-0017: Windows Viewport Shared-Memory Transport

- **Status:** Accepted
- **Date:** 2026-07-11
- **Deciders:** Hyge core team

## Context

The external PySide6 editor needs to consume full RGBA viewport frames without
placing image payloads on the authenticated JSON control connection.  The
workspace otherwise forbids `unsafe` outside renderer crates, while Windows
named file mappings require a narrow FFI boundary.

## Decision

R-088 introduces `hyge-editor-shm`, a small Windows-oriented crate that owns
only named mapping handles and byte access. It is the sole exception for the
documented Win32 mapping FFI. `hyge-editor` retains `#![forbid(unsafe_code)]`
and owns the versioned ring ABI, session lifecycle, input validation, and
control messages.

Frames use a three-slot, little-endian RGBA8 sRGB ring. The producer commits
pixels before publishing an even sequence number; consumers copy only a slot
whose header and trailer agree. A transport generation changes on reconnect or
capacity-changing resize, so a stale mapping cannot be reused as a live one.

## Consequences

- Windows 10/11 is the functional target for this transport in v0.1.
- TCP remains control-only; frame bytes never cross the JSON protocol.
- A later GPU-native transport can replace this crate behind the same editor
  transport contract.
