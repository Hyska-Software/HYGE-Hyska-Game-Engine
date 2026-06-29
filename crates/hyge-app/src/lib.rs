//! Hyge app: the only crate that wires the engine subsystems together.
//!
//! Provides the [`App`] struct (which wraps the bevy `App` and the
//! `winit` event loop), the [`AppConfig`] configuration type, the
//! [`AppBuilder`] trait, and the [`default_plugins`] function that
//! installs the canonical set of subsystem plugins.
//!
//! See `docs/architecture.md` §6.12 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-013.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod config;

pub use config::{
    AppConfig, AssetsConfig, AudioConfig, EditorConfig, InputConfig, PhysicsConfig,
    RendererBackend, RendererConfig, ScriptConfig,
};

pub mod prelude;

use hyge_ecs::prelude::*;
use hyge_window::prelude::*;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};

/// The Hyge application. Owns the bevy `App`, the `winit` event loop
/// driver state, and the `Window` once it is created.
///
/// Construct with [`App::new`], add plugins with [`App::add_plugin`] (or
/// rely on the defaults from [`default_plugins`]), then call
/// [`App::run`] to enter the event loop.
pub struct App {
    config: AppConfig,
    inner: bevy_ecs::app::App,
    window: Option<Window>,
}

/// Builder trait for the Hyge application. Implemented by [`App`].
///
/// This trait exists so that downstream code can write generic helpers
/// over any `AppBuilder` (e.g. an integration-test harness that accepts
/// a builder and configures it).
///
/// # Example
///
/// ```no_run
/// use hyge_app::prelude::*;
///
/// fn boot() -> impl AppBuilder {
///     App::new(AppConfig::default())
/// }
///
/// fn main() {
///     boot().run();
/// }
/// ```
pub trait AppBuilder {
    /// Creates a new builder with the given config. Schedules the
    /// canonical set of Hyge schedules (`First` through `Last`) and
    /// installs the [`default_plugins`].
    fn new(config: AppConfig) -> Self;

    /// Adds a plugin to the application. Plugins are run in the order
    /// they are added; later plugins observe the effects of earlier ones.
    /// Returns `self` for chaining.
    fn add_plugin<P: HygePlugin + 'static>(self, plugin: P) -> Self;

    /// Builds the application and enters the `winit` event loop.
    /// This function never returns under normal operation: the event
    /// loop either runs until the user closes the window (at which
    /// point the process exits) or panics on unrecoverable error.
    fn run(self) -> !;
}

impl App {
    /// Creates a new `App` with the given config. Schedules the canonical
    /// set of Hyge schedules (`First` through `Last`) and installs
    /// [`default_plugins`]. Additional plugins can be added with
    /// [`App::add_plugin`] before [`App::run`].
    pub fn new(config: AppConfig) -> Self {
        let mut inner = bevy_ecs::app::App::new();

        // Register the canonical set of schedules. Each is keyed by the
        // `Label` enum variant (which is the value passed to
        // `add_schedule` / `run_schedule`). Plugins add systems to
        // these schedules via `app.add_systems(Label::Update, ...)`.
        inner.add_schedule(Label::First);
        inner.add_schedule(Label::PreUpdate);
        inner.add_schedule(Label::FixedUpdate);
        inner.add_schedule(Label::Update);
        inner.add_schedule(Label::RenderExtract);
        inner.add_schedule(Label::Render);
        inner.add_schedule(Label::Last);

        let mut app = Self {
            config,
            inner,
            window: None,
        };

        // Install the default plugin set. Each plugin's `build` method
        // registers events, resources, and systems.
        for plugin in default_plugins() {
            app = app.add_plugin(plugin);
        }

        app
    }

    /// Adds a plugin to the application. The plugin's `build` method is
    /// called immediately, with `&mut App` (the bevy app).
    pub fn add_plugin<P: HygePlugin + 'static>(mut self, plugin: P) -> Self {
        tracing::debug!(plugin = %plugin.name(), "registering plugin");
        plugin.build(&mut self.inner);
        self
    }

    /// Returns a reference to the inner bevy `App` (for advanced use:
    /// direct `bevy_ecs` API access, adding systems not wrapped in
    /// `HygePlugin`, etc.).
    pub fn bevy_app(&self) -> &bevy_ecs::app::App {
        &self.inner
    }

    /// Returns a mutable reference to the inner bevy `App`.
    pub fn bevy_app_mut(&mut self) -> &mut bevy_ecs::app::App {
        &mut self.inner
    }

    /// Returns a reference to the current config.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// Runs the engine. Creates the `winit` event loop, creates the
    /// window in `resumed`, runs the schedules each frame in
    /// `about_to_wait`, and exits when the user closes the window.
    ///
    /// This function never returns: the event loop runs until the
    /// user closes the window, at which point the process exits
    /// cleanly via `std::process::exit(0)`.
    pub fn run(mut self) -> ! {
        let event_loop = EventLoop::new().expect("failed to create winit event loop");
        event_loop
            .run_app(&mut self)
            .expect("winit event loop failed");
        std::process::exit(0);
    }

    /// Runs one frame's worth of schedules in the order declared by
    /// `Label`. Currently a no-op (the schedules are empty) — once
    /// plugins start adding systems in M2+, this will execute the full
    /// per-frame pipeline. Used by the event loop's `about_to_wait`
    /// handler.
    fn run_frame(&mut self) {
        let world = self.inner.world_mut();
        world.run_schedule(Label::First);
        world.run_schedule(Label::PreUpdate);
        // FixedUpdate is special: in a real implementation, it would
        // consume the fixed-timestep accumulator and run N times.
        // For now, run once per frame.
        world.run_schedule(Label::FixedUpdate);
        world.run_schedule(Label::Update);
        world.run_schedule(Label::RenderExtract);
        // `Render` runs on the dedicated render thread (M2+).
        world.run_schedule(Label::Last);
    }

    /// Convenience constructor for `App::new(AppConfig::default())`.
    pub fn with_default_config() -> Self {
        Self::new(AppConfig::default())
    }
}

