//! Minimal Hyge example: open a window with a clear color.
//!
//! Run with `cargo run --example hello_window`.
//!
//! The example creates a default [`AppConfig`] (with a configurable title
//! and a dark gray clear color), constructs an [`App`], and runs the
//! `winit` event loop. The window stays open until the user closes it.

use hyge_app::prelude::*;
use hyge_window::prelude::WindowConfig;

fn main() {
    let config = AppConfig {
        window: WindowConfig {
            title: "Hyge Hello Window".to_string(),
            width: 1024,
            height: 768,
            ..WindowConfig::default()
        },
        clear_color: [0.1, 0.2, 0.3, 1.0],
        ..AppConfig::default()
    };

    App::new(config).run();
}
