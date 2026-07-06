//! R-043 frame extraction: walks the ECS world and produces a
//! [`FrameSnapshot`] the renderer can consume.
//!
//! The extraction step is intentionally trivial — it iterates
//! the world once, copies the relevant components into the
//! renderer's POD layout, and returns the snapshot. No GPU work
//! happens here; the snapshot is uploaded to the bindless
//! table by the renderer in `render_frame`.
//!
//! R-064 adds the [`StaticMesh`](crate::components::StaticMesh) path: entities
//! carrying typed asset handles (`Handle<MeshAsset>`, `Handle<MaterialAsset>`)
//! are resolved through the [`AssetServer`] and grouped into `DrawCommand`s
//! sorted by `material_id`. The legacy [`MeshHandle`](crate::components::MeshHandle)
//! / [`MaterialHandle`](crate::components::MaterialHandle) path remains for
//! backward compatibility; both paths emit into the same [`FrameSnapshot`] with
//! contiguous `first_instance` offsets.

use bytemuck::{Pod, Zeroable};

use hyge_asset::prelude::{AssetServer, GpuUploadResult};
use hyge_ecs::prelude::*;

use crate::components::{
    AmbientLight, DirectionalLight, GlobalTransform, LightComponent, MaterialHandle, MeshHandle,
    PointLight, SpotLight, StaticBounds, StaticMesh, WorldTransform,
};

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

impl Light {
    /// Builds a directional light from array fields.
    #[must_use]
    pub fn directional(direction: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            position: [0.0, 0.0, 0.0, 2.0],
            color: [color[0], color[1], color[2], intensity],
            direction: [direction[0], direction[1], direction[2], 0.0],
        }
    }

    /// Builds a point light from array fields.
    #[must_use]
    pub fn point(position: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            position: [position[0], position[1], position[2], 0.0],
            color: [color[0], color[1], color[2], intensity],
            direction: [0.0, -1.0, 0.0, 0.0],
        }
    }

    /// Builds a spot light from array fields.
    #[must_use]
    pub fn spot(
        position: [f32; 3],
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        outer_cos: f32,
    ) -> Self {
        Self {
            position: [position[0], position[1], position[2], 1.0],
            color: [color[0], color[1], color[2], intensity],
            direction: [direction[0], direction[1], direction[2], outer_cos],
        }
    }
}

