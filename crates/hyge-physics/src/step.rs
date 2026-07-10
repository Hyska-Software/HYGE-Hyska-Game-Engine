//! Fixed-timestep physics stepping helpers.
//!
//! The app consumes [`PhysicsTime`] and runs [`hyge_ecs::schedule::Label::FixedUpdate`]
//! once per fixed substep. The system in this module advances the active
//! backend by exactly one fixed timestep each time that schedule runs.

use crate::{PhysicsConfig, PhysicsTime};

#[cfg(feature = "physics-rapier")]
use crate::rapier_impl::RapierPhysicsWorld;

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

/// Advances the active physics backend by one fixed timestep.
#[cfg(feature = "physics-rapier")]
pub fn physics_step_system(
    config: bevy_ecs::prelude::Res<PhysicsConfig>,
    mut world: bevy_ecs::prelude::ResMut<RapierPhysicsWorld>,
    mut events: bevy_ecs::prelude::EventWriter<crate::CollisionEvent>,
) {
    world.as_mut().step(&config);
    for event in world.as_mut().drain_collision_events() {
        events.send(event);
    }
}

/// No-op physics step used when the Rapier backend feature is disabled.
#[cfg(not(feature = "physics-rapier"))]
pub fn physics_step_system(mut _events: bevy_ecs::prelude::EventWriter<crate::CollisionEvent>) {}

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

    #[test]
    fn accumulator_consumes_expected_steps() {
        let config = PhysicsConfig {
            fixed_timestep: 0.1,
            max_substeps: 5,
            ..PhysicsConfig::default()
        };
        let mut time = PhysicsTime::default();

        assert_eq!(accumulate_fixed_steps(&mut time, &config, 0.05), 0);
        assert!((time.accumulator - 0.05).abs() < 1.0e-6);
        assert_eq!(accumulate_fixed_steps(&mut time, &config, 0.15), 2);
        assert!(time.accumulator.abs() < 1.0e-6);
    }

    #[cfg(feature = "physics-rapier")]
    #[test]
    fn deterministic_ball_drop_for_100_dynamic_bodies() {
        use bevy_ecs::prelude::Entity;
        use hyge_core::prelude::Vec3;

        use crate::{Collider, ColliderShape, PhysicsPosition, RigidBody, RigidBodyKind};

        fn run() -> Vec<Vec3> {
            let config = PhysicsConfig::default();
            let mut world = RapierPhysicsWorld::new(&config);
            world.ensure_body(
                Entity::from_raw(10_000),
                &RigidBody {
                    kind: RigidBodyKind::Fixed,
                    ..RigidBody::default()
                },
                &Collider {
                    shape: ColliderShape::cuboid(Vec3::new(100.0, 0.5, 100.0)),
                    ..Collider::default()
                },
                PhysicsPosition::from_translation(Vec3::new(0.0, -0.5, 0.0)),
            );

            for i in 0..100_u32 {
                let x = (i % 10) as f32 * 4.0 - 18.0;
                let z = (i / 10) as f32 * 4.0 - 18.0;
                let y = 10.0 + (i / 10) as f32;
                world.ensure_body(
                    Entity::from_raw(i + 1),
                    &RigidBody::default(),
                    &Collider {
                        shape: ColliderShape::Ball(0.5),
                        ..Collider::default()
                    },
                    PhysicsPosition::from_translation(Vec3::new(x, y, z)),
                );
            }

            for _ in 0..600 {
                world.step(&config);
            }

            (1..=100_u32)
                .map(|i| {
                    world
                        .position(Entity::from_raw(i))
                        .expect("dynamic body should remain in the Rapier world")
                })
                .collect()
        }

        let first = run();
        let second = run();
        assert_eq!(first.len(), 100);
        assert_eq!(first.len(), second.len());

        for (a, b) in first.iter().zip(&second) {
            assert!(
                (*a - *b).length() < 1.0e-5,
                "positions diverged: {a:?} vs {b:?}"
            );
        }

        let reference = first[0];
        assert!(
            (reference.y - 0.5).abs() < 0.05,
            "unexpected landing y: {reference:?}"
        );
    }
}
