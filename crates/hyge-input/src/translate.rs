//! Device-event translation into action state.

use hyge_core::prelude::Vec2;
use hyge_ecs::prelude::*;
use hyge_window::prelude::{DeviceEvent, DeviceEventKind};

use crate::{
    action::{Action, ActionMap},
    binding::{Axis2, Binding, Modifiers, MouseAxis},
};

/// Translates queued device events into the physical state held by `ActionMap`.
pub fn translate_system(mut events: EventReader<DeviceEvent>, mut map: ResMut<ActionMap>) {
    for event in events.read() {
        match &event.event {
            DeviceEventKind::Key { key, pressed, .. } => set_key(&mut map, key, *pressed),
            DeviceEventKind::MouseButton { button, pressed } => {
                set_button(&mut map, *button, *pressed)
            }
            DeviceEventKind::MouseMotion { dx, dy } => map.mouse_delta += Vec2::new(*dx, *dy),
            DeviceEventKind::MouseWheel { dx, dy } => {
                *map.mouse_axes.entry(MouseAxis::X).or_default() += *dx;
                *map.mouse_axes.entry(MouseAxis::Y).or_default() += *dy;
            }
            DeviceEventKind::GamepadButton {
                id,
                button,
                pressed,
            } => set_gamepad_button(&mut map, *id, *button, *pressed),
            DeviceEventKind::GamepadAxis { id, axis, value } => {
                map.gamepad_axes.insert((*id, *axis), *value);
            }
            DeviceEventKind::WindowFocus { focused: false } => clear_devices(&mut map),
            DeviceEventKind::GamepadConnected { .. }
            | DeviceEventKind::GamepadDisconnected { .. }
            | DeviceEventKind::WindowFocus { focused: true } => {}
        }
    }
    evaluate_actions(&mut map);
}

/// Clears per-frame values after consumers have observed the state.
pub fn flush_system(mut map: ResMut<ActionMap>) {
    map.mouse_delta = Vec2::ZERO;
    map.mouse_axes.clear();
    for action in map.actions.values_mut() {
        match action {
            Action::Button(value) => {
                value.just_pressed = false;
                value.just_released = false;
            }
            Action::Axis(value) => {
                value.raw = 0.0;
                value.value = 0.0;
            }
            Action::Vec2(value) => {
                value.raw = Vec2::ZERO;
                value.value = Vec2::ZERO;
            }
        }
    }
}

fn set_key(map: &mut ActionMap, key: &str, pressed: bool) {
    let key = key.to_ascii_lowercase();
    if pressed {
        map.keys.insert(key);
    } else {
        map.keys.remove(&key);
    }
}
fn set_button(map: &mut ActionMap, button: u32, pressed: bool) {
    if pressed {
        map.mouse_buttons.insert(button);
    } else {
        map.mouse_buttons.remove(&button);
    }
}
fn set_gamepad_button(map: &mut ActionMap, id: u32, button: u32, pressed: bool) {
    if pressed {
        map.gamepad_buttons.insert((id, button));
    } else {
        map.gamepad_buttons.remove(&(id, button));
    }
}
fn clear_devices(map: &mut ActionMap) {
    map.keys.clear();
    map.mouse_buttons.clear();
    map.gamepad_buttons.clear();
    map.gamepad_axes.clear();
}
fn modifiers_match(required: Modifiers, map: &ActionMap) -> bool {
    (!required.ctrl || map.keys.contains("control") || map.keys.contains("ctrl"))
        && (!required.shift || map.keys.contains("shift"))
        && (!required.alt || map.keys.contains("alt"))
        && (!required.logo || map.keys.contains("super") || map.keys.contains("logo"))
}

fn evaluate_actions(map: &mut ActionMap) {
    let names: Vec<String> = map.actions.keys().cloned().collect();
    for name in names {
        let Some(bindings) = map.bindings.get(&name).cloned() else {
            continue;
        };
        let next = match map.actions.get(&name) {
            Some(Action::Button(value)) => {
                let active = bindings.iter().any(|binding| match binding {
                    Binding::Keyboard {
                        key,
                        axis: None,
                        modifier,
                        ..
                    } => map.keys.contains(key) && modifiers_match(*modifier, map),
                    Binding::MouseButton { button } => map.mouse_buttons.contains(button),
                    Binding::GamepadButton { id, button } => {
                        map.gamepad_buttons.contains(&(*id, *button))
                    }
                    _ => false,
                });
                Action::Button(crate::action::ButtonAction {
                    state: active,
                    just_pressed: active && !value.state,
                    just_released: !active && value.state,
                    value: if active { 1.0 } else { 0.0 },
                })
            }
            Some(Action::Axis(_)) => {
                let raw = bindings
                    .iter()
                    .map(|binding| scalar_value(binding, map))
                    .sum::<f32>();
                Action::Axis(crate::action::AxisAction { value: raw, raw })
            }
            Some(Action::Vec2(_)) => {
                let raw = bindings
                    .iter()
                    .fold(Vec2::ZERO, |sum, binding| sum + vec_value(binding, map));
                Action::Vec2(crate::action::Vec2Action { value: raw, raw })
            }
            None => continue,
        };
        if let Some(action) = map.actions.get_mut(&name) {
            *action = next;
        }
    }
}