/// The per-frame snapshot the renderer consumes.
#[derive(Resource, Clone, Debug, Default)]
pub struct FrameSnapshot {
    /// Per-instance GPU rows.
    pub instances: Vec<Instance>,
    /// Per-draw commands.
    pub draw_commands: Vec<DrawCommand>,
    /// Lights in the scene (directional + point + spot).
    pub lights: Vec<Light>,
    /// Ambient light.
    pub ambient: Option<Light>,
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
/// The current implementation is a single pass that groups
/// every renderable entity by `(mesh_id, material_id)`, then
/// emits one [`DrawCommand`] per group with
/// `instance_count = group.len()`. This satisfies the R-043
/// acceptance "groups by (mesh_id, material_id) into
/// DrawCommand with instance_count": a scene with 1000
/// entities that share 3 (mesh, material) pairs produces 3
/// draws, not 1000.
pub fn render_extract(world: &mut World) -> FrameSnapshot {
    render_extract_with_culling(world, None)
}

/// Walks a `World` and produces a [`FrameSnapshot`], optionally
/// applying CPU frustum culling to entities that carry
/// [`StaticBounds`].
///
/// Entities without `StaticBounds` remain visible so older scenes and
/// tests keep their previous behavior.
pub fn render_extract_with_culling(
    world: &mut World,
    frustum: Option<&hyge_core::prelude::Frustum>,
) -> FrameSnapshot {
    use std::collections::BTreeMap;

    let mut snapshot = FrameSnapshot::empty();

    // 1. Extract lights from the legacy LightComponent and the canonical
    //    typed light components.
    {
        let mut query = world.query::<&LightComponent>();
        for light in query.iter(world) {
            snapshot.lights.push(Light::from(*light));
        }
    }
    {
        let mut query = world.query::<&PointLight>();
        for light in query.iter(world) {
            snapshot
                .lights
                .push(Light::point(light.color, light.color, light.intensity));
        }
    }
    {
        let mut query = world.query::<&SpotLight>();
        for light in query.iter(world) {
            snapshot.lights.push(Light::spot(
                light.color,
                light.direction,
                light.color,
                light.intensity,
                light.outer_cos,
            ));
        }
    }
    {
        let mut query = world.query::<&DirectionalLight>();
        for light in query.iter(world) {
            snapshot.lights.push(Light::directional(
                light.direction,
                light.color,
                light.illuminance,
            ));
        }
    }
    {
        let mut query = world.query::<&AmbientLight>();
        if let Some(light) = query.iter(world).next() {
            snapshot.ambient = Some(Light {
                position: [0.0; 4],
                color: [
                    light.color[0],
                    light.color[1],
                    light.color[2],
                    light.intensity,
                ],
                direction: [0.0; 4],
            });
        }
    }

    // 2. Extract renderables from the two renderable-component paths:
    //    a) Legacy `MeshHandle` / `MaterialHandle` (raw bindless indices).
    //    b) Canonical `StaticMesh` (typed handles resolved via AssetServer).
    //
    // Both paths group by (mesh_id, material_id) into `DrawCommand`s. The
    // legacy path sorts by mesh_id first (preserving R-043 order so snapshot
    // tests remain byte-stable); the StaticMesh path sorts by material_id
    // first (R-064 acceptance: "sort by material_id") so the renderer can
    // minimize pipeline transitions.

    let mut first_instance: u32 = 0;

    // 2a. Legacy path — `MeshHandle` / `MaterialHandle`.
    //
    // The BTreeMap key packs mesh_id in the high bits and material_id in the
    // low bits, so the iteration order is (mesh_id, material_id) ascending.
    // This stability is required by the M3 snapshot tests.
    let mut legacy_groups: BTreeMap<u64, Vec<Instance>> = BTreeMap::new();
    {
        let mut query = world.query::<(
            &MeshHandle,
            &MaterialHandle,
            Option<&WorldTransform>,
            Option<&GlobalTransform>,
            Option<&StaticBounds>,
        )>();
        for (mesh, material, world_transform, global_transform, bounds) in query.iter(world) {
            // Prefer the canonical GlobalTransform; fall back to the legacy
            // WorldTransform so existing M3 scenes keep rendering.
            let transform: WorldTransform = global_transform
                .copied()
                .map(Into::into)
                .or(world_transform.copied())
                .unwrap_or_default();
            if let (Some(frustum), Some(bounds)) = (frustum, bounds) {
                let local = hyge_core::prelude::Aabb::new(
                    hyge_core::prelude::Vec3::from_array(bounds.min),
                    hyge_core::prelude::Vec3::from_array(bounds.max),
                );
                let matrix = world_transform_to_mat4(transform);
                let world_bounds = transform_aabb(local, matrix);
                if !frustum.intersects_aabb(&world_bounds) {
                    continue;
                }
            }
            let instance = Instance {
                transform: transform.cols,
                mesh_id: mesh.0,
                material_id: material.0,
                _pad: [0; 2],
            };
            let key: u64 = ((mesh.0 as u64) << 32) | (material.0 as u64);
            legacy_groups.entry(key).or_default().push(instance);
        }
    }
    for (key, group) in &legacy_groups {
        let mesh_id = (key >> 32) as u32;
        let material_id = (key & 0xFFFF_FFFF) as u32;
        let instance_count = group.len() as u32;
        snapshot.instances.extend_from_slice(group);
        snapshot.draw_commands.push(DrawCommand {
            mesh_id,
            material_id,
            first_instance,
            instance_count,
            aabb_min: [0.0; 3],
            aabb_max: [1.0; 3],
            _pad: [0; 2],
        });
        first_instance = bump_first_instance(first_instance, instance_count);
    }

    // 2b. StaticMesh path — canonical typed handles resolved via the
    //     `AssetServer` (R-064).
    //
    // The BTreeMap key packs material_id in the high bits and mesh_id in the
    // low bits, so the iteration order is material_id ascending (then
    // mesh_id), satisfying the R-064 acceptance "sort by material_id".
    //
    // If no `AssetServer` resource is present in the world (e.g. a unit test
    // that only uses the legacy path), this path is a no-op. Entities whose
    // handles have not yet been uploaded are silently skipped; they will be
    // picked up on a subsequent frame once the upload completes.
    //
    // `AssetServer` is `Clone` (it shares its inner state via `Arc`), so we
    // clone out of the `World` resource borrow before running the mutable
    // query. This keeps the borrow checker happy without an `unsafe` block
    // or re-entrant `World` access.
    let mut static_groups: BTreeMap<u64, Vec<Instance>> = BTreeMap::new();
    let server_opt: Option<AssetServer> = world.get_resource::<AssetServer>().cloned();
    if let Some(server) = server_opt {
        let mut query = world.query::<(
            &StaticMesh,
            Option<&WorldTransform>,
            Option<&GlobalTransform>,
            Option<&StaticBounds>,
        )>();
        for (static_mesh, world_transform, global_transform, bounds) in query.iter(world) {
            // Resolve the typed handles to raw bindless slot indices.
            let Some(GpuUploadResult::Mesh(mesh_id)) = server.bindless_for(static_mesh.mesh.id())
            else {
                // Mesh not yet uploaded (or evicted) — skip silently.
                tracing::trace!(
                    asset_id = ?static_mesh.mesh.id(),
                    "StaticMesh mesh handle unresolved during extract; skipping entity"
                );
                continue;
            };
            let Some(GpuUploadResult::Material(material_id)) =
                server.bindless_for(static_mesh.material.id())
            else {
                tracing::trace!(
                    asset_id = ?static_mesh.material.id(),
                    "StaticMesh material handle unresolved during extract; skipping entity"
                );
                continue;
            };
            let mesh_idx = mesh_id.index();
            let material_idx = material_id.index();

            let transform: WorldTransform = global_transform
                .copied()
                .map(Into::into)
                .or(world_transform.copied())
                .unwrap_or_default();
            if let (Some(frustum), Some(bounds)) = (frustum, bounds) {
                let local = hyge_core::prelude::Aabb::new(
                    hyge_core::prelude::Vec3::from_array(bounds.min),
                    hyge_core::prelude::Vec3::from_array(bounds.max),
                );
                let matrix = world_transform_to_mat4(transform);
                let world_bounds = transform_aabb(local, matrix);
                if !frustum.intersects_aabb(&world_bounds) {
                    continue;
                }
            }
            let instance = Instance {
                transform: transform.cols,
                mesh_id: mesh_idx,
                material_id: material_idx,
                _pad: [0; 2],
            };
            // Sort by material_id first (high bits), then mesh_id (low bits).
            let key: u64 = ((material_idx as u64) << 32) | (mesh_idx as u64);
            static_groups.entry(key).or_default().push(instance);
        }
    }
    for (key, group) in &static_groups {
        let material_id = (key >> 32) as u32;
        let mesh_id = (key & 0xFFFF_FFFF) as u32;
        let instance_count = group.len() as u32;
        snapshot.instances.extend_from_slice(group);
        snapshot.draw_commands.push(DrawCommand {
            mesh_id,
            material_id,
            first_instance,
            instance_count,
            aabb_min: [0.0; 3],
            aabb_max: [1.0; 3],
            _pad: [0; 2],
        });
        first_instance = bump_first_instance(first_instance, instance_count);
    }

    snapshot
}

/// Increments `first_instance` by `instance_count`, panicking on `u32`
/// overflow. The draw ABI uses `u32` for both fields; overflowing that
/// contract is a hard error rather than something to silently saturate. If a
/// future scene needs more than `u32::MAX` instances the ABI must widen
/// first.
fn bump_first_instance(first_instance: u32, instance_count: u32) -> u32 {
    first_instance
        .checked_add(instance_count)
        .expect("render_extract: first_instance overflowed u32 ABI")
}

fn world_transform_to_mat4(transform: WorldTransform) -> hyge_core::prelude::Mat4 {
    hyge_core::prelude::Mat4::from_cols_array(&[
        transform.cols[0][0],
        transform.cols[1][0],
        transform.cols[2][0],
        0.0,
        transform.cols[0][1],
        transform.cols[1][1],
        transform.cols[2][1],
        0.0,
        transform.cols[0][2],
        transform.cols[1][2],
        transform.cols[2][2],
        0.0,
        transform.cols[0][3],
        transform.cols[1][3],
        transform.cols[2][3],
        1.0,
    ])
}

fn transform_aabb(
    aabb: hyge_core::prelude::Aabb,
    transform: hyge_core::prelude::Mat4,
) -> hyge_core::prelude::Aabb {
    let corners = [
        hyge_core::prelude::Vec3::new(aabb.min.x, aabb.min.y, aabb.min.z),
        hyge_core::prelude::Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
        hyge_core::prelude::Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
        hyge_core::prelude::Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
        hyge_core::prelude::Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
        hyge_core::prelude::Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
        hyge_core::prelude::Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
        hyge_core::prelude::Vec3::new(aabb.max.x, aabb.max.y, aabb.max.z),
    ];
    let mut out = hyge_core::prelude::Aabb::EMPTY;
    for corner in corners {
        out.merge(&hyge_core::prelude::Aabb::from_point(
            transform.transform_point3(corner),
        ));
    }
    out
}

/// Exclusive ECS system that writes the current frame's
/// [`FrameSnapshot`] into the world as a resource. Intended to
/// be scheduled on [`hyge_ecs::schedule::Label::RenderExtract`]
/// so the render thread can consume the snapshot after the main
/// world has finished its update schedules.
pub fn render_extract_system(world: &mut World) {
    let snapshot = render_extract(world);
    if let Some(mut res) = world.get_resource_mut::<FrameSnapshot>() {
        *res = snapshot;
    } else {
        world.insert_resource(snapshot);
    }
}

/// Registers the render-extract system into the provided
/// schedule. This is a small convenience wrapper so downstream
/// crates can say `add_render_extract_system(&mut schedule)`
/// without needing to name the system function directly.
pub fn add_render_extract_system(schedule: &mut bevy_ecs::schedule::Schedule) {
    schedule.add_systems(render_extract_system);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{MeshHandle, StaticMesh, WorldTransform};
    use hyge_asset::prelude::{Handle, MaterialAsset, MeshAsset};

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
        // One entity in this group -> instance_count = 1.
        assert_eq!(snapshot.draw_commands[0].instance_count, 1);
        assert_eq!(snapshot.instances[0].mesh_id, 1);
        assert_eq!(snapshot.instances[0].material_id, 2);
    }

