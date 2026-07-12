//! R-102 generation invalidation and server teardown evidence.

use std::net::TcpStream;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use hyge_editor::{EditorServer, EditorServerConfig, SharedViewportTransport};
use hyge_editor_protocol::{read_envelope, write_envelope, Envelope, MessageType};

fn connect(address: std::net::SocketAddr, session_id: Option<String>) -> (TcpStream, Envelope) {
    let mut stream = TcpStream::connect(address).expect("connect editor server");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set timeout");
    let mut hello = Envelope::hello("hello", "hyge-local-dev");
    if let Some(session_id) = session_id {
        hello.payload["session_id"] = serde_json::Value::String(session_id);
    }
    write_envelope(&mut stream, &hello).expect("write hello");
    let response = read_envelope(&mut stream).expect("read hello ack");
    (stream, response)
}

#[test]
fn reconnect_replaces_old_generation_before_shutdown() {
    let server = Arc::new(EditorServer::bind(EditorServerConfig::default()).expect("bind"));
    let address = server.local_addr().expect("address");
    let runner = Arc::clone(&server);
    let thread = thread::spawn(move || runner.run().expect("run"));

    let (old, hello) = connect(address, None);
    let session_id = hello.payload["session_id"]
        .as_str()
        .expect("session id")
        .to_owned();
    let (_new, resumed) = connect(address, Some(session_id));
    assert_eq!(resumed.message_type, MessageType::HelloAck);
    assert_eq!(resumed.payload["resumed"], serde_json::Value::Bool(true));

    assert!(old.peer_addr().is_ok());

    server.shutdown();
    thread.join().expect("server shutdown");
}

#[test]
fn shutdown_wakes_server_and_is_idempotent() {
    let server = Arc::new(EditorServer::bind(EditorServerConfig::default()).expect("bind"));
    let runner = Arc::clone(&server);
    let thread = thread::spawn(move || runner.run().expect("run"));
    thread::sleep(Duration::from_millis(20));
    server.shutdown();
    server.shutdown();
    thread.join().expect("join server");
}

#[test]
fn backend_restart_rejects_old_session_and_accepts_fresh_recovery() {
    let first = Arc::new(EditorServer::bind(EditorServerConfig::default()).expect("bind first"));
    let first_address = first.local_addr().expect("first address");
    let first_runner = Arc::clone(&first);
    let first_thread = thread::spawn(move || first_runner.run().expect("run first"));
    let (first_stream, hello) = connect(first_address, None);
    let old_session = hello.payload["session_id"]
        .as_str()
        .expect("old session")
        .to_owned();
    drop(first_stream);
    first.shutdown();
    first_thread.join().expect("stop first backend");
    drop(first);

    let second = Arc::new(EditorServer::bind(EditorServerConfig::default()).expect("bind second"));
    let second_address = second.local_addr().expect("second address");
    let second_runner = Arc::clone(&second);
    let second_thread = thread::spawn(move || second_runner.run().expect("run second"));
    let (_stale_stream, stale_response) = connect(second_address, Some(old_session));
    assert_eq!(stale_response.message_type, MessageType::EngineError);
    assert_eq!(
        stale_response.error.expect("session error").code,
        "session_not_found"
    );
    let (_fresh_stream, fresh_response) = connect(second_address, None);
    assert_eq!(fresh_response.message_type, MessageType::HelloAck);
    assert!(!fresh_response.payload["resumed"].as_bool().unwrap_or(true));
    second.shutdown();
    second_thread.join().expect("stop second backend");
}

#[test]
fn viewport_transport_close_is_idempotent_and_releases_producer() {
    let mut transport = SharedViewportTransport::create(
        "Local\\hyge-r102-test".to_owned(),
        7,
        64 + 3 * (64 + 4 * 4 * 4),
    )
    .expect("transport");
    transport.close();
    transport.close();
    assert!(!transport.is_mapped());
    assert!(transport.publish(4, 4, 1, 1, &[0; 64]).is_err());
}

#[test]
fn invalid_project_is_an_actionable_error_not_a_service_panic() {
    let server = Arc::new(EditorServer::bind(EditorServerConfig::default()).expect("bind"));
    let address = server.local_addr().expect("address");
    let runner = Arc::clone(&server);
    let thread = thread::spawn(move || runner.run().expect("run"));
    let (mut stream, hello) = connect(address, None);
    assert_eq!(hello.message_type, MessageType::HelloAck);
    write_envelope(
        &mut stream,
        &Envelope::new(
            "project",
            MessageType::OpenProject,
            serde_json::json!({"path": "missing-project"}),
        ),
    )
    .expect("write project request");
    let first = read_envelope(&mut stream).expect("first response");
    let second = read_envelope(&mut stream).expect("second response");
    assert_eq!(first.message_type, MessageType::LifecycleStatus);
    assert_eq!(second.message_type, MessageType::LifecycleStatus);
    assert_eq!(
        second.payload["state"],
        serde_json::Value::String("failed".into())
    );
    let response = read_envelope(&mut stream).expect("diagnostic error");
    assert_eq!(response.message_type, MessageType::EngineError);
    let error = response.error.expect("structured error");
    assert!(error.recoverable);
    assert_eq!(error.operation.as_deref(), Some("open_project"));
    assert_eq!(error.path.as_deref(), Some("missing-project"));
    server.shutdown();
    thread.join().expect("server shutdown");
}