fn scalar_value(binding: &Binding, map: &ActionMap) -> f32 {
    match binding {
        Binding::Keyboard {
            key,
            axis: None,
            scale,
            modifier,
        } if map.keys.contains(key) && modifiers_match(*modifier, map) => *scale,
        Binding::MouseAxis { axis, scale } => {
            map.mouse_axes.get(axis).copied().unwrap_or_default() * scale
        }
        Binding::MouseDelta { scale } => map.mouse_delta.length() * scale,
        Binding::GamepadAxis { id, axis, scale } => {
            map.gamepad_axes
                .get(&(*id, *axis))
                .copied()
                .unwrap_or_default()
                * scale
        }
        _ => 0.0,
    }
}
fn vec_value(binding: &Binding, map: &ActionMap) -> Vec2 {
    match binding {
        Binding::Keyboard {
            key,
            axis: Some(axis),
            scale,
            modifier,
        } if map.keys.contains(key) && modifiers_match(*modifier, map) => {
            axis_value(*axis) * *scale
        }
        Binding::MouseDelta { scale } => map.mouse_delta * *scale,
        Binding::GamepadAxis { id, axis, scale } => Vec2::new(
            map.gamepad_axes
                .get(&(*id, *axis))
                .copied()
                .unwrap_or_default()
                * scale,
            0.0,
        ),
        _ => Vec2::ZERO,
    }
}
fn axis_value(axis: Axis2) -> Vec2 {
    match axis {
        Axis2::X => Vec2::X,
        Axis2::Y => Vec2::Y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::parse_toml;

    fn map() -> ActionMap {
        parse_toml(r#"[actions.move]
type = "vec2"
bindings = [{ kind = "keyboard", key = "w", axis = "y", scale = 1.0 }, { kind = "keyboard", key = "d", axis = "x", scale = 1.0 }]
[actions.jump]
type = "button"
bindings = [{ kind = "keyboard", key = "space" }]
"#).expect("test bindings")
    }

    #[test]
    fn wasd_updates_vec2_and_button_edges() {
        let mut map = map();
        set_key(&mut map, "w", true);
        set_key(&mut map, "space", true);
        evaluate_actions(&mut map);
        assert_eq!(
            map.actions["move"],
            Action::Vec2(crate::action::Vec2Action {
                value: Vec2::Y,
                raw: Vec2::Y
            })
        );
        assert!(matches!(
            map.actions["jump"],
            Action::Button(crate::action::ButtonAction {
                just_pressed: true,
                state: true,
                ..
            })
        ));
        evaluate_actions(&mut map);
        assert!(matches!(
            map.actions["jump"],
            Action::Button(crate::action::ButtonAction {
                just_pressed: false,
                state: true,
                ..
            })
        ));
        set_key(&mut map, "space", false);
        evaluate_actions(&mut map);
        assert!(matches!(
            map.actions["jump"],
            Action::Button(crate::action::ButtonAction {
                just_released: true,
                state: false,
                ..
            })
        ));
    }

    #[test]
    fn scaled_bindings_aggregate_and_flush() {
        let mut map = parse_toml(r#"[actions.look]
type = "axis"
bindings = [{ kind = "mouse_axis", axis = "x", scale = 0.5 }, { kind = "mouse_axis", axis = "x", scale = 2.0 }]
"#).expect("test bindings");
        map.mouse_axes.insert(crate::binding::MouseAxis::X, 4.0);
        evaluate_actions(&mut map);
        assert_eq!(
            map.actions["look"],
            Action::Axis(crate::action::AxisAction {
                value: 10.0,
                raw: 10.0
            })
        );
        flush_system_direct(&mut map);
        assert_eq!(
            map.actions["look"],
            Action::Axis(crate::action::AxisAction::default())
        );
    }

    fn flush_system_direct(map: &mut ActionMap) {
        map.mouse_delta = Vec2::ZERO;
        map.mouse_axes.clear();
        for action in map.actions.values_mut() {
            if let Action::Axis(value) = action {
                *value = crate::action::AxisAction::default();
            }
        }
    }
}