    /// R-043 acceptance #3: 1000+ entities that share
    /// `(mesh, material)` collapse into a small number of
    /// `DrawCommand`s with `instance_count` reflecting the
    /// per-group count. The test uses a smaller fixture
    /// (10 + 5 + 3 = 18 entities / 3 groups) to stay fast.
    #[test]
    fn extract_groups_by_mesh_and_material() {
        let mut world = World::new();
        // Group A: (m=1, mat=1) x 10.
        for _ in 0..10 {
            world.spawn((MeshHandle(1), MaterialHandle(1), WorldTransform::identity()));
        }
        // Group B: (m=1, mat=2) x 5.
        for _ in 0..5 {
            world.spawn((MeshHandle(1), MaterialHandle(2), WorldTransform::identity()));
        }
        // Group C: (m=2, mat=1) x 3.
        for _ in 0..3 {
            world.spawn((MeshHandle(2), MaterialHandle(1), WorldTransform::identity()));
        }

        let snapshot = render_extract(&mut world);
        // Three (mesh, material) groups -> three draws.
        assert_eq!(snapshot.draw_count(), 3);
        assert_eq!(snapshot.instance_count(), 18);

        // Find each group by its (mesh, material) and
        // check instance_count. Order is sorted by the
        // BTreeMap key (mesh << 32 | mat).
        let mut counts: Vec<(u32, u32, u32)> = snapshot
            .draw_commands
            .iter()
            .map(|dc| (dc.mesh_id, dc.material_id, dc.instance_count))
            .collect();
        counts.sort_by_key(|&(m, mat, _)| (m, mat));
        assert_eq!(
            counts,
            vec![(1, 1, 10), (1, 2, 5), (2, 1, 3)],
            "grouping must produce one DrawCommand per (mesh, material) with the right instance_count"
        );

        // first_instance values must be a contiguous
        // walk: the first group's `first_instance` is 0,
        // and the total spans 0..18.
        let total: u32 = snapshot
            .draw_commands
            .iter()
            .map(|dc| dc.instance_count)
            .sum();
        assert_eq!(total, 18);

        // Each instance buffer entry must reference one
        // of the three (mesh, material) pairs.
        for inst in &snapshot.instances {
            let in_group_a_or_b =
                inst.mesh_id == 1 && (inst.material_id == 1 || inst.material_id == 2);
            let in_group_c = inst.mesh_id == 2 && inst.material_id == 1;
            assert!(
                in_group_a_or_b || in_group_c,
                "instance must belong to one of the three groups: {inst:?}"
            );
        }
    }

