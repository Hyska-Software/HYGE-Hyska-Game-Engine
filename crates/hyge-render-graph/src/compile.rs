//! Compiled render graph: a topologically-ordered pass sequence with
//! per-pass barriers and a transient resource table.
//!
//! [`RenderGraph::compile`](crate::graph::RenderGraph::compile)
//! returns a [`CompiledGraph`]. The caller then calls
//! [`CompiledGraph::execute`] once per frame, providing a
//! `wgpu::CommandEncoder` to record into.
//!
//! The execute method walks the pass sequence in order, emits the
//! barriers computed during compile, and calls each pass's
//! `record()` method. At the skeleton stage the wgpu-level transition
//! emission is a TODO (R-021 will wire it up); the per-pass barrier
//! list is fully computed and exposed for inspection.

use hyge_core::prelude::*;

use crate::allocator::TransientAllocator;
use crate::barrier::Barrier;
use crate::pass::{Pass, PassContext, ResourceTable};
use crate::resource::{ResourceHandle, ResourceKind, ResourceLifetime};
use crate::graph::ResourceEntry;

/// One pass in the compiled graph, plus the barriers that lead into it.
pub struct CompiledPass {
    /// Stable id assigned at graph build.
    pub id: crate::graph::PassId,
    /// Cached pass name.
    pub name: String,
    /// Cached reads.
    pub reads: Vec<ResourceHandle>,
    /// Cached writes.
    pub writes: Vec<ResourceHandle>,
    /// The pass implementation (taken from the graph by `compile`).
    pass: Box<dyn Pass>,
    /// Barriers emitted by the compiler just before this pass records.
    pub barriers_before: Vec<Barrier>,
}

impl CompiledPass {
    /// Returns the pass id.
    #[must_use]
    pub fn id(&self) -> crate::graph::PassId {
        self.id
    }
    /// Returns the pass name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Returns the resources this pass reads.
    #[must_use]
    pub fn reads(&self) -> &[ResourceHandle] {
        &self.reads
    }
    /// Returns the resources this pass writes.
    #[must_use]
    pub fn writes(&self) -> &[ResourceHandle] {
        &self.writes
    }
    /// Returns the barriers that the compiler wants emitted just
    /// before this pass records.
    #[must_use]
    pub fn barriers(&self) -> &[Barrier] {
        &self.barriers_before
    }
}

/// The result of [`RenderGraph::compile`](crate::graph::RenderGraph::compile):
/// an ordered pass sequence plus the resource table and a
/// [`TransientAllocator`] for first-touch allocations.
pub struct CompiledGraph {
    passes: Vec<CompiledPass>,
    resources: Vec<ResourceEntry>,
    allocator: TransientAllocator,
    table: ResourceTable,
}

impl CompiledGraph {
    /// Internal constructor used by [`RenderGraph::compile`](crate::graph::RenderGraph::compile).
    pub(crate) fn from_parts(passes: Vec<CompiledPass>, resources: Vec<ResourceEntry>) -> Self {
        let mut table = ResourceTable::default();
        // Ensure every resource handle has a slot in the table so
        // pass lookups don't panic.
        for r in &resources {
            match r.kind() {
                ResourceKind::Texture(_) => table.ensure_texture_slot(r.handle()),
                ResourceKind::Buffer(_) => table.ensure_buffer_slot(r.handle()),
            }
        }
        let mut allocator = TransientAllocator::new();
        for r in &resources {
            if r.lifetime().is_transient() {
                allocator.register(r.handle(), r.kind().clone());
            }
        }
        Self {
            passes,
            resources,
            allocator,
            table,
        }
    }

    /// Returns the pass sequence in execution order.
    #[must_use]
    pub fn passes(&self) -> &[CompiledPass] {
        &self.passes
    }

    /// Returns a flat vector of every barrier in execution order.
    /// The first barrier of pass `i` follows the last barrier of pass
    /// `i-1`; the slice is a concatenation of each pass's
    /// `barriers_before` list.
    #[must_use]
    pub fn barriers(&self) -> Vec<Barrier> {
        let mut out = Vec::new();
        for p in &self.passes {
            out.extend_from_slice(&p.barriers_before);
        }
        out
    }

    /// Returns the resource table for the compiled graph.
    ///
    /// Use the [`ResourceTable`](crate::pass::ResourceTable)'s
    /// `set_texture` / `set_buffer` to install persistent resources
    /// before calling [`CompiledGraph::execute`].
    #[must_use]
    pub fn table(&self) -> &ResourceTable {
        &self.table
    }

