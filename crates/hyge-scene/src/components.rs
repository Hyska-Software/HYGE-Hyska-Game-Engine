//! R-060 / R-061 ECS component catalog.
//!
//! This module defines the canonical scene components used by the Hyge
//! engine. Components are first-class Rust types that derive [`Component`]
//! and [`Reflect`] so they can be inspected by the editor and manipulated by
//! scripts (ADR-0008).
//!
//! The module keeps a small set of "legacy" render-facing POD components
//! (`WorldTransform`, `LightComponent`, `MeshHandle`, `MaterialHandle`)
//! introduced in R-043/M3. They remain public for backward compatibility
//! with existing render tests and are gradually superseded by the
//! canonical scene types (`Transform` + `GlobalTransform` and the typed
//! light components).
//!
//! Because `glam` types do not implement `Reflect` out of the box,
//! vector/quaternion fields are stored as plain `[f32; N]` arrays.
//! Conversion helpers keep the public API ergonomic.

use bevy_ecs::prelude::{Component, Entity};
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::Reflect;
use bytemuck::{Pod, Zeroable};
use hyge_core::prelude::{Mat4, Quat, Vec3};

// =============================================================================
// R-060 — Core scene components
// =============================================================================

/// Local transform of an entity relative to its parent.
///
/// The local transformation matrix is `T * R * S`:
/// scale first, then rotation, then translation. This matches the
/// convention used by glTF and most DCC tools.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct Transform {
    /// Translation in parent space.
    pub translation: [f32; 3],
    /// Rotation in parent space, stored as XYZW quaternion.
    pub rotation: [f32; 4],
    /// Non-uniform scale in parent space.
    pub scale: [f32; 3],
}

impl Transform {
    /// Returns the identity transform (zero translation, identity rotation,
    /// unit scale).
    #[must_use]
    pub fn identity() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }

    /// Builds a transform from a translation, rotation and scale.
    #[must_use]
    pub fn from_trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            translation: translation.into(),
            rotation: rotation.into(),
            scale: scale.into(),
        }
    }

    /// Computes the column-major 4x4 matrix representing this local transform.
    #[must_use]
    pub fn compute_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::from(self.scale),
            Quat::from_array(self.rotation),
            Vec3::from(self.translation),
        )
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

/// World-space transform of an entity.
///
/// This is the authoritative world matrix used by the renderer and by
/// gameplay systems. It is written by
/// [`transform_propagate_system`](crate::transform::transform_propagate_system)
/// every frame.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct GlobalTransform {
    /// Column-major 4x4 matrix stored as four columns.
    pub matrix: [[f32; 4]; 4],
}

impl GlobalTransform {
    /// Returns the identity global transform.
    #[must_use]
    pub fn identity() -> Self {
        Self {
            matrix: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// Returns the matrix as a `glam::Mat4`.
    #[must_use]
    pub fn to_matrix(&self) -> Mat4 {
        Mat4::from_cols_array_2d(&self.matrix)
    }
}

impl From<Mat4> for GlobalTransform {
    fn from(matrix: Mat4) -> Self {
        Self {
            matrix: matrix.to_cols_array_2d(),
        }
    }
}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self::identity()
    }
}

/// Parent entity reference. An entity with this component belongs to a
/// hierarchy rooted at `0`.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq)]
#[reflect(Component)]
pub struct Parent(pub Entity);

impl Default for Parent {
    fn default() -> Self {
        Self(Entity::PLACEHOLDER)
    }
}

/// Children entity references, stored on the parent.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Eq, Default)]
#[reflect(Component)]
pub struct Children(pub Vec<Entity>);

/// Human-readable name for an entity.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Eq, Default)]
#[reflect(Component)]
pub struct Name(pub String);

