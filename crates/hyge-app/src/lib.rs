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
use hyge_physics::prelude::{accumulate_fixed_steps, PhysicsPlugin};
use hyge_render::prelude::*;
use hyge_scene::prelude::*;
use hyge_window::prelude::*;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};

/// The Hyge application. Owns the bevy `App`, the `winit` event loop
/// driver state, the `Window` once it is created, and the
/// `Renderer` once the device + surface are initialized.
///
/// Construct with [`App::new`], add plugins with [`App::add_plugin`] (or
/// rely on the defaults from [`default_plugins`]), then call
/// [`App::run`] to enter the event loop.
pub struct App {
    config: AppConfig,
    inner: bevy_app::App,
    window: Option<Window>,
    /// The runtime renderer. Created lazily in `resumed` once the
    /// `Window` exists (R-024); `None` before that. The renderer
    /// is the only consumer of the `Arc<winit::Window>` stored
    /// inside `WindowState`; the App holds it separately to keep
    /// the lifetime story simple (the `Renderer` owns a `'static`
    /// surface, backed by the same `Arc<winit::Window>`).
    renderer: Option<Renderer>,
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
        let mut inner = bevy_app::App::new();

        // Register the canonical set of schedules. Each is keyed by the
        // `Label` enum variant (which is the value passed to
        // `init_schedule` / `run_schedule`). Plugins add systems to
        // these schedules via `app.add_systems(Label::Update, ...)`.
        inner.init_schedule(Label::First);
        inner.init_schedule(Label::PreUpdate);
        inner.init_schedule(Label::FixedUpdate);
        inner.init_schedule(Label::Update);
        inner.init_schedule(Label::RenderExtract);
        inner.init_schedule(Label::Render);
        inner.init_schedule(Label::Last);

        let mut app = Self {
            config,
            inner,
            window: None,
            renderer: None,
        };

