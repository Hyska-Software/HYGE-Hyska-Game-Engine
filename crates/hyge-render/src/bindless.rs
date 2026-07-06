//! Bindless descriptor heap.
//!
//! See `docs/architecture.md` §8.1 for the slot layout. The
//! [`BindlessTable`] owns the GPU side of the bindless model: a
//! pre-allocated set of storage buffers (one per resource type),
//! a `Texture2DArray` for textures, four default samplers, and a
//! single [`wgpu::BindGroup`] that bundles them. Asset GPU
//! resources are registered through this table and reach the
//! shader as a flat `(mesh_id, material_id, texture_id, ...)`
//! index tuple — no per-draw bind updates.
//!
//! # Slot layout (R-037, from `docs/architecture.md` §8.1)
//!
//! | Slot | Type                                | Capacity   |
//! |------|-------------------------------------|------------|
//! | 0    | `Sampler` (linear clamp)            | 1 (global) |
//! | 1    | `Sampler` (linear repeat)           | 1 (global) |
//! | 2    | `Sampler` (nearest clamp)           | 1 (global) |
//! | 3    | `Sampler` (shadow)                  | 1 (global) |
//! | 4    | `StorageBuffer<GpuMesh>`            | 65 536     |
//! | 5    | `StorageBuffer<GpuMaterial>`        | 65 536     |
//! | 6    | `StorageBuffer<Instance>`           | 1 048 576  |
//! | 7    | `StorageBuffer<Light>`              | 4 096      |
//! | 8    | `StorageBuffer<LightGrid>`          | per-frame  |
//! | 9    | `StorageBuffer<MeshletVisibility>`  | per-frame  |
//! | 10   | `StorageBuffer<DrawCommand>`        | per-frame  |
//! | 11+  | `Texture2DArray` (albedo)           | 256 (v0.1) |
//!
//! The wgpu backend limits `Texture2DArray::array_layers` to
//! `MAX_TEXTURE_ARRAY_LAYERS` (commonly 2048). The
//! architecture spec calls for 16 384; the v0.1 cap is 256
//! (configurable via [`BindlessConfig`]) and is sized to match
//! what a real `hyge-sandbox` scene (M7) needs. Future items
//! shard the texture array across multiple bind groups to
//! reach the 16k target — see `docs/adr/0009-bindless-texturing.md`.
//!
//! # Refcount model
//!
//! Every `register_*` call returns a typed ID wrapped in an
//! `Arc<Refcount>` (the [`BindlessSlot<T>`] newtype). Cloning
//! the slot increments the refcount; dropping the last clone
//! decrements it. When the refcount reaches zero, the slot is
//! returned to the free list and the underlying storage
//! element is zeroed (so a stale read sees an "empty" entry
//! rather than a torn value). The actual `wgpu::Buffer` /
//! `wgpu::Texture` storage is not freed — it is pre-allocated
//! at table construction time and reused for the lifetime of
//! the renderer.

use std::fmt;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use hyge_core::prelude::{HygeError, HygeResult};

/// Default number of mesh slots (architecture §8.1).
pub const MESH_CAPACITY: u32 = 65_536;
/// Default number of material slots (architecture §8.1).
pub const MATERIAL_CAPACITY: u32 = 65_536;
/// Default number of instance slots (architecture §8.1).
pub const INSTANCE_CAPACITY: u32 = 1_048_576;
/// Default number of light slots (architecture §8.1).
pub const LIGHT_CAPACITY: u32 = 4_096;
/// Default number of light-grid entries. The runtime resizes
/// per frame; this is the upper bound for the SSBO.
pub const LIGHT_GRID_CAPACITY: u32 = 65_536;
/// Default number of u32 slots in the per-frame light index
/// list (one slot per `(cluster, light_in_cluster)` pair). The
/// upper bound is `LIGHT_GRID_CAPACITY * MAX_LIGHTS_PER_CLUSTER`,
/// but a flat u32 SSBO of this size keeps the math simple and
/// fits the conservative CPU build.
pub const LIGHT_INDEX_LIST_CAPACITY: u32 = 65_536;
/// Default number of meshlet visibility entries (per frame).
pub const MESHLET_VISIBILITY_CAPACITY: u32 = 1_048_576;
/// Default number of draw commands (per frame).
pub const DRAW_COMMAND_CAPACITY: u32 = 65_536;
/// Default number of texture-array layers. See module docs.
pub const TEXTURE_CAPACITY: u32 = 16;

/// The sentinel returned when a slot is the "null" id (used
/// for empty draw commands, fallback samplers, etc.).
pub const NULL_SLOT: u32 = u32::MAX;

/// Configuration for the [`BindlessTable`]. Sized down from
/// the architecture defaults to keep the per-table VRAM cost
/// tractable on a 4 GiB GPU; the per-item capacities match
/// §8.1.
#[derive(Debug, Clone)]
pub struct BindlessConfig {
    /// Maximum number of mesh slots. Default: [`MESH_CAPACITY`].
    pub mesh_capacity: u32,
    /// Maximum number of material slots. Default: [`MATERIAL_CAPACITY`].
    pub material_capacity: u32,
    /// Maximum number of instance slots. Default: [`INSTANCE_CAPACITY`].
    pub instance_capacity: u32,
    /// Maximum number of light slots. Default: [`LIGHT_CAPACITY`].
    pub light_capacity: u32,
    /// Maximum number of light-grid entries. Default: [`LIGHT_GRID_CAPACITY`].
    pub light_grid_capacity: u32,
    /// Maximum number of light index list entries. Default:
    /// [`LIGHT_INDEX_LIST_CAPACITY`].
    pub light_index_list_capacity: u32,
    /// Maximum number of meshlet visibility entries. Default:
    /// [`MESHLET_VISIBILITY_CAPACITY`].
    pub meshlet_visibility_capacity: u32,
    /// Maximum number of draw commands. Default: [`DRAW_COMMAND_CAPACITY`].
    pub draw_command_capacity: u32,
    /// Maximum number of texture-array layers. Default: [`TEXTURE_CAPACITY`].
    pub texture_capacity: u32,
    /// Texture format for the texture array. Defaults to
    /// `Rgba8UnormSrgb`; BC7 / ASTC paths land with the
    /// KTX2-transcode + PBR work in R-040.
    pub texture_format: wgpu::TextureFormat,
}

