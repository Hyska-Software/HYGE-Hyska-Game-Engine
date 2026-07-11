//! R-082 real project, scene, save and reconnect lifecycle evidence.

use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use hyge_editor::{EditorServer, EditorServerConfig, EditorSessionRuntime, LifecycleState};
use hyge_editor_protocol::{read_envelope, write_envelope, Envelope, MessageType};
use hyge_scene::{
    Environment, PostProcessProfile, Prefab, PrefabAssets, PrefabId, PrefabInstance, PrefabNode,
    Transform, WorldDocument,
};

struct TempProject(PathBuf);

static NEXT_PROJECT: AtomicU64 = AtomicU64::new(1);

impl TempProject {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hyge-r082-{}-{}",
            std::process::id(),
            NEXT_PROJECT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create project");
        Self(path)
    }

    fn write_fixture(&self) -> PathBuf {
        let prefab = Prefab::new(
            "r082-root",
            PrefabNode::named("root"),
            PrefabAssets::default(),
        );
        fs::write(
            self.0.join("root.hyge-prefab"),
            prefab.to_bytes().expect("prefab bytes"),
        )
        .expect("write prefab");
        let document = WorldDocument {
            env: Environment::empty(),
            root_prefab_instances: vec![PrefabInstance::new(
                prefab.prefab_id,
                Transform::identity(),
            )],
            post_process: PostProcessProfile::default(),
        };
        let scene = self.0.join("main.hyge-world");
        fs::write(&scene, document.to_bytes().expect("scene bytes")).expect("write scene");
        scene
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn runtime_opens_real_scene_saves_revision_and_releases_lock() {
    let project = TempProject::new();
    let scene = project.write_fixture();
    let mut runtime = EditorSessionRuntime::new();
    let project_snapshot = runtime.open_project(&project.0).expect("open project");
    assert!(matches!(project_snapshot.state, LifecycleState::Degraded));
    let scene_snapshot = runtime.open_scene(&scene).expect("open scene");
    assert_eq!(scene_snapshot.state, LifecycleState::Ready);
    assert_eq!(
        scene_snapshot.scene.as_deref(),
        scene.canonicalize().ok().as_deref()
    );
    let saved = runtime.save_scene().expect("save scene");
    assert_eq!(saved.revision, 1);
    assert_eq!(
        fs::read_to_string(project.0.join(".hyge/editor.revision")).expect("revision"),
        "1"
    );
    let mut contending = EditorSessionRuntime::new();
    assert!(contending.open_project(&project.0).is_err());
    let invalid = project.0.join("invalid.hyge-world");
    let invalid_document = WorldDocument {
        env: Environment::empty(),
        root_prefab_instances: vec![PrefabInstance::new(
            PrefabId::compute(b"missing"),
            Transform::identity(),
        )],
        post_process: PostProcessProfile::default(),
    };
    fs::write(
        &invalid,
        invalid_document.to_bytes().expect("invalid fixture bytes"),
    )
    .expect("write invalid fixture");
    assert!(runtime.open_scene(&invalid).is_err());
    assert_eq!(
        runtime.snapshot().scene,
        Some(scene.canonicalize().expect("canonical scene"))
    );
    drop(runtime);
    let mut second = EditorSessionRuntime::new();
    second.open_project(&project.0).expect("lock released");
}

#[test]
fn tcp_open_save_reconnect_and_shutdown_reports_lifecycle() {
    let project = TempProject::new();
    let scene = project.write_fixture();
    let server = EditorServer::bind(EditorServerConfig::default()).expect("bind");
    let address = server.local_addr().expect("address");
    let thread = thread::spawn(move || server.run().expect("server run"));

    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    write_envelope(&mut stream, &Envelope::hello("hello", "hyge-local-dev")).expect("hello");
    let hello = read_envelope(&mut stream).expect("hello response");
    let session_id = hello.payload["session_id"]
        .as_str()
        .expect("session id")
        .to_owned();

    let open = Envelope::new(
        "open-project",
        MessageType::OpenProject,
        serde_json::json!({"path": project.0}),
    );
    write_envelope(&mut stream, &open).expect("open project");
    assert_eq!(
        read_envelope(&mut stream).expect("loading").message_type,
        MessageType::LifecycleStatus
    );
    let ready = read_envelope(&mut stream).expect("ready");
    assert_eq!(ready.message_type, MessageType::LifecycleStatus);
    let completed = read_envelope(&mut stream).expect("open completed");
    assert_eq!(completed.message_type, MessageType::CommandCompleted);

    let open_scene = Envelope::new(
        "open-scene",
        MessageType::OpenScene,
        serde_json::json!({"path": scene}),
    );
    write_envelope(&mut stream, &open_scene).expect("open scene");
    assert_eq!(
        read_envelope(&mut stream)
            .expect("scene loading")
            .message_type,
        MessageType::LifecycleStatus
    );
    assert_eq!(
        read_envelope(&mut stream)
            .expect("scene ready")
            .message_type,
        MessageType::LifecycleStatus
    );
    assert_eq!(
        read_envelope(&mut stream)
            .expect("scene completed")
            .message_type,
        MessageType::CommandCompleted
    );

    let save = Envelope::new("save", MessageType::SaveScene, serde_json::json!({}));
    write_envelope(&mut stream, &save).expect("save");
    assert_eq!(
        read_envelope(&mut stream)
            .expect("save status")
            .message_type,
        MessageType::LifecycleStatus
    );
    let saved = read_envelope(&mut stream).expect("save completed");
    assert_eq!(saved.payload["revision"], 1);
    drop(stream);

    let mut reconnect = TcpStream::connect(address).expect("reconnect");
    reconnect
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut hello = Envelope::hello("resume", "hyge-local-dev");
    hello.payload["session_id"] = serde_json::json!(session_id);
    write_envelope(&mut reconnect, &hello).expect("resume hello");
    assert_eq!(
        read_envelope(&mut reconnect)
            .expect("resume response")
            .payload["resumed"],
        true
    );
    write_envelope(
        &mut reconnect,
        &Envelope::new(
            "shutdown",
            MessageType::ServerShutdown,
            serde_json::json!({}),
        ),
    )
    .expect("shutdown");
    assert_eq!(
        read_envelope(&mut reconnect)
            .expect("shutdown response")
            .message_type,
        MessageType::ServerShutdown
    );
    thread.join().expect("server thread");
    let mut reopened = EditorSessionRuntime::new();
    reopened
        .open_project(&project.0)
        .expect("shutdown releases project lock");
}

#[cfg(windows)]
#[test]
fn server_shutdown_terminates_owned_frontend_child() {
    let server = EditorServer::bind(EditorServerConfig::default()).expect("bind");
    let address = server.local_addr().expect("address");
    let child = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"])
        .spawn()
        .expect("spawn frontend fixture");
    let child_pid = child.id().to_string();
    server.attach_frontend(child).expect("attach frontend");
    let thread = thread::spawn(move || server.run().expect("server run"));
    let mut stream = TcpStream::connect(address).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    write_envelope(
        &mut stream,
        &Envelope::hello("hello-child", "hyge-local-dev"),
    )
    .expect("hello");
    let _ = read_envelope(&mut stream).expect("hello response");
    write_envelope(
        &mut stream,
        &Envelope::new(
            "shutdown-child",
            MessageType::ServerShutdown,
            serde_json::json!({}),
        ),
    )
    .expect("shutdown");
    let _ = read_envelope(&mut stream).expect("shutdown response");
    thread.join().expect("server thread");
    thread::sleep(Duration::from_millis(100));
    let tasklist = Command::new("tasklist").output().expect("tasklist");
    let output = String::from_utf8_lossy(&tasklist.stdout);
    assert!(
        !output.contains(&child_pid),
        "frontend child {child_pid} is still running"
    );
}
