//! Optional `rapier3d` backend glue.
//!
//! R-070 keeps this module intentionally thin: conversion helpers and module
//! boundaries are present so R-071 can add full world synchronization without
//! changing the public crate layout.

use std::num::NonZeroUsize;
use std::{collections::HashMap, sync::Mutex};

use bevy_ecs::prelude::{Entity, Resource};
use hyge_core::prelude::Vec3;
use rapier3d::pipeline::EventHandler;
use rapier3d::prelude::{
    BroadPhase, CCDSolver, ColliderHandle, ColliderSet, ImpulseJointSet, IntegrationParameters,
    IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline, QueryPipeline,
    RigidBodyBuilder, RigidBodyHandle, RigidBodySet, Vector,
};

pub mod body;
pub mod char_controller;
pub mod collider;
pub mod island;
pub mod joint;

use crate::components::{Collider, PhysicsPosition, PhysicsVelocity, RigidBody, RigidBodyKind};
use crate::config::PhysicsConfig;
use crate::query::{QueryFilter, RayHit, ShapeHit, SpatialQuery};

use self::body::rigid_body_type;
use self::collider::collider_builder;
use self::island::{IslandBuildInput, IslandBuildResult, RapierIslandBuilder};

/// Rapier-backed physics world resource.
#[derive(Resource)]
pub struct RapierPhysicsWorld {
    pipeline: PhysicsPipeline,
    gravity: Vector<f32>,
    integration_parameters: IntegrationParameters,
    island_manager: IslandManager,
    broad_phase: BroadPhase,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd_solver: CCDSolver,
    query_pipeline: QueryPipeline,
    body_handles: HashMap<Entity, RigidBodyHandle>,
    collider_handles: HashMap<Entity, ColliderHandle>,
    handle_entities: HashMap<RigidBodyHandle, Entity>,
    island_builder: RapierIslandBuilder,
    step_generation: u64,
    collision_events: Vec<crate::CollisionEvent>,
}

impl Default for RapierPhysicsWorld {
    fn default() -> Self {
        let config = PhysicsConfig::default();
        Self::new(&config)
    }
}

impl RapierPhysicsWorld {
    /// Creates a Rapier world from Hyge physics configuration.
    #[must_use]
    pub fn new(config: &PhysicsConfig) -> Self {
        let integration_parameters = IntegrationParameters {
            dt: config.fixed_timestep,
            num_solver_iterations: solver_iterations(config),
            ..IntegrationParameters::default()
        };

        Self {
            pipeline: PhysicsPipeline::new(),
            gravity: vec3_to_rapier(config.gravity),
            integration_parameters,
            island_manager: IslandManager::new(),
            broad_phase: BroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            query_pipeline: QueryPipeline::new(),
            body_handles: HashMap::new(),
            collider_handles: HashMap::new(),
            handle_entities: HashMap::new(),
            island_builder: RapierIslandBuilder::default(),
            step_generation: 0,
            collision_events: Vec::new(),
        }
    }

    /// Inserts a body and collider for an entity if it does not already exist.
    pub fn ensure_body(
        &mut self,
        entity: Entity,
        body: &RigidBody,
        collider: &Collider,
        position: PhysicsPosition,
    ) {
        if self.body_handles.contains_key(&entity) {
            return;
        }

        let mut builder = match body.kind {
            RigidBodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            RigidBodyKind::Fixed => RigidBodyBuilder::fixed(),
            RigidBodyKind::KinematicPosition => RigidBodyBuilder::kinematic_position_based(),
            RigidBodyKind::KinematicVelocity => RigidBodyBuilder::kinematic_velocity_based(),
        }
        .translation(vec3_to_rapier(position.as_vec3()))
        .gravity_scale(body.gravity_scale)
        .linear_damping(body.linear_damping)
        .angular_damping(body.angular_damping);

        if body.ccd {
            builder = builder.ccd_enabled(true);
        }

        let body_handle = self.bodies.insert(builder.build());
        let collider_handle = self.colliders.insert_with_parent(
            collider_builder(collider)
                .active_events(rapier3d::prelude::ActiveEvents::COLLISION_EVENTS)
                .build(),
            body_handle,
            &mut self.bodies,
        );

        self.body_handles.insert(entity, body_handle);
        self.collider_handles.insert(entity, collider_handle);
        self.handle_entities.insert(body_handle, entity);
    }

    /// Steps the Rapier simulation once.
    pub fn step(&mut self, config: &PhysicsConfig) {
        self.integration_parameters.dt = config.fixed_timestep;
        self.integration_parameters.num_solver_iterations = solver_iterations(config);
        self.gravity = vec3_to_rapier(config.gravity);
        self.step_generation += 1;

        self.island_builder.poll();
        self.island_builder.submit(IslandBuildInput {
            generation: self.step_generation,
            dynamic_bodies: self.dynamic_body_count(),
            colliders: self.colliders.len(),
        });

        let collector = RapierEventCollector::default();
        self.pipeline.step(
            &self.gravity,
            &self.integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd_solver,
            Some(&mut self.query_pipeline),
            &(),
            &collector,
        );
        for (started, first, second) in collector.drain() {
            let Some(entity_a) = self.entity_for_collider(first) else {
                continue;
            };
            let Some(entity_b) = self.entity_for_collider(second) else {
                continue;
            };
            self.collision_events.push(crate::CollisionEvent {
                entity_a,
                entity_b,
                started,
                contact: None,
            });
        }
    }

