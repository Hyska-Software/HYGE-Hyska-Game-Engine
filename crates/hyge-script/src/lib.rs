//! Sandboxed Lua scripting for Hyge.
//!
//! Scripts execute through [`ScriptEngine`] and interact with the ECS through
//! the `hyge.*` API. Component serialization and mutation are driven by the
//! same `bevy_reflect` registry used by scene and editor code.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod api;
pub mod components;
pub mod engine;
pub mod events;
pub mod plugin;
pub mod prelude;
pub mod reflect_bind;
pub mod sandbox;

pub use engine::ScriptEngine;
