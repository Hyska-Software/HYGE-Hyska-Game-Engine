//! Top-level CLI definition for `hyge-tools`.
//!
//! `Cli` is the `clap`-derived argument parser. `Cmd` is the `Subcommand`
//! enum that names every subcommand required by the engine's CLI surface
//! (see `docs/architecture.md` §6.14). All six variants parse and dispatch;
//! only `Import` and `Cook` are *implemented* in this phase (R-033). The
//! other four return a clear "not yet implemented"
//! [`hyge_core::result::HygeError`] so the
//! dispatcher is exercised end-to-end today and the gap is visible in
//! `--help` and in the runtime message.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use hyge_core::result::HygeResult;

use crate::cmd;

/// Top-level `hyge-tools` command line.
///
/// Parsed from `std::env::args_os()` by [`crate::run`]. `clap` derives the
/// `--help` / `--version` flags and the subcommand dispatch table from the
/// [`Cmd`] enum embedded below.
#[derive(Debug, Parser)]
#[command(
    name = "hyge-tools",
    version,
    about = "Hyge engine CLI: cook assets, render headless, inspect hashes, serve projects, diagnose projects.",
    long_about = None
)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub cmd: Cmd,
}

/// Every subcommand the `hyge-tools` binary understands.
///
/// The full set is enumerated here so the `--help` output, the dispatch
/// table, and the architecture sketch (`docs/architecture.md` §6.14) stay
/// in lockstep. Only [`Cmd::Import`] and [`Cmd::Cook`] are wired to real
/// implementations in this phase (R-033); the other variants parse cleanly
/// and return [`hyge_core::result::HygeError::Unsupported`] from their `cmd/<name>::run`
/// shims, so the dispatcher is still exercised end-to-end.
#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Cook a single source asset into the project cache.
    ///
    /// Computes the BLAKE3 content hash of `path` and stages a sidecar
    /// manifest under `<out>/<hash>.source-path` recording the source
    /// location. Detailed format conversion (glTF → `.hyge-mesh`,
    /// textures → `.ktx2`, meshlet bake) is implemented by R-034..R-037.
    Import {
        /// Path to the source asset on disk (e.g. a `.gltf` / `.glb`).
        path: PathBuf,

        /// Project-relative output directory for cooked assets.
        #[arg(long, short)]
        out: PathBuf,
    },

    /// Cook every source asset under a project's `assets/source/` tree.
    ///
    /// Walks the project, dispatches [`Cmd::Import`] per file, and
    /// reports a per-extension summary. Format-specific pipelines (mesh
    /// baking, KTX2 transcode, meshopt) are layered in by R-034..R-037.
    Cook {
        /// Path to the project root.
        project: PathBuf,

        /// Optional override for the cooked output directory. When
        /// omitted, defaults to `<project>/assets/cook/`.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },

    /// Render a cooked scene to a PNG without opening a window.
    ///
    /// Stub for now. The headless render path ships in a later roadmap
    /// item (see `docs/roadmap.toml` R-127).
    Headless {
        /// Path to a `.hyge-world` cooked scene.
        #[arg(long)]
        scene: PathBuf,

        /// Inline camera spec: `perspective fov=60 pos=0,5,-10 look=0,1,0`.
        #[arg(long)]
        camera: String,

        /// Output PNG path.
        #[arg(long, short)]
        out: PathBuf,

        /// Render width in pixels.
        #[arg(long, default_value_t = 1920)]
        width: u32,

        /// Render height in pixels.
        #[arg(long, default_value_t = 1080)]
        height: u32,

        /// Sample count (MSAA).
        #[arg(long, default_value_t = 1)]
        samples: u32,
    },

    /// Print metadata for a single asset hash.
    ///
    /// Stub for now. Asset inspection ships in a later roadmap item.
    Inspect {
        /// 32-byte BLAKE3 hash of the asset to inspect.
        hash: String,
    },

    /// Serve a project over HTTP for the editor.
    ///
    /// Stub for now. The asset server ships in a later roadmap item
    /// (R-150).
    Serve {
        /// Path to the project root to serve.
        project: PathBuf,

        /// TCP port to bind.
        #[arg(long, short, default_value_t = 7777)]
        port: u16,
    },

    /// Diagnose a project: missing assets, orphan cache files, schema
    /// version mismatches.
    ///
    /// Stub for now. The full diagnostic suite ships in a later
    /// roadmap item (R-160).
    Doctor {
        /// Path to the project root to diagnose.
        project: PathBuf,
    },
}

impl Cmd {
    /// Dispatches the active subcommand to its `cmd/*::run` implementation.
    ///
    /// Every variant maps to exactly one function, and every function
    /// returns the same [`HygeResult<()>`] shape so the dispatcher stays
    /// trivial. Unimplemented subcommands return
    /// [`hyge_core::result::HygeError::Unsupported`] with a phase reference so the user gets
    /// a useful message instead of a panic.
    ///
    /// # Errors
    ///
    /// - [`hyge_core::result::HygeError::Unsupported`] for `headless`, `inspect`, `serve`,
    ///   and `doctor` (deferred to a later roadmap item).
    /// - Any error produced by the dispatched `cmd/<name>::run`
    ///   implementation, propagated via `?`.
    pub fn dispatch(&self) -> HygeResult<()> {
        match self {
            Self::Import { path, out } => cmd::import::run(path, out).map(|_| ()),
            Self::Cook { project, out } => cmd::cook::run(project, out.as_deref()).map(|_| ()),
            Self::Headless {
                scene,
                camera,
                out,
                width,
                height,
                samples,
            } => cmd::headless::run(scene, camera, out, *width, *height, *samples),
            Self::Inspect { hash } => cmd::inspect::run(hash),
            Self::Serve { project, port } => cmd::serve::run(project, *port),
            Self::Doctor { project } => cmd::doctor::run(project),
        }
    }
}

