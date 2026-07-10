//! Rust editor service for the Hyge Qt frontend.
//!
//! The service owns editor session metadata and the engine/editor boundary.
//! The PySide6 process is a presentation client; it never receives a direct
//! pointer or ABI handle to the ECS world.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth;
mod server;
mod state;

pub use server::{EditorServer, EditorServerConfig};
pub use state::EditorState;

/// Common editor exports.
pub mod prelude {
    pub use crate::{EditorServer, EditorServerConfig, EditorState};
}
