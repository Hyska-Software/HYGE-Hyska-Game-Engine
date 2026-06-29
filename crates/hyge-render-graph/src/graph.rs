//! [`RenderGraph`]: the user-facing builder for a render-graph DAG.
//!
//! Internally a [`RenderGraph`] owns a [`petgraph::graph::DiGraph`] of
//! `PassNode`s, plus a side table of [`ResourceEntry`] (the
//! descriptors declared via [`RenderGraph::add_resource`]). Edges
//! between passes are inferred from each pass's `reads` / `writes`
//! declarations: for every resource `R` written by pass `A` and
//! read by pass `B`, an edge `A → B` is added. The result is a
//! directed acyclic graph (DAG) that the compiler topsorts and walks
//! to emit barriers.
//!
//! # Cycle handling
//!
//! [`RenderGraph::compile`] calls
//! [`petgraph::algo::toposort`]; on a cycle it returns
//! [`HygeError::RenderGraphCycle`] with the name of the offending
//! node. Cycles are programmer errors — review the `reads` /
//! `writes` declarations on your passes. The graph does not silently
//! pick an arbitrary order.
//!
//! # Multi-writer resources
//!
//! Each resource is allowed to be written by at most one pass; a
//! resource written twice yields [`HygeError::InvalidArgument`]
//! during compile. Multiple readers are fine.

use std::collections::HashMap;
use std::fmt;

use indexmap::IndexMap;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use tracing::trace;

use hyge_core::prelude::*;

use crate::barrier::Barrier;
use crate::compile::CompiledGraph;
use crate::pass::Pass;
use crate::resource::{ResourceHandle, ResourceKind, ResourceLifetime};

/// Stable handle to a pass registered in a [`RenderGraph`].
///
/// Passes are identified by an incrementing `u32`; the index is
/// unique within the graph that minted the handle. Handles are
/// `Copy` and compare by index.
#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct PassId(u32);

impl PassId {
    /// Returns the raw index of this pass id.
    #[inline]
    #[must_use]
    pub fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for PassId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "p{}", self.0)
    }
}

/// Per-pass storage in the graph.
struct PassNode {
    /// Stable id.
    id: PassId,
    /// Cached pass name.
    name: String,
    /// The actual pass. `None` after `compile()` consumes it.
    pass: Option<Box<dyn Pass>>,
    /// Cached `reads` snapshot, captured at registration.
    reads: Vec<ResourceHandle>,
    /// Cached `writes` snapshot, captured at registration.
    writes: Vec<ResourceHandle>,
}

/// Per-resource storage in the graph (separate from the `DiGraph`).
#[derive(Clone, Debug)]
pub struct ResourceEntry {
    handle: ResourceHandle,
    kind: ResourceKind,
    lifetime: ResourceLifetime,
}

/// A directed acyclic graph of render passes.
///
/// Use [`RenderGraph::new`] to construct an empty graph, add resources
/// with [`RenderGraph::add_resource`], add passes with
/// [`RenderGraph::add_pass`], and call [`RenderGraph::compile`] to
/// produce a [`CompiledGraph`].
#[derive(Default)]
pub struct RenderGraph {
    pass_nodes: HashMap<PassId, PassNode>,
    pass_to_node: HashMap<PassId, NodeIndex>,
    /// Resources indexed by handle. `IndexMap` keeps the insertion
    /// order so the compiled pass list is stable across runs and
    /// tests don't have to rely on `HashMap`'s randomized order.
    resources: IndexMap<ResourceHandle, ResourceEntry>,
    graph: DiGraph<PassId, ()>,
    next_pass_id: u32,
    next_resource_id: u32,
}

impl fmt::Debug for RenderGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderGraph")
            .field("pass_count", &self.pass_nodes.len())
            .field("resource_count", &self.resources.len())
            .field("edge_count", &self.graph.edge_count())
            .finish()
    }
}

impl RenderGraph {
    /// Creates an empty render graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares a resource in the graph and returns its handle.
    ///
    /// The `kind` and `lifetime` are stored in the resource table and
    /// consulted by [`RenderGraph::compile`] to (a) validate
    /// pass→resource wiring and (b) drive the
    /// [`TransientAllocator`](crate::allocator::TransientAllocator)
    /// for transient lifetimes.
    pub fn add_resource(
        &mut self,
        kind: ResourceKind,
        lifetime: ResourceLifetime,
    ) -> ResourceHandle {
        let handle = ResourceHandle::from_index(self.next_resource_id);
        self.next_resource_id += 1;
        self.resources.insert(
            handle,
            ResourceEntry {
                handle,
                kind,
                lifetime,
            },
        );
        trace!(?handle, "render graph: add resource");
        handle
    }