    /// Drains collision transitions emitted by the latest Rapier step.
    pub fn drain_collision_events(&mut self) -> Vec<crate::CollisionEvent> {
        self.collision_events.drain(..).collect()
    }

    /// Returns the world-space position for an entity body.
    #[must_use]
    pub fn position(&self, entity: Entity) -> Option<Vec3> {
        let handle = self.body_handles.get(&entity)?;
        let body = self.bodies.get(*handle)?;
        let translation = body.translation();
        Some(Vec3::new(translation.x, translation.y, translation.z))
    }

    /// Returns the world-space velocity for an entity body.
    #[must_use]
    pub fn velocity(&self, entity: Entity) -> Option<PhysicsVelocity> {
        let handle = self.body_handles.get(&entity)?;
        let body = self.bodies.get(*handle)?;
        let linvel = body.linvel();
        let angvel = body.angvel();
        Some(PhysicsVelocity {
            linear: [linvel.x, linvel.y, linvel.z],
            angular: [angvel.x, angvel.y, angvel.z],
        })
    }

    /// Returns the latest completed worker-thread island metadata result.
    #[must_use]
    pub fn last_island_result(&self) -> Option<IslandBuildResult> {
        self.island_builder.last_result()
    }

    fn dynamic_body_count(&self) -> usize {
        self.bodies
            .iter()
            .filter(|(_, body)| body.body_type() == rigid_body_type(RigidBodyKind::Dynamic))
            .count()
    }

    fn entity_for_collider(&self, collider: ColliderHandle) -> Option<Entity> {
        self.collider_handles
            .iter()
            .find_map(|(entity, handle)| (*handle == collider).then_some(*entity))
    }
}

#[derive(Default)]
struct RapierEventCollector {
    collisions: Mutex<Vec<(bool, ColliderHandle, ColliderHandle)>>,
}

impl EventHandler for RapierEventCollector {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        event: rapier3d::prelude::CollisionEvent,
        _contact_pair: Option<&rapier3d::prelude::ContactPair>,
    ) {
        let started = event.started();
        if let Ok(mut collisions) = self.collisions.lock() {
            collisions.push((started, event.collider1(), event.collider2()));
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _contact_pair: &rapier3d::prelude::ContactPair,
        _total_force_magnitude: f32,
    ) {
    }
}

impl RapierEventCollector {
    fn drain(&self) -> Vec<(bool, ColliderHandle, ColliderHandle)> {
        self.collisions
            .lock()
            .map(|mut collisions| collisions.drain(..).collect())
            .unwrap_or_default()
    }
}

impl SpatialQuery for RapierPhysicsWorld {
    fn cast_ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_toi: f32,
        filter: QueryFilter,
    ) -> Option<RayHit> {
        if dir.length_squared() == 0.0 || max_toi < 0.0 {
            return None;
        }

        let ray = rapier3d::prelude::Ray::new(
            rapier3d::na::Point3::new(origin.x, origin.y, origin.z),
            vec3_to_rapier(dir.normalize()),
        );
        let hit = self.query_pipeline.cast_ray_and_get_normal(
            &self.bodies,
            &self.colliders,
            &ray,
            max_toi,
            true,
            rapier3d::prelude::QueryFilter::default(),
        )?;
        let parent = self.colliders.get(hit.0)?.parent()?;
        let entity = *self.handle_entities.get(&parent)?;
        if !filter.allows(entity) {
            return None;
        }
        Some(RayHit {
            entity,
            toi: hit.1.toi,
            normal: Vec3::new(hit.1.normal.x, hit.1.normal.y, hit.1.normal.z),
        })
    }

    fn cast_shape(
        &self,
        _shape: crate::ColliderShape,
        _origin: Vec3,
        _dir: Vec3,
        _max_toi: f32,
        _filter: QueryFilter,
    ) -> Option<ShapeHit> {
        None
    }

    fn intersections_with(
        &self,
        _shape: crate::ColliderShape,
        _origin: Vec3,
        _rotation: hyge_core::prelude::Quat,
        _filter: QueryFilter,
    ) -> Vec<Entity> {
        Vec::new()
    }
}

fn vec3_to_rapier(v: Vec3) -> Vector<f32> {
    Vector::new(v.x, v.y, v.z)
}

fn solver_iterations(config: &PhysicsConfig) -> NonZeroUsize {
    NonZeroUsize::new(config.num_solver_iterations).unwrap_or(NonZeroUsize::MIN)
}
