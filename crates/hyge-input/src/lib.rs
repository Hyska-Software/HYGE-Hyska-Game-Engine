//! Hyge's device-independent action input layer.
//!
//! The crate owns TOML bindings, action state, edge detection, hot reload,
//! and conversion of `hyge-window` device events into gameplay actions.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod action;
pub mod binding;
pub mod hot_reload;
pub mod plugin;
pub mod prelude;
pub mod translate;
