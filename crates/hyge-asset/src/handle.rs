//! Typed asset handles and loaded asset values.

use std::{marker::PhantomData, sync::Arc};

use crate::asset::{Asset, AssetId};

/// A typed, copyable handle to an asset stored by content hash.
///
/// Handles are intentionally just an [`AssetId`] plus a marker type. The
/// marker prevents accidentally using a mesh handle where a material handle is
/// expected while keeping the runtime representation small and copyable.
#[derive(Debug, Eq, PartialEq, Hash)]
pub struct Handle<A: Asset> {
    id: AssetId,
    _marker: PhantomData<A>,
}

impl<A: Asset> Handle<A> {
    /// Creates a typed handle for an asset id.
    #[inline]
    pub const fn new(id: AssetId) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    /// Returns the content-addressed id this handle points at.
    #[inline]
    pub const fn id(self) -> AssetId {
        self.id
    }
}

impl<A: Asset> Copy for Handle<A> {}

impl<A: Asset> Clone for Handle<A> {
    fn clone(&self) -> Self {
        *self
    }
}

/// A loaded asset value and its hot-reload version.
///
/// The asset is stored in an [`Arc`] so readers can retain the old version
/// while the asset server swaps in a newer version.
#[derive(Debug)]
pub struct LoadedAsset<A: Asset> {
    /// The loaded asset instance.
    pub asset: Arc<A>,
    /// Monotonically increasing version assigned by the asset server.
    pub version: u64,
}

impl<A: Asset> LoadedAsset<A> {
    /// Creates a loaded asset wrapper with a server-assigned version.
    pub fn new(asset: Arc<A>, version: u64) -> Self {
        Self { asset, version }
    }
}

impl<A: Asset> Clone for LoadedAsset<A> {
    fn clone(&self) -> Self {
        Self {
            asset: Arc::clone(&self.asset),
            version: self.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq)]
    struct TestAsset(u32);

    impl Asset for TestAsset {
        type Data = u32;

        fn hash(data: &Self::Data) -> blake3::Hash {
            blake3::hash(&data.to_le_bytes())
        }

        fn extensions() -> &'static [&'static str] {
            &["test"]
        }

        fn load(
            bytes: &[u8],
            _ctx: &mut crate::context::LoadContext,
        ) -> hyge_core::result::HygeResult<Self::Data> {
            bytes
                .first()
                .copied()
                .map(u32::from)
                .ok_or_else(|| hyge_core::result::HygeError::parse("empty test asset"))
        }
    }

    #[test]
    fn handle_is_copy_and_preserves_id() {
        let id = AssetId::from(blake3::hash(b"handle"));
        let handle = Handle::<TestAsset>::new(id);
        let copied = handle;

        assert_eq!(handle.id(), id);
        assert_eq!(copied.id(), id);
    }

    #[test]
    fn loaded_asset_clones_arc_and_preserves_version() {
        let loaded = LoadedAsset::new(Arc::new(TestAsset(7)), 42);
        let cloned = loaded.clone();

        assert_eq!(loaded.version, 42);
        assert_eq!(cloned.version, 42);
        assert_eq!(Arc::strong_count(&loaded.asset), 2);
        assert_eq!(cloned.asset.0, 7);
    }
}
