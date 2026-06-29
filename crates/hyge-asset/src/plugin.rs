//! `hyge-asset` plugin registration.

use hyge_ecs::plugin::HygePlugin;

use crate::watcher::ReloadQueue;

/// Asset subsystem plugin.
///
/// R-030 only establishes asset identity and handle types, so this plugin is
/// intentionally a no-op. Later R-03x items register the server resource,
/// events, hot-reload systems, and loader registry here.
#[derive(Clone, Copy, Debug, Default)]
pub struct AssetPlugin;

impl HygePlugin for AssetPlugin {
    fn name(&self) -> &'static str {
        "hyge-asset"
    }

    fn build(&self, app: &mut bevy_app::App) {
        app.insert_resource(ReloadQueue::default());
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use hyge_ecs::prelude::*;

    use super::*;

    #[test]
    fn asset_plugin_registers_as_noop() {
        let mut app = App::new();
        app.add_hyge_plugin(AssetPlugin);
        assert_eq!(AssetPlugin.name(), "hyge-asset");
        assert!(app.world().get_resource::<ReloadQueue>().is_some());
    }
}
