//! Rust editor service for the Hyge Qt frontend.
//!
//! The service owns editor session metadata and the engine/editor boundary.
//! The PySide6 process is a presentation client; it never receives a direct
//! pointer or ABI handle to the ECS world.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth;
mod lifecycle;
mod lock;
mod project;
mod server;
mod snapshots;
mod state;

pub use lifecycle::{EditorSessionRuntime, LifecycleSnapshot, LifecycleState};
pub use server::{EditorServer, EditorServerConfig};
pub use snapshots::{
    build_snapshot, ComponentDescriptor, EditorSnapshot, EntityId, EntitySnapshot, FieldDescriptor,
    HierarchyNode, ReflectedComponent, SnapshotDiagnostic,
};
pub use state::{EditorState, SessionSnapshot};

/// Common editor exports.
pub mod prelude {
    pub use crate::{
        ComponentDescriptor, EditorServer, EditorServerConfig, EditorSnapshot, EditorState,
        EntityId, EntitySnapshot, FieldDescriptor, HierarchyNode, ReflectedComponent,
        SnapshotDiagnostic,
    };
}