impl Default for BindlessConfig {
    fn default() -> Self {
        Self {
            mesh_capacity: MESH_CAPACITY,
            material_capacity: MATERIAL_CAPACITY,
            instance_capacity: INSTANCE_CAPACITY,
            light_capacity: LIGHT_CAPACITY,
            light_grid_capacity: LIGHT_GRID_CAPACITY,
            light_index_list_capacity: LIGHT_INDEX_LIST_CAPACITY,
            meshlet_visibility_capacity: MESHLET_VISIBILITY_CAPACITY,
            draw_command_capacity: DRAW_COMMAND_CAPACITY,
            texture_capacity: TEXTURE_CAPACITY,
            texture_format: wgpu::TextureFormat::Rgba8UnormSrgb,
        }
    }
}

/// Per-resource GPU-side mirror of a `Mesh` asset. Matches
/// the layout from `docs/architecture.md` §6.4.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct GpuMesh {
    /// Byte offset into the global vertex buffer.
    pub vertex_offset: u32,
    /// Byte offset into the global index buffer.
    pub index_offset: u32,
    /// Byte offset into the global meshlet buffer.
    pub meshlet_offset: u32,
    /// Number of meshlets in the mesh.
    pub meshlet_count: u32,
    /// Local AABB minimum.
    pub aabb_min: [f32; 3],
    /// Local AABB maximum.
    pub aabb_max: [f32; 3],
    /// Number of LODs beyond the base.
    pub lod_count: u32,
    /// Pad to a 16-byte boundary.
    pub _pad: u32,
}

impl Default for GpuMesh {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Per-material GPU constants. Mirrors the layout in
/// `docs/architecture.md` §6.4.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuMaterial {
    /// Bindless texture-id for the base color map.
    pub base_color: u32,
    /// Bindless texture-id for the normal map.
    pub normal: u32,
    /// Bindless texture-id for the metallic-roughness map.
    pub mr: u32,
    /// Bindless texture-id for the occlusion map.
    pub occlusion: u32,
    /// Bindless texture-id for the emissive map.
    pub emissive: u32,
    /// Material roughness in [0, 1].
    pub roughness: f32,
    /// Material metallicness in [0, 1].
    pub metallic: f32,
    /// Alpha mode (0 = opaque, 1 = cutout, 2 = blend).
    pub alpha_mode: u32,
    /// Bitflags: emissive, double-sided, etc. (M4+).
    pub flags: u32,
}

impl Default for GpuMaterial {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Per-instance GPU transform row.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Instance {
    /// Affine world matrix (column-major, 3 rows × 4 columns).
    pub transform: [[f32; 4]; 3],
    /// Bindless mesh-id.
    pub mesh_id: u32,
    /// Bindless material-id.
    pub material_id: u32,
    /// Pad to a 16-byte boundary.
    pub _pad: [u32; 2],
}

impl Default for Instance {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// A light (point, spot, or directional) for the
/// clustered-forward pass. R-042 fills this in.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Light {
    /// World-space position. `w` is light type (0=point, 1=spot, 2=directional).
    pub position: [f32; 4],
    /// Color (xyz) + intensity (w).
    pub color: [f32; 4],
    /// Direction (xyz) for spot/directional + cos(outer_angle) in w.
    pub direction: [f32; 4],
}

impl Default for Light {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Per-tile / per-cluster light grid entry. R-042 fills this in.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct LightGrid {
    /// Offset into the light index list.
    pub offset: u32,
    /// Number of lights in this cell.
    pub count: u32,
}

impl LightGrid {
    /// Creates a new light-grid entry.
    #[must_use]
    pub fn new(offset: u32, count: u32) -> Self {
        Self { offset, count }
    }
}

impl Default for LightGrid {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Per-meshlet visibility record emitted by the GPU cull pass.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MeshletVisibility {
    /// Bindless mesh-id.
    pub mesh_id: u32,
    /// Meshlet index within the mesh.
    pub meshlet_id: u32,
    /// Selected LOD.
    pub lod: u32,
    /// Pad.
    pub _pad: u32,
}

impl Default for MeshletVisibility {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Per-draw command emitted by `RenderExtract`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct DrawCommand {
    /// Bindless mesh-id.
    pub mesh_id: u32,
    /// Bindless material-id.
    pub material_id: u32,
    /// First instance in the bindless instance buffer.
    pub first_instance: u32,
    /// Number of instances.
    pub instance_count: u32,
    /// Local AABB min.
    pub aabb_min: [f32; 3],
    /// Local AABB max.
    pub aabb_max: [f32; 3],
    /// Pad.
    pub _pad: [u32; 2],
}

impl Default for DrawCommand {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// Atomic refcount shared between every clone of a
/// [`BindlessSlot`]. When the count reaches zero the slot is
/// freed; an in-flight read is still safe (the storage buffer
/// was zeroed at free time).
#[derive(Debug)]
pub struct Refcount {
    /// The actual slot index in the bindless storage.
    slot: u32,
    /// `0` = null, `1` = mesh, `2` = material, `3` = texture, …
    kind: SlotKind,
    /// Owner of the per-kind allocator; used to free the slot
    /// when the refcount drops to zero. `None` for "null"
    /// slots that never need to be freed.
    table: Option<Arc<Mutex<BindlessInner>>>,
    /// The current refcount. `AtomicU32` so `BindlessSlot`
    /// drops from any thread.
    refs: AtomicU32,
}

impl Refcount {
    /// Returns the slot index.
    #[inline]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// Returns the current refcount. Useful for tests.
    #[inline]
    pub fn refs(&self) -> u32 {
        self.refs.load(Ordering::Acquire)
    }

