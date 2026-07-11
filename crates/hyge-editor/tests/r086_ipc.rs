//! R-086 real loopback IPC evidence.

use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyge_asset::{AssetDb, AssetId};
use hyge_editor::{EditorServer, EditorServerConfig};
use hyge_editor_protocol::{read_envelope, write_envelope, Envelope, MessageType};
use hyge_render::profiler::{FrameStats, PassStats};
use tracing_subscriber::prelude::*;

fn root() -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("hyge_r086_ipc_{suffix}"));
    fs::create_dir_all(root.join("assets")).expect("assets");
    root
}

fn send(stream: &mut TcpStream, envelope: Envelope) -> Envelope {
    write_envelope(stream, &envelope).expect("write envelope");
    read_envelope(stream).expect("read envelope")
}

#[test]
fn asset_console_and_profiler_round_trip_through_tcp() {
    let project = root();
    let mut db = AssetDb::open(&project.join(".hyge.db")).expect("db");
    let asset = AssetId::from(blake3::hash(b"ipc-asset"));
    let source = project.join("assets").join("ipc.bin");
    fs::write(&source, b"ipc asset").expect("source");
    db.insert(&asset, &source).expect("asset");

    let server = Arc::new(
        EditorServer::bind(EditorServerConfig {
            session_token: "token".into(),
            ..EditorServerConfig::default()
        })
        .expect("bind"),
    );
    let address = server.local_addr().expect("address");
    let server_thread = Arc::clone(&server);
    let thread = thread::spawn(move || server_thread.run().expect("server"));
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");
    let hello = send(&mut stream, Envelope::hello("hello", "token"));
    assert_eq!(hello.message_type, MessageType::HelloAck);
    let session_id = hello.payload["session_id"]
        .as_str()
        .expect("session")
        .to_owned();

    let project_response = Envelope::new(
        "project",
        MessageType::OpenProject,
        serde_json::json!({"path": project}),
    );
    write_envelope(&mut stream, &project_response).expect("project");
    let _ = read_envelope(&mut stream).expect("loading");
    let _ = read_envelope(&mut stream).expect("ready");
    let completed = read_envelope(&mut stream).expect("completed");
    assert_eq!(completed.message_type, MessageType::CommandCompleted);

    let services = server
        .session_data_services(&session_id)
        .expect("session services");
    tracing::subscriber::with_default(
        tracing_subscriber::registry().with(services.console_layer()),
        || tracing::info!(target: "hyge.ipc", "tracing round trip"),
    );
    services.profiler.record_frame_stats(
        &FrameStats {
            frame_time_ms: 10.0,
            fps: 100.0,
            total_gpu_time_ms: 2.0,
            passes: vec![PassStats {
                name: "ipc".into(),
                gpu_time_ms: 2.0,
            }],
            draw_calls: 3,
            instance_count: 4,
        },
        99,
    );

    let snapshot = send(
        &mut stream,
        Envelope::new(
            "assets",
            MessageType::RequestAssetSnapshot,
            serde_json::json!({}),
        ),
    );
    assert_eq!(snapshot.message_type, MessageType::AssetSnapshot);
    assert_eq!(
        snapshot.payload["nodes"].as_array().expect("nodes").len(),
        1
    );

    let console = send(
        &mut stream,
        Envelope::new(
            "console",
            MessageType::RequestConsoleSnapshot,
            serde_json::json!({}),
        ),
    );
    assert_eq!(console.message_type, MessageType::ConsoleSnapshot);
    assert_eq!(console.payload["lines"][0]["message"], "tracing round trip");
    let profiler = send(
        &mut stream,
        Envelope::new(
            "profiler",
            MessageType::RequestProfilerSnapshot,
            serde_json::json!({}),
        ),
    );
    assert_eq!(profiler.message_type, MessageType::ProfilerSnapshot);
    assert_eq!(profiler.payload["samples"][0]["draw_calls"], 3);

    let shutdown = send(
        &mut stream,
        Envelope::new(
            "shutdown",
            MessageType::ServerShutdown,
            serde_json::json!({}),
        ),
    );
    assert_eq!(shutdown.message_type, MessageType::ServerShutdown);
    drop(stream);
    thread.join().expect("server thread");
}

#[test]
fn profiler_sink_data_is_serializable_for_ipc() {
    let services = hyge_editor::EditorDataServices::default();
    services.profiler.record_frame_stats(
        &FrameStats {
            frame_time_ms: 10.0,
            fps: 100.0,
            total_gpu_time_ms: 2.0,
            passes: vec![PassStats {
                name: "ipc".into(),
                gpu_time_ms: 2.0,
            }],
            draw_calls: 3,
            instance_count: 4,
        },
        99,
    );
    let payload = serde_json::to_value(services.profiler.snapshot()).expect("json");
    assert_eq!(payload["samples"][0]["draw_calls"], 3);
    assert_eq!(payload["samples"][0]["asset_cache_bytes"], 99);
}
