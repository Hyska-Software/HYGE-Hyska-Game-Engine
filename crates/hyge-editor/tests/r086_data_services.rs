//! R-086 bounded data service coverage.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hyge_asset::importer::mesh::{self, MeshData, Vertex};
use hyge_asset::{AssetDb, AssetId};
use hyge_editor::{ConsoleFilter, ConsoleLayer, EditorDataServices};
use hyge_render::profiler::{FrameStats, PassStats};
use tracing_subscriber::prelude::*;

fn project_root(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hyge_r086_{name}_{suffix}"));
    fs::create_dir_all(&root).expect("root");
    root
}

fn id(seed: &[u8]) -> AssetId {
    AssetId::from(blake3::hash(seed))
}

#[test]
fn asset_snapshot_reads_sorted_graph_from_asset_db() {
    let root = project_root("assets");
    let mut db = AssetDb::open(&root.join(".hyge.db")).expect("db");
    let parent = id(b"parent");
    let child = id(b"child");
    db.insert(&parent, std::path::Path::new("cache/parent.hyge-mat"))
        .expect("parent");
    db.insert(&child, std::path::Path::new("cache/child.ktx2"))
        .expect("child");
    db.add_dependency(&parent, &child).expect("edge");

    let services = EditorDataServices::default();
    let snapshot = services.asset_snapshot(&root).expect("snapshot");
    assert_eq!(snapshot.nodes.len(), 2);
    assert_eq!(snapshot.edges.len(), 1);
    assert!(snapshot.nodes[0].asset_id < snapshot.nodes[1].asset_id);
    assert_eq!(snapshot.edges[0].parent, hex(parent));
    assert_eq!(snapshot.edges[0].child, hex(child));
}

#[test]
fn console_layer_retains_last_thousand_and_filters() {
    let buffer = hyge_editor::ConsoleBuffer::default();
    let subscriber = tracing_subscriber::registry().with(ConsoleLayer::new(buffer.clone()));
    tracing::subscriber::with_default(subscriber, || {
        for index in 0..1001 {
            tracing::info!(target: "hyge.test", "line {index}");
        }
        tracing::error!(target: "other", "important");
    });
    let all = buffer.snapshot(ConsoleFilter::default());
    assert_eq!(all.lines.len(), 1000);
    assert!(all
        .lines
        .iter()
        .any(|line| line.message.contains("important")));
    let filtered = buffer.snapshot(ConsoleFilter {
        min_level: Some("error".into()),
        target_prefix: Some("other".into()),
    });
    assert_eq!(filtered.lines.len(), 1);
    assert_eq!(filtered.lines[0].message, "important");
}

#[test]
fn profiler_snapshot_is_bounded_and_preserves_metrics() {
    let services = EditorDataServices::default();
    let stats = FrameStats {
        frame_time_ms: 16.0,
        fps: 62.5,
        total_gpu_time_ms: 4.0,
        passes: vec![PassStats {
            name: "main".into(),
            gpu_time_ms: 4.0,
        }],
        draw_calls: 7,
        instance_count: 11,
    };
    for _ in 0..241 {
        services.profiler.record_frame_stats(&stats, 1234);
    }
    let snapshot = services.profiler.snapshot();
    assert_eq!(snapshot.samples.len(), 240);
    let last = snapshot.samples.last().expect("sample");
    assert_eq!(last.draw_calls, 7);
    assert_eq!(last.instance_count, 11);
    assert_eq!(last.asset_cache_bytes, 1234);
    assert_eq!(last.passes[0].name, "main");
}

#[test]
fn preview_output_path_and_bytes_are_deterministic() {
    let root = project_root("preview");
    let mut db = AssetDb::open(&root.join(".hyge.db")).expect("db");
    let asset = id(b"preview");
    let source = root.join("source.hyge-mesh");
    let mesh = MeshData::from_triangle_list(
        vec![
            Vertex {
                position: [-1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.0, 0.0],
            },
            Vertex {
                position: [1.0, -1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [1.0, 0.0],
            },
            Vertex {
                position: [0.0, 1.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                uv: [0.5, 1.0],
            },
        ],
        vec![0, 1, 2],
    );
    fs::write(&source, mesh::to_bytes(&mesh).expect("mesh bytes")).expect("source");
    db.insert(&asset, &source).expect("asset");
    let services = EditorDataServices::default();
    services.previews.set_project(Some(&root));
    let first = services
        .previews
        .request(&hex(asset), "job-1")
        .expect("preview");
    let second = services
        .previews
        .request(&hex(asset), "job-2")
        .expect("preview");
    assert_eq!(first.state, "completed");
    assert_eq!(first.path, second.path);
    assert_eq!(first.hash, second.hash);
    assert_eq!((first.width, first.height), (Some(256), Some(256)));
    assert!(root.join(first.path.expect("path")).exists());
}

fn hex(id: AssetId) -> String {
    id.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
