//! ECS plugin and gamepad capture.

use crate::{
    action::ActionMap,
    binding::load_file,
    hot_reload::BindingWatcher,
    translate::{flush_system, translate_system},
};
use gilrs::Gilrs;
use hyge_ecs::prelude::*;
use std::path::PathBuf;

/// Runtime input configuration.
#[derive(Clone, Debug)]
pub struct InputConfig {
    /// Optional binding file.
    pub binding_file: Option<PathBuf>,
    /// Enables binding hot reload.
    pub enable_hot_reload: bool,
}
impl Default for InputConfig {
    fn default() -> Self {
        Self {
            binding_file: None,
            enable_hot_reload: true,
        }
    }
}

/// Optional gamepad polling resource.
pub struct GamepadInput {
    pub(crate) gilrs: Option<Gilrs>,
}

/// Internal binding reload state.
#[derive(Default)]
pub struct InputHotReload {
    pub(crate) watcher: Option<BindingWatcher>,
}

/// Registers action translation and gamepad capture.
pub struct InputPlugin {
    config: InputConfig,
}
impl InputPlugin {
    /// Creates an input plugin with the supplied configuration.
    pub fn new(config: InputConfig) -> Self {
        Self { config }
    }
}

impl HygePlugin for InputPlugin {
    fn name(&self) -> &'static str {
        "hyge-input"
    }
    fn build(&self, app: &mut bevy_app::App) {
        let map = self
            .config
            .binding_file
            .as_deref()
            .and_then(|path| load_file(path).ok())
            .unwrap_or_default();
        app.insert_resource(map);
        app.insert_non_send_resource(GamepadInput {
            gilrs: Gilrs::new().ok(),
        });
        let watcher = if self.config.enable_hot_reload {
            self.config
                .binding_file
                .clone()
                .and_then(|path| BindingWatcher::new(path).ok())
        } else {
            None
        };
        app.insert_non_send_resource(InputHotReload { watcher });
        app.add_systems(
            Label::PreUpdate,
            (poll_gamepad_system, reload_system, translate_system)
                .chain()
                .in_set(InputSet::Translate),
        );
        app.add_systems(Label::Last, flush_system.in_set(InputSet::Flush));
    }
}

fn poll_gamepad_system(
    mut gamepad: NonSendMut<GamepadInput>,
    mut events: EventWriter<hyge_window::events::DeviceEvent>,
) {
    let Some(gilrs) = gamepad.gilrs.as_mut() else {
        return;
    };
    while let Some(event) = gilrs.next_event() {
        use gilrs::EventType;
        let id = gamepad_id(event.id);
        match event.event {
            EventType::Connected => {
                events.send(hyge_window::events::DeviceEvent {
                    event: hyge_window::events::DeviceEventKind::GamepadConnected {
                        id,
                        name: gilrs.gamepad(event.id).name().to_string(),
                    },
                });
            }
            EventType::Disconnected => {
                events.send(hyge_window::events::DeviceEvent {
                    event: hyge_window::events::DeviceEventKind::GamepadDisconnected { id },
                });
            }
            EventType::ButtonPressed(button, _) | EventType::ButtonReleased(button, _) => {
                events.send(hyge_window::events::DeviceEvent {
                    event: hyge_window::events::DeviceEventKind::GamepadButton {
                        id,
                        button: gamepad_button_id(button),
                        pressed: matches!(event.event, EventType::ButtonPressed(_, _)),
                    },
                });
            }
            EventType::AxisChanged(axis, value, _) => {
                events.send(hyge_window::events::DeviceEvent {
                    event: hyge_window::events::DeviceEventKind::GamepadAxis {
                        id,
                        axis: axis as u32,
                        value,
                    },
                });
            }
            _ => {}
        }
    }
}

fn reload_system(mut map: ResMut<ActionMap>, state: NonSend<InputHotReload>) {
    let Some(watcher) = &state.watcher else {
        return;
    };
    if let Some(Ok(new_map)) = watcher.poll() {
        map.replace(new_map);
    }
}

fn gamepad_button_id(button: gilrs::Button) -> u32 {
    use gilrs::Button::*;
    match button {
        South => 0,
        East => 1,
        North => 2,
        West => 3,
        C => 4,
        Z => 5,
        LeftTrigger => 6,
        LeftTrigger2 => 7,
        RightTrigger => 8,
        RightTrigger2 => 9,
        Select => 10,
        Start => 11,
        Mode => 12,
        LeftThumb => 13,
        RightThumb => 14,
        DPadUp => 15,
        DPadDown => 16,
        DPadLeft => 17,
        DPadRight => 18,
        Unknown => u32::MAX,
    }
}

fn gamepad_id(id: gilrs::GamepadId) -> u32 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyge_window::events::{DeviceEvent, DeviceEventKind};

    #[test]
    fn plugin_translates_a_mocked_key_event() {
        let mut app = bevy_app::App::new();
        app.init_schedule(Label::PreUpdate);
        app.init_schedule(Label::Last);
        app.add_event::<DeviceEvent>();
        InputPlugin::new(InputConfig::default()).build(&mut app);
        let map = crate::binding::parse_toml(
            r#"[actions.move]
type = "vec2"
bindings = [{ kind = "keyboard", key = "w", axis = "y" }]
"#,
        )
        .expect("test bindings");
        app.world_mut().insert_resource(map);
        app.world_mut().send_event(DeviceEvent {
            event: DeviceEventKind::Key {
                scancode: 0,
                key: "w".into(),
                pressed: true,
            },
        });
        app.world_mut().run_schedule(Label::PreUpdate);
        assert_eq!(
            app.world().resource::<ActionMap>().actions["move"],
            crate::action::Action::Vec2(crate::action::Vec2Action {
                value: hyge_core::prelude::Vec2::Y,
                raw: hyge_core::prelude::Vec2::Y
            })
        );
    }
}
