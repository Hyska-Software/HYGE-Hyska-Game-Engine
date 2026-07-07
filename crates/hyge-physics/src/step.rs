//! Fixed-timestep physics stepping helpers.
//!
//! R-071 fills this module with backend stepping. R-070 only defines the
//! accumulator helper used by the app schedule integration.

use crate::{PhysicsConfig, PhysicsTime};

/// Adds `delta_seconds` to the physics accumulator and returns how many fixed
/// substeps should be executed this frame.
pub fn accumulate_fixed_steps(
    time: &mut PhysicsTime,
    config: &PhysicsConfig,
    delta_seconds: f32,
) -> u32 {
    time.timestep = config.fixed_timestep;
    time.accumulator += delta_seconds.max(0.0);

    let mut steps = 0;
    while time.accumulator >= time.timestep && steps < config.max_substeps {
        time.accumulator -= time.timestep;
        steps += 1;
    }

    if steps == config.max_substeps && time.accumulator >= time.timestep {
        time.accumulator = 0.0;
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_caps_substeps() {
        let config = PhysicsConfig {
            fixed_timestep: 0.1,
            max_substeps: 2,
            ..PhysicsConfig::default()
        };
        let mut time = PhysicsTime::default();

        let steps = accumulate_fixed_steps(&mut time, &config, 0.5);

        assert_eq!(steps, 2);
        assert_eq!(time.accumulator, 0.0);
    }
}