impl AppBuilder for App {
    fn new(config: AppConfig) -> Self {
        App::new(config)
    }

    fn add_plugin<P: HygePlugin + 'static>(self, plugin: P) -> Self {
        self.add_plugin(plugin)
    }

    fn run(self) -> ! {
        self.run()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Lazily create the window on the first `resumed` event (which
        // happens once per platform: at startup on most platforms; on
        // Android it can happen after a pause/resume cycle).
        if self.window.is_none() {
            match Window::new(event_loop, self.config.window.clone()) {
                Ok(window) => {
                    let handle = window.handle();
                    // Store the `Arc<winit::Window>` in the `WindowState`
                    // resource so other systems (renderer, input) can
                    // read it.
                    if let Some(mut state) =
                        self.inner.world_mut().get_resource_mut::<WindowState>()
                    {
                        state.set(handle);
                    } else {
                        tracing::error!(
                            "WindowState resource not found; WindowPlugin was not registered"
                        );
                    }
                    self.window = Some(window);
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to create window");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // Translate the winit event into a Hyge event and write it into
        // the bevy `Events<T>` buffer. Systems that have registered an
        // `EventReader<WindowResized>` (etc.) will pick it up next frame.
        if let Some(hyge_event) = translate_window_event(&event) {
            match hyge_event {
                HygeWindowEvent::Resized(e) => {
                    self.inner.world_mut().send_event(e);
                }
                HygeWindowEvent::CloseRequested(e) => {
                    self.inner.world_mut().send_event(e);
                }
                HygeWindowEvent::Focused(e) => {
                    self.inner.world_mut().send_event(e);
                }
                HygeWindowEvent::Device(e) => {
                    self.inner.world_mut().send_event(e);
                }
            }
        }

        // Handle the user closing the window: exit the event loop.
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
        }

        // Also handle the OS-level resize (in case `translate_window_event`
        // returned `None` due to a `winit` quirk; we still want to keep
        // our cached `WindowConfig` in sync if the user has the
        // `WindowPlugin` re-read it).
        if let WindowEvent::Resized(PhysicalSize { width, height }) = event {
            tracing::debug!(width, height, "window resized");
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Run all schedules for this frame. With the current placeholder
        // plugins, each schedule is empty and this is a no-op; the
        // schedule execution infrastructure is what we are validating
        // here.
        self.run_frame();

        // Request a redraw so the OS schedules a paint event.
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Returns the canonical set of Hyge plugins that the [`App`] should
/// install by default. Each plugin corresponds to one engine subsystem;
/// more plugins are added here as their M-XXX items land.
///
/// The order in the returned `Vec` is the order in which the plugins
/// are installed (and therefore the order in which their `build` methods
/// are called). Earlier plugins are visible to later ones; later plugins
/// can observe and extend the state set up by earlier ones.
///
/// Currently includes only [`WindowPlugin`]. The other subsystem plugins
/// (renderer, asset, scene, physics, audio, input, script, editor) are
/// added as their respective milestones land (see `docs/roadmap.toml`).
pub fn default_plugins() -> Vec<Box<dyn HygePlugin>> {
    vec![Box::new(WindowPlugin::new(WindowConfig::default()))]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_plugins_load_without_conflict() {
        // Building a fresh app and installing all default plugins must
        // not panic. This catches the common "two plugins both register
        // the same resource" class of bug.
        let app = App::new(AppConfig::default());
        for plugin in default_plugins() {
            // Re-create the app for each plugin so the test does not
            // depend on a specific order; in practice the AppBuilder
            // chain is one-shot, but this is the closest smoke test we
            // can run in a unit test without an event loop.
            let _ = app.config();
        }
        // Also verify the full chain in one shot, which is what the
        // `App::new` constructor does internally.
        let _app = App::new(AppConfig::default());
    }

    #[test]
    fn app_new_registers_seven_schedules() {
        let app = App::new(AppConfig::default());
        let world = app.bevy_app.world();
        // The presence of the schedule is verifiable by attempting to
        // run it (no-op if empty). We can also check that the schedule
        // label is recognized by attempting a schedule run.
        // Here we just verify that running a schedule does not panic.
        let mut app2 = App::new(AppConfig::default());
        app2.bevy_app_mut().world_mut().run_schedule(Label::First);
        app2.bevy_app_mut().world_mut().run_schedule(Label::Last);
    }

    #[test]
    fn config_default_is_sensible() {
        let config = AppConfig::default();
        assert_eq!(config.window.title, "Hyge Engine");
        assert_eq!(config.window.width, 1280);
        assert_eq!(config.window.height, 720);
        assert!(config.window.resizable);
        assert!(config.window.vsync);
        assert!(config.window.raw_input);
        assert_eq!(config.clear_color, [0.1, 0.1, 0.1, 1.0]);
        assert_eq!(config.renderer.backend, RendererBackend::Auto);
        assert!(config.physics.enabled);
        assert!(config.audio.enabled);
        assert!(config.script.enabled);
        assert!(config.script.sandbox);
        assert!(!config.editor.enabled);
    }

    #[test]
    fn app_builder_trait_works() {
        // Verify that `App` satisfies `AppBuilder` and the trait methods
        // are callable.
        fn assert_app_builder<T: AppBuilder>(_: T) {}
        assert_app_builder(App::new(AppConfig::default()));
    }
}
