//! Prefab identity: BLAKE3-keyed [`PrefabId`].
//!
//! Every `.hyge-prefab` file is content-addressed by the hash of its
//! serialized msgpack bytes. The identity is stable across imports and
//! round-trips, which lets the asset DB and scene format reference prefabs
//! by hash instead of by path.

use serde::{Deserialize, Serialize};

use hyge_asset::AssetId;

/// A BLAKE3-keyed prefab identity.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrefabId(pub AssetId);

impl PrefabId {
    /// The null sentinel used for an absent prefab reference.
    pub const NULL: Self = Self(AssetId::NULL);

    /// Computes the prefab id from raw serialized bytes.
    #[must_use]
    pub fn compute(bytes: &[u8]) -> Self {
        Self(AssetId::from(blake3::hash(bytes)))
    }
}

impl From<AssetId> for PrefabId {
    fn from(id: AssetId) -> Self {
        Self(id)
    }
}

impl From<PrefabId> for AssetId {
    fn from(id: PrefabId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_is_deterministic() {
        let bytes = b"prefab-bytes";
        let a = PrefabId::compute(bytes);
        let b = PrefabId::compute(bytes);
        assert_eq!(a, b);
    }

    #[test]
    fn different_bytes_different_id() {
        let a = PrefabId::compute(b"a");
        let b = PrefabId::compute(b"b");
        assert_ne!(a, b);
    }

    #[test]
    fn null_sentinel() {
        assert!(PrefabId::NULL.0.is_null());
        assert!(!PrefabId::compute(b"x").0.is_null());
    }
}
