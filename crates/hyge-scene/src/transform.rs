//! Transform propagation system.
//!
//! Walks the entity hierarchy defined by [`Parent`](crate::components::Parent)
//! and [`Children`](crate::components::Children) and writes a
//! [`GlobalTransform`](crate::components::GlobalTransform) for every entity.
//!
//! The system runs in the [`TransformSet::Propagate`](hyge_ecs::set::TransformSet)
//! set so later transform-dependent systems (render extract, physics sync,
//! audio update) see consistent world matrices.

use std::collections::HashMap;

use bevy_ecs::prelude::{Changed, Commands, Entity, Query, RemovedComponents, World};
use bevy_ecs::system::SystemState;
use hyge_core::prelude::Mat4;

use crate::components::{Children, GlobalTransform, Parent, Transform};

/// Propagates local [`Transform`] values through the hierarchy into
/// [`GlobalTransform`].
///
/// The algorithm:
/// 1. Snapshot the hierarchy and local transforms.
/// 2. Find roots (entities without `Parent`).
/// 3. DFS from each root, computing `parent_global * local`.
/// 4. Write the resulting world matrices back to `GlobalTransform`.
///
/// This is intentionally simple and deterministic; future revisions can
/// optimize the walk or cache dirtiness.
#[allow(clippy::type_complexity)]
pub fn transform_propagate_system(world: &mut World) {
    let mut state: SystemState<(
        Query<(Entity, &Transform, Option<&Parent>, Option<&Children>)>,
        Query<(Entity, &mut GlobalTransform)>,
    )> = SystemState::new(world);

    let (snapshot_query, mut globals_query) = state.get_mut(world);

    let mut locals: HashMap<Entity, Mat4> = HashMap::new();
    let mut parents: HashMap<Entity, Entity> = HashMap::new();
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();

    for (entity, transform, parent, entity_children) in snapshot_query.iter() {
        locals.insert(entity, transform.compute_matrix());
        if let Some(parent) = parent {
            parents.insert(entity, parent.0);
        }
        if let Some(entity_children) = entity_children {
            children.insert(entity, entity_children.0.clone());
        }
    }

    let roots: Vec<Entity> = locals
        .keys()
        .copied()
        .filter(|entity| !parents.contains_key(entity))
        .collect();

    let mut computed: HashMap<Entity, Mat4> = HashMap::new();
    for root in roots {
        propagate_recursive(root, Mat4::IDENTITY, &locals, &children, &mut computed);
    }

    for (entity, mut global) in &mut globals_query {
        if let Some(matrix) = computed.get(&entity) {
            *global = GlobalTransform::from(*matrix);
        }
    }
}

fn propagate_recursive(
    entity: Entity,
    parent_global: Mat4,
    locals: &HashMap<Entity, Mat4>,
    children: &HashMap<Entity, Vec<Entity>>,
    computed: &mut HashMap<Entity, Mat4>,
) {
    let Some(local) = locals.get(&entity).copied() else {
        return;
    };

    let global = parent_global * local;
    computed.insert(entity, global);

    if let Some(entity_children) = children.get(&entity) {
        for child in entity_children {
            propagate_recursive(*child, global, locals, children, computed);
        }
    }
}

/// Cleanup system: when a [`Parent`] component is removed, remove the
/// corresponding [`Children`] entry from the former parent.
///
/// This is not a full hierarchy maintenance system (adding/removing
/// `Parent` should also update `Children`), but it prevents stale
/// references after hot-reload or scripted reparenting.
#[allow(clippy::type_complexity)]
pub fn hierarchy_cleanup_system(world: &mut World) {
    let mut state: SystemState<(
        Query<(Entity, &Parent), Changed<Parent>>,
        Query<&mut Children>,
        RemovedComponents<Parent>,
        Commands,
    )> = SystemState::new(world);

    let (changed_parents, mut children, mut removed_parents, mut commands) = state.get_mut(world);

    let mut updates: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (entity, parent) in changed_parents.iter() {
        updates.entry(parent.0).or_default().push(entity);
    }

    for entity in removed_parents.read() {
        commands.entity(entity).remove::<Children>();
    }

    for (parent, new_children) in updates {
        if let Ok(mut children) = children.get_mut(parent) {
            for child in new_children {
                if !children.0.contains(&child) {
                    children.0.push(child);
                }
            }
        } else {
            commands.entity(parent).insert(Children(new_children));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyge_core::prelude::{Mat4, Quat, Vec3};

    #[test]
    fn propagate_root_identity() {
        let mut world = World::new();
        world.spawn((Transform::identity(), GlobalTransform::identity()));

        let mut schedule = hyge_ecs::prelude::Schedule::new(hyge_ecs::schedule::Label::Update);
        schedule.add_systems(transform_propagate_system);
        schedule.run(&mut world);

        let global = world.query::<&GlobalTransform>().single(&world);
        assert_eq!(global.to_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn propagate_two_level_hierarchy() {
        let mut world = World::new();

        let parent = world
            .spawn((
                Transform::from_trs(Vec3::X, Quat::IDENTITY, Vec3::ONE),
                GlobalTransform::identity(),
            ))
            .id();

        let child = world
            .spawn((
                Transform::from_trs(Vec3::Y, Quat::IDENTITY, Vec3::ONE),
                GlobalTransform::identity(),
                Parent(parent),
            ))
            .id();

        world.entity_mut(parent).insert(Children(vec![child]));

        let mut schedule = hyge_ecs::prelude::Schedule::new(hyge_ecs::schedule::Label::Update);
        schedule.add_systems(transform_propagate_system);
        schedule.run(&mut world);

        let parent_global = world
            .query::<(&GlobalTransform, &Transform)>()
            .iter(&world)
            .find(|(_, t)| t.translation == [1.0, 0.0, 0.0])
            .map(|(g, _)| g.to_matrix())
            .unwrap();
        let child_global = world
            .query::<(&GlobalTransform, &Parent)>()
            .iter(&world)
            .find(|(_, p)| p.0 == parent)
            .map(|(g, _)| g.to_matrix())
            .unwrap();

        assert_eq!(parent_global, Mat4::from_translation(Vec3::X));
        assert_eq!(
            child_global,
            Mat4::from_translation(Vec3::X) * Mat4::from_translation(Vec3::Y)
        );
    }

    #[test]
    fn propagate_scale_accumulates() {
        let mut world = World::new();

        let parent = world
            .spawn((
                Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0)),
                GlobalTransform::identity(),
            ))
            .id();

        let child = world
            .spawn((
                Transform::from_trs(Vec3::ONE, Quat::IDENTITY, Vec3::splat(3.0)),
                GlobalTransform::identity(),
                Parent(parent),
            ))
            .id();

        world.entity_mut(parent).insert(Children(vec![child]));

        let mut schedule = hyge_ecs::prelude::Schedule::new(hyge_ecs::schedule::Label::Update);
        schedule.add_systems(transform_propagate_system);
        schedule.run(&mut world);

        let child_global = world
            .query::<(&GlobalTransform, &Parent)>()
            .iter(&world)
            .find(|(_, p)| p.0 == parent)
            .map(|(g, _)| g.to_matrix())
            .unwrap();

        let scale = child_global.to_scale_rotation_translation().0;
        assert!((scale - Vec3::splat(6.0)).length() < 1e-5);
    }
}