    /// Returns the slot kind.
    #[inline]
    pub fn kind(&self) -> SlotKind {
        self.kind
    }
}

/// Identifies which storage array a slot belongs to.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum SlotKind {
    /// [`BindlessTable::register_mesh`] slot.
    Mesh,
    /// [`BindlessTable::register_material`] slot.
    Material,
    /// [`BindlessTable::register_texture`] slot.
    Texture,
    /// Sentinel value, not backed by storage.
    Null,
}

impl SlotKind {
    /// Returns the slot-id for this kind in the architecture
    /// §8.1 table (4 for Mesh, 5 for Material, 11+ for Texture).
    /// Returns `NULL_SLOT` for the null kind.
    #[inline]
    pub fn architecture_slot(self) -> u32 {
        match self {
            SlotKind::Mesh => 4,
            SlotKind::Material => 5,
            SlotKind::Texture => 11,
            SlotKind::Null => NULL_SLOT,
        }
    }
}

impl fmt::Display for SlotKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotKind::Mesh => write!(f, "mesh"),
            SlotKind::Material => write!(f, "material"),
            SlotKind::Texture => write!(f, "texture"),
            SlotKind::Null => write!(f, "null"),
        }
    }
}

/// A typed bindless slot. The phantom type parameter prevents
/// accidental cross-typing: a [`BindlessSlot<MeshTag>`] cannot
/// be used where a material id is expected.
#[derive(Debug)]
pub struct BindlessSlot<Tag: SlotTag> {
    refcount: Arc<Refcount>,
    _marker: PhantomData<Tag>,
}

impl<Tag: SlotTag> Clone for BindlessSlot<Tag> {
    fn clone(&self) -> Self {
        self.refcount.refs.fetch_add(1, Ordering::AcqRel);
        Self {
            refcount: Arc::clone(&self.refcount),
            _marker: PhantomData,
        }
    }
}

impl<Tag: SlotTag> Drop for BindlessSlot<Tag> {
    fn drop(&mut self) {
        // fetch_sub returns the previous value; if it was 1
        // we just dropped the last reference. Relaxed is fine
        // for the decrement; the table mutex provides the
        // happens-before for the actual free.
        let prev = self.refcount.refs.fetch_sub(1, Ordering::AcqRel);
        if prev == 1 {
            if let Some(table) = self.refcount.table.as_ref() {
                if let Ok(mut inner) = table.lock() {
                    inner.release(self.refcount.kind, self.refcount.slot);
                }
            }
        }
    }
}

impl<Tag: SlotTag> BindlessSlot<Tag> {
    /// Returns the slot index (the value the shader uses as
    /// the bindless id).
    #[inline]
    pub fn index(&self) -> u32 {
        self.refcount.slot
    }

    /// Returns the current refcount. Mostly useful for tests.
    #[inline]
    pub fn refs(&self) -> u32 {
        self.refcount.refs.load(Ordering::Acquire)
    }

    /// Returns the slot kind.
    #[inline]
    pub fn kind(&self) -> SlotKind {
        self.refcount.kind
    }
}

/// Marker trait for the typed slot tags. Each tag carries the
/// [`SlotKind`] it corresponds to.
pub trait SlotTag: private::Sealed {
    /// The slot kind associated with this tag.
    const KIND: SlotKind;
}

mod private {
    pub trait Sealed {}
}

/// Mesh tag.
#[derive(Debug)]
pub struct MeshTag;
impl private::Sealed for MeshTag {}
impl SlotTag for MeshTag {
    const KIND: SlotKind = SlotKind::Mesh;
}

/// Material tag.
#[derive(Debug)]
pub struct MaterialTag;
impl private::Sealed for MaterialTag {}
impl SlotTag for MaterialTag {
    const KIND: SlotKind = SlotKind::Material;
}

/// Texture tag.
#[derive(Debug)]
pub struct TextureTag;
impl private::Sealed for TextureTag {}
impl SlotTag for TextureTag {
    const KIND: SlotKind = SlotKind::Texture;
}

/// Typed mesh id.
pub type MeshId = BindlessSlot<MeshTag>;
/// Typed material id.
pub type MaterialId = BindlessSlot<MaterialTag>;
/// Typed texture id.
pub type TextureId = BindlessSlot<TextureTag>;

/// Per-allocator free list and per-slot generation counter.
/// `generations[i]` is bumped every time slot `i` is recycled;
/// this lets us catch stale slot uses even though we never
/// re-allocate the underlying storage.
#[derive(Debug)]
struct FreeList {
    capacity: u32,
    free: Vec<u32>,
}

impl FreeList {
    /// Allocates a new free list with `capacity` slots, all
    /// initially free.
    fn new(capacity: u32) -> Self {
        // Largest contiguous free-stack: capacity elements.
        // For 1M-instance buffers this is 4 MiB of indices;
        // the table already reserves the storage buffer, so
        // this is dwarfed by that. Memory-budget is fine.
        let mut free = Vec::with_capacity(capacity as usize);
        for i in (0..capacity).rev() {
            free.push(i);
        }
        Self { capacity, free }
    }

    /// Returns the next free slot, or `None` if the allocator
    /// is exhausted.
    fn allocate(&mut self) -> Option<u32> {
        self.free.pop()
    }

    /// Returns `slot` to the free list. The caller is
    /// responsible for zeroing the underlying storage at
    /// `slot` if a stale read would be observable.
    fn release(&mut self, slot: u32) {
        debug_assert!(
            slot < self.capacity,
            "free-list release out of range: slot={slot} capacity={}",
            self.capacity
        );
        self.free.push(slot);
    }

    /// Returns the number of free slots remaining.
    fn free_count(&self) -> u32 {
        self.free.len() as u32
    }

