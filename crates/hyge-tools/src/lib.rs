//! Hyge tools: the engine CLI binary (`hyge-tools`).
//!
//! Subcommands:
//!
//! - [`cmd::import::run`] — cook a single source asset into the project cache.
//! - [`cmd::cook::run`] — cook every asset under a project's `assets/source/`
//!   tree, dispatching to `import` per file.
//! - [`cmd::headless`], [`cmd::inspect`], [`cmd::serve`], [`cmd::doctor`] —
//!   placeholders. They parse their arguments and return an "unsupported"
//!   error so the dispatcher is exercised end-to-end; full implementations
//!   land in later roadmap items (R-091, R-127, R-150, R-160).
//!
//! See `docs/architecture.md` §6.14 for the planned public surface and
//! `docs/roadmap.toml` R-033 for the skeleton milestone.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod cli;
pub mod cmd;

pub mod prelude;

use clap::Parser;

use hyge_core::result::HygeResult;

/// Runs the CLI from the process arguments (typically
/// [`std::env::args_os`]).
///
/// Parses the top-level [`Cli`](cli::Cli) with `clap`, then dispatches the
/// active [`Cmd`](cli::Cmd) variant to the corresponding
/// `cmd::<variant>::run` function. The binary entry point in `main.rs`
/// calls this and converts any [`HygeError`](hyge_core::result::HygeError)
/// into a non-zero exit code with a human-readable message.
///
/// # Errors
///
/// - Returns [`HygeError::Parse`](hyge_core::result::HygeError::Parse) when
///   `clap` rejects the argument vector (propagated from
///   `Cli::try_parse_from`).
/// - Returns the error produced by the dispatched subcommand.
pub fn run() -> HygeResult<()> {
    let cli = cli::Cli::parse();
    cli.cmd.dispatch()
}
