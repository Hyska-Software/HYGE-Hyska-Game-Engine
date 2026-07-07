//! Hyge physics plugin registration.

use bevy_app::App;
use hyge_ecs::prelude::HygePlugin;

use crate::{config::PhysicsTime, events::ContactForceEvent, CollisionEvent, PhysicsConfig};

/// Registers physics resources and events.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicsPlugin;

impl HygePlugin for PhysicsPlugin {
    fn name(&self) -> &'static str {
        "hyge-physics"
    }

    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsConfig>()
            .init_resource::<PhysicsTime>()
            .add_event::<CollisionEvent>()
            .add_event::<ContactForceEvent>();
    }
}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use hyge_ecs::prelude::AppHygeExt;

    use super::*;

    #[test]
    fn plugin_registers_resources() {
        let mut app = App::new();
        app.add_hyge_plugin(PhysicsPlugin);

        assert!(app.world().get_resource::<PhysicsConfig>().is_some());
        assert!(app.world().get_resource::<PhysicsTime>().is_some());
    }
}
