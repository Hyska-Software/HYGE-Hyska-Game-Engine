//! `hyge-tools editor` — start the Rust editor service and optional PySide6 UI.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use hyge_core::result::HygeResult;
use hyge_editor::{EditorServer, EditorServerConfig};

/// Starts the loopback editor backend. The backend remains the owner of the
/// engine state; the optional Python process is only a protocol client.
pub fn run(project: &Path, port: u16, frontend: Option<&Path>) -> HygeResult<()> {
    let project = validate_project(project)?;
    let token = editor_token();
    let server = EditorServer::bind(EditorServerConfig {
        bind_address: format!("127.0.0.1:{port}"),
        session_token: token.clone(),
        ..EditorServerConfig::default()
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

fn validate_project(project: &Path) -> HygeResult<PathBuf> {
    let canonical = project.canonicalize()?;
    if !canonical.is_dir() {
        return Err(hyge_core::result::HygeError::invalid_argument(format!(
            "editor project is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn editor_token() -> String {
    if let Ok(token) = std::env::var("HYGE_EDITOR_TOKEN") {
        if !token.is_empty() {
            return token;
        }
    }
    let mut random = [0_u8; 32];
    if getrandom::getrandom(&mut random).is_ok() {
        return blake3::hash(&random).to_hex().to_string();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let material = format!("{now}:{}:{:p}", std::process::id(), &now);
    blake3::hash(material.as_bytes()).to_hex().to_string()
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn validates_existing_directories_only() {
        let root = std::env::temp_dir().join(format!("hyge-editor-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create temp project");
        assert_eq!(
            validate_project(&root).expect("directory"),
            root.canonicalize().expect("canonical")
        );
        let file = root.join("project.txt");
        fs::write(&file, b"project").expect("write file");
        assert!(validate_project(&file).is_err());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn generated_tokens_are_nonempty() {
        assert!(!editor_token().is_empty());
    }
}
