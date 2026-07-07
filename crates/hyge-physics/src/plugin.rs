//! Hyge physics plugin registration.

use bevy_app::App;
use hyge_ecs::prelude::{HygePlugin, Label};

use crate::{
    config::PhysicsTime, events::ContactForceEvent, step::physics_step_system, CollisionEvent,
    PhysicsConfig,
};

#[cfg(feature = "physics-rapier")]
use crate::rapier_impl::RapierPhysicsWorld;

/// Registers physics resources, events, and the fixed-step system.
#[derive(Clone, Copy, Debug)]
pub struct PhysicsPlugin {
    config: PhysicsConfig,
}

impl PhysicsPlugin {
    /// Creates a physics plugin with explicit configuration.
    #[must_use]
    pub fn new(config: PhysicsConfig) -> Self {
        Self { config }
    }
}

impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self::new(PhysicsConfig::default())
    }
}

impl HygePlugin for PhysicsPlugin {
    fn name(&self) -> &'static str {
        "hyge-physics"
    }

    fn build(&self, app: &mut App) {
        app.insert_resource(self.config)
            .insert_resource(PhysicsTime {
                accumulator: 0.0,
                timestep: self.config.fixed_timestep,
            })
            .add_event::<CollisionEvent>()
            .add_event::<ContactForceEvent>()
            .add_systems(Label::FixedUpdate, physics_step_system);

        #[cfg(feature = "physics-rapier")]
        app.insert_resource(RapierPhysicsWorld::new(&self.config));
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
        app.add_hyge_plugin(PhysicsPlugin::default());

        assert!(app.world().get_resource::<PhysicsConfig>().is_some());
        assert!(app.world().get_resource::<PhysicsTime>().is_some());
    }
}
