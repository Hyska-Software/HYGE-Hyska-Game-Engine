//! Hyge editor: the full visual editor built on `egui` + `egui_dock`.
//!
//! Panels: viewport (running the same render graph as runtime), hierarchy
//! (tree view + drag-drop reparent), inspector (reflect-driven component
//! editor), content browser (file tree + thumbnails), asset graph, console
//! (`tracing` ring buffer), profiler (frame stats sparkline), undo/redo.
//!
//! See `docs/architecture.md` §6.13 for the planned public surface.
//! Implementation is tracked in `docs/roadmap.toml` under R-080..R-089.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

/// Profiler panel placeholder used before the full editor lands.
pub mod profiler {
    use hyge_render::prelude::*;

    /// Minimal editor-profiler facade over the renderer's [`FrameStats`].
    pub struct ProfilerPanelPlaceholder;

    impl ProfilerPanelPlaceholder {
        /// Returns the stats that the future profiler panel will chart.
        #[must_use]
        pub fn frame_stats(stats: &FrameStats) -> &FrameStats {
            stats
        }

        /// Draws the R-025 debug overlay. The full editor profiler panel
        /// will replace this with a docked chart in R-08x.
        pub fn show_overlay(ctx: &egui::Context, stats: &FrameStats) {
            debug_overlay(ctx, stats);
        }
    }
}

/// Common editor exports.
pub mod prelude {
    /// Profiler panel placeholder export.
    pub use crate::profiler::ProfilerPanelPlaceholder;
}