impl Name {
    /// Convenience constructor.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

/// Marker component: the entity survives a scene hot-reload.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[reflect(Component)]
pub struct PersistOnReload;

// =============================================================================
// Legacy render-facing components (R-043 / M3)
// =============================================================================

/// The bindless mesh id (slot index in the bindless table).
/// Mirrors the `BindlessSlot<MeshTag>` index exposed by
/// `hyge_render::bindless`; stored on the ECS as a plain `u32`
/// so the scene does not need to import the render type.
#[derive(Component, Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Component)]
pub struct MeshHandle(pub u32);

/// The bindless material id.
#[derive(Component, Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Reflect)]
#[reflect(Component)]
pub struct MaterialHandle(pub u32);

/// World-space transform of a renderable entity. Three rows of
/// a column-major affine matrix; the fourth component is the
/// `w` of the homogeneous row and is left at `1.0`.
#[repr(C)]
#[derive(Component, Copy, Clone, Debug, Default, Pod, Zeroable, Reflect)]
#[reflect(Component)]
pub struct WorldTransform {
    /// Column-major 3x4 affine matrix.
    pub cols: [[f32; 4]; 3],
}

impl WorldTransform {
    /// Builds an identity transform.
    #[must_use]
    pub fn identity() -> Self {
        Self {
            cols: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        }
    }

    /// Builds a translation-only transform.
    #[must_use]
    pub fn from_translation(x: f32, y: f32, z: f32) -> Self {
        Self {
            cols: [[1.0, 0.0, 0.0, x], [0.0, 1.0, 0.0, y], [0.0, 0.0, 1.0, z]],
        }
    }
}

impl From<GlobalTransform> for WorldTransform {
    fn from(global: GlobalTransform) -> Self {
        let m = global.to_matrix();
        Self {
            cols: [
                [m.x_axis.x, m.y_axis.x, m.z_axis.x, m.w_axis.x],
                [m.x_axis.y, m.y_axis.y, m.z_axis.y, m.w_axis.y],
                [m.x_axis.z, m.y_axis.z, m.z_axis.z, m.w_axis.z],
            ],
        }
    }
}

/// Packed GPU-friendly light component (legacy R-043).
/// The typed light components below are the canonical scene source.
#[derive(Component, Copy, Clone, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct LightComponent {
    /// World-space position. `w` is light type (0=point, 1=spot,
    /// 2=directional).
    pub position: [f32; 4],
    /// RGB color in `xyz`, scalar intensity in `w`.
    pub color_intensity: [f32; 4],
    /// Direction (xyz) for spot/directional, cos(outer_angle) in `w`.
    pub direction_cos_outer: [f32; 4],
}

impl LightComponent {
    /// Builds a directional sun.
    #[must_use]
    pub fn sun(direction: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            position: [0.0, 0.0, 0.0, 2.0],
            color_intensity: [color[0], color[1], color[2], intensity],
            direction_cos_outer: [direction[0], direction[1], direction[2], 0.0],
        }
    }

    /// Builds a point light.
    #[must_use]
    pub fn point(position: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            position: [position[0], position[1], position[2], 0.0],
            color_intensity: [color[0], color[1], color[2], intensity],
            direction_cos_outer: [0.0, -1.0, 0.0, 0.0],
        }
    }
}

// =============================================================================
// R-061 — Lights
// =============================================================================

/// A point light emits uniformly in every direction from a position.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct PointLight {
    /// World-space color (linear RGB).
    pub color: [f32; 3],
    /// Luminous intensity in candela.
    pub intensity: f32,
    /// Attenuation radius; the light has no effect beyond this distance.
    pub range: f32,
}

/// A spot light emits in a cone from a position.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct SpotLight {
    /// World-space color (linear RGB).
    pub color: [f32; 3],
    /// Luminous intensity in candela.
    pub intensity: f32,
    /// Cosine of the inner (fully lit) cone half-angle.
    pub inner_cos: f32,
    /// Cosine of the outer (attenuated to zero) cone half-angle.
    pub outer_cos: f32,
    /// Local forward direction of the light.
    pub direction: [f32; 3],
    /// Attenuation radius.
    pub range: f32,
}

