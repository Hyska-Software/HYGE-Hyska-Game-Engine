//! `hyge-tools editor` — start the Rust editor service and optional PySide6 UI.

use std::path::Path;
use std::process::{Child, Command};

use hyge_core::result::HygeResult;
use hyge_editor::{EditorServer, EditorServerConfig};

/// Starts the loopback editor backend. The backend remains the owner of the
/// engine state; the optional Python process is only a protocol client.
pub fn run(project: &Path, port: u16, frontend: Option<&Path>) -> HygeResult<()> {
    let project = project.canonicalize()?;
    let token = std::env::var("HYGE_EDITOR_TOKEN").unwrap_or_else(|_| "hyge-local-dev".into());
    let server = EditorServer::bind(EditorServerConfig {
        bind_address: format!("127.0.0.1:{port}"),
        session_token: token.clone(),
    })?;
    let address = server.local_addr()?;
    tracing::info!(%address, project = %project.display(), "hyge editor service listening");
    println!("HYGE_EDITOR_ADDRESS={address}");

    let mut child = frontend
        .map(|path| spawn_frontend(path, &project, &address.to_string(), &token))
        .transpose()?;
    let result = server.run().map_err(hyge_core::result::HygeError::from);
    if let Some(child) = child.as_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn spawn_frontend(
    path: &Path,
    project: &Path,
    address: &str,
    token: &str,
) -> std::io::Result<Child> {
    Command::new("python")
        .arg(path)
        .env("HYGE_EDITOR_ADDRESS", address)
        .env("HYGE_EDITOR_TOKEN", token)
        .env("HYGE_PROJECT", project)
        .spawn()
}
