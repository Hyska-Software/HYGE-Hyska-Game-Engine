//! Audio plugin registration.
//!
//! Registers the [`AudioServer`], events, and spatial audio systems that
//! synchronise the ECS [`AudioListener`] and [`AudioSource`] components with
//! the Kira backend.

use bevy_app::App;
use bevy_ecs::prelude::*;
use hyge_core::prelude::Vec3;
use hyge_ecs::prelude::{AudioSet, HygePlugin, IntoSystemConfigs, Label};

use crate::components::{AudioListener, AudioSource};
use crate::events::{PlaySound, StopSound};
use crate::server::AudioServer;
use crate::spatial::SpatialListener;

/// Registers audio resources, events, and spatial systems.
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
            .add_systems(
                Label::Update,
                spatial_listener_sync_system.in_set(AudioSet::Update),
            )
            .add_systems(
                Label::Update,
                spatial_emitter_sync_system.in_set(AudioSet::Update),
            )
            .add_systems(Label::Update, audio_event_system.in_set(AudioSet::Events));
    }
}

/// Syncs the first [`AudioListener`] found in the ECS to the Kira listener.
pub fn spatial_listener_sync_system(
    listeners: Query<&AudioListener>,
    mut server: ResMut<AudioServer>,
) {
    let Some(listener) = listeners.iter().next() else {
        return;
    };
    let spatial = SpatialListener {
        position: Vec3::from(listener.position),
        forward: Vec3::from(listener.forward),
        up: Vec3::from(listener.up),
    };
    server.set_listener(spatial);
}

/// Manages spatial emitters: creates Kira spatial sub tracks for entities
/// with [`AudioSource`] that have `spatial: true`, and updates their positions
/// when [`AudioListener`] positions change.
pub fn spatial_emitter_sync_system(
    sources: Query<(Entity, &AudioSource)>,
    listeners: Query<&AudioListener>,
    mut server: ResMut<AudioServer>,
) {
    let listener = listeners.iter().next();
    let lid = server.listener_id();

    for (entity, source) in sources.iter() {
        if !source.spatial {
            continue;
        }
        let position = if let Some(listener) = listener {
            Vec3::from(listener.position)
        } else {
            Vec3::ZERO
        };
        let emitter_pos = position + Vec3::new(source.range * 0.5, 0.0, 0.0);
        server.add_spatial_emitter(entity, emitter_pos, lid);
    }
}

/// Processes play/stop events.
pub fn audio_event_system(
    mut play_events: EventReader<PlaySound>,
    mut _stop_events: EventReader<StopSound>,
    mut server: ResMut<AudioServer>,
) {
    for event in play_events.read() {
        server.record_play_request(event.source);
    }
}

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

    #[test]
    fn spatial_systems_run_without_backend() {
        let mut app = App::new();
        app.init_resource::<AudioServer>();
        app.add_systems(
            Label::Update,
            spatial_listener_sync_system.in_set(AudioSet::Update),
        );
        app.add_systems(
            Label::Update,
            spatial_emitter_sync_system.in_set(AudioSet::Update),
        );
        app.world_mut().spawn(AudioListener {
            position: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
        });
        app.world_mut().spawn(AudioSource {
            spatial: true,
            range: 25.0,
            ..AudioSource::default()
        });
        app.world_mut().run_schedule(Label::Update);
    }
}
