//! Asset identity and loading traits.

use bevy_reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::context::LoadContext;

/// A BLAKE3 content-addressed asset identity.
///
/// The bytes are the 32-byte BLAKE3 hash of an imported or source asset.
/// Content addressing lets Hyge deduplicate assets and invalidate cached
/// data by hash instead of by path.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize, Reflect)]
#[serde(transparent)]
pub struct AssetId(pub [u8; 32]);

impl AssetId {
    /// The all-zero sentinel used to represent an absent asset reference.
    pub const NULL: Self = Self([0u8; 32]);

    /// Constructs an asset id from raw BLAKE3 bytes.
    #[inline]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 32-byte BLAKE3 hash.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns true when this id is the all-zero sentinel.
    #[inline]
    pub const fn is_null(&self) -> bool {
        let mut i = 0;
        while i < 32 {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }
}

impl From<blake3::Hash> for AssetId {
    fn from(hash: blake3::Hash) -> Self {
        Self(*hash.as_bytes())
    }
}

impl From<[u8; 32]> for AssetId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl From<hyge_core::id::AssetId> for AssetId {
    fn from(id: hyge_core::id::AssetId) -> Self {
        Self(*id.as_bytes())
    }
}

impl From<AssetId> for hyge_core::id::AssetId {
    fn from(id: AssetId) -> Self {
        Self::from_bytes(id.0)
    }
}

/// A loadable Hyge asset type.
///
/// Implementors define the intermediate decoded [`Self::Data`] produced by
/// the CPU loading path. Later roadmap items add the registry and server that
/// turn this contract into asynchronous asset loading and hot-reload.
pub trait Asset: Send + Sync + 'static {
    /// CPU-side data produced by the asset loader before runtime upload or
    /// finalization.
    type Data: Send + Sync + 'static;

    /// Computes the BLAKE3 hash used as the content-addressed identity for
    /// this asset data.
    fn hash(data: &Self::Data) -> blake3::Hash;

    /// Returns the file extensions this asset can load, without leading dots.
    fn extensions() -> &'static [&'static str];

    /// Loads CPU-side asset data from bytes.
    ///
    /// # Errors
    ///
    /// Returns [`hyge_core::result::HygeError`] when bytes are malformed or
    /// when a dependency requested through [`LoadContext`] cannot be resolved.
    fn load(bytes: &[u8], ctx: &mut LoadContext) -> hyge_core::result::HygeResult<Self::Data>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BytesAsset;

    impl Asset for BytesAsset {
        type Data = Vec<u8>;

        fn hash(data: &Self::Data) -> blake3::Hash {
            blake3::hash(data)
        }

        fn extensions() -> &'static [&'static str] {
            &["bytes", "bin"]
        }

        fn load(bytes: &[u8], _ctx: &mut LoadContext) -> hyge_core::result::HygeResult<Self::Data> {
            Ok(bytes.to_vec())
        }
    }

    #[test]
    fn asset_id_from_blake3_hash_uses_hash_bytes() {
        let hash = blake3::hash(b"asset-id");
        let id = AssetId::from(hash);
        assert_eq!(id.as_bytes(), hash.as_bytes());
    }

    #[test]
    fn asset_id_is_copy_eq_hash_and_null_aware() {
        use std::collections::HashSet;

        let id = AssetId::from(blake3::hash(b"copy"));
        let copied = id;
        let mut set = HashSet::new();
        set.insert(id);

        assert_eq!(copied, id);
        assert!(set.contains(&copied));
        assert!(AssetId::NULL.is_null());
        assert!(!id.is_null());
    }

    #[test]
    fn asset_trait_loads_and_hashes_data() {
        let mut ctx = LoadContext::default();
        let data = BytesAsset::load(b"hello", &mut ctx).expect("bytes load succeeds");

        assert_eq!(data, b"hello");
        assert_eq!(BytesAsset::hash(&data), blake3::hash(b"hello"));
        assert_eq!(BytesAsset::extensions(), &["bytes", "bin"]);
    }

    #[test]
    fn asset_id_converts_to_and_from_core_asset_id() {
        let asset_id = AssetId::from(blake3::hash(b"bridge"));
        let core_id = hyge_core::id::AssetId::from(asset_id);
        let round_trip = AssetId::from(core_id);

        assert_eq!(asset_id, round_trip);
    }
}