/// A directional light (sun / moon).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct DirectionalLight {
    /// Light direction (from the light toward the scene).
    pub direction: [f32; 3],
    /// Color (linear RGB).
    pub color: [f32; 3],
    /// Illuminance in lux.
    pub illuminance: f32,
}

/// Ambient light applied to the whole scene.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct AmbientLight {
    /// Ambient color (linear RGB).
    pub color: [f32; 3],
    /// Scalar intensity.
    pub intensity: f32,
}

// =============================================================================
// R-061 — Camera
// =============================================================================

/// Runtime camera component.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct Camera {
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Near plane distance.
    pub near: f32,
    /// Far plane distance.
    pub far: f32,
    /// Aspect ratio override; `None` uses the target's aspect ratio.
    pub aspect_ratio: Option<f32>,
    /// Rendering order for split-screen / picture-in-picture.
    pub priority: i32,
}

impl Camera {
    /// Builds a perspective camera with sensible defaults.
    #[must_use]
    pub fn perspective(fov_y: f32, near: f32, far: f32) -> Self {
        Self {
            fov_y,
            near,
            far,
            aspect_ratio: None,
            priority: 0,
        }
    }
}

/// Marker for the editor fly camera; runtime uses [`Camera`].
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[reflect(Component)]
pub struct EditorCamera;

// =============================================================================
// R-061 — Audio
// =============================================================================

/// An audio source attached to an entity.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct AudioSource {
    /// Asset path to the audio clip.
    pub clip: String,
    /// Playback volume in linear units.
    pub volume: f32,
    /// Pitch multiplier.
    pub pitch: f32,
    /// Whether the source loops.
    pub r#loop: bool,
    /// Whether the source starts playing as soon as it spawns.
    pub auto_play: bool,
    /// Target bus name.
    pub bus: String,
}

impl AudioSource {
    /// Builds a one-shot audio source.
    #[must_use]
    pub fn once(clip: impl Into<String>) -> Self {
        Self {
            clip: clip.into(),
            volume: 1.0,
            pitch: 1.0,
            r#loop: false,
            auto_play: true,
            bus: String::new(),
        }
    }
}

/// Audio rolloff model.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AudioRolloff {
    /// No distance attenuation.
    #[default]
    None,
    /// Linear rolloff.
    Linear,
    /// Inverse distance rolloff.
    Inverse,
    /// Exponential rolloff.
    Exponential,
}

/// An audio listener attached to an entity (typically the main camera).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct AudioListener {
    /// Output bus index; `-1` means the default master bus.
    pub bus_index: i32,
    /// Rolloff model used for spatial sources.
    pub rolloff: AudioRolloff,
}

/// Named audio bus routing.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Eq, Default)]
#[reflect(Component)]
pub struct AudioBus {
    /// Bus name.
    pub name: String,
}

// =============================================================================
// R-061 — Scripting
// =============================================================================

/// Reference to a Lua script asset.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct ScriptRef {
    /// Asset path (`.lua`).
    pub path: String,
    /// Optional named table / module within the script.
    pub table: Option<String>,
    /// Whether the script is currently executed.
    pub enabled: bool,
}

impl ScriptRef {
    /// Builds an enabled script reference.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            table: None,
            enabled: true,
        }
    }

    /// Sets the optional table/module name.
    #[must_use]
    pub fn table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }
}

// =============================================================================
// R-061 — Physics stubs (delegated to hyge-physics in phase 7)
// =============================================================================

/// Rigid-body type hint.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RigidBodyKind {
    /// Static immovable collider.
    #[default]
    Static,
    /// Kinematic rigid body.
    Kinematic,
    /// Dynamic simulated rigid body.
    Dynamic,
}

