//! R-063 — Scene environment types.
//!
//! [`Environment`] aggregates the global scene-level configuration that is
//! independent of any single entity: skybox asset, sun light, fog and ambient
//! lighting. [`PostProcessProfile`] carries the per-scene default for the
//! post-process chain rendered by `hyge-render`.
//!
//! These types are the *scene descriptor* — they live inside `.hyge-world`
//! files and are loaded by [`crate::world::WorldLoader`]. They are converted
//! into renderer-side resources (e.g. `hyge_render::ibl::EnvironmentBake`,
//! `hyge_render::post::PostProcessConfig`) in later roadmap items (R-064 /
//! R-065).
//!
//! Because `glam` types do not implement `Reflect` out of the box, vector
//! fields are stored as plain `[f32; N]` arrays — mirroring the convention
//! used by [`crate::components`].

use bevy_ecs::prelude::Component;
use bevy_ecs::reflect::ReflectComponent;
use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

use hyge_asset::AssetId;

use crate::components::DirectionalLight;

// =============================================================================
// Environment
// =============================================================================

/// Global scene environment.
///
/// Stored inside a `.hyge-world` file (see [`crate::world::WorldDocument`])
/// and applied to the renderer once the scene is loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    /// Skybox / IBL asset reference (`.hyge-env`). `None` means the scene
    /// uses a flat clear color.
    pub skybox: Option<AssetId>,
    /// Primary directional light (sun / moon). `None` disables the sun.
    pub sun: Option<DirectionalLight>,
    /// Optional fog configuration.
    pub fog: Option<FogParams>,
    /// Scene-global ambient lighting.
    pub ambient: AmbientParams,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            skybox: None,
            sun: Some(DirectionalLight {
                direction: [0.0, -1.0, 0.0],
                color: [1.0, 1.0, 1.0],
                illuminance: 80_000.0,
            }),
            fog: None,
            ambient: AmbientParams::default(),
        }
    }
}

impl Environment {
    /// Builds an empty environment: no skybox, no sun, no fog, dim ambient.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            skybox: None,
            sun: None,
            fog: None,
            ambient: AmbientParams {
                color: [0.0; 3],
                intensity: 0.0,
            },
        }
    }
}

// =============================================================================
// Fog
// =============================================================================

/// Fog parameters applied scene-wide. Mirrors [`crate::components::FogVolume`]
/// but stored at the scene level instead of as an entity-bound volume.
#[derive(Component, Reflect, Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Component)]
pub struct FogParams {
    /// Fog color (linear RGB).
    pub color: [f32; 3],
    /// Volumetric density (0.0 = no fog).
    pub density: f32,
    /// Height at which the exponential falloff starts.
    pub height_start: f32,
    /// Height falloff rate (per metre); higher = thinner band.
    pub height_falloff: f32,
}

impl Default for FogParams {
    fn default() -> Self {
        Self {
            color: [0.7, 0.8, 0.9],
            density: 0.02,
            height_start: 0.0,
            height_falloff: 10.0,
        }
    }
}

// =============================================================================
// Ambient
// =============================================================================

/// Scene-global ambient lighting parameters.
///
/// Distinct from the entity-bound [`crate::components::AmbientLight`] which is
/// attached to specific entities (e.g. light probes). The scene ambient
/// contributes a constant radiance to every surface.
#[derive(Component, Reflect, Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Component)]
pub struct AmbientParams {
    /// Ambient color (linear RGB).
    pub color: [f32; 3],
    /// Scalar intensity.
    pub intensity: f32,
}

impl Default for AmbientParams {
    fn default() -> Self {
        Self {
            color: [0.1, 0.1, 0.12],
            intensity: 0.3,
        }
    }
}

// =============================================================================
// Post-process profile
// =============================================================================

/// Per-scene default for the post-process chain.
///
/// This is the scene-level descriptor stored in `.hyge-world`. It is converted
/// into `hyge_render::post::PostProcessConfig` by the render integration
/// (R-064 / R-065). The fields mirror the renderer's config so the conversion
/// is trivial, but the type is kept separate to avoid a hard dependency on
/// `hyge-render` from `hyge-scene`.
#[derive(Component, Reflect, Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[reflect(Component)]
pub struct PostProcessProfile {
    /// Enables temporal anti-aliasing.
    pub taa: bool,
    /// Enables SMAA.
    pub smaa: bool,
    /// Additive bloom intensity.
    pub bloom_intensity: f32,
    /// Bright-pass threshold (linear luma).
    pub bloom_threshold: f32,
    /// Number of bloom downsample levels.
    pub bloom_levels: u32,
    /// Exposure multiplier applied before ACES tonemapping.
    pub exposure: f32,
}

impl Default for PostProcessProfile {
    fn default() -> Self {
        Self {
            taa: true,
            smaa: true,
            bloom_intensity: 0.2,
            bloom_threshold: 1.0,
            bloom_levels: 5,
            exposure: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_default_has_sun() {
        let env = Environment::default();
        assert!(env.sun.is_some());
        assert!(env.fog.is_none());
        assert!(env.skybox.is_none());
    }

    #[test]
    fn environment_empty_has_nothing() {
        let env = Environment::empty();
        assert!(env.sun.is_none());
        assert!(env.fog.is_none());
        assert!(env.skybox.is_none());
        assert_eq!(env.ambient.intensity, 0.0);
    }

    #[test]
    fn fog_params_default_is_reasonable() {
        let fog = FogParams::default();
        assert!(fog.density > 0.0);
        assert!(fog.density < 1.0);
    }

    #[test]
    fn ambient_params_default() {
        let amb = AmbientParams::default();
        assert!(amb.intensity > 0.0);
    }

    #[test]
    fn post_process_profile_default_enables_taa_smaa() {
        let pp = PostProcessProfile::default();
        assert!(pp.taa);
        assert!(pp.smaa);
        assert!(pp.bloom_levels >= 1);
    }

    #[test]
    fn environment_serde_round_trip() {
        let env = Environment {
            skybox: Some(AssetId::from(blake3::hash(b"skybox"))),
            sun: Some(DirectionalLight {
                direction: [0.0, -1.0, 0.2],
                color: [1.0, 0.9, 0.8],
                illuminance: 50_000.0,
            }),
            fog: Some(FogParams::default()),
            ambient: AmbientParams::default(),
        };
        let bytes = rmp_serde::to_vec(&env).expect("serialize");
        let restored: Environment = rmp_serde::from_slice(&bytes).expect("deserialize");
        assert_eq!(env, restored);
    }
}
