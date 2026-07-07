//! Physics resources and runtime configuration.

use bevy_ecs::prelude::Resource;
use hyge_core::prelude::Vec3;

/// Global physics simulation configuration.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PhysicsConfig {
    /// Fixed simulation timestep in seconds.
    pub fixed_timestep: f32,
    /// Maximum number of fixed substeps consumed per render frame.
    pub max_substeps: u32,
    /// World gravity vector.
    pub gravity: Vec3,
    /// Number of solver iterations for the active backend.
    pub num_solver_iterations: usize,
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            fixed_timestep: 1.0 / 60.0,
            max_substeps: 5,
            gravity: Vec3::new(0.0, -9.81, 0.0),
            num_solver_iterations: 4,
        }
    }
}

/// Accumulator state for the fixed-timestep physics loop.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct PhysicsTime {
    /// Unconsumed simulation time in seconds.
    pub accumulator: f32,
    /// Fixed timestep in seconds.
    pub timestep: f32,
}

impl Default for PhysicsTime {
    fn default() -> Self {
        Self {
            accumulator: 0.0,
            timestep: PhysicsConfig::default().fixed_timestep,
        }
    }
}