/// Rigid-body component (stub).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct RigidBody {
    /// Rigid-body kind.
    pub kind: RigidBodyKind,
    /// Linear velocity in m/s.
    pub linear_velocity: [f32; 3],
    /// Angular velocity in rad/s.
    pub angular_velocity: [f32; 3],
    /// Linear damping.
    pub linear_damping: f32,
    /// Angular damping.
    pub angular_damping: f32,
    /// Is gravity enabled for this body?
    pub gravity: bool,
    /// Continuous collision detection flag.
    pub ccd: bool,
    /// Mass in kg; `0.0` means auto-compute from colliders.
    pub mass: f32,
}

/// Collider shape (stub).
#[derive(Reflect, Clone, Debug, PartialEq, Default)]
pub enum ColliderShape {
    /// Empty / trigger collider.
    #[default]
    Empty,
    /// Sphere of the given radius.
    Sphere(f32),
    /// Axis-aligned bounding box of the given half-extents.
    Box([f32; 3]),
    /// Capsule aligned with the Y axis.
    Capsule {
        /// Half-height of the capsule cylinder portion.
        half_height: f32,
        /// Radius of the capsule hemispheres.
        radius: f32,
    },
    /// Convex hull from a set of points.
    ConvexHull(Vec<[f32; 3]>),
    /// Triangle mesh collider. The string is the asset path.
    Trimesh(String),
}

/// Collider component (stub).
#[derive(Component, Reflect, Clone, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct Collider {
    /// Collider shape.
    pub shape: ColliderShape,
    /// Static friction coefficient.
    pub friction: f32,
    /// Restitution coefficient.
    pub restitution: f32,
    /// Density in kg/m^3; used for mass computation.
    pub density: f32,
    /// Is this collider a trigger (sensor)?
    pub is_sensor: bool,
    /// Collision layer.
    pub layer: u32,
    /// Collision mask.
    pub mask: u32,
}

/// Character controller component (stub).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct CharacterController {
    /// Maximum horizontal speed in m/s.
    pub max_speed: f32,
    /// Jump impulse in m/s.
    pub jump_impulse: f32,
    /// Is the character on the ground?
    pub grounded: bool,
    /// Slope angle limit in degrees.
    pub slope_limit: f32,
    /// Step height in meters.
    pub step_height: f32,
}

/// Joint component (stub).
#[derive(Component, Reflect, Clone, Debug, PartialEq, Eq)]
#[reflect(Component)]
pub struct Joint {
    /// Entity of the other body connected by this joint.
    pub other: Entity,
    /// Joint type identifier used by the physics backend.
    pub kind: String,
}

impl Default for Joint {
    fn default() -> Self {
        Self {
            other: Entity::PLACEHOLDER,
            kind: String::new(),
        }
    }
}

// =============================================================================
// R-061 — Volumes (placeholders)
// =============================================================================

/// Post-process volume (placeholder).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct PostProcessVolume {
    /// Blend weight of this volume.
    pub weight: f32,
    /// Priority order (higher wins).
    pub priority: i32,
    /// Is this a global volume?
    pub global: bool,
}

