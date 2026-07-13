//! `hyge-tools editor` — start the Rust editor service and optional PySide6 UI.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use hyge_core::result::HygeResult;
use hyge_editor::{EditorServer, EditorServerConfig};

/// Starts the loopback editor backend. The backend remains the owner of the
/// engine state; the optional Python process is only a protocol client.
pub fn run(
    project: &Path,
    port: u16,
    frontend: Option<&Path>,
    scene: Option<&Path>,
    evidence_dir: Option<&Path>,
    external_scene: Option<&Path>,
) -> HygeResult<()> {
    let project = validate_project(project)?;
    let scene = scene
        .map(|scene| validate_scene(&project, scene))
        .transpose()?;
    let external_scene = external_scene
        .map(|scene| validate_scene(&project, scene))
        .transpose()?;
    let frontend = frontend.map(validate_frontend).transpose()?;
    let token = editor_token();
    let server = EditorServer::bind(EditorServerConfig {
        bind_address: format!("127.0.0.1:{port}"),
        session_token: token.clone(),
        ..EditorServerConfig::default()
    })?;
    let address = server.local_addr()?;
    tracing::info!(%address, project = %project.display(), "hyge editor service listening");
    println!("HYGE_EDITOR_ADDRESS={address}");

    let child = frontend
        .map(|path| {
            spawn_frontend(
                path.as_path(),
                &project,
                scene.as_deref(),
                evidence_dir,
                external_scene.as_deref(),
                &address.to_string(),
                &token,
            )
        })
        .transpose()?;
    if let Some(child) = child {
        server.attach_frontend(child)?;
    }
    server.run().map_err(hyge_core::result::HygeError::from)
}

fn validate_scene(project: &Path, scene: &Path) -> HygeResult<PathBuf> {
    let candidate = if scene.is_absolute() {
        scene.to_path_buf()
    } else {
        project.join(scene)
    };
    let canonical = candidate.canonicalize()?;
    if !canonical.starts_with(project)
        || canonical
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("hyge-world")
    {
        return Err(hyge_core::result::HygeError::invalid_argument(format!(
            "editor scene must be a .hyge-world below the project root: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
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

fn validate_frontend(path: &Path) -> HygeResult<PathBuf> {
    let canonical = path.canonicalize().map_err(|error| {
        hyge_core::result::HygeError::invalid_argument(format!(
            "editor frontend does not exist: {} ({error})",
            path.display()
        ))
    })?;
    if !canonical.is_file() {
        return Err(hyge_core::result::HygeError::invalid_argument(format!(
            "editor frontend is not a file: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn frontend_invocation(path: &Path) -> (OsString, Vec<OsString>) {
    let is_executable = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
    if is_executable {
        (path.as_os_str().to_owned(), Vec::new())
    } else {
        (OsString::from("python"), vec![path.as_os_str().to_owned()])
    }
}

fn spawn_frontend(
    path: &Path,
    project: &Path,
    scene: Option<&Path>,
    evidence_dir: Option<&Path>,
    external_scene: Option<&Path>,
    address: &str,
    token: &str,
) -> std::io::Result<Child> {
    let (program, args) = frontend_invocation(path);
    let mut command = Command::new(program);
    command
        .args(args)
        .env("HYGE_EDITOR_ADDRESS", address)
        .env("HYGE_EDITOR_TOKEN", token)
        .env("HYGE_PROJECT", project);
    if let Some(scene) = scene {
        command.env("HYGE_SCENE", scene);
    }
    if let Some(evidence_dir) = evidence_dir {
        command.env("HYGE_EDITOR_EVIDENCE_DIR", evidence_dir);
    }
    if let Some(external_scene) = external_scene {
        command
            .env("HYGE_EDITOR_E2E", "1")
            .env("HYGE_EDITOR_EXTERNAL_SCENE", external_scene);
    }
    command.spawn()
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

    #[test]
    fn validates_scene_below_project() {
        let root = std::env::temp_dir().join(format!("hyge-editor-scene-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create temp project");
        let scene = root.join("main.hyge-world");
        fs::write(&scene, b"scene").expect("write scene");
        let canonical_root = root.canonicalize().expect("canonical root");
        assert_eq!(
            validate_scene(&canonical_root, Path::new("main.hyge-world")).expect("scene"),
            scene.canonicalize().expect("canonical scene")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn packaged_executable_is_launched_directly() {
        let (program, args) = frontend_invocation(Path::new("package/HygeEditor.exe"));
        assert_eq!(program, OsString::from("package/HygeEditor.exe"));
        assert!(args.is_empty());
    }

    #[test]
    fn source_frontend_keeps_python_compatibility() {
        let (program, args) = frontend_invocation(Path::new("tools/main.py"));
        assert_eq!(program, OsString::from("python"));
        assert_eq!(args, vec![OsString::from("tools/main.py")]);
    }

    #[test]
    fn validates_frontend_files() {
        let root =
            std::env::temp_dir().join(format!("hyge-editor-frontend-{}", std::process::id()));
        fs::create_dir_all(&root).expect("create frontend directory");
        let file = root.join("HygeEditor.exe");
        fs::write(&file, b"fixture").expect("write frontend fixture");
        assert_eq!(
            validate_frontend(&file).expect("frontend"),
            file.canonicalize().expect("canonical")
        );
        assert!(validate_frontend(&root.join("missing.exe")).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
