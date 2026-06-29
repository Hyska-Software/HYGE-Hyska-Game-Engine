//! M0 smoke test: `App` boots and is structurally complete.
//!
//! This integration test verifies the structural invariants of the M0
//! Definition of Done (R-014). The *behavioral* test — that a window
//! actually opens and renders the clear color — is performed by:
//!   1. The `hello_window` example (`cargo run --example hello_window`).
//!   2. The headless-render CI job in `.github/workflows/ci.yml` which
//!      validates the `hyge-runtime-test` integration test (a real
//!      snapshot test that lands in M2+).
//!
//! This file is the structural gate: if it does not pass, the workspace
//! cannot be built, and the behavioral tests are moot.

use hyge_app::prelude::*;
use hyge_ecs::schedule::Label;
use hyge_window::{WindowConfig, WindowState};

/// `App::new(AppConfig::default())` must succeed without panicking, install
/// the default plugins, and register the 7 canonical schedules.
#[test]
fn m0_app_boots_with_default_config() {
    let app = App::new(AppConfig::default());

    // The clear color defaults to dark gray, the title to "Hyge Engine".
    assert_eq!(app.config().clear_color, [0.1, 0.1, 0.1, 1.0]);
    assert_eq!(app.config().window.title, "Hyge Engine");

    // Default plugins installed the window subsystem; the WindowConfig
    // and WindowState resources must be present.
    let world = app.bevy_app().world();
    assert!(
        world.get_resource::<WindowConfig>().is_some(),
        "WindowConfig resource must be present after App::new"
    );
    assert!(
        world.get_resource::<WindowState>().is_some(),
        "WindowState resource must be present after App::new"
    );

    // The 7 canonical schedules are registered. Running each one on a
    // fresh App must be a no-op (no systems added yet) and must not
    // panic.
    for label in [
        Label::First,
        Label::PreUpdate,
        Label::FixedUpdate,
        Label::Update,
        Label::RenderExtract,
        Label::Render,
        Label::Last,
    ] {
        let mut app = App::new(AppConfig::default());
        app.bevy_app_mut().world_mut().run_schedule(label);
    }
}

/// The clear color is configurable via `AppConfig::clear_color`. Verify
/// that a custom value flows through to the App's stored config.
#[test]
fn m0_app_accepts_custom_clear_color() {
    let config = AppConfig {
        clear_color: [0.5, 0.5, 0.5, 1.0],
        ..AppConfig::default()
    };
    let app = App::new(config);
    assert_eq!(app.config().clear_color, [0.5, 0.5, 0.5, 1.0]);
}

/// The window title is configurable via `AppConfig::window.title`. Verify
/// that a custom value flows through to the App's stored config and
/// to the `WindowConfig` resource that the window plugin registered.
#[test]
fn m0_app_accepts_custom_window_title() {
    let config = AppConfig {
        window: WindowConfig {
            title: "My Game".to_string(),
            ..WindowConfig::default()
        },
        ..AppConfig::default()
    };
    let app = App::new(config);
    assert_eq!(app.config().window.title, "My Game");

    let world = app.bevy_app().world();
    let stored = world
        .get_resource::<WindowConfig>()
        .expect("WindowConfig resource must be present");
    assert_eq!(stored.title, "My Game");
}

/// The `AppBuilder` trait is implemented for `App`, so the trait can be
/// used to abstract over the engine entry point.
#[test]
fn m0_app_satisfies_app_builder_trait() {
    fn assert_app_builder<T: AppBuilder + ?Sized>(_: T) {}
    assert_app_builder(App::new(AppConfig::default()));
}

/// The M0 DoD includes a working example. The `hello_window` example
/// builds and runs (verified by CI); this test guards against the
/// example file being deleted or its path being moved.
#[test]
fn m0_hello_window_example_exists() {
    // The example is declared in `crates/hyge-app/Cargo.toml`. We
    // verify the source file exists on disk at the expected path; if
    // it is ever moved or renamed, this test will fail and force an
    // update to the CI workflow that builds examples.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let example_path = std::path::Path::new(manifest_dir)
        .join("examples")
        .join("hello_window.rs");
    assert!(
        example_path.exists(),
        "M0 example must exist at {}",
        example_path.display()
    );
}