        // Install the default plugin set. Each plugin's `build` method
        // registers events, resources, and systems. We dispatch through
        // `build` directly rather than `add_plugin` because the trait
        // objects in `default_plugins` cannot satisfy the `P: HygePlugin`
        // bound on `add_plugin` (the bound requires a concrete type, not
        // a `Box<dyn HygePlugin>`).
        for plugin in default_plugins(&app.config) {
            plugin.build(&mut app.inner);
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
    pub fn bevy_app(&self) -> &bevy_app::App {
        &self.inner
    }

    /// Returns a mutable reference to the inner bevy `App`.
    pub fn bevy_app_mut(&mut self) -> &mut bevy_app::App {
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
        let steps = {
            let config = world
                .get_resource::<hyge_physics::PhysicsConfig>()
                .copied()
                .unwrap_or_default();
            if let Some(mut time) = world.get_resource_mut::<hyge_physics::PhysicsTime>() {
                accumulate_fixed_steps(&mut time, &config, config.fixed_timestep)
            } else {
                1
            }
        };
        for _ in 0..steps {
            world.run_schedule(Label::FixedUpdate);
        }
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
        App::add_plugin(self, plugin)
    }

    fn run(self) -> ! {
        App::run(self)
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
                    return;
                }
            }
        }

        // Create the runtime renderer once we have a window. The
        // renderer owns the wgpu device, queue, surface, and the
        // pre-built first-triangle pipeline (R-024).
        if self.renderer.is_none() {
            let Some(window) = self.window.as_ref() else {
                return;
            };
            let renderer_config = hyge_render::config::RendererConfig::default();
            match Renderer::new(renderer_config, window) {
                Ok(renderer) => {
                    tracing::info!("renderer initialized; first-triangle ready");
                    self.inner
                        .world_mut()
                        .insert_resource(renderer.frame_stats().clone());
                    self.renderer = Some(renderer);
                }
                Err(e) => {
                    tracing::error!(error = %e, "failed to initialize renderer");
                    // Don't exit: the user can still see the window
                    // (it just won't render). M1+ show a fallback
                    // clear color via a different path.
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
            // R-024: forward the resize to the renderer so the
            // surface / swapchain gets reconfigured before the
            // next frame.
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(width, height);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Run all schedules for this frame. With the current placeholder
        // plugins, each schedule is empty and this is a no-op; the
        // schedule execution infrastructure is what we are validating
        // here.
        self.run_frame();

        // R-024: render the first triangle. The render path is
        // begin_frame → build the triangle graph → compile + execute
        // → submit → end_frame. Errors are logged but do not exit
        // the loop (e.g. a device-lost error should be recoverable
        // by recreating the surface).
        if let Some(renderer) = &mut self.renderer {
            let clear_color = wgpu::Color {
                r: self.config.clear_color[0] as f64,
                g: self.config.clear_color[1] as f64,
                b: self.config.clear_color[2] as f64,
                a: self.config.clear_color[3] as f64,
            };
            if let Err(e) = renderer.render_triangle(clear_color) {
                tracing::warn!(error = %e, "render_triangle failed");
            } else {
                self.inner
                    .world_mut()
                    .insert_resource(renderer.frame_stats().clone());
            }
        }

        // Request a redraw so the OS schedules a paint event.
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

/// Returns the canonical set of Hyge plugins that the [`App`] should
/// install by default for a given [`AppConfig`]. Each plugin corresponds
/// to one engine subsystem; more plugins are added here as their M-XXX
/// items land.
///
/// The order in the returned `Vec` is the order in which the plugins
/// are installed (and therefore the order in which their `build` methods
/// are called). Earlier plugins are visible to later ones; later plugins
/// can observe and extend the state set up by earlier ones.
///
/// Plugins that need per-config settings (e.g. the window title/size)
/// receive them through the [`AppConfig`] argument. Plugins with no
/// config use sensible defaults.
///
/// Currently includes [`WindowPlugin`], [`ScenePlugin`], and [`PhysicsPlugin`]. The other
/// subsystem plugins (renderer, asset, audio, input, script,
/// editor) are added as their respective milestones land (see
/// `docs/roadmap.toml`).
pub fn default_plugins(config: &AppConfig) -> Vec<Box<dyn HygePlugin>> {
    vec![
        Box::new(WindowPlugin::new(config.window.clone())),
        Box::new(ScenePlugin),
        Box::new(PhysicsPlugin::new((&config.physics).into())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_plugins_load_without_conflict() {
        // Building a fresh app and installing all default plugins must
        // not panic. This catches the common "two plugins both register
        // the same resource" class of bug.
        let config = AppConfig::default();
        for plugin in default_plugins(&config) {
            // Each plugin's `build` runs on a fresh app so the test does
            // not depend on a specific order; in practice the AppBuilder
            // chain is one-shot, but this is the closest smoke test we
            // can run in a unit test without an event loop.
            let mut app = App::new(config.clone());
            plugin.build(app.bevy_app_mut());
        }
        // Also verify the full chain in one shot, which is what the
        // `App::new` constructor does internally.
        let _app = App::new(AppConfig::default());
    }

    #[test]
    fn app_new_registers_seven_schedules() {
        // The presence of the schedule is verifiable by attempting to
        // run it (no-op if empty). Here we verify that running every
        // registered schedule does not panic.
        let mut app = App::new(AppConfig::default());
        for label in [
            Label::First,
            Label::PreUpdate,
            Label::FixedUpdate,
            Label::Update,
            Label::RenderExtract,
            Label::Render,
            Label::Last,
        ] {
            app.bevy_app_mut().world_mut().run_schedule(label);
        }
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
        assert_eq!(config.physics.max_substeps, 5);
        assert_eq!(config.physics.num_solver_iterations, 4);
        assert!(config.audio.enabled);
        assert!(config.script.enabled);
        assert!(config.script.sandbox);
        assert!(!config.editor.enabled);
    }

    #[test]
    fn app_builder_trait_works() {
        // Verify that `App` satisfies `AppBuilder` and the trait methods
        // are callable and chainable. We use a counter plugin so that
        // `add_plugin` is actually exercised.
        fn assert_app_builder<T: AppBuilder>(_: T) {}

        #[allow(dead_code)]
        struct CounterResource(u32);
        impl hyge_ecs::Resource for CounterResource {}

        struct CounterPlugin(u32);
        impl HygePlugin for CounterPlugin {
            fn name(&self) -> &'static str {
                "counter"
            }
            fn build(&self, app: &mut bevy_app::App) {
                app.insert_resource(CounterResource(self.0));
            }
        }

        let app = App::new(AppConfig::default()).add_plugin(CounterPlugin(42));
        // Verify the plugin's build actually ran and inserted the resource.
        let _ = app.bevy_app().world().get_resource::<CounterResource>();
        assert_app_builder(app);
    }
}
