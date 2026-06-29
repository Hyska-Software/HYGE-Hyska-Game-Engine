# ADR-0011: winit + gilrs + Raw Input + TOML Bindings

- **Status:** Accepted
- **Date:** 2026-06-29
- **Deciders:** Hyge core team
- **Supersedes:** —
- **Superseded by:** —

## Context

The engine must interact with the platform (window, input devices) and let users remap their controls without recompiling.

The platform layer must:

- Open a window and own a `wgpu::Surface` for the renderer.
- Provide raw, high-precision mouse delta (no OS-level cursor acceleration bleed-through).
- Support keyboard, mouse, and gamepads uniformly through an `Action<T>` abstraction.
- Hot-reload control bindings.

## Decision

Adopt:

- **`winit` 0.30+** for the event loop and window management.
- **`gilrs` 0.10+** for gamepad (cross-platform: XInput on Windows, evdev on Linux, GCController on macOS).
- **`RegisterRawInputDevices`** (Windows) for high-precision mouse and keyboard, with an `evdev` backend on Linux behind the same `RawInput` trait (for future parity).
- **`Action<T>` abstraction** with bindings stored in TOML (`assets/input.bind.toml`) and hot-reloaded via `notify`.

## Consequences

### Positive

- **`winit` is the de-facto standard** for cross-platform Rust windowing; it ships with `wgpu` examples.
- **Raw input on Windows** gives true mouse-delta precision for FPS-style controls; not subject to OS cursor acceleration.
- **`gilrs` covers all major platforms** for gamepads without per-platform code.
- **TOML bindings + hot-reload** means users can iterate on controls without restarting.
- **Action abstraction** decouples game code from device identity (`hyge.input.action("move")` works for keyboard, mouse, and gamepad interchangeably).

### Negative

- Windows raw input adds ~500 lines of `windows-sys` code behind `#[cfg(windows)]`; test surface is Windows-only in CI.
- Linux raw input requires `evdev` direct access (root or `uinput` group), which is not portable; we ship the trait and document the requirement.
- `winit`'s API is still evolving; we pin a version and use `bevy_window` patterns where they help.

## Alternatives Considered

### SDL2 via `sdl2` crate

- **Pros:** Mature; very portable; full input handling built in.
- **Cons:** C dependency; less idiomatic in Rust; non-trivial `build.rs`.
- **Rejected because:** the C dependency contradicts the "pure Rust" principle; `winit` + `gilrs` covers our needs.

### Custom event loop

- **Pros:** Total control.
- **Cons:** Reinvents what `winit` provides; years of edge cases (HiDPI, multi-monitor, accessibility, IME).
- **Rejected because:** the cost is huge and `winit` is well-maintained.

### Per-device code without `Action<T>`

- **Pros:** Simpler.
- **Cons:** Game code couples to device identity; remap story becomes painful.
- **Rejected because:** the Action layer is the user-facing contract; without it, the engine is unfriendly to game developers.

## References

- `docs/architecture.md` §6.9 (hyge-window), §6.10 (hyge-input)
- `winit` documentation: <https://docs.rs/winit>
- `gilrs` documentation: <https://docs.rs/gilrs>
- `RegisterRawInputDevices` (Windows docs)
- ADR-0013 (editor) — the editor uses the same Action map for editor controls