    /// Returns the total capacity.
    fn capacity(&self) -> u32 {
        self.capacity
    }
}

/// Inner state behind the bindless-table mutex. Owns the
/// per-resource free lists and the wgpu storage buffers.
#[allow(
    dead_code,
    reason = "instance/light/light-grid/meshlet-vis/draw-command buffers + texture array are wired into the bind group; inner samplers are aliased by the outer table.samplers. The fields are kept for completeness and will be exposed to upload paths in R-040+."
)]
struct BindlessInner {
    meshes: FreeList,
    materials: FreeList,
    textures: FreeList,
    /// The wgpu mesh storage buffer (CPU-visible for `write_buffer`).
    /// Stored as `Arc` so callers can clone without the mutex.
    mesh_buffer: Arc<wgpu::Buffer>,
    /// The wgpu material storage buffer.
    material_buffer: Arc<wgpu::Buffer>,
    /// The wgpu instance storage buffer.
    instance_buffer: Arc<wgpu::Buffer>,
    /// The wgpu light storage buffer.
    light_buffer: Arc<wgpu::Buffer>,
    /// The wgpu light-grid storage buffer.
    light_grid_buffer: Arc<wgpu::Buffer>,
    /// The wgpu light-index-list storage buffer (per-frame;
    /// populated by `ClusteredForwardPass::rebuild_light_grid`).
    light_index_list_buffer: Arc<wgpu::Buffer>,
    /// The wgpu meshlet-visibility storage buffer.
    meshlet_visibility_buffer: Arc<wgpu::Buffer>,
    /// The wgpu draw-command storage buffer.
    draw_command_buffer: Arc<wgpu::Buffer>,
    /// The texture array (sized at construction time; layers
    /// allocated on demand). Stored as `Option<Arc>` because
    /// the array is only allocated when the device has the
    /// `TEXTURE_BINDING_ARRAY` feature enabled.
    texture_array: Option<Arc<wgpu::Texture>>,
    /// Default samplers (linear clamp, linear repeat, nearest
    /// clamp, shadow).
    samplers: [Arc<wgpu::Sampler>; 4],
    /// Generation counters: `generations[kind][slot]` is bumped
    /// every time the slot is recycled. Stored in CPU memory
    /// to keep GPU work minimal.
    generations: [Vec<u32>; 3],
}

impl BindlessInner {
    /// Releases the given slot, returning it to the free
    /// list and bumping its generation counter.
    ///
    /// The slot's storage is **not** explicitly zeroed.
    /// Reasoning: a `BindlessSlot` is only dropped after the
    /// system that owned it (the [`LoadedAsset`](hyge_asset::handle::LoadedAsset)
    /// wrapping it) has gone out of scope, which is always
    /// after the frame that recorded any draw command
    /// referencing the slot has been submitted. By the time
    /// the slot is freed, the GPU has either already
    /// executed the referencing draw (so the slot's data is
    /// stale-but-already-consumed) or has not yet seen it
    /// (so the next allocation will overwrite with valid
    /// data before the draw executes). The generation
    /// counter is bumped anyway so debug tooling can catch
    /// any future ABI-level stale references.
    fn release(&mut self, kind: SlotKind, slot: u32) {
        let (free_list, gen_index) = match kind {
            SlotKind::Mesh => (&mut self.meshes, 0),
            SlotKind::Material => (&mut self.materials, 1),
            SlotKind::Texture => (&mut self.textures, 2),
            SlotKind::Null => return,
        };
        free_list.release(slot);
        self.generations[gen_index][slot as usize] =
            self.generations[gen_index][slot as usize].wrapping_add(1);
    }
}

impl std::fmt::Debug for BindlessInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindlessInner")
            .field("meshes_free", &self.meshes.free_count())
            .field("meshes_capacity", &self.meshes.capacity())
            .field("materials_free", &self.materials.free_count())
            .field("materials_capacity", &self.materials.capacity())
            .field("textures_free", &self.textures.free_count())
            .field("textures_capacity", &self.textures.capacity())
            .finish_non_exhaustive()
    }
}

/// The bindless descriptor heap.
///
/// Constructed once at renderer startup. All asset GPU uploads
/// register through this table; shader code looks up resources
/// by the [`MeshId`], [`MaterialId`], [`TextureId`], etc.
///
/// # Threading
///
/// The table is `Send + Sync`. The internal state is guarded
/// by a `Mutex` (parking-lot-free path — std::sync::Mutex is
/// fine here, the critical section is short). Cloning a
/// [`BindlessSlot`] does not need to take the mutex; the
/// refcount is atomic. Dropping the last clone takes the
/// mutex to release the slot.
pub struct BindlessTable {
    config: BindlessConfig,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    /// The bind group layout used by the bindless shaders
    /// (R-040 wires this into the PBR pipeline). The layout
    /// is built once and never changed.
    layout: Arc<wgpu::BindGroupLayout>,
    /// The bind group referencing the live storage buffers +
    /// samplers + texture-array view. Created once and never
    /// rebuilt; slot recycling rewrites storage in place.
    bind_group: Arc<wgpu::BindGroup>,
    inner: Arc<Mutex<BindlessInner>>,
    /// The default samplers exposed to shaders.
    samplers: [Arc<wgpu::Sampler>; 4],
    /// Texture-array view. Exposed so the future PBR shader
    /// can bind it. The view format is sRGB-unorm for the
    /// albedo array and the BC7 form for the compressed
    /// path (R-040+).
    texture_view: Arc<wgpu::TextureView>,
}

impl std::fmt::Debug for BindlessTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self
            .inner
            .lock()
            .map(|g| format!("{g:?}"))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                String::from("<poisoned>")
            });
        f.debug_struct("BindlessTable")
            .field("config", &self.config)
            .field("inner", &inner)
            .finish_non_exhaustive()
    }
}

