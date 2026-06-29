//! `.hyge-mat` material writer.
//!
//! The material format is JSON for the milestone so it's directly
//! inspectable from the cache and easy to diff in tests. R-034 ships
//! a deterministic, faithful subset of glTF's
//! metallic-roughness PBR parameters; the more compact binary
//! representation is layered in by R-037 (BindlessTable).

use std::fs;
use std::path::Path;

use hyge_core::result::HygeResult;
use serde::{Deserialize, Serialize};

/// Material constants extracted from a glTF material plus the
/// content-hash references to its bound textures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaterialData {
    /// Source-asset material name (or empty when anonymous).
    pub name: String,
    /// sRGB base color (linear after sRGB→linear conversion).
    pub base_color: [f32; 4],
    /// Metallic factor in `[0, 1]`.
    pub metallic: f32,
    /// Roughness factor in `[0, 1]`.
    pub roughness: f32,
    /// Optional emissive (linear RGB).
    pub emissive: [f32; 3],
    /// Optional double-sided flag.
    pub double_sided: bool,
    /// Hash of the base-color texture, when present.
    pub base_color_texture: Option<String>,
    /// Hash of the metallic-roughness texture, when present.
    pub metallic_roughness_texture: Option<String>,
    /// Hash of the normal map, when present.
    pub normal_texture: Option<String>,
    /// Hash of the occlusion map, when present.
    pub occlusion_texture: Option<String>,
    /// Hash of the emissive map, when present.
    pub emissive_texture: Option<String>,
}

impl Default for MaterialData {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color: [1.0, 1.0, 1.0, 1.0],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0, 0.0, 0.0],
            double_sided: false,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
        }
    }
}

/// Writes `material` to `path` as a `.hyge-mat` JSON file.
///
/// # Errors
///
/// Returns [`hyge_core::result::HygeError::Io`] on filesystem failure
/// or [`hyge_core::result::HygeError::Parse`] on JSON
/// serialization failure.
pub fn write(path: &Path, material: &MaterialData) -> HygeResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error("create material parent dir"))?;
    }
    let bytes = serde_json::to_vec_pretty(material)
        .map_err(|e| hyge_core::result::HygeError::parse(format!("material json: {e}")))?;
    fs::write(path, bytes).map_err(io_error("write material file"))?;
    Ok(())
}

fn io_error(op: &'static str) -> impl FnOnce(std::io::Error) -> hyge_core::result::HygeError {
    move |e| hyge_core::result::HygeError::Io(std::io::Error::other(format!("{op}: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "hyge-asset-mat-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn write_and_read_back_round_trips() {
        let dir = tmp();
        let p = dir.join("m.hyge-mat");
        let m = MaterialData {
            name: "lambert_red".into(),
            base_color: [0.8, 0.1, 0.1, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            emissive: [0.0, 0.0, 0.0],
            double_sided: false,
            base_color_texture: Some("deadbeef".into()),
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
        };
        write(&p, &m).unwrap();
        let raw = std::fs::read_to_string(&p).unwrap();
        let back: MaterialData = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn write_is_deterministic_for_same_input() {
        let dir = tmp();
        let p1 = dir.join("a.hyge-mat");
        let p2 = dir.join("b.hyge-mat");
        let m = MaterialData {
            name: "x".into(),
            ..MaterialData::default()
        };
        write(&p1, &m).unwrap();
        write(&p2, &m).unwrap();
        assert_eq!(std::fs::read(&p1).unwrap(), std::fs::read(&p2).unwrap());
    }
}