#[cfg(test)]
mod tests {
    use hyge_core::result::HygeError;

    use super::*;

    fn parse(argv: &[&str]) -> clap::error::Result<Cli> {
        Cli::try_parse_from(argv)
    }

    #[test]
    fn parses_top_level_help() {
        assert!(parse(&["hyge-tools", "--help"]).is_err());
    }

    #[test]
    fn parses_import_subcommand() {
        let cli = parse(&[
            "hyge-tools",
            "import",
            "assets/source/cube.gltf",
            "--out",
            "assets/cook/",
        ])
        .expect("import must parse");
        match cli.cmd {
            Cmd::Import { path, out } => {
                assert_eq!(path, PathBuf::from("assets/source/cube.gltf"));
                assert_eq!(out, PathBuf::from("assets/cook/"));
            }
            other => panic!("expected Import, got {other:?}"),
        }
    }

    #[test]
    fn parses_cook_subcommand_with_and_without_out() {
        let cli = parse(&["hyge-tools", "cook", "path/to/project"]).expect("cook must parse");
        match cli.cmd {
            Cmd::Cook { project, out } => {
                assert_eq!(project, PathBuf::from("path/to/project"));
                assert!(out.is_none());
            }
            other => panic!("expected Cook, got {other:?}"),
        }

        let cli = parse(&[
            "hyge-tools",
            "cook",
            "path/to/project",
            "--out",
            "custom/cook",
        ])
        .expect("cook --out must parse");
        match cli.cmd {
            Cmd::Cook { project, out } => {
                assert_eq!(project, PathBuf::from("path/to/project"));
                assert_eq!(out, Some(PathBuf::from("custom/cook")));
            }
            other => panic!("expected Cook, got {other:?}"),
        }
    }

    #[test]
    fn parses_headless_subcommand_with_defaults() {
        let cli = parse(&[
            "hyge-tools",
            "headless",
            "--scene",
            "assets/cook/sandbox.hyge-world",
            "--camera",
            "perspective fov=60 pos=0,5,-10 look=0,1,0",
            "--out",
            "render.png",
        ])
        .expect("headless must parse");
        match cli.cmd {
            Cmd::Headless {
                scene,
                camera,
                out,
                width,
                height,
                samples,
            } => {
                assert_eq!(scene, PathBuf::from("assets/cook/sandbox.hyge-world"));
                assert!(camera.starts_with("perspective"));
                assert_eq!(out, PathBuf::from("render.png"));
                assert_eq!(width, 1920);
                assert_eq!(height, 1080);
                assert_eq!(samples, 1);
            }
            other => panic!("expected Headless, got {other:?}"),
        }
    }

    #[test]
    fn parses_inspect_subcommand() {
        let cli = parse(&["hyge-tools", "inspect", "abcd1234ef"]).expect("inspect must parse");
        match cli.cmd {
            Cmd::Inspect { hash } => assert_eq!(hash, "abcd1234ef"),
            other => panic!("expected Inspect, got {other:?}"),
        }
    }

    #[test]
    fn parses_serve_subcommand_with_default_port() {
        let cli = parse(&["hyge-tools", "serve", "path/to/project"]).expect("serve must parse");
        match cli.cmd {
            Cmd::Serve { project, port } => {
                assert_eq!(project, PathBuf::from("path/to/project"));
                assert_eq!(port, 7777);
            }
            other => panic!("expected Serve, got {other:?}"),
        }
    }

    #[test]
    fn parses_doctor_subcommand() {
        let cli = parse(&["hyge-tools", "doctor", "path/to/project"]).expect("doctor must parse");
        match cli.cmd {
            Cmd::Doctor { project } => assert_eq!(project, PathBuf::from("path/to/project")),
            other => panic!("expected Doctor, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_returns_unsupported_for_stub_subcommands() {
        let cli = parse(&["hyge-tools", "inspect", "abcd1234ef"]).expect("inspect must parse");
        let err = cli.cmd.dispatch().expect_err("inspect must be a stub");
        assert!(matches!(err, HygeError::Unsupported(_)), "got {err:?}");

        let cli = parse(&["hyge-tools", "doctor", "p"]).expect("doctor must parse");
        let err = cli.cmd.dispatch().expect_err("doctor must be a stub");
        assert!(matches!(err, HygeError::Unsupported(_)), "got {err:?}");

        let cli = parse(&["hyge-tools", "serve", "p", "--port", "9000"]).expect("serve must parse");
        let err = cli.cmd.dispatch().expect_err("serve must be a stub");
        assert!(matches!(err, HygeError::Unsupported(_)), "got {err:?}");
    }
}