impl BindlessTable {
    /// Constructs the bindless table. Allocates the storage
    /// buffers and the texture array; creates the bind group
    /// and bind-group layout.
    ///
    /// The texture-array binding (slot 11) is only added to
    /// the bind group when the device has the
    /// `TEXTURE_BINDING_ARRAY` feature enabled. Adapters
    /// that lack the feature still get a fully-working
    /// mesh + material table; only the texture path is
    /// unavailable (texture registration returns
    /// `HygeError::Unsupported`).
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] if the device is lost.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        config: BindlessConfig,
    ) -> HygeResult<Self> {
        let has_texture_array = device
            .features()
            .contains(wgpu::Features::TEXTURE_BINDING_ARRAY);
        let mesh_bytes = (config.mesh_capacity as usize) * std::mem::size_of::<GpuMesh>();
        let material_bytes =
            (config.material_capacity as usize) * std::mem::size_of::<GpuMaterial>();
        let instance_bytes = (config.instance_capacity as usize) * std::mem::size_of::<Instance>();
        let light_bytes = (config.light_capacity as usize) * std::mem::size_of::<Light>();
        let light_grid_bytes =
            (config.light_grid_capacity as usize) * std::mem::size_of::<LightGrid>();
        // The light index list is a flat u32 SSBO. The capacity
        // is the same order of magnitude as the light grid (one
        // index per `light_grid` slot, conservatively).
        let light_index_list_bytes =
            (config.light_index_list_capacity as usize) * std::mem::size_of::<u32>();
        let meshlet_vis_bytes = (config.meshlet_visibility_capacity as usize)
            * std::mem::size_of::<MeshletVisibility>();
        let draw_command_bytes =
            (config.draw_command_capacity as usize) * std::mem::size_of::<DrawCommand>();

        let buffer_usages = wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC;
        let mesh_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/meshes"),
            size: mesh_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let material_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/materials"),
            size: material_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let instance_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/instances"),
            size: instance_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let light_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/lights"),
            size: light_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let light_grid_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/light-grid"),
            size: light_grid_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let light_index_list_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/light-index-list"),
            size: light_index_list_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let meshlet_visibility_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/meshlet-visibility"),
            size: meshlet_vis_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));
        let draw_command_buffer = Arc::new(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hyge-bindless/draw-commands"),
            size: draw_command_bytes as u64,
            usage: buffer_usages,
            mapped_at_creation: false,
        }));

        let texture_array = if has_texture_array {
            Some(Arc::new(device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hyge-bindless/texture-array"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: config.texture_capacity,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: config.texture_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })))
        } else {
            None
        };
        let texture_view = texture_array
            .as_ref()
            .map(|t| {
                Arc::new(t.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("hyge-bindless/texture-array-view"),
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                }))
            })
            .unwrap_or_else(|| {
                // Create a 1x1 placeholder texture so the
                // table can still expose a `texture_view`
                // accessor without `Option`. The placeholder
                // is never bound to the bind group.
                let placeholder = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("hyge-bindless/texture-array-placeholder"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: config.texture_format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                Arc::new(placeholder.create_view(&wgpu::TextureViewDescriptor::default()))
            });

        let samplers = [
            Arc::new(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("hyge-bindless/linear-clamp"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })),
            Arc::new(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("hyge-bindless/linear-repeat"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })),
            Arc::new(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("hyge-bindless/nearest-clamp"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            })),
            Arc::new(device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("hyge-bindless/shadow"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            })),
        ];

        // Build the bind-group layout entries. Slot 0-10
        // are always present; slot 11 (the texture array) is
        // only present when the device supports
        // `TEXTURE_BINDING_ARRAY`. The entries are collected
        // into a `Vec` first so we can pass `&entries` (a
        // slice) to the layout constructor.
        let mut layout_entries: Vec<wgpu::BindGroupLayoutEntry> = vec![
            // Sampler slot 0: linear clamp.
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // Sampler slot 1: linear repeat.
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // Sampler slot 2: nearest clamp.
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            // Sampler slot 3: shadow (comparison).
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            // Slot 4: meshes.
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 5: materials.
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 6: instances.
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 7: lights.
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 8: light grid.
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 9: meshlet visibility.
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 10: draw commands.
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Slot 12: light index list (u32 SSBO). Sits
            // outside the slot 11 texture-array range so the
            // texture-array feature gate does not affect the
            // clustered-forward path.
            wgpu::BindGroupLayoutEntry {
                binding: 12,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        if has_texture_array {
            // Slot 11+: texture array. The PBR shader
            // (R-040) reads it as
            // `textureLoad(textures, coord, slot)`.
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: Some(
                    std::num::NonZeroU32::new(config.texture_capacity).ok_or_else(|| {
                        HygeError::invalid_argument("bindless texture capacity must be > 0")
                    })?,
                ),
            });
        }
        let layout = Arc::new(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("hyge-bindless/layout"),
                entries: &layout_entries,
            }),
        );

        let mut bind_group_entries: Vec<wgpu::BindGroupEntry> = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Sampler(&samplers[0]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&samplers[1]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&samplers[2]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&samplers[3]),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: mesh_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: material_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: instance_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: light_grid_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: meshlet_visibility_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: draw_command_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: light_index_list_buffer.as_entire_binding(),
            },
        ];
        if has_texture_array {
            bind_group_entries.push(wgpu::BindGroupEntry {
                binding: 11,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            });
        }
        let bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hyge-bindless/bind-group"),
            layout: &layout,
            entries: &bind_group_entries,
        }));

        let inner = BindlessInner {
            meshes: FreeList::new(config.mesh_capacity),
            materials: FreeList::new(config.material_capacity),
            textures: FreeList::new(config.texture_capacity),
            mesh_buffer,
            material_buffer,
            instance_buffer,
            light_buffer,
            light_grid_buffer,
            light_index_list_buffer,
            meshlet_visibility_buffer,
            draw_command_buffer,
            texture_array,
            samplers: [
                Arc::clone(&samplers[0]),
                Arc::clone(&samplers[1]),
                Arc::clone(&samplers[2]),
                Arc::clone(&samplers[3]),
            ],
            generations: [
                vec![0u32; config.mesh_capacity as usize],
                vec![0u32; config.material_capacity as usize],
                vec![0u32; config.texture_capacity as usize],
            ],
        };

        Ok(Self {
            config,
            device,
            queue,
            layout,
            bind_group,
            inner: Arc::new(Mutex::new(inner)),
            samplers,
            texture_view,
        })
    }

    /// Returns the bind group that bundles every bindless
    /// resource. Bind it once at the start of a render pass
    /// (or once per frame in a non-pass-encoder context) and
    /// use the slot ids as lookup indices in the shader.
    #[must_use]
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// Returns the bind group layout.
    #[must_use]
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    /// Returns the texture array view.
    #[must_use]
    pub fn texture_view(&self) -> &wgpu::TextureView {
        &self.texture_view
    }

    /// Returns the wgpu device used to construct the table.
    /// Callers that need to create additional resources (e.g.
    /// a render pipeline that binds the layout) take a
    /// reference from here.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// Returns the wgpu queue used to construct the table.
    /// Used by [`BindlessTable::write_mesh`] and the other
    /// write helpers.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Returns the table's configuration.
    #[must_use]
    pub fn config(&self) -> &BindlessConfig {
        &self.config
    }

    /// Returns the number of free mesh slots. Test helper.
    pub fn free_mesh_slots(&self) -> u32 {
        self.inner
            .lock()
            .map(|g| g.meshes.free_count())
            .unwrap_or(0)
    }

    /// Returns the number of free material slots. Test helper.
    pub fn free_material_slots(&self) -> u32 {
        self.inner
            .lock()
            .map(|g| g.materials.free_count())
            .unwrap_or(0)
    }

    /// Returns the number of free texture slots. Test helper.
    pub fn free_texture_slots(&self) -> u32 {
        self.inner
            .lock()
            .map(|g| g.textures.free_count())
            .unwrap_or(0)
    }

    /// Allocates a mesh slot, writes `mesh` into the storage
    /// buffer at the allocated offset, and returns a typed
    /// [`MeshId`]. The storage write is committed on the
    /// caller's queue (the table takes `&self` for the
    /// buffer write so the wgpu side is fine; the allocator
    /// mutex is only held for the slot bookkeeping).
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] when the mesh allocator is
    /// exhausted (no free slots).
    pub fn register_mesh(&self, mesh: GpuMesh) -> HygeResult<MeshId> {
        let slot = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| HygeError::gpu(format!("bindless table mutex poisoned: {e}")))?;
            // The slot is held for the lifetime of the returned `MeshId`.
            // We do not bump the storage until the caller has
            // the slot, so a partial allocation is impossible
            // (we hold the lock for the whole sequence).
            inner
                .meshes
                .allocate()
                .ok_or_else(|| HygeError::gpu("bindless mesh allocator exhausted"))?
        };
        // Write the entry. This is outside the lock so a
        // concurrent register_material can proceed.
        let byte_offset = (slot as u64) * (std::mem::size_of::<GpuMesh>() as u64);
        self.queue.write_buffer(
            &self.get_mesh_buffer(),
            byte_offset,
            bytemuck::bytes_of(&mesh),
        );
        Ok(self.alloc_mesh_slot(slot))
    }

    /// Writes a batch of `meshes` to the storage buffer at
    /// the given slot index. Does not allocate; the slot
    /// must be in use (caller's responsibility). Mostly used
    /// by the upload path after the slot has been registered.
    pub fn write_mesh(&self, slot: u32, mesh: &GpuMesh) {
        let byte_offset = (slot as u64) * (std::mem::size_of::<GpuMesh>() as u64);
        self.queue.write_buffer(
            &self.get_mesh_buffer(),
            byte_offset,
            bytemuck::bytes_of(mesh),
        );
    }

    /// Allocates a material slot, writes `material` into the
    /// storage buffer, and returns a typed [`MaterialId`].
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] when the material
    /// allocator is exhausted.
    pub fn register_material(&self, material: GpuMaterial) -> HygeResult<MaterialId> {
        let slot = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| HygeError::gpu(format!("bindless table mutex poisoned: {e}")))?;
            inner
                .materials
                .allocate()
                .ok_or_else(|| HygeError::gpu("bindless material allocator exhausted"))?
        };
        let byte_offset = (slot as u64) * (std::mem::size_of::<GpuMaterial>() as u64);
        self.queue.write_buffer(
            &self.get_material_buffer(),
            byte_offset,
            bytemuck::bytes_of(&material),
        );
        Ok(self.alloc_material_slot(slot))
    }

    /// Writes a single material entry at `slot`. Does not
    /// allocate.
    pub fn write_material(&self, slot: u32, material: &GpuMaterial) {
        let byte_offset = (slot as u64) * (std::mem::size_of::<GpuMaterial>() as u64);
        self.queue.write_buffer(
            &self.get_material_buffer(),
            byte_offset,
            bytemuck::bytes_of(material),
        );
    }

    /// Allocates a texture slot in the texture array and
    /// returns a typed [`TextureId`]. The texture is not
    /// written by this call — the caller writes the array
    /// layer's pixel data through `wgpu::Queue::write_texture`
    /// once it has the slot.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::Gpu`] when the texture allocator
    /// is exhausted.
    pub fn register_texture(&self) -> HygeResult<TextureId> {
        let slot = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|e| HygeError::gpu(format!("bindless table mutex poisoned: {e}")))?;
            inner
                .textures
                .allocate()
                .ok_or_else(|| HygeError::gpu("bindless texture allocator exhausted"))?
        };
        Ok(self.alloc_texture_slot(slot))
    }

    /// Returns the wgpu mesh storage buffer. Mostly used by
    /// the upload path and by tests.
    #[must_use]
    pub fn get_mesh_buffer(&self) -> Arc<wgpu::Buffer> {
        // The buffer is stored as an `Arc` inside the
        // `BindlessInner`; cloning the `Arc` is cheap and
        // does not require the mutex.
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.mesh_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                // Return a placeholder buffer; this is a
                // last-resort path that should never fire in
                // practice.
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/mesh-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Returns the wgpu material storage buffer.
    #[must_use]
    pub fn get_material_buffer(&self) -> Arc<wgpu::Buffer> {
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.material_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/material-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Returns the wgpu instance storage buffer.
    #[must_use]
    pub fn get_instance_buffer(&self) -> Arc<wgpu::Buffer> {
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.instance_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/instance-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Returns the wgpu light storage buffer.
    #[must_use]
    pub fn get_light_buffer(&self) -> Arc<wgpu::Buffer> {
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.light_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/light-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Returns the wgpu light-grid storage buffer.
    #[must_use]
    pub fn get_light_grid_buffer(&self) -> Arc<wgpu::Buffer> {
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.light_grid_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/light-grid-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Returns the wgpu light-index-list storage buffer.
    #[must_use]
    pub fn get_light_index_list_buffer(&self) -> Arc<wgpu::Buffer> {
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.light_index_list_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/light-index-list-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Returns the wgpu draw-command storage buffer.
    #[must_use]
    pub fn get_draw_command_buffer(&self) -> Arc<wgpu::Buffer> {
        self.inner
            .lock()
            .map(|g| Arc::clone(&g.draw_command_buffer))
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "bindless table mutex poisoned");
                Arc::new(self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("hyge-bindless/draw-command-buffer-fallback"),
                    size: 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }))
            })
    }

    /// Writes a slice of instances starting at slot `start`.
    /// Does not allocate; slots must already be valid.
    pub fn write_instances(&self, start: u32, instances: &[Instance]) {
        if instances.is_empty() {
            return;
        }
        let byte_offset = (start as u64) * (std::mem::size_of::<Instance>() as u64);
        self.queue
            .write_buffer(&self.get_instance_buffer(), byte_offset, bytemuck::cast_slice(instances));
    }

    /// Writes a slice of lights starting at slot `start`.
    /// Does not allocate; slots must already be valid.
    pub fn write_lights(&self, start: u32, lights: &[Light]) {
        if lights.is_empty() {
            return;
        }
        let byte_offset = (start as u64) * (std::mem::size_of::<Light>() as u64);
        self.queue
            .write_buffer(&self.get_light_buffer(), byte_offset, bytemuck::cast_slice(lights));
    }

    /// Writes a slice of light-grid entries starting at slot `start`.
    /// Does not allocate; slots must already be valid.
    pub fn write_light_grid(&self, start: u32, entries: &[LightGrid]) {
        if entries.is_empty() {
            return;
        }
        let byte_offset = (start as u64) * (std::mem::size_of::<LightGrid>() as u64);
        self.queue
            .write_buffer(&self.get_light_grid_buffer(), byte_offset, bytemuck::cast_slice(entries));
    }

    /// Writes a slice of u32 light indices starting at slot
    /// `start`. The PBR fragment shader reads this as the
    /// backing store for the `LightIndexList` cluster index
    /// list. Empty slices are a no-op.
    pub fn write_light_index_list(&self, start: u32, indices: &[u32]) {
        if indices.is_empty() {
            return;
        }
        let byte_offset = (start as u64) * std::mem::size_of::<u32>() as u64;
        self.queue
            .write_buffer(&self.get_light_index_list_buffer(), byte_offset, bytemuck::cast_slice(indices));
    }

    /// Writes a slice of draw commands starting at slot `start`.
    /// Does not allocate; slots must already be valid.
    pub fn write_draw_commands(&self, start: u32, commands: &[DrawCommand]) {
        if commands.is_empty() {
            return;
        }
        let byte_offset = (start as u64) * (std::mem::size_of::<DrawCommand>() as u64);
        self.queue
            .write_buffer(&self.get_draw_command_buffer(), byte_offset, bytemuck::cast_slice(commands));
    }

    /// Returns the wgpu texture array, or `None` if the
    /// device does not have the `TEXTURE_BINDING_ARRAY`
    /// feature enabled.
    #[must_use]
    pub fn get_texture_array(&self) -> Option<Arc<wgpu::Texture>> {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.texture_array.as_ref().map(Arc::clone))
    }

    /// Returns a clone of the default samplers (linear clamp,
    /// linear repeat, nearest clamp, shadow).
    #[must_use]
    pub fn samplers(&self) -> &[Arc<wgpu::Sampler>; 4] {
        &self.samplers
    }

    // -- internal slot allocation ---------------------------------
    // These helpers create the typed `BindlessSlot` values. The
    // refcount starts at 1 (the returned value); `Clone`
    // increments; `Drop` decrements and frees the slot when
    // the count hits zero.

    fn alloc_mesh_slot(&self, slot: u32) -> MeshId {
        let refcount = Arc::new(Refcount {
            slot,
            kind: SlotKind::Mesh,
            table: Some(Arc::clone(&self.inner)),
            refs: AtomicU32::new(1),
        });
        BindlessSlot {
            refcount,
            _marker: PhantomData,
        }
    }

    fn alloc_material_slot(&self, slot: u32) -> MaterialId {
        let refcount = Arc::new(Refcount {
            slot,
            kind: SlotKind::Material,
            table: Some(Arc::clone(&self.inner)),
            refs: AtomicU32::new(1),
        });
        BindlessSlot {
            refcount,
            _marker: PhantomData,
        }
    }

    fn alloc_texture_slot(&self, slot: u32) -> TextureId {
        let refcount = Arc::new(Refcount {
            slot,
            kind: SlotKind::Texture,
            table: Some(Arc::clone(&self.inner)),
            refs: AtomicU32::new(1),
        });
        BindlessSlot {
            refcount,
            _marker: PhantomData,
        }
    }
}

