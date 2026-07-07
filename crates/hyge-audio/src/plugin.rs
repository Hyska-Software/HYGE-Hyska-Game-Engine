//! Audio plugin registration.

use bevy_app::App;
use hyge_ecs::prelude::{AudioSet, HygePlugin, IntoSystemConfigs, Label};

use crate::{AudioServer, PlaySound, StopSound};

/// Registers audio resources, events, and placeholder systems.
#[derive(Clone, Copy, Debug, Default)]
pub struct AudioPlugin;

impl HygePlugin for AudioPlugin {
    fn name(&self) -> &'static str {
        "hyge-audio"
    }

    fn build(&self, app: &mut App) {
        app.init_resource::<AudioServer>()
            .add_event::<PlaySound>()
            .add_event::<StopSound>()
            .add_systems(Label::Update, audio_update_system.in_set(AudioSet::Update))
            .add_systems(Label::Update, audio_event_system.in_set(AudioSet::Events));
    }
}

/// Updates listener and emitter state.
pub fn audio_update_system() {}

/// Processes play/stop events.
pub fn audio_event_system() {}

#[cfg(test)]
mod tests {
    use bevy_app::App;
    use hyge_ecs::prelude::AppHygeExt;

    use super::*;

    #[test]
    fn plugin_registers_server_and_events() {
        let mut app = App::new();
        app.add_hyge_plugin(AudioPlugin);
        assert!(app.world().get_resource::<AudioServer>().is_some());
    }
}
