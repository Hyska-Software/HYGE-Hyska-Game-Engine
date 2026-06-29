//! Insta snapshot test for the 5-pass barrier-inference algorithm.
//!
//! Builds a fixed 5-pass graph (shadow → gbuffer → lighting → tonemap
//! → present), compiles it, and snapshots every barrier the compiler
//! emits in execution order. The snapshot is the source of truth for
//! the algorithm's behavior; if you change the inference logic, run
//! `cargo insta review` to accept the new output, then commit the
//! updated snapshot file.
//!
//! Run with:
//! ```bash
//! & "C:\Users\estev\.cargo\bin\cargo.exe" test -p hyge-render-graph \
//!     --test barrier_inference
//! ```
//!
//! The test requires a `wgpu` adapter (CI installs lavapipe on Linux
//! for the headless render job). On machines without a GPU the test
//! early-returns so the snapshot is not exercised.

use std::fmt;

use hyge_render_graph::prelude::*;

/// One row in the snapshot: which pass emitted the barrier, plus
/// the barrier's resource handle, kind, and `(from, to)` usage pair.
/// The struct is local to the test so the snapshot stays readable
/// across bitflag `Debug` changes.
#[derive(Debug)]
struct SnapshotBarrier {
    pass: String,
    resource: u32,
    kind: &'static str,
    from: String,
    to: String,
}

impl fmt::Display for SnapshotBarrier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<22} r{:<2} {:<7} {} -> {}",
            self.pass, self.resource, self.kind, self.from, self.to
        )
    }
}

fn snapshot_for(compiled: &CompiledGraph) -> Vec<SnapshotBarrier> {
    let mut out = Vec::new();
    for (i, pass) in compiled.passes().iter().enumerate() {
        for barrier in pass.barriers() {
            let (kind, from, to) = match barrier {
                Barrier::Texture { from, to, .. } => {
                    ("Texture", format!("{from:?}"), format!("{to:?}"))
                }
                Barrier::Buffer { from, to, .. } => {
                    ("Buffer", format!("{from:?}"), format!("{to:?}"))
                }
            };
            out.push(SnapshotBarrier {
                pass: format!("{i}:{}", pass.name()),
                resource: barrier.resource().index(),
                kind,
                from,
                to,
            });
        }
    }
    out
}

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
            label: Some("hyge-render-graph-snapshot"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::default(),
        },
        None,
    ))
    .ok()?;
    Some(device)
}

/// A fixed pass fixture with `reads` / `writes` and per-resource
/// `texture_usages` declared. The body is the same one R-020 used;
/// the snapshot test reuses it to keep the algorithm coverage
/// reproducible.
struct FixedPass {
    name_: &'static str,
    reads_: Vec<ResourceHandle>,
    writes_: Vec<ResourceHandle>,
    tex_usage: Vec<(ResourceHandle, wgpu::TextureUsages)>,
}