    /// Returns a mutable resource table for installing persistent
    /// resources before calling [`CompiledGraph::execute`].
    #[must_use]
    pub fn table_mut(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    /// Returns the transient allocator driving first-touch allocation
    /// for `ResourceLifetime::Transient` resources.
    #[must_use]
    pub fn allocator(&self) -> &TransientAllocator {
        &self.allocator
    }

    /// Returns a mutable transient allocator (R-022 will use this
    /// for first-touch slot allocation).
    #[must_use]
    pub fn allocator_mut(&mut self) -> &mut TransientAllocator {
        &mut self.allocator
    }

    /// Returns the descriptors for every resource in the graph.
    #[must_use]
    pub fn resource_entries(&self) -> &[ResourceEntry] {
        &self.resources
    }

    /// Executes the compiled graph into the given `wgpu::CommandEncoder`.
    ///
    /// For each pass, in topological order:
    /// 1. A debug group is pushed with the pass name.
    /// 2. The barriers the compiler inferred for this pass are
    ///    "emitted" (the skeleton logs them; the wgpu transition
    ///    call is a TODO for R-021).
    /// 3. The pass's `record()` method is called with a
    ///    [`PassContext`].
    /// 4. The debug group is popped.
    ///
    /// Persistent resources must be installed in the resource table
    /// before calling `execute`; transient resources are populated
    /// lazily in R-022.
    pub fn execute(&mut self, encoder: &mut wgpu::CommandEncoder) {
        for pass in &mut self.passes {
            encoder.push_debug_group(pass.name.as_str());
            // Emit barriers: in wgpu 22, per-resource transitions are
            // not directly exposed in the high-level API; the
            // barrier list is emitted by the actual pass via the
            // encoder (each pass declares its transitions inside
            // `record`). We log the inferred barriers so tests can
            // observe them. R-021 will replace this with explicit
            // wgpu transition calls.
            for barrier in &pass.barriers_before {
                tracing::debug!("barrier: {barrier}");
            }
            {
                let mut ctx = PassContext::new(&self.table, encoder);
                pass.pass.record(&mut ctx);
            }
            encoder.pop_debug_group();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::RenderGraph;
    use crate::pass::PassContext;
    use crate::resource::{BufferDesc, TextureDesc};
    use crate::graph::PassId;

    /// Records the pass name and the barriers it observed, so tests
    /// can inspect the compile output without a real `wgpu::Device`.
    struct NoopRecordPass {
        name_: &'static str,
        reads_: Vec<ResourceHandle>,
        writes_: Vec<ResourceHandle>,
        tex_usage: Vec<(ResourceHandle, wgpu::TextureUses)>,
    }
    impl Pass for NoopRecordPass {
        fn name(&self) -> &str {
            self.name_
        }
        fn reads(&self) -> Vec<ResourceHandle> {
            self.reads_.clone()
        }
        fn writes(&self) -> Vec<ResourceHandle> {
            self.writes_.clone()
        }
        fn texture_usages(&self) -> Vec<(ResourceHandle, wgpu::TextureUses)> {
            self.tex_usage.clone()
        }
        fn record(&mut self, _ctx: &mut PassContext<'_>) {}
    }

    /// Attempts to create a wgpu device for tests that need one.
    /// Returns `None` if the environment has no adapter (e.g. CI
    /// without a GPU ICD); the calling test should early-return in
    /// that case.
    fn try_create_device() -> Option<wgpu::Device> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: true,
            compatible_surface: None,
        }))?;
        let (device, _queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("hyge-render-graph-test"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: wgpu::MemoryHints::default(),
            },
            None,
        ))
        .ok()?;
        Some(device)
    }

    #[test]
    fn three_pass_graph_compiles_in_topological_order_with_barriers() {
        let Some(device) = try_create_device() else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };

        // Build the graph: gbuffer -> lighting -> tonemap, with a
        // linear dependency chain via the `exposure` resource
        // (lighting writes it, tonemap reads it). Without that edge
        // tonemap could legally appear before lighting in the topo
        // order, which is unstable to assert on.
        let mut g = RenderGraph::new();

        let color = g.add_resource(
            ResourceKind::Texture(TextureDesc::new_2d(
                1920,
                1080,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )),
            ResourceLifetime::Transient,
        );
        let depth = g.add_resource(
            ResourceKind::Texture(TextureDesc::new_2d(
                1920,
                1080,
                wgpu::TextureFormat::Depth32Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )),
            ResourceLifetime::Transient,
        );
        let exposure = g.add_resource(
            ResourceKind::Texture(TextureDesc::new_2d(
                1920,
                1080,
                wgpu::TextureFormat::R16Float,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            )),
            ResourceLifetime::Transient,
        );

        let gbuffer_id: PassId = g.add_pass(NoopRecordPass {
            name_: "gbuffer",
            reads_: Vec::new(),
            writes_: vec![color, depth],
            tex_usage: vec![
                (color, wgpu::TextureUses::RENDER_ATTACHMENT),
                (depth, wgpu::TextureUses::RENDER_ATTACHMENT),
            ],
        });
        let _lighting_id: PassId = g.add_pass(NoopRecordPass {
            name_: "lighting",
            reads_: vec![color],
            writes_: vec![color, exposure],
            tex_usage: vec![
                (color, wgpu::TextureUses::RENDER_ATTACHMENT | wgpu::TextureUses::TEXTURE_BINDING),
                (exposure, wgpu::TextureUses::RENDER_ATTACHMENT),
            ],
        });
        let _tonemap_id: PassId = g.add_pass(NoopRecordPass {
            name_: "tonemap",
            reads_: vec![color, exposure],
            writes_: vec![color],
            tex_usage: vec![(color, wgpu::TextureUses::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)],
        });

        let compiled = g.compile(&device).expect("compile should succeed");

        // Topological order: gbuffer, lighting, tonemap (the exposure
        // edge lighting → tonemap makes the chain linear).
        assert_eq!(compiled.passes().len(), 3);
        assert_eq!(compiled.passes()[0].name(), "gbuffer");
        assert_eq!(compiled.passes()[1].name(), "lighting");
        assert_eq!(compiled.passes()[2].name(), "tonemap");
        assert_eq!(compiled.passes()[0].id(), gbuffer_id);

        // Barriers emitted: the gbuffer pass is first-touch on
        // color/depth (one barrier each); the lighting pass is
        // first-touch on exposure and reuses color; the tonemap
        // pass reuses both color and exposure.
        let barriers = compiled.barriers();
        assert!(
            !barriers.is_empty(),
            "first-touch barriers should be emitted for color, depth, and exposure"
        );
        let barrier_resources: std::collections::HashSet<ResourceHandle> =
            barriers.iter().map(Barrier::resource).collect();
        assert!(barrier_resources.contains(&color));
        assert!(barrier_resources.contains(&depth));
        assert!(barrier_resources.contains(&exposure));

        // All barriers should be texture barriers (this graph has no buffers).
        assert!(barriers.iter().all(Barrier::is_texture));

        // Per-pass barrier placement: the gbuffer pass is the
        // first to touch color/depth so it has the most barriers.
        assert!(!compiled.passes()[0].barriers().is_empty());
    }

    #[test]
    fn cycle_in_pass_graph_is_rejected() {
        // Two passes that read each other's resource produce a cycle.
        // We need a device to invoke compile().
        let Some(device) = try_create_device() else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };

        // Two resources, two passes that each read the other's
        // resource and write their own. This is a cycle: A→B→A.
        let mut g = RenderGraph::new();
        let ra = g.add_resource(
            ResourceKind::Texture(TextureDesc::new_2d(
                1,
                1,
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING,
            )),
            ResourceLifetime::Transient,
        );
        let rb = g.add_resource(
            ResourceKind::Texture(TextureDesc::new_2d(
                1,
                1,
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING,
            )),
            ResourceLifetime::Transient,
        );
        g.add_pass(NoopRecordPass {
            name_: "a",
            reads_: vec![rb],
            writes_: vec![ra],
            tex_usage: Vec::new(),
        });
        g.add_pass(NoopRecordPass {
            name_: "b",
            reads_: vec![ra],
            writes_: vec![rb],
            tex_usage: Vec::new(),
        });
        // a → b (via rb: a reads rb, b writes rb)
        // b → a (via ra: b reads ra, a writes ra)
        // ⇒ cycle a → b → a.
        let err = g.compile(&device).expect_err("cycle must be rejected");
        assert!(
            matches!(err, HygeError::RenderGraphCycle(_)),
            "expected RenderGraphCycle, got {err:?}",
        );
    }

    #[test]
    fn double_writer_is_rejected() {
        let Some(device) = try_create_device() else {
            eprintln!("no wgpu adapter; skipping");
            return;
        };
        let mut g = RenderGraph::new();
        let r = g.add_resource(
            ResourceKind::Buffer(BufferDesc::new(64, wgpu::BufferUsages::STORAGE)),
            ResourceLifetime::Transient,
        );
        g.add_pass(NoopRecordPass {
            name_: "a",
            reads_: Vec::new(),
            writes_: vec![r],
            tex_usage: Vec::new(),
        });
        g.add_pass(NoopRecordPass {
            name_: "b",
            reads_: Vec::new(),
            writes_: vec![r],
            tex_usage: Vec::new(),
        });
        let err = g.compile(&device).expect_err("double writer must be rejected");
        assert!(
            matches!(err, HygeError::InvalidArgument(_)),
            "expected InvalidArgument, got {err:?}",
        );
    }
}
