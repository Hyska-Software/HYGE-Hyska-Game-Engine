//! R-043 frame extraction: walks the ECS world and produces a
//! [`FrameSnapshot`] the renderer can consume.
//!
//! The extraction step is intentionally trivial — it iterates
//! the world once, copies the relevant components into the
//! renderer's POD layout, and returns the snapshot. No GPU work
//! happens here; the snapshot is uploaded to the bindless
//! table by the renderer in `render_frame`.
//!
//! Future revisions (R-043b, M6) will:
//! - Frustum-cull the entities before they land in the snapshot.
//! - Group entities by (mesh_id, material_id) so the
//!   `DrawCommand::instance_count` reflects instancing.
//! - Sort the draw list by material to minimize pipeline
//!   transitions.

use bytemuck::{Pod, Zeroable};

use hyge_ecs::prelude::*;

use crate::components::{LightComponent, MaterialHandle, MeshHandle, WorldTransform};

/// A single rendered instance, GPU-ready. Mirrors
/// `hyge_render::bindless::Instance`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct Instance {
    /// Column-major 3x4 affine transform.
    pub transform: [[f32; 4]; 3],
    /// Bindless mesh id.
    pub mesh_id: u32,
    /// Bindless material id.
    pub material_id: u32,
    /// Pad to a 16-byte boundary.
    pub _pad: [u32; 2],
}

/// A draw command. Mirrors `hyge_render::bindless::DrawCommand`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct DrawCommand {
    /// Bindless mesh id.
    pub mesh_id: u32,
    /// Bindless material id.
    pub material_id: u32,
    /// First instance in the instance buffer.
    pub first_instance: u32,
    /// Number of instances.
    pub instance_count: u32,
    /// Local AABB min (placeholder; R-043b populates from
    /// the mesh asset).
    pub aabb_min: [f32; 3],
    /// Local AABB max (placeholder).
    pub aabb_max: [f32; 3],
    /// Pad.
    pub _pad: [u32; 2],
}

/// A light ready for the bindless light buffer. Mirrors
/// `hyge_render::bindless::Light`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
pub struct Light {
    /// Position xyz + light type in w.
    pub position: [f32; 4],
    /// Color rgb + intensity in w.
    pub color: [f32; 4],
    /// Direction xyz + cos(outer_angle) in w.
    pub direction: [f32; 4],
}

impl From<LightComponent> for Light {
    fn from(c: LightComponent) -> Self {
        Self {
            position: c.position,
            color: c.color_intensity,
            direction: c.direction_cos_outer,
        }
    }
}

/// The per-frame snapshot the renderer consumes.
#[derive(Clone, Debug, Default)]
pub struct FrameSnapshot {
    /// Per-instance GPU rows.
    pub instances: Vec<Instance>,
    /// Per-draw commands.
    pub draw_commands: Vec<DrawCommand>,
    /// Lights in the scene (directional + point + spot).
    pub lights: Vec<Light>,
}

impl FrameSnapshot {
    /// Creates an empty snapshot.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the number of draw commands.
    #[must_use]
    pub fn draw_count(&self) -> usize {
        self.draw_commands.len()
    }

    /// Returns the number of instances.
    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Returns the number of lights.
    #[must_use]
    pub fn light_count(&self) -> usize {
        self.lights.len()
    }
}

/// Walks a `World` and produces a [`FrameSnapshot`].
///
/// The current implementation is a single pass that copies every
/// entity with the `MeshHandle` component into the instance
/// buffer. Draw commands are emitted one-per-instance; the
/// R-043b instancing pass deduplicates by (mesh_id, material_id).
pub fn render_extract(world: &mut World) -> FrameSnapshot {
    let mut snapshot = FrameSnapshot::empty();

    // 1. Extract lights.
    {
        let mut query = world.query::<&LightComponent>();
        for light in query.iter(world) {
            snapshot.lights.push(Light::from(*light));
        }
    }

    // 2. Extract renderables.
    {
        let mut query = world
            .query::<(&MeshHandle, &MaterialHandle, Option<&WorldTransform>)>();
        let mut first_instance_per_draw: u32 = 0;
        for (mesh, material, transform) in query.iter(world) {
            let transform = transform.copied().unwrap_or_default();
            snapshot.instances.push(Instance {
                transform: transform.cols,
                mesh_id: mesh.0,
                material_id: material.0,
                _pad: [0; 2],
            });
            snapshot.draw_commands.push(DrawCommand {
                mesh_id: mesh.0,
                material_id: material.0,
                first_instance: first_instance_per_draw,
                instance_count: 1,
                aabb_min: [0.0; 3],
                aabb_max: [1.0; 3],
                _pad: [0; 2],
            });
            first_instance_per_draw = first_instance_per_draw.saturating_add(1);
        }
    }

    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_world_with_sun() -> World {
        let mut world = World::new();
        world.spawn(LightComponent::sun([0.0, -1.0, 0.0], [1.0, 1.0, 1.0], 1.0));
        world
    }

    #[test]
    fn extract_empty_world_produces_empty_snapshot() {
        let mut world = World::new();
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.draw_count(), 0);
        assert_eq!(snapshot.instance_count(), 0);
        assert_eq!(snapshot.light_count(), 0);
    }

    #[test]
    fn extract_sun_light() {
        let mut world = make_world_with_sun();
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.light_count(), 1);
        let light = &snapshot.lights[0];
        assert_eq!(light.position[3], 2.0); // directional
        assert_eq!(light.color[3], 1.0); // intensity
    }

    #[test]
    fn extract_single_entity() {
        let mut world = World::new();
        world.spawn((
            MeshHandle(1),
            MaterialHandle(2),
            WorldTransform::from_translation(0.0, 0.0, 5.0),
        ));
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.draw_count(), 1);
        assert_eq!(snapshot.instance_count(), 1);
        assert_eq!(snapshot.draw_commands[0].mesh_id, 1);
        assert_eq!(snapshot.draw_commands[0].material_id, 2);
        assert_eq!(snapshot.draw_commands[0].first_instance, 0);
        assert_eq!(snapshot.draw_commands[0].instance_count, 1);
        assert_eq!(snapshot.instances[0].mesh_id, 1);
        assert_eq!(snapshot.instances[0].material_id, 2);
    }

    #[test]
    fn extract_multiple_entities_offsets_first_instance() {
        let mut world = World::new();
        world.spawn((MeshHandle(1), MaterialHandle(2), WorldTransform::identity()));
        world.spawn((MeshHandle(3), MaterialHandle(4), WorldTransform::identity()));
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.draw_count(), 2);
        assert_eq!(snapshot.draw_commands[0].first_instance, 0);
        assert_eq!(snapshot.draw_commands[1].first_instance, 1);
    }

    #[test]
    fn extract_entity_without_transform_uses_default() {
        let mut world = World::new();
        world.spawn((MeshHandle(7), MaterialHandle(8)));
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.instance_count(), 1);
        assert_eq!(snapshot.instances[0].mesh_id, 7);
    }

    #[test]
    fn light_from_component_packs_fields() {
        let c = LightComponent::point([1.0, 2.0, 3.0], [0.5, 0.5, 0.5], 1.5);
        let l = Light::from(c);
        assert_eq!(l.position[0], 1.0);
        assert_eq!(l.position[3], 0.0); // point
        assert_eq!(l.color[3], 1.5);
    }
}