    #[test]
    fn extract_multiple_entities_offsets_first_instance() {
        let mut world = World::new();
        // Two distinct (mesh, material) groups. After
        // grouping, each gets its own DrawCommand; the
        // second draw's first_instance = the first group's
        // instance_count.
        world.spawn((MeshHandle(1), MaterialHandle(2), WorldTransform::identity()));
        world.spawn((MeshHandle(3), MaterialHandle(4), WorldTransform::identity()));
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.draw_count(), 2);
        // The (mesh, material) keys are sorted; the (1, 2)
        // group is first.
        let dc_a = snapshot
            .draw_commands
            .iter()
            .find(|dc| dc.mesh_id == 1 && dc.material_id == 2)
            .expect("group A present");
        let dc_b = snapshot
            .draw_commands
            .iter()
            .find(|dc| dc.mesh_id == 3 && dc.material_id == 4)
            .expect("group B present");
        assert_eq!(dc_a.first_instance, 0);
        assert_eq!(dc_a.instance_count, 1);
        assert_eq!(dc_b.first_instance, 1);
        assert_eq!(dc_b.instance_count, 1);
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

    #[test]
    fn render_extract_system_inserts_snapshot_resource() {
        let mut world = World::new();
        world.spawn((MeshHandle(1), MaterialHandle(2), WorldTransform::identity()));
        render_extract_system(&mut world);
        let snapshot = world
            .get_resource::<FrameSnapshot>()
            .expect("snapshot resource inserted");
        assert_eq!(snapshot.draw_count(), 1);
        assert_eq!(snapshot.instance_count(), 1);
    }

