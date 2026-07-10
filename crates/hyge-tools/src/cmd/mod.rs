//! `hyge-tools` subcommand implementations.
//!
//! Each variant of [`crate::cli::Cmd`] maps to exactly one module below.
//! Only [`import`] and [`cook`] are real implementations in this phase
//! (R-033); [`headless`], [`inspect`], [`serve`], and [`doctor`] are
//! stubs that return
//! [`hyge_core::result::HygeError::Unsupported`] and are filled in by
//! later roadmap items (R-091, R-127, R-150, R-160).

pub mod cook;
pub mod doctor;
pub mod editor;
pub mod headless;
pub mod import;
pub mod inspect;
pub mod serve;

/// Canonical Hyge asset source directory under a project root.
///
/// `hyge-tools cook` walks this directory (recursively) and dispatches
/// [`import::run`] per file. The constant lives here so every other
/// module can reference the same path without re-typing the literal.
pub const ASSETS_SOURCE_DIR: &str = "assets/source";

/// Canonical Hyge cooked-asset cache directory under a project root.
pub const ASSETS_COOK_DIR: &str = "assets/cook";
