//! Minimal Hyge example: open a window and render a hardcoded
//! triangle (R-024).
//!
//! Run with `cargo run --example hello_window`.
//!
//! The example creates a default [`AppConfig`] (with a configurable
//! title and a dark blue clear color), constructs an [`App`], and
//! runs the `winit` event loop. The App initializes the
//! [`hyge_render::Renderer`] on the first `resumed` event and
//! calls `render_triangle` every `about_to_wait`. The triangle
//! stays drawn until the user closes the window.

use hyge_app::prelude::*;
use hyge_window::prelude::WindowConfig;

fn main() {
    let config = AppConfig {
        window: WindowConfig {
            title: "Hyge Hello Window — first triangle".to_string(),
            width: 1024,
            height: 768,
            ..WindowConfig::default()
        },
        // Dark blue clear color; the red/green/blue triangle is
        // drawn on top of it.
        clear_color: [0.05, 0.10, 0.20, 1.0],
        ..AppConfig::default()
    };

    App::new(config).run();
}
