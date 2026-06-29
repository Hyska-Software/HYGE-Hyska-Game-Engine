//! The [`HygePlugin`] trait and the [`App`] extension that installs plugins.

use bevy_app::App;

/// A Hyge plugin: a unit of engine subsystem registration.
///
/// Implement this trait for any type that wants to register systems,
/// resources, events, schedules, or other plugins into the [`App`]. Use
/// [`AppHygeExt::add_hyge_plugin`] to install a plugin.
///
/// # Example
///
/// ```no_run
/// use hyge_ecs::prelude::*;
/// use bevy_app::App;
///
/// struct PhysicsPlugin;
///
/// impl HygePlugin for PhysicsPlugin {
///     fn name(&self) -> &'static str { "hyge-physics" }
///     fn build(&self, app: &mut App) {
///         app.add_systems(Label::Update, step_physics);
///     }
/// }
///
/// fn step_physics() {}
/// ```
pub trait HygePlugin: Send + Sync + 'static {
    /// Returns the plugin's display name, used in logs and diagnostics.
    ///
    /// The default implementation returns the Rust type name, which is
    /// usually good enough. Override when you want a shorter or more
    /// user-friendly name.
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Configures the [`App`] by registering systems, resources, events,
    /// schedules, etc. Called exactly once per plugin, at registration
    /// time (when [`AppHygeExt::add_hyge_plugin`] is called).
    fn build(&self, app: &mut App);
}

/// Extension trait for [`App`] that adds Hyge plugin installation.
///
/// Imported automatically via `use hyge_ecs::prelude::*;`.
pub trait AppHygeExt {
    /// Adds a [`HygePlugin`] to the app, calling its `build` method
    /// immediately. Returns `&mut App` for chaining.
    ///
    /// Plugins are registered in the order they are added; later plugins
    /// can observe the effects of earlier ones (e.g. a resource inserted
    /// by an earlier plugin is visible to a later plugin's `build`).
    fn add_hyge_plugin<P: HygePlugin>(&mut self, plugin: P) -> &mut Self;
}

impl AppHygeExt for App {
    fn add_hyge_plugin<P: HygePlugin>(&mut self, plugin: P) -> &mut Self {
        tracing::debug!(plugin = %plugin.name(), "registering hyge plugin");
        plugin.build(self);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_ecs::prelude::*;

    /// Marker resource inserted by `TestPluginA` so that `TestPluginB` can
    /// verify A ran first.
    #[derive(Resource)]
    struct PluginRanA;

    /// Marker resource inserted by `TestPluginB`.
    #[derive(Resource)]
    struct PluginRanB;

    struct TestPluginA;
    struct TestPluginB;

    impl HygePlugin for TestPluginA {
        fn name(&self) -> &'static str {
            "test-a"
        }
        fn build(&self, app: &mut App) {
            app.insert_resource(PluginRanA);
        }
    }

    impl HygePlugin for TestPluginB {
        fn name(&self) -> &'static str {
            "test-b"
        }
        fn build(&self, app: &mut App) {
            // A must have run before B.
            assert!(
                app.world().get_resource::<PluginRanA>().is_some(),
                "TestPluginA must be registered before TestPluginB"
            );
            app.insert_resource(PluginRanB);
        }
    }

    #[test]
    fn plugin_registration_order() {
        let mut app = App::new();
        app.add_hyge_plugin(TestPluginA);
        app.add_hyge_plugin(TestPluginB);
        assert!(app.world().get_resource::<PluginRanA>().is_some());
        assert!(app.world().get_resource::<PluginRanB>().is_some());
    }

    #[test]
    fn plugin_default_name_uses_type_name() {
        struct NoNamePlugin;
        impl HygePlugin for NoNamePlugin {
            fn build(&self, _app: &mut App) {}
        }
        let p = NoNamePlugin;
        assert!(
            p.name().contains("NoNamePlugin"),
            "default name should contain the type name, got: {}",
            p.name()
        );
    }

    #[test]
    fn add_hyge_plugin_returns_app_for_chaining() {
        let mut app = App::new();
        app.add_hyge_plugin(TestPluginA)
            .add_hyge_plugin(TestPluginB);
        assert!(app.world().get_resource::<PluginRanA>().is_some());
        assert!(app.world().get_resource::<PluginRanB>().is_some());
    }
}
