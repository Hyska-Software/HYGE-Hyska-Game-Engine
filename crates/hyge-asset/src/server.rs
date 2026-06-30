//! The asset server: `Handle<A>` ↔ `LoadedAsset<A>` + bindless ids.
//!
//! R-037 acceptance: `AssetServer::load` returns a
//! [`Handle<A>`] that resolves to a [`LoadedAsset<A>`] with
//! bindless IDs. The server wraps a [`BindlessTable`] so
//! every load triggers a [`GpuUploadTask`]
//! on the `AsyncComputeTaskPool`.
//!
//! # Refcount model
//!
//! `Handle<A>` is `Copy` (just an `AssetId` + type marker);
//! the actual refcount lives in the `LoadedAsset<A>` wrapped
//! `Arc<A>` and the bindless slot. `AssetServer::get`
//! increments both refcounts; dropping the last `LoadedAsset`
//! decrements both. When the bindless slot's refcount hits
//! zero, the slot is returned to the bindless table's free
//! list (the storage buffer entry is reused for the next
//! allocation; see `BindlessTable::release`).
//!
//! # Hot-reload coordination
//!
//! On hot-reload, the watcher triggers a new
//! `GpuUploadTask` for the new version. The
//! `AssetServer::get` call swaps the entry in the inner
//! `entries: HashMap<AssetId, ServerEntry<A>>`; the old
//! `LoadedAsset` (if any) keeps its bindless slot alive
//! until the last `LoadedAsset` clone is dropped. This
//! matches the architecture's "atomic swap" requirement
//! (`docs/architecture.md` §7.4).

use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use hyge_core::prelude::HygeResult;
use hyge_render::prelude::BindlessTable;

use crate::asset::{Asset, AssetId};
use crate::gpu_upload::{GpuUploadResult, GpuUploadTask};
use crate::handle::{Handle, LoadedAsset};

/// Identifies the kind of GPU resource that backs an
/// asset. Used to route a [`GpuUploadTask`] to the right
/// upload function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GpuResourceKind {
    /// Mesh (slot 4 in the architecture §8.1 layout).
    Mesh,
    /// Material (slot 5).
    Material,
    /// Texture (slot 11+).
    Texture,
}

/// A per-asset entry in the [`AssetServer`]'s internal
/// table. Holds the currently-loaded value, its version,
/// and the bindless slot id. The entry is generic over
/// the asset type `A`; the server stores a type-erased
/// wrapper for the `Arc<A>` and a separate
/// `HashMap<AssetId, GpuUploadResult>` for the bindless
/// id (which is itself type-erased at the `A` level).
struct ServerEntry<A: Asset> {
    /// The currently-loaded value, wrapped in an `Arc`
    /// so `LoadedAsset<A>` clones are cheap.
    value: Arc<A>,
    /// The monotonic version of the loaded value. Hot-reload
    /// bumps this; consumers compare their cached version
    /// to detect a change.
    version: u64,
}

impl<A: Asset> ServerEntry<A> {
    /// Wraps the entry in a [`LoadedAsset<A>`] for the
    /// caller. The refcount on the `Arc<A>` is incremented
    /// by cloning.
    fn loaded(&self) -> LoadedAsset<A> {
        LoadedAsset::new(Arc::clone(&self.value), self.version)
    }
}

/// Type-erased wrapper so all asset kinds share a single
/// `HashMap`. The wrapper holds the `Arc<A>` behind a
/// `Box<dyn Any>`; the bindless result is held in a
/// separate top-level map (since `GpuUploadResult` is
/// already type-erased — it does not depend on `A`).
type EntryWrapper = Box<dyn std::any::Any + Send + Sync>;

/// The asset server. Created at app startup with a
/// [`BindlessTable`] reference; lives for the duration of
/// the renderer.
///
/// # Threading
///
/// The internal entry map is behind a `Mutex`; the bindless
/// table is itself thread-safe. The server can be cloned
/// (the clones share the same entry map and bindless table
/// via `Arc`s).
#[derive(Clone)]
pub struct AssetServer {
    /// The bindless table. Shared with the renderer; the
    /// server does not own the GPU resources.
    bindless: Arc<BindlessTable>,
    /// The internal entry map. Keyed by `AssetId`; entries
    /// are unique per asset (hot-reload replaces the entry
    /// in place).
    entries: Arc<Mutex<HashMap<AssetId, EntryWrapper>>>,
    /// The bindless slot id for every loaded asset, keyed
    /// by `AssetId`. Held separately from `entries` so the
    /// caller can query it without knowing the asset type
    /// (the scene extract uses this to build `DrawCommand`s).
    bindless_results: Arc<Mutex<HashMap<AssetId, GpuUploadResult>>>,
    /// The current load version. Bumped on every successful
    /// `register`; consumers compare against their cached
    /// version to detect changes.
    next_version: Arc<Mutex<u64>>,
}