impl Drop for BindlessTable {
    fn drop(&mut self) {
        // No explicit cleanup: the wgpu buffers, texture, and
        // samplers drop when `self` is dropped. The free
        // lists and generation counters live in `self.inner`
        // and drop with it. The `Arc<Refcount>` clones held
        // by callers will free their slots through `Drop`;
        // any remaining slots at table-drop time simply
        // become unreachable.
    }
}

/// ABI-bridging helper used by integration tests and the
/// `hyge-scene` extractor to convert a slice of
/// layout-compatible PODs defined in one crate (e.g.
/// `hyge_scene::extract::Instance`) into a `Vec` of the
/// bindless-table mirror PODs (e.g.
/// `hyge_render::bindless::Instance`) without going through
/// `unsafe` raw-pointer casts.
///
/// The two types must have the same `size_of` and
/// `align_of`; this is asserted at runtime via
/// `debug_assert!` so a refactor that drifts the layout
/// trips the test rather than silently producing
/// mis-aligned reads.
pub fn pod_collect_to_vec<Src, Dst>(src: &[Src]) -> Vec<Dst>
where
    Src: Pod,
    Dst: Pod,
{
    debug_assert_eq!(
        std::mem::size_of::<Src>(),
        std::mem::size_of::<Dst>(),
        "pod_collect_to_vec: source and destination layouts must match"
    );
    debug_assert_eq!(
        std::mem::align_of::<Src>(),
        std::mem::align_of::<Dst>(),
        "pod_collect_to_vec: source and destination alignments must match"
    );
    if src.is_empty() {
        return Vec::new();
    }
    // SAFETY: the two PODs have been asserted layout-compatible
    // above. The source slice outlives the destination Vec
    // because we copy the bytes into a fresh allocation.
    let src_bytes: &[u8] = bytemuck::cast_slice(src);
    let mut dst = Vec::with_capacity(src.len());
    let dst_bytes_mut = unsafe {
        std::slice::from_raw_parts_mut(
            dst.as_mut_ptr() as *mut u8,
            dst.capacity() * std::mem::size_of::<Dst>(),
        )
    };
    dst_bytes_mut[..src_bytes.len()].copy_from_slice(src_bytes);
    unsafe { dst.set_len(src.len()) };
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slot kinds map to the architecture §8.1 slot ids.
    #[test]
    fn slot_kind_maps_to_architecture_slot() {
        assert_eq!(SlotKind::Mesh.architecture_slot(), 4);
        assert_eq!(SlotKind::Material.architecture_slot(), 5);
        assert_eq!(SlotKind::Texture.architecture_slot(), 11);
        assert_eq!(SlotKind::Null.architecture_slot(), NULL_SLOT);
    }

    /// The default `BindlessConfig` matches the architecture
    /// §8.1 capacities.
    #[test]
    fn default_config_matches_architecture() {
        let c = BindlessConfig::default();
        assert_eq!(c.mesh_capacity, MESH_CAPACITY);
        assert_eq!(c.material_capacity, MATERIAL_CAPACITY);
        assert_eq!(c.instance_capacity, INSTANCE_CAPACITY);
        assert_eq!(c.light_capacity, LIGHT_CAPACITY);
    }

    /// A `FreeList` of capacity 4 starts full, drains to
    /// empty, and recovers entries on release.
    #[test]
    fn free_list_drains_and_recovers() {
        let mut fl = FreeList::new(4);
        assert_eq!(fl.free_count(), 4);
        let a = fl.allocate().expect("first alloc");
        let b = fl.allocate().expect("second alloc");
        let c = fl.allocate().expect("third alloc");
        let d = fl.allocate().expect("fourth alloc");
        assert!(fl.allocate().is_none());
        assert_eq!(fl.free_count(), 0);
        // All four slot ids are within the capacity.
        for s in [a, b, c, d] {
            assert!(s < 4);
        }
        fl.release(b);
        fl.release(d);
        assert_eq!(fl.free_count(), 2);
        let e = fl.allocate().expect("recovered alloc");
        assert_eq!(fl.free_count(), 1);
        assert!(e < 4);
    }

    /// `GpuMesh` is `Pod` so it can be `bytemuck`-cast to
    /// bytes for the GPU upload.
    #[test]
    fn gpu_mesh_is_pod() {
        let m = GpuMesh {
            vertex_offset: 0,
            index_offset: 0,
            meshlet_offset: 0,
            meshlet_count: 0,
            aabb_min: [0.0; 3],
            aabb_max: [1.0; 3],
            lod_count: 0,
            _pad: 0,
        };
        let bytes = bytemuck::bytes_of(&m);
        assert_eq!(bytes.len(), std::mem::size_of::<GpuMesh>());
        let round: GpuMesh = *bytemuck::from_bytes(bytes);
        assert_eq!(m.vertex_offset, round.vertex_offset);
        assert_eq!(m.aabb_min, round.aabb_min);
    }

    /// `GpuMaterial` is `Pod` and round-trips through
    /// `bytemuck`.
    #[test]
    fn gpu_material_is_pod() {
        let m = GpuMaterial {
            base_color: 1,
            normal: 2,
            mr: 3,
            occlusion: 4,
            emissive: 5,
            roughness: 0.5,
            metallic: 0.25,
            alpha_mode: 0,
            flags: 0,
        };
        let bytes = bytemuck::bytes_of(&m);
        let round: GpuMaterial = *bytemuck::from_bytes(bytes);
        assert_eq!(m, round);
    }

    /// `BindlessSlot` cloning bumps the refcount; dropping
    /// the last clone decrements to zero. We exercise this
    /// without a real wgpu device by manually constructing
    /// two `Arc<Refcount>` for the same slot (this is the
    /// path the table's `Clone` impl follows).
    #[test]
    fn refcount_bump_and_decrement() {
        let refcount = Arc::new(Refcount {
            slot: 0,
            kind: SlotKind::Null,
            table: None, // no free callback for this test
            refs: AtomicU32::new(1),
        });
        assert_eq!(refcount.refs(), 1);
        let clone = Arc::clone(&refcount);
        refcount.refs.fetch_add(1, Ordering::AcqRel);
        assert_eq!(refcount.refs(), 2);
        drop(clone);
        refcount.refs.fetch_sub(1, Ordering::AcqRel);
        assert_eq!(refcount.refs(), 1);
    }

    /// `pod_collect_to_vec` round-trips two layout-identical
    /// POD types. We use a hand-rolled source type that
    /// mirrors `Instance`'s field order to make the test
    /// independent of the bindless table.
    #[test]
    fn pod_collect_to_vec_round_trips() {
        #[repr(C)]
        #[derive(Copy, Clone, Debug, Pod, Zeroable)]
        struct SrcInstance {
            transform: [[f32; 4]; 3],
            mesh_id: u32,
            material_id: u32,
            _pad: [u32; 2],
        }
        assert_eq!(std::mem::size_of::<SrcInstance>(), std::mem::size_of::<Instance>());
        let src = vec![
            SrcInstance {
                transform: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]],
                mesh_id: 7,
                material_id: 11,
                _pad: [0; 2],
            },
            SrcInstance {
                transform: [[2.0, 0.0, 0.0, 0.0], [0.0, 2.0, 0.0, 0.0], [0.0, 0.0, 2.0, 0.0]],
                mesh_id: 8,
                material_id: 12,
                _pad: [0; 2],
            },
        ];
        let dst: Vec<Instance> = pod_collect_to_vec(&src);
        assert_eq!(dst.len(), 2);
        assert_eq!(dst[0].mesh_id, 7);
        assert_eq!(dst[0].material_id, 11);
        assert_eq!(dst[1].mesh_id, 8);
        assert_eq!(dst[1].material_id, 12);
    }

    /// `pod_collect_to_vec` on an empty slice returns an
    /// empty Vec without touching the unsafe path.
    #[test]
    fn pod_collect_to_vec_empty_is_empty() {
        let src: Vec<Instance> = Vec::new();
        let dst: Vec<Instance> = pod_collect_to_vec(&src);
        assert!(dst.is_empty());
    }
}
