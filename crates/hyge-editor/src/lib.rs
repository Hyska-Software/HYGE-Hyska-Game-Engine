//! Rust editor service for the Hyge Qt frontend.
//!
//! The service owns editor session metadata and the engine/editor boundary.
//! The PySide6 process is a presentation client; it never receives a direct
//! pointer or ABI handle to the ECS world.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod auth;
mod commands;
mod data;
mod history;
mod lifecycle;
mod lock;
mod project;
mod server;
mod snapshots;
mod state;
mod transport;
mod viewport;

pub use commands::{
    AddComponentCommand, Command, CommandEffect, CommandFailure, DestroyCommand, DuplicateCommand,
    EditComponentCommand, EditComponentsCommand, EditorCommand, InstantiateCommand,
    RemoveComponentCommand, ReparentCommand,
};
pub use data::{
    AssetDependencyEdge, AssetNode, AssetSnapshot, ConsoleBuffer, ConsoleFilter, ConsoleLayer,
    ConsoleLine, ConsoleSnapshot, EditorDataServices, PreviewManager, PreviewResult, ProfilerPass,
    ProfilerSample, ProfilerSnapshot, ProfilerStore,
};
pub use history::CommandHistory;
pub use lifecycle::{EditorSessionRuntime, LifecycleSnapshot, LifecycleState};
pub use server::{EditorServer, EditorServerConfig};
pub use snapshots::{
    build_snapshot, ComponentDescriptor, EditorSnapshot, EntityId, EntitySnapshot, FieldDescriptor,
    HierarchyNode, ReflectedComponent, SnapshotDiagnostic,
};
pub use state::{EditorState, SessionSnapshot};
pub use transport::{
    CameraCommand, FrameHeader, InputBridge, SharedViewportTransport, TransportState,
    ViewportInput, ViewportInputBatch, ViewportRing,
};
pub use viewport::{EditorCameraState, EditorRenderBridge, ViewportRenderState, ViewportState};

/// Common editor exports.
pub mod prelude {
    pub use crate::{
        AddComponentCommand, Command, CommandEffect, CommandFailure, CommandHistory,
        ComponentDescriptor, DestroyCommand, DuplicateCommand, EditComponentCommand,
        EditComponentsCommand, EditorCameraState, EditorCommand, EditorRenderBridge, EditorServer,
        EditorServerConfig, EditorSnapshot, EditorState, EntityId, EntitySnapshot, FieldDescriptor,
        HierarchyNode, ReflectedComponent, RemoveComponentCommand, ReparentCommand,
        SnapshotDiagnostic, ViewportRenderState, ViewportState,
    };
}
