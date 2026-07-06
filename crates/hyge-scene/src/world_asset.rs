//! R-063 — World asset loader.
//!
//! Wires the [`WorldDocument`] type into `hyge-asset` so `.hyge-world` files
//! can be loaded through the standard asset server machinery, mirroring the
//! `PrefabAsset` pattern from R-062.

use hyge_asset::{Asset, LoadContext};

use crate::world::WorldDocument;

/// Marker type for the `.hyge-world` asset format.
///
/// Implemented as a unit struct so the asset registry can dispatch on type
/// without requiring a runtime tag.
#[derive(Debug, Clone, Copy, Default)]
pub struct WorldAsset;

impl Asset for WorldAsset {
    type Data = WorldDocument;

    fn hash(data: &Self::Data) -> blake3::Hash {
        // The content-addressed identity is the BLAKE3 of the canonical
        // serialized bytes, mirroring the prefab format. We round-trip
        // through `to_bytes` so the asset DB key matches what would be
        // written to disk.
        match data.to_bytes() {
            Ok(bytes) => blake3::hash(&bytes),
            // Fallback to a zero hash if serialization fails unexpectedly —
            // this path should not happen for a well-formed document but is
            // required because `hash` cannot return `Err`.
            Err(_) => blake3::Hash::from([0u8; 32]),
        }
    }

    fn extensions() -> &'static [&'static str] {
        &["hyge-world"]
    }

    fn load(bytes: &[u8], _ctx: &mut LoadContext) -> hyge_core::result::HygeResult<Self::Data> {
        WorldDocument::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Transform;
    use crate::env::{Environment, PostProcessProfile};
    use crate::prefab_id::PrefabId;
    use crate::world::{PrefabInstance, WorldDocument};
    use hyge_asset::Asset;

    fn sample_document() -> WorldDocument {
        WorldDocument {
            env: Environment::empty(),
            root_prefab_instances: vec![PrefabInstance::new(PrefabId::NULL, Transform::identity())],
            post_process: PostProcessProfile::default(),
        }
    }

    #[test]
    fn world_asset_extensions() {
        assert_eq!(WorldAsset::extensions(), &["hyge-world"]);
    }

    #[test]
    fn world_asset_load_round_trips() {
        let doc = sample_document();
        let bytes = doc.to_bytes().expect("serialize");

        let mut ctx = LoadContext::default();
        let loaded = WorldAsset::load(&bytes, &mut ctx).expect("load");
        assert_eq!(loaded, doc);
    }

    #[test]
    fn world_asset_hash_matches_serialized_bytes() {
        let doc = sample_document();
        let bytes = doc.to_bytes().expect("serialize");
        let expected = blake3::hash(&bytes);
        assert_eq!(WorldAsset::hash(&doc), expected);
    }

    #[test]
    fn world_asset_load_rejects_bad_bytes() {
        let mut ctx = LoadContext::default();
        let err = WorldAsset::load(b"not msgpack", &mut ctx).unwrap_err();
        assert!(matches!(err, hyge_core::result::HygeError::Parse(_)));
    }
}