/// Fog volume (placeholder).
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct FogVolume {
    /// Fog density.
    pub density: f32,
    /// Fog color.
    pub color: [f32; 3],
    /// Height falloff start.
    pub height_start: f32,
    /// Height falloff scale.
    pub height_falloff: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_reflect::Reflect;

    fn assert_reflect_round_trip<T>(value: T)
    where
        T: Reflect + Clone + PartialEq + std::fmt::Debug + 'static,
    {
        let reflected = (&value as &dyn Reflect).clone_value();
        let mut restored = value.clone();
        restored.apply(&*reflected);
        assert_eq!(value, restored);
    }

    #[test]
    fn transform_default_is_identity() {
        let t = Transform::default();
        assert_eq!(t.translation, [0.0; 3]);
        assert_eq!(t.rotation, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(t.scale, [1.0; 3]);
        assert_eq!(t.compute_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn global_transform_default_is_identity() {
        let g = GlobalTransform::default();
        assert_eq!(g.to_matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn parent_default_is_placeholder() {
        assert_eq!(Parent::default().0, Entity::PLACEHOLDER);
    }

    #[test]
    fn point_light_default_round_trip() {
        let light = PointLight {
            color: [1.0, 0.5, 0.25],
            intensity: 42.0,
            range: 10.0,
        };
        assert_reflect_round_trip(light);
    }

    #[test]
    fn spot_light_reflect_round_trip() {
        let light = SpotLight {
            color: [1.0, 1.0, 1.0],
            intensity: 100.0,
            inner_cos: 0.9,
            outer_cos: 0.7,
            direction: [0.0, -1.0, 0.0],
            range: 20.0,
        };
        assert_reflect_round_trip(light);
    }

    #[test]
    fn directional_light_reflect_round_trip() {
        let light = DirectionalLight {
            direction: [0.0, -1.0, 0.0],
            color: [1.0, 1.0, 1.0],
            illuminance: 80_000.0,
        };
        assert_reflect_round_trip(light);
    }

    #[test]
    fn ambient_light_reflect_round_trip() {
        let light = AmbientLight {
            color: [0.1; 3],
            intensity: 0.5,
        };
        assert_reflect_round_trip(light);
    }

    #[test]
    fn camera_reflect_round_trip() {
        let camera = Camera::perspective(1.0, 0.1, 1000.0);
        assert_reflect_round_trip(camera);
    }

    #[test]
    fn editor_camera_reflect_round_trip() {
        let camera = EditorCamera;
        assert_reflect_round_trip(camera);
    }

    #[test]
    fn audio_source_reflect_round_trip() {
        let source = AudioSource::once("sfx/explosion.wav");
        assert_reflect_round_trip(source);
    }

    #[test]
    fn audio_listener_reflect_round_trip() {
        let listener = AudioListener {
            bus_index: 0,
            rolloff: AudioRolloff::Inverse,
        };
        assert_reflect_round_trip(listener);
    }

    #[test]
    fn script_ref_reflect_round_trip() {
        let script = ScriptRef {
            path: "scripts/player.lua".into(),
            table: Some("Player".into()),
            enabled: true,
        };
        assert_reflect_round_trip(script);
    }

    #[test]
    fn rigid_body_reflect_round_trip() {
        let body = RigidBody {
            kind: RigidBodyKind::Dynamic,
            linear_velocity: [1.0, 0.0, 0.0],
            angular_velocity: [0.0; 3],
            linear_damping: 0.1,
            angular_damping: 0.1,
            gravity: true,
            ccd: false,
            mass: 70.0,
        };
        assert_reflect_round_trip(body);
    }

    #[test]
    fn collider_reflect_round_trip() {
        let collider = Collider {
            shape: ColliderShape::Box([1.0, 2.0, 3.0]),
            friction: 0.5,
            restitution: 0.1,
            density: 1000.0,
            is_sensor: false,
            layer: 1,
            mask: u32::MAX,
        };
        assert_reflect_round_trip(collider);
    }

    #[test]
    fn character_controller_reflect_round_trip() {
        let cc = CharacterController {
            max_speed: 5.0,
            jump_impulse: 8.0,
            grounded: false,
            slope_limit: 45.0,
            step_height: 0.3,
        };
        assert_reflect_round_trip(cc);
    }

    #[test]
    fn joint_default_round_trip() {
        let joint = Joint::default();
        assert_reflect_round_trip(joint);
    }

    #[test]
    fn post_process_volume_reflect_round_trip() {
        let vol = PostProcessVolume {
            weight: 1.0,
            priority: 0,
            global: true,
        };
        assert_reflect_round_trip(vol);
    }

    #[test]
    fn fog_volume_reflect_round_trip() {
        let vol = FogVolume {
            density: 0.05,
            color: [0.7, 0.8, 0.9],
            height_start: 0.0,
            height_falloff: 10.0,
        };
        assert_reflect_round_trip(vol);
    }
}