    /// Registers a pass and returns its id.
    ///
    /// The pass's `reads()` and `writes()` methods are called once at
    /// registration to snapshot the resource wiring; subsequent
    /// changes to those methods after registration are not picked
    /// up.
    pub fn add_pass<P: Pass>(&mut self, pass: P) -> PassId {
        let id = PassId(self.next_pass_id);
        self.next_pass_id += 1;
        let name = pass.name().to_owned();
        let reads = pass.reads();
        let writes = pass.writes();
        let node_idx = self.graph.add_node(id);
        self.pass_to_node.insert(id, node_idx);
        self.pass_nodes.insert(
            id,
            PassNode {
                id,
                name,
                pass: Some(Box::new(pass)),
                reads,
                writes,
            },
        );
        trace!(?id, name = %self.pass_nodes[&id].name, "render graph: add pass");
        id
    }

    /// Borrows the pass implementation registered for the given id.
    #[must_use]
    pub fn pass(&self, id: PassId) -> Option<&dyn Pass> {
        self.pass_nodes.get(&id).and_then(|n| n.pass.as_deref())
    }

    /// Borrows the resource entry registered for the given handle.
    #[must_use]
    pub fn resource(&self, handle: ResourceHandle) -> Option<&ResourceEntry> {
        self.resources.get(&handle)
    }

    /// Returns the number of passes registered.
    #[must_use]
    pub fn pass_count(&self) -> usize {
        self.pass_nodes.len()
    }

