//! Tracing initialization and the engine log level.
//!
//! Call [`init_tracing`] once at startup (typically from `hyge-app`) to
//! install a `tracing-subscriber` with an `EnvFilter` driven by
//! `RUST_LOG` / `HYGE_LOG`. Calling it more than once is a no-op.

use std::sync::Once;

/// Tracks whether [`init_tracing`] has already been called. A `Once` is
/// used so that the subscriber is installed exactly once per process even
/// in the face of multiple `App` constructions (which happens in tests).
static INIT: Once = Once::new();

/// Initializes the global `tracing` subscriber.
///
/// The filter is read from the `RUST_LOG` env var (or `HYGE_LOG` if you
/// prefer to namespace the engine's logs). If neither is set, the default
/// is `info,hyge=debug` (i.e. quiet globally, but verbose for Hyge's own
/// crates).
///
/// Safe to call multiple times; only the first call has any effect.
///
/// # Example
///
/// ```no_run
/// use hyge_core::log::init_tracing;
///
/// init_tracing();
/// tracing::info!("engine booted");
/// ```
pub fn init_tracing() {
    INIT.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,hyge=debug"));

        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_thread_ids(false)
            .with_line_number(false)
            .with_file(false)
            .init();
    });
}
