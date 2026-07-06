//! R-062 — Prefab asset loader.
//!
//! Wires the [`Prefab`] type into `hyge-asset` so `.hyge-prefab` files can be
//! loaded through the standard asset server machinery.

use hyge_asset::{Asset, LoadContext};

use crate::prefab::Prefab;

/// Marker type for the `.hyge-prefab` asset format.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrefabAsset;

impl Asset for PrefabAsset {
    type Data = Prefab;

    fn hash(data: &Self::Data) -> blake3::Hash {
        // Prefab already stores its content-addressed id; hash from the
        // canonical serialized bytes so the asset DB stays consistent.
        match data.to_bytes() {
            Ok(bytes) => blake3::hash(&bytes),
            // Fallback to the prefab id if serialization fails unexpectedly.
            Err(_) => blake3::Hash::from(data.prefab_id.0 .0),
        }
    }

    fn extensions() -> &'static [&'static str] {
        &["hyge-prefab"]
    }

    fn load(bytes: &[u8], _ctx: &mut LoadContext) -> hyge_core::result::HygeResult<Self::Data> {
        Prefab::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefab::{PrefabAssets, PrefabNode};
    use hyge_asset::Asset;

    #[test]
    fn prefab_asset_load() {
        let prefab = Prefab::new(
            "asset-test",
            PrefabNode::named("root"),
            PrefabAssets::default(),
        );
        let bytes = prefab.to_bytes().expect("serialize");

        let loaded = PrefabAsset::load(&bytes, &mut LoadContext::default()).expect("load");
        assert_eq!(loaded.name, "asset-test");
        assert_eq!(loaded.root.name, "root");
        assert_eq!(loaded.prefab_id, prefab.prefab_id);
    }

    #[test]
    fn prefab_asset_extensions() {
        assert_eq!(PrefabAsset::extensions(), &["hyge-prefab"]);
    }
}
