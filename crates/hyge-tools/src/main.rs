//! `hyge-tools` binary entry point.
//!
//! Thin shim over [`hyge_tools::run`]: initializes logging, runs the CLI, and
//! converts any [`HygeError`](hyge_core::result::HygeError) into a non-zero
//! exit code with a human-readable message on stderr.

use std::process::ExitCode;

use hyge_core::log::init_tracing;
use hyge_core::result::HygeError;

fn main() -> ExitCode {
    init_tracing();

    match hyge_tools::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err);
            ExitCode::FAILURE
        }
    }
}

fn report_error(err: &HygeError) {
    tracing::error!(error = %err, "hyge-tools failed");
    eprintln!("error: {err}");
}