    #[test]
    fn add_render_extract_system_runs_in_schedule() {
        use hyge_ecs::prelude::Schedule;
        use hyge_ecs::schedule::Label;

        let mut world = World::new();
        world.spawn((MeshHandle(7), MaterialHandle(8), WorldTransform::identity()));
        let mut schedule = Schedule::new(Label::RenderExtract);
        add_render_extract_system(&mut schedule);
        schedule.run(&mut world);
        let snapshot = world
            .get_resource::<FrameSnapshot>()
            .expect("snapshot resource inserted by schedule");
        assert_eq!(snapshot.draw_count(), 1);
        assert_eq!(snapshot.instance_count(), 1);
        assert_eq!(snapshot.draw_commands[0].mesh_id, 7);
        assert_eq!(snapshot.draw_commands[0].material_id, 8);
    }

    #[test]
    fn extract_prefers_global_transform() {
        use crate::components::GlobalTransform;
        use hyge_core::prelude::{Mat4, Vec3};

        let mut world = World::new();
        world.spawn((
            MeshHandle(1),
            MaterialHandle(1),
            GlobalTransform::from(Mat4::from_translation(Vec3::Y)),
            WorldTransform::from_translation(0.0, 0.0, 5.0),
        ));
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.instances[0].transform[0][3], 0.0);
        assert_eq!(snapshot.instances[0].transform[1][3], 1.0);
        assert_eq!(snapshot.instances[0].transform[2][3], 0.0);
    }

    #[test]
    fn extract_canonical_point_light() {
        let mut world = World::new();
        world.spawn(PointLight {
            color: [1.0, 0.0, 0.0],
            intensity: 2.0,
            range: 5.0,
        });
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.light_count(), 1);
        assert_eq!(snapshot.lights[0].position[3], 0.0); // point
        assert_eq!(snapshot.lights[0].color[0], 1.0);
        assert_eq!(snapshot.lights[0].color[3], 2.0);
    }

    #[test]
    fn extract_canonical_directional_light() {
        let mut world = World::new();
        world.spawn(DirectionalLight {
            color: [0.0, 1.0, 0.0],
            illuminance: 50_000.0,
            direction: [0.0, -1.0, 0.0],
        });
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.light_count(), 1);
        assert_eq!(snapshot.lights[0].position[3], 2.0); // directional
    }

    #[test]
    fn extract_ambient_light() {
        let mut world = World::new();
        world.spawn(AmbientLight {
            color: [1.0, 1.0, 1.0],
            intensity: 0.1,
        });
        let snapshot = render_extract(&mut world);
        assert!(snapshot.ambient.is_some());
        assert_eq!(snapshot.ambient.unwrap().color[3], 0.1);
    }

    // =========================================================================
    // R-064 — StaticMesh path
    // =========================================================================

    /// Two handles with distinct ids so we can recognize them in tests.
    fn dummy_handles() -> (Handle<MeshAsset>, Handle<MaterialAsset>) {
        let mesh_id = hyge_asset::prelude::AssetId::from(blake3::hash(b"r-064-mesh"));
        let material_id = hyge_asset::prelude::AssetId::from(blake3::hash(b"r-064-mat"));
        (
            Handle::<MeshAsset>::new(mesh_id),
            Handle::<MaterialAsset>::new(material_id),
        )
    }

    /// R-064 acceptance #1: "Query With<StaticMesh> iterates entities."
    ///
    /// Without an `AssetServer` resource in the world, `StaticMesh` entities
    /// cannot be resolved and produce no draw commands. The test verifies the
    /// query executes without panicking and the snapshot is empty (the
    /// graceful-degradation path).
    #[test]
    fn extract_static_mesh_no_server_produces_no_draws() {
        let (mesh, material) = dummy_handles();
        let mut world = World::new();
        world.spawn((StaticMesh::new(mesh, material), WorldTransform::identity()));
        let snapshot = render_extract(&mut world);
        assert_eq!(
            snapshot.draw_count(),
            0,
            "no AssetServer -> StaticMesh entities are skipped silently"
        );
        assert_eq!(snapshot.instance_count(), 0);
    }

    /// R-064: `StaticMesh` entities coexist with legacy `MeshHandle` entities
    /// in the same snapshot. The legacy entities still emit their draws; the
    /// StaticMesh entities (with no server) produce nothing. This guards the
    /// backward-compatibility contract.
    #[test]
    fn extract_legacy_and_static_paths_coexist() {
        let (mesh, material) = dummy_handles();
        let mut world = World::new();
        // Legacy entity.
        world.spawn((MeshHandle(1), MaterialHandle(2), WorldTransform::identity()));
        // StaticMesh entity (unresolved handle -> skipped).
        world.spawn((StaticMesh::new(mesh, material), WorldTransform::identity()));

        let snapshot = render_extract(&mut world);
        // Only the legacy entity contributes a DrawCommand.
        assert_eq!(snapshot.draw_count(), 1);
        assert_eq!(snapshot.instance_count(), 1);
        assert_eq!(snapshot.draw_commands[0].mesh_id, 1);
        assert_eq!(snapshot.draw_commands[0].material_id, 2);
    }

    /// R-064: a `StaticMesh` entity with a transform but no server is still
    /// query-able (no panic); the snapshot instances vec stays empty. This is
    /// the "handle resolution -> mesh_id, material_id from BindlessTable"
    /// acceptance bullet in its negative form.
    #[test]
    fn extract_static_mesh_unresolved_handles_skipped() {
        let (mesh, material) = dummy_handles();
        let mut world = World::new();
        // Spawn 5 StaticMesh entities — none resolve without an AssetServer.
        for _ in 0..5 {
            world.spawn((
                StaticMesh::new(mesh, material),
                WorldTransform::from_translation(1.0, 2.0, 3.0),
            ));
        }
        let snapshot = render_extract(&mut world);
        assert_eq!(snapshot.draw_count(), 0);
        assert_eq!(snapshot.instance_count(), 0);
    }
}
