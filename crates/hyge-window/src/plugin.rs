//! The [`WindowPlugin`] and the [`WindowState`] resource.

use std::sync::Arc;

use bevy_app::App;
use hyge_ecs::prelude::*;
use winit::window::Window as WinitWindow;

use crate::config::WindowConfig;
use crate::events::{DeviceEvent, WindowCloseRequested, WindowFocused, WindowResized};

/// The window plugin.
///
/// Register with `App::add_hyge_plugin(WindowPlugin::new(config))` from
/// `hyge-app` (or the user's main function). The plugin:
///   1. Inserts [`WindowConfig`] as a resource (so the application can
///      read the config back at runtime).
///   2. Inserts [`WindowState`] as a default resource; the application
///      populates it after the event loop creates the actual window.
///   3. Registers [`WindowResized`], [`WindowCloseRequested`],
///      [`WindowFocused`], and [`DeviceEvent`] event types so systems can
///      subscribe with `EventReader`.
pub struct WindowPlugin {
    config: WindowConfig,
}

impl WindowPlugin {
    /// Creates a new `WindowPlugin` with the given config.
    pub fn new(config: WindowConfig) -> Self {
        Self { config }
    }
}

impl HygePlugin for WindowPlugin {
    fn name(&self) -> &'static str {
        "hyge-window"
    }

    fn build(&self, app: &mut App) {
        tracing::debug!(config = ?self.config, "registering hyge-window plugin");
        app.insert_resource(self.config.clone());
        app.init_resource::<WindowState>();
        app.add_event::<WindowResized>();
        app.add_event::<WindowCloseRequested>();
        app.add_event::<WindowFocused>();
        app.add_event::<DeviceEvent>();
    }
}

/// Holds the optional window after the event loop has created it.
///
/// The plugin registers this resource as `Default` (empty) at startup.
/// After `hyge-app` (or the user's main function) creates the actual
/// `winit::Window`, it stores the `Arc<WinitWindow>` here via
/// [`WindowState::set`]. Other systems (e.g. the renderer's surface
/// creation, the input layer's raw input registration) read it back via
/// [`WindowState::get`].
#[derive(Resource, Default)]
pub struct WindowState {
    window: Option<Arc<WinitWindow>>,
}

impl WindowState {
    /// Sets the window. Called once, after the event loop has created it.
    pub fn set(&mut self, window: Arc<WinitWindow>) {
        self.window = Some(window);
    }

    /// Returns the window, if it has been set.
    pub fn get(&self) -> Option<&Arc<WinitWindow>> {
        self.window.as_ref()
    }

    /// Returns `true` if the window has been set.
    pub fn is_set(&self) -> bool {
        self.window.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_app::App;
    use bevy_ecs::event::Events;

    #[test]
    fn plugin_registers_events_and_config() {
        let mut app = App::new();
        let config = WindowConfig {
            title: "test-window".to_string(),
            width: 1024,
            height: 768,
            ..WindowConfig::default()
        };
        app.add_hyge_plugin(WindowPlugin::new(config.clone()));

        let world = app.world_mut();
        assert!(
            world.get_resource::<Events<WindowResized>>().is_some(),
            "WindowResized events must be registered"
        );
        assert!(
            world
                .get_resource::<Events<WindowCloseRequested>>()
                .is_some(),
            "WindowCloseRequested events must be registered"
        );
        assert!(
            world.get_resource::<Events<WindowFocused>>().is_some(),
            "WindowFocused events must be registered"
        );
        assert!(
            world.get_resource::<Events<DeviceEvent>>().is_some(),
            "DeviceEvent events must be registered"
        );

        let stored = world
            .get_resource::<WindowConfig>()
            .expect("WindowConfig must be inserted");
        assert_eq!(stored.title, "test-window");
        assert_eq!(stored.width, 1024);
        assert_eq!(stored.height, 768);
    }

    #[test]
    fn plugin_registers_window_state() {
        let mut app = App::new();
        app.add_hyge_plugin(WindowPlugin::new(WindowConfig::default()));
        let world = app.world_mut();
        let state = world
            .get_resource::<WindowState>()
            .expect("WindowState must be inserted");
        assert!(!state.is_set(), "WindowState starts empty");
        assert!(state.get().is_none());
    }

    #[test]
    fn plugin_name_is_hyge_window() {
        let plugin = WindowPlugin::new(WindowConfig::default());
        assert_eq!(plugin.name(), "hyge-window");
    }
}
