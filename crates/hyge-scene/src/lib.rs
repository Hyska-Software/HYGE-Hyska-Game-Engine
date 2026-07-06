//! Hyge scene: glTF loader, prefab system, instancing extraction, and the
//! canonical ECS component catalog.
//!
//! Owns the `Prefab` / `PrefabNode` / `ComponentOverride` types and the
//! `.hyge-prefab` / `.hyge-world` serialization formats. Performs meshlet
//! bake via `meshopt` during import (delegated to the importer in `hyge-tools`).
//!
//! See `docs/architecture.md` §6.6 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-035, R-060..R-064.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod components;
pub mod extract;
pub mod plugin;
pub mod prelude;
pub mod transform;

pub use components::{
    AmbientLight, AudioBus, AudioListener, AudioRolloff, AudioSource, Camera, CharacterController,
    Children, Collider, ColliderShape, DirectionalLight, EditorCamera, FogVolume, GlobalTransform,
    Joint, LightComponent, MaterialHandle, MeshHandle, Name, Parent, PersistOnReload, PointLight,
    PostProcessVolume, RigidBody, RigidBodyKind, ScriptRef, SpotLight, Transform, WorldTransform,
};
pub use plugin::ScenePlugin;