    /// Returns the number of resources declared.
    #[must_use]
    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }

    /// Compiles the graph against the given device.
    ///
    /// Steps:
    /// 1. Validate that each resource is written by at most one pass.
    /// 2. Infer pass-to-pass edges from `reads` / `writes` (a pass
    ///    that writes `R` and a pass that reads `R` get an edge).
    /// 3. Toposort the resulting DAG (cycle → `RenderGraphCycle`).
    /// 4. Walk the topological order emitting [`Barrier`]s for each
    ///    resource transition.
    /// 5. Transfer the pass objects into the returned
    ///    [`CompiledGraph`].
    ///
    /// The `_device` argument is unused at the skeleton stage; it is
    /// accepted in the API so the transient allocator (R-022) can
    /// allocate `wgpu` resources without a signature change.
    ///
    /// # Errors
    ///
    /// Returns [`HygeError::InvalidArgument`] if a resource is
    /// written by more than one pass, and
    /// [`HygeError::RenderGraphCycle`] if the inferred graph
    /// contains a cycle.
    pub fn compile(&mut self, _device: &wgpu::Device) -> HygeResult<CompiledGraph> {
        // Step 1: collect writers / readers per resource.
        let mut writer: HashMap<ResourceHandle, PassId> = HashMap::new();
        let mut readers: HashMap<ResourceHandle, Vec<PassId>> = HashMap::new();
        for node in self.pass_nodes.values() {
            for r in &node.writes {
                if let Some(prev) = writer.get(r) {
                    return Err(HygeError::invalid_argument(format!(
                        "resource {r} is written by both pass {} and pass {}; \
                         each resource may be written by at most one pass",
                        prev.index(),
                        node.id.index(),
                    )));
                }
                writer.insert(*r, node.id);
            }
            for r in &node.reads {
                readers.entry(*r).or_default().push(node.id);
            }
        }

        // Step 2: infer edges from writer → reader.
        for (resource, writer_id) in &writer {
            if let Some(rs) = readers.get(resource) {
                for reader_id in rs {
                    if reader_id == writer_id {
                        continue;
                    }
                    let from = self.pass_to_node[writer_id];
                    let to = self.pass_to_node[reader_id];
                    if self.graph.find_edge(from, to).is_none() {
                        self.graph.add_edge(from, to, ());
                    }
                }
            }
        }

        // Step 3: toposort. Cycle → HygeError::RenderGraphCycle.
        let topo = toposort(&self.graph, None).map_err(|cycle| {
            let node = cycle.node_id();
            let pid = self.graph[node];
            let name = self
                .pass_nodes
                .get(&pid)
                .map_or_else(|| format!("pass {pid}"), |n| n.name.clone());
            HygeError::render_graph_cycle(format!("pass {pid} ({name})"))
        })?;

        // Build a quick lookup of per-pass texture/buffer usage hints.
        let mut tex_usage: HashMap<(PassId, ResourceHandle), wgpu::TextureUsages> = HashMap::new();
        let mut buf_usage: HashMap<(PassId, ResourceHandle), wgpu::BufferUsages> = HashMap::new();
        for node in self.pass_nodes.values() {
            if let Some(pass) = node.pass.as_ref() {
                for (h, u) in pass.texture_usages() {
                    tex_usage.insert((node.id, h), u);
                }
                for (h, u) in pass.buffer_usages() {
                    buf_usage.insert((node.id, h), u);
                }
            }
        }

        // Step 4: barrier inference.
        //
        // Algorithm (see `barrier.rs` module-level rustdoc for the
        // full prose):
        //   - Walk passes in topological order.
        //   - For every resource the pass touches, look up the prior
        //     access pattern in `state`.
        //     * If the resource was already touched in this frame
        //       and the new usage differs from the prior usage,
        //       emit a barrier.
        //     * If the resource is touched for the first time in
        //       this frame and is `ResourceLifetime::Transient`,
        //       emit a barrier iff the requested usage is not the
        //       "uninitialized" sentinel.
        //     * If the resource is touched for the first time in
        //       this frame and is `ResourceLifetime::Persistent`,
        //       ALWAYS emit a barrier — the runtime does not know
        //       the prior state of a persistent resource (it may
        //       have been written by the host, the swapchain, or
        //       an external system) so we conservatively force a
        //       transition so the underlying `wgpu` backend
        //       inserts a real `transition_resources` call at
        //       execute time.
        //   - Always update `state` to the new usage.
        //
        // `state` is a [`crate::barrier::BarrierStateTable`] — the
        // only `unsafe` in the crate lives in that type. The
        // miri test in `barrier.rs` validates its `set_len`-based
        // grow path; the state updates here exercise it on the
        // hot path.
        let mut state = crate::barrier::BarrierStateTable::new();
        let mut barriers_per_pass: Vec<Vec<Barrier>> = Vec::with_capacity(topo.len());
        let mut pass_ids_in_order: Vec<PassId> = Vec::with_capacity(topo.len());

        for node_idx in &topo {
            let pid = self.graph[*node_idx];
            pass_ids_in_order.push(pid);
            let node = self
                .pass_nodes
                .get(&pid)
                .expect("topo pass id present in graph");

            let mut barriers = Vec::new();
            // Walk the resources this pass touches; union reads and writes,
            // dedup.
            let mut touched: Vec<ResourceHandle> = node.reads.clone();
            for w in &node.writes {
                if !node.reads.contains(w) {
                    touched.push(*w);
                }
            }
            for r in &touched {
                let entry = self
                    .resources
                    .get(r)
                    .expect("pass references a declared resource");
                let is_persistent = entry.lifetime().is_persistent();
                match &entry.kind {
                    ResourceKind::Texture(_) => {
                        let to = tex_usage
                            .get(&(pid, *r))
                            .copied()
                            .unwrap_or_else(wgpu::TextureUsages::empty);
                        let is_first_touch = state.get(*r).is_none();
                        let from = match state.get(*r) {
                            Some(crate::barrier::AccessState::Texture(u)) => u,
                            _ => wgpu::TextureUsages::empty(),
                        };
                        // Emission rule (see module rustdoc):
                        //   * first touch + Transient + to == empty          -> no barrier
                        //   * first touch + Transient + to != empty          -> barrier
                        //   * first touch + Persistent                       -> barrier (always)
                        //   * subsequent touch + from != to                  -> barrier
                        //   * subsequent touch + from == to                  -> no barrier
                        let should_emit = if is_first_touch {
                            is_persistent || !to.is_empty()
                        } else {
                            !crate::barrier::AccessState::Texture(from)
                                .matches(crate::barrier::AccessState::Texture(to))
                        };
                        if should_emit {
                            barriers.push(Barrier::Texture {
                                resource: *r,
                                from,
                                to,
                            });
                        }
                        state.set(*r, crate::barrier::AccessState::Texture(to));
                    }
                    ResourceKind::Buffer(_) => {
                        let to = buf_usage
                            .get(&(pid, *r))
                            .copied()
                            .unwrap_or_else(wgpu::BufferUsages::empty);
                        let is_first_touch = state.get(*r).is_none();
                        let from = match state.get(*r) {
                            Some(crate::barrier::AccessState::Buffer(u)) => u,
                            _ => wgpu::BufferUsages::empty(),
                        };
                        let should_emit = if is_first_touch {
                            // For buffers, the "uninitialized" sentinel
                            // is the empty bitflag; we always emit on
                            // first touch of a Persistent buffer, and
                            // emit on a Transient buffer only when the
                            // requested usage is non-empty.
                            is_persistent || !to.is_empty()
                        } else {
                            !crate::barrier::AccessState::Buffer(from)
                                .matches(crate::barrier::AccessState::Buffer(to))
                        };
                        if should_emit {
                            barriers.push(Barrier::Buffer {
                                resource: *r,
                                from,
                                to,
                            });
                        }
                        state.set(*r, crate::barrier::AccessState::Buffer(to));
                    }
                }
            }
            barriers_per_pass.push(barriers);
        }

        // Step 5: build CompiledGraph by transferring the pass objects.
        let mut compiled_passes = Vec::with_capacity(pass_ids_in_order.len());
        for (i, pid) in pass_ids_in_order.iter().enumerate() {
            let node = self
                .pass_nodes
                .get_mut(pid)
                .expect("topo pass id present in graph");
            let pass = node
                .pass
                .take()
                .expect("pass was not yet consumed by a previous compile");
            let barriers = barriers_per_pass[i].clone();
            let name = node.name.clone();
            compiled_passes.push(crate::compile::CompiledPass {
                id: *pid,
                name,
                reads: node.reads.clone(),
                writes: node.writes.clone(),
                pass,
                barriers_before: barriers,
            });
        }

        // Convert the resource map to a stable-order vector so the
        // CompiledGraph can index by handle without re-hashing.
        let mut resource_vec: Vec<ResourceEntry> = self.resources.values().cloned().collect();
        resource_vec.sort_by_key(|e| e.handle().index());

        Ok(CompiledGraph::from_parts(compiled_passes, resource_vec))
    }
}