impl Pass for FixedPass {
    fn name(&self) -> &str {
        self.name_
    }
    fn reads(&self) -> Vec<ResourceHandle> {
        self.reads_.clone()
    }
    fn writes(&self) -> Vec<ResourceHandle> {
        self.writes_.clone()
    }
    fn texture_usages(&self) -> Vec<(ResourceHandle, wgpu::TextureUsages)> {
        self.tex_usage.clone()
    }
    fn record(&mut self, _ctx: &mut PassContext<'_>) {}
}

fn build_5_pass_graph() -> (RenderGraph, [ResourceHandle; 5]) {
    let mut g = RenderGraph::new();
    let shadow_map = g.add_resource(
        ResourceKind::Texture(TextureDesc::new_2d(
            2048,
            2048,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )),
        ResourceLifetime::Transient,
    );
    let gbuffer_color = g.add_resource(
        ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )),
        ResourceLifetime::Transient,
    );
    let gbuffer_depth = g.add_resource(
        ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        )),
        ResourceLifetime::Transient,
    );
    let lighting_color = g.add_resource(
        ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )),
        ResourceLifetime::Transient,
    );
    // `final_color` is Persistent to exercise the "first touch on
    // Persistent ⇒ always emit a barrier" path in the algorithm.
    let final_color = g.add_resource(
        ResourceKind::Texture(TextureDesc::new_2d(
            1920,
            1080,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )),
        ResourceLifetime::Persistent,
    );
    let handles = [
        shadow_map,
        gbuffer_color,
        gbuffer_depth,
        lighting_color,
        final_color,
    ];

    // Pass 0: shadow — writes shadow_map with RENDER_ATTACHMENT.
    g.add_pass(FixedPass {
        name_: "shadow",
        reads_: Vec::new(),
        writes_: vec![shadow_map],
        tex_usage: vec![(shadow_map, wgpu::TextureUsages::RENDER_ATTACHMENT)],
    });
    // Pass 1: gbuffer — reads shadow_map (TEXTURE_BINDING), writes
    // gbuffer_color + gbuffer_depth with RENDER_ATTACHMENT.
    g.add_pass(FixedPass {
        name_: "gbuffer",
        reads_: vec![shadow_map],
        writes_: vec![gbuffer_color, gbuffer_depth],
        tex_usage: vec![
            (shadow_map, wgpu::TextureUsages::TEXTURE_BINDING),
            (gbuffer_color, wgpu::TextureUsages::RENDER_ATTACHMENT),
            (gbuffer_depth, wgpu::TextureUsages::RENDER_ATTACHMENT),
        ],
    });
    // Pass 2: lighting — reads gbuffer_color + shadow_map as
    // TEXTURE_BINDING, writes lighting_color as
    // RENDER_ATTACHMENT | TEXTURE_BINDING.
    g.add_pass(FixedPass {
        name_: "lighting",
        reads_: vec![gbuffer_color, shadow_map],
        writes_: vec![lighting_color],
        tex_usage: vec![
            (gbuffer_color, wgpu::TextureUsages::TEXTURE_BINDING),
            (shadow_map, wgpu::TextureUsages::TEXTURE_BINDING),
            (
                lighting_color,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        ],
    });
    // Pass 3: tonemap — reads lighting_color (TEXTURE_BINDING),
    // writes final_color (Persistent, RENDER_ATTACHMENT |
    // TEXTURE_BINDING).
    g.add_pass(FixedPass {
        name_: "tonemap",
        reads_: vec![lighting_color],
        writes_: vec![final_color],
        tex_usage: vec![
            (lighting_color, wgpu::TextureUsages::TEXTURE_BINDING),
            (
                final_color,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        ],
    });
    // Pass 4: present — reads final_color as TEXTURE_BINDING.
    g.add_pass(FixedPass {
        name_: "present",
        reads_: vec![final_color],
        writes_: Vec::new(),
        tex_usage: vec![(final_color, wgpu::TextureUsages::TEXTURE_BINDING)],
    });

    (g, handles)
}

#[test]
fn five_pass_graph_barrier_inference_snapshot() {
    let Some(device) = try_create_device() else {
        eprintln!("no wgpu adapter; skipping snapshot test");
        return;
    };

    let (mut g, _handles) = build_5_pass_graph();
    let compiled = g.compile(&device).expect("5-pass graph must compile");

    // Topological order: shadow → gbuffer → lighting → tonemap →
    // present. The exposure-style edge is gbuffer_color (RENDER_ATTACHMENT
    // → RENDER_ATTACHMENT | TEXTURE_BINDING → RENDER_ATTACHMENT) and
    // final_color (Persistent first-touch).
    assert_eq!(compiled.passes().len(), 5);
    let names: Vec<&str> = compiled.passes().iter().map(|p| p.name()).collect();
    assert_eq!(
        names,
        ["shadow", "gbuffer", "lighting", "tonemap", "present"]
    );

    let snapshot = snapshot_for(&compiled);
    insta::assert_debug_snapshot!(snapshot);
}