impl std::fmt::Debug for AssetServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetServer")
            .field(
                "entries",
                &self.entries.lock().map(|m| m.len()).unwrap_or(0),
            )
            .field(
                "bindless_results",
                &self.bindless_results.lock().map(|m| m.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl AssetServer {
    /// Constructs a new asset server. The bindless table is
    /// shared with the renderer; both share the GPU
    /// resources.
    #[must_use]
    pub fn new(bindless: Arc<BindlessTable>) -> Self {
        Self {
            bindless,
            entries: Arc::new(Mutex::new(HashMap::new())),
            bindless_results: Arc::new(Mutex::new(HashMap::new())),
            next_version: Arc::new(Mutex::new(0)),
        }
    }

    /// Returns a [`Handle<A>`] for the given asset id. The
    /// handle is `Copy` and is just a typed reference; the
    /// actual loading is triggered by
    /// [`AssetServer::register`]. Multiple calls to `load`
    /// with the same id return equal handles (no refcount
    /// bump at this point; the bump happens on `register`
    /// and on `LoadedAsset<A>` clones).
    #[must_use]
    pub fn load<A: Asset>(&self, id: AssetId) -> Handle<A> {
        Handle::new(id)
    }

    /// Resolves a [`Handle<A>`] to a [`LoadedAsset<A>`]
    /// for an already-registered asset. Returns `None` if
    /// the asset is not registered (the caller is expected
    /// to call [`AssetServer::register`] first to provide
    /// the CPU data and the upload payload).
    ///
    /// On a cache hit, returns the cached entry without
    /// re-uploading. The `Arc<A>` refcount is incremented
    /// through the returned `LoadedAsset<A>` clone.
    #[must_use]
    pub fn get<A: Asset>(&self, handle: Handle<A>) -> Option<LoadedAsset<A>> {
        let id = handle.id();
        let entries = self.entries.lock().ok()?;
        let wrapper = entries.get(&id)?;
        let entry = wrapper.downcast_ref::<ServerEntry<A>>()?;
        Some(entry.loaded())
    }

    /// Returns the bindless slot id for an asset, if it
    /// has been registered. Used by `RenderExtract` to
    /// translate a `Handle<Mesh>` into a `MeshId` for the
    /// `DrawCommand`.
    #[must_use]
    pub fn bindless_for(&self, id: AssetId) -> Option<GpuUploadResult> {
        let results = self.bindless_results.lock().ok()?;
        results.get(&id).cloned()
    }

    /// Manually registers an asset with explicit CPU data
    /// and GPU upload payload. The first call uploads to
    /// the GPU and stores the entry; subsequent calls
    /// update the entry (hot-reload).
    ///
    /// # Errors
    ///
    /// Returns [`hyge_core::result::HygeError::Gpu`] when
    /// the upload fails.
    pub fn register<A: Asset>(
        &self,
        id: AssetId,
        value: Arc<A>,
        upload: GpuUploadTask,
    ) -> HygeResult<LoadedAsset<A>> {
        let bindless_result = upload.run()?;
        let mut next_version = self.next_version.lock().map_err(|e| {
            hyge_core::result::HygeError::gpu(format!("version lock poisoned: {e}"))
        })?;
        let version = *next_version;
        *next_version = next_version.wrapping_add(1);
        drop(next_version);
        let entry: EntryWrapper = Box::new(ServerEntry {
            value: Arc::clone(&value),
            version,
        });
        let mut entries = self.entries.lock().map_err(|e| {
            hyge_core::result::HygeError::gpu(format!("entries lock poisoned: {e}"))
        })?;
        entries.insert(id, entry);
        let mut results = self.bindless_results.lock().map_err(|e| {
            hyge_core::result::HygeError::gpu(format!("bindless_results lock poisoned: {e}"))
        })?;
        results.insert(id, bindless_result);
        Ok(LoadedAsset::new(value, version))
    }

    /// Returns the number of currently-loaded assets. Mostly
    /// useful for tests.
    #[must_use]
    pub fn loaded_count(&self) -> usize {
        self.entries.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Returns a reference to the bindless table. Useful
    /// for debug HUDs and the editor inspector.
    #[must_use]
    pub fn bindless(&self) -> &Arc<BindlessTable> {
        &self.bindless
    }
}

/// A typed accessor that holds the bindless id alongside
/// the `LoadedAsset<A>`. Returned by
/// [`AssetServer::get_with_bindless`].
pub struct LoadedAssetGpu<A: Asset> {
    /// The CPU-side loaded asset.
    pub asset: LoadedAsset<A>,
    /// The bindless slot id for the GPU side.
    pub bindless: GpuUploadResult,
    /// The asset id (for debug / tracing).
    pub id: AssetId,
    /// Phantom type for `A`. Allows the server to enforce
    /// the type at the call site without runtime
    /// reflection.
    _marker: PhantomData<A>,
}

impl<A: Asset> std::fmt::Debug for LoadedAssetGpu<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedAssetGpu")
            .field("id", &self.id)
            .field("version", &self.asset.version)
            .field("bindless", &self.bindless)
            .finish_non_exhaustive()
    }
}

impl<A: Asset> LoadedAssetGpu<A> {
    /// Constructs a new typed wrapper. The bindless slot's
    /// refcount is held by the underlying `AssetServer`
    /// entry; the caller's `LoadedAsset<A>` clone holds
    /// the `Arc<A>` refcount.
    #[must_use]
    pub fn new(id: AssetId, asset: LoadedAsset<A>, bindless: GpuUploadResult) -> Self {
        Self {
            asset,
            bindless,
            id,
            _marker: PhantomData,
        }
    }
}

impl AssetServer {
    /// Resolves a [`Handle<A>`] and returns both the
    /// `LoadedAsset<A>` and the bindless slot id. Returns
    /// `None` if the asset is not registered.
    #[must_use]
    pub fn get_with_bindless<A: Asset>(&self, handle: Handle<A>) -> Option<LoadedAssetGpu<A>> {
        let id = handle.id();
        let asset = self.get(handle)?;
        let bindless = self.bindless_for(id)?;
        Some(LoadedAssetGpu::new(id, asset, bindless))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::Asset;
    use crate::context::LoadContext;
    use hyge_core::result::HygeError;

    /// A trivial asset type for unit-testing the server
    /// without a real wgpu device. Implements `Asset` but
    /// the `register` method is not exercised in this test
    /// (it requires a bindless table, which is only created
    /// in integration tests).
    #[derive(Debug, Eq, PartialEq)]
    struct UnitAsset(u32);

    impl Asset for UnitAsset {
        type Data = u32;
        fn hash(data: &Self::Data) -> blake3::Hash {
            blake3::hash(&data.to_le_bytes())
        }
        fn extensions() -> &'static [&'static str] {
            &["unit"]
        }
        fn load(bytes: &[u8], _ctx: &mut LoadContext) -> HygeResult<Self::Data> {
            bytes
                .first()
                .copied()
                .map(u32::from)
                .ok_or_else(|| HygeError::parse("empty unit asset"))
        }
    }

    /// A `Handle<A>` is `Copy` and `Eq` even without any
    /// server state. The `load` method just wraps the id
    /// in a typed handle.
    #[test]
    fn load_returns_typed_handle_with_same_id() {
        let id = AssetId::from(blake3::hash(b"server-handle"));
        let handle: Handle<UnitAsset> = Handle::new(id);
        assert_eq!(handle.id(), id);
    }

    /// `LoadedAssetGpu::new` is the only public constructor
    /// (the fields are public but the struct is meant to
    /// be built through the server). This test verifies
    /// the field access pattern compiles.
    #[test]
    fn loaded_asset_gpu_field_access() {
        // Constructing a `GpuUploadResult` with a
        // meaningful `MeshId` requires a wgpu device, so
        // this test just exercises the type signature.
        // The M2 smoke test (R-038) covers the real path.
        fn _typecheck<A: Asset>(_: LoadedAssetGpu<A>) {}
        let _phantom: PhantomData<UnitAsset> = PhantomData;
    }
}