// `ResourceEntry` is `Clone` (it has no wgpu resources yet at the
// graph level — the `ResourceTable` inside `CompiledGraph` holds those).
// We expose a small accessor for the compile step.
impl ResourceEntry {
    /// Returns the stable resource handle.
    #[must_use]
    pub fn handle(&self) -> ResourceHandle {
        self.handle
    }
    /// Returns the resource kind (texture or buffer descriptor).
    #[must_use]
    pub fn kind(&self) -> &ResourceKind {
        &self.kind
    }
    /// Returns the resource lifetime.
    #[must_use]
    pub fn lifetime(&self) -> ResourceLifetime {
        self.lifetime
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass::PassContext;
    use crate::resource::{BufferDesc, TextureDesc};

    struct NoopPass(&'static str);
    impl Pass for NoopPass {
        fn name(&self) -> &str {
            self.0
        }
        fn reads(&self) -> Vec<ResourceHandle> {
            Vec::new()
        }
        fn writes(&self) -> Vec<ResourceHandle> {
            Vec::new()
        }
        fn record(&mut self, _ctx: &mut PassContext<'_>) {}
    }

    #[test]
    fn empty_graph() {
        let g = RenderGraph::new();
        assert_eq!(g.pass_count(), 0);
        assert_eq!(g.resource_count(), 0);
    }

    #[test]
    fn add_resource_and_pass() {
        let mut g = RenderGraph::new();
        let tex = g.add_resource(
            ResourceKind::Texture(TextureDesc::new_2d(
                1,
                1,
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING,
            )),
            ResourceLifetime::Transient,
        );
        let p = g.add_pass(NoopPass("noop"));
        assert_eq!(g.pass_count(), 1);
        assert_eq!(g.resource_count(), 1);
        assert!(g.pass(p).is_some());
        assert!(g.resource(tex).is_some());
    }

    #[test]
    fn pass_id_display_is_compact() {
        let mut g = RenderGraph::new();
        let p = g.add_pass(NoopPass("alpha"));
        assert_eq!(format!("{p}"), "p0");
        let _ = g.add_pass(NoopPass("beta"));
    }

    #[test]
    fn double_writer_setup() {
        struct WritePass {
            name_: &'static str,
            r: ResourceHandle,
        }
        impl Pass for WritePass {
            fn name(&self) -> &str {
                self.name_
            }
            fn reads(&self) -> Vec<ResourceHandle> {
                Vec::new()
            }
            fn writes(&self) -> Vec<ResourceHandle> {
                vec![self.r]
            }
            fn record(&mut self, _ctx: &mut PassContext<'_>) {}
        }

        // The graph builder happily accepts two writers; the actual
        // double-writer rejection lives behind compile(). The full
        // check is exercised in the compile.rs test that runs against
        // a real wgpu device.
        let mut g = RenderGraph::new();
        let r = g.add_resource(
            ResourceKind::Buffer(BufferDesc::new(64, wgpu::BufferUsages::STORAGE)),
            ResourceLifetime::Transient,
        );
        g.add_pass(WritePass { name_: "a", r });
        g.add_pass(WritePass { name_: "b", r });
        assert_eq!(g.pass_count(), 2);
    }
}
