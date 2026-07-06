//! The [`ScenePlugin`] registers the canonical scene component catalog and
//! the transform propagation / render-extract systems.

use bevy_app::App;
use hyge_ecs::prelude::*;

use crate::extract::{add_render_extract_system, FrameSnapshot};
use crate::transform::{hierarchy_cleanup_system, transform_propagate_system};

/// Hyge scene plugin.
///
/// Registers:
/// - the [`FrameSnapshot`] resource;
/// - the transform propagation system in [`TransformSet::Propagate`];
/// - a lightweight hierarchy cleanup system;
/// - the render-extract system in [`Label::RenderExtract`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ScenePlugin;

impl HygePlugin for ScenePlugin {
    fn name(&self) -> &'static str {
        "hyge-scene"
    }

    fn build(&self, app: &mut App) {
        tracing::debug!("building hyge-scene plugin");

        // Resource consumed by the renderer and produced by extract.
        app.init_resource::<FrameSnapshot>();

        // Transform propagation runs during the variable update so gameplay
        // systems can reparent entities before the render extract.
        app.add_systems(
            Label::Update,
            transform_propagate_system.in_set(TransformSet::Propagate),
        );

        // Light hierarchy maintenance; kept in the same set for ordering.
        app.add_systems(
            Label::Update,
            hierarchy_cleanup_system.in_set(TransformSet::Flush),
        );

        // Render extract produces the per-frame snapshot.
        let mut render_extract_schedule = Schedule::new(Label::RenderExtract);
        add_render_extract_system(&mut render_extract_schedule);
        app.add_schedule(render_extract_schedule);
    }
}
