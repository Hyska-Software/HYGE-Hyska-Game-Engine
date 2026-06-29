//! Hyge app: assembles every plugin and runs the engine.
//!
//! Provides the `AppBuilder` trait, `AppConfig`, and the `default_plugins()`
//! function that returns the canonical set of subsystem plugins. The run loop
//! drives the `First` / `PreUpdate` / `FixedUpdate` / `Update` / `RenderExtract`
//! / `Last` schedules and sends `FrameSnapshot`s to the render thread.
//!
//! See `docs/architecture.md` §6.12 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-013..R-014.
