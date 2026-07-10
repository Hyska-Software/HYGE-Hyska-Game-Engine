//! TOML binding schema and validation.

use std::{collections::BTreeMap, fmt, path::Path};

use serde::Deserialize;

use crate::action::{Action, ActionMap, AxisAction, ButtonAction, Vec2Action};

/// Two-dimensional destination for a keyboard binding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Axis2 {
    /// Horizontal axis.
    X,
    /// Vertical axis.
    Y,
}

/// Mouse axis supported by the binding schema.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MouseAxis {
    /// Horizontal wheel movement.
    X,
    /// Vertical wheel movement.
    Y,
}

/// Modifier requirements for a keyboard binding.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct Modifiers {
    /// Control must be held.
    #[serde(default)]
    pub ctrl: bool,
    /// Shift must be held.
    #[serde(default)]
    pub shift: bool,
    /// Alt must be held.
    #[serde(default)]
    pub alt: bool,
    /// Logo/Super must be held.
    #[serde(default)]
    pub logo: bool,
}

/// A physical input binding.
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    /// A keyboard key, optionally projected onto a Vec2 axis.
    Keyboard {
        /// Human-readable key name.
        key: String,
        /// Optional Vec2 destination axis.
        axis: Option<Axis2>,
        /// Scale applied to the value.
        scale: f32,
        /// Required modifiers.
        modifier: Modifiers,
    },
    /// A mouse button.
    MouseButton {
        /// Button number.
        button: u32,
    },
    /// A mouse wheel axis.
    MouseAxis {
        /// Axis.
        axis: MouseAxis,
        /// Scale.
        scale: f32,
    },
    /// Raw mouse movement.
    MouseDelta {
        /// Scale applied to both components.
        scale: f32,
    },
    /// A gamepad button.
    GamepadButton {
        /// Gamepad ID.
        id: u32,
        /// Button number.
        button: u32,
    },
    /// A gamepad axis.
    GamepadAxis {
        /// Gamepad ID.
        id: u32,
        /// Axis number.
        axis: u32,
        /// Scale.
        scale: f32,
    },
}

impl Binding {
    fn kind(&self) -> &'static str {
        match self {
            Self::Keyboard { .. } => "keyboard",
            Self::MouseButton { .. } => "mouse_button",
            Self::MouseAxis { .. } => "mouse_axis",
            Self::MouseDelta { .. } => "mouse_delta",
            Self::GamepadButton { .. } => "gamepad_button",
            Self::GamepadAxis { .. } => "gamepad_axis",
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawFile {
    actions: BTreeMap<String, RawAction>,
}

#[derive(Debug, Deserialize)]
struct RawAction {
    #[serde(rename = "type")]
    action_type: String,
    bindings: Vec<toml::Value>,
}

/// Error returned when a TOML binding file is malformed or inconsistent.
#[derive(Debug)]
pub struct BindingError(String);

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BindingError {}

/// Parses and validates an action map from TOML text.
pub fn parse_toml(text: &str) -> Result<ActionMap, BindingError> {
    let raw: RawFile =
        toml::from_str(text).map_err(|e| BindingError(format!("invalid input TOML: {e}")))?;
    let mut map = ActionMap::new();
    for (name, definition) in raw.actions {
        let action_kind = definition.action_type.to_ascii_lowercase();
        let mut bindings = Vec::with_capacity(definition.bindings.len());
        for (index, value) in definition.bindings.iter().enumerate() {
            let binding = parse_binding(value)
                .map_err(|e| BindingError(format!("actions.{name}.bindings[{index}]: {e}")))?;
            validate_binding(&action_kind, &binding)
                .map_err(|e| BindingError(format!("actions.{name}: {e}")))?;
            bindings.push(binding);
        }
        let action = match action_kind.as_str() {
            "button" => Action::Button(ButtonAction::default()),
            "axis" => Action::Axis(AxisAction::default()),
            "vec2" => Action::Vec2(Vec2Action::default()),
            other => {
                return Err(BindingError(format!(
                    "actions.{name}.type: unknown action type {other:?}"
                )))
            }
        };
        map.actions.insert(name.clone(), action);
        map.bindings.insert(name, bindings);
    }
    Ok(map)
}

/// Loads and parses a binding file from disk.
pub fn load_file(path: &Path) -> Result<ActionMap, BindingError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| BindingError(format!("{}: {e}", path.display())))?;
    parse_toml(&text)
}

fn parse_binding(value: &toml::Value) -> Result<Binding, String> {
    let kind = value
        .get("kind")
        .and_then(toml::Value::as_str)
        .ok_or("missing kind")?;
    let number = |field: &str, default: f32| {
        value
            .get(field)
            .and_then(toml::Value::as_float)
            .unwrap_or(f64::from(default)) as f32
    };
    match kind {
        "keyboard" => {
            let key = value
                .get("key")
                .and_then(toml::Value::as_str)
                .ok_or("keyboard binding requires key")?
                .to_ascii_lowercase();
            let axis = value
                .get("axis")
                .and_then(toml::Value::as_str)
                .map(|axis| match axis.to_ascii_lowercase().as_str() {
                    "x" => Ok(Axis2::X),
                    "y" => Ok(Axis2::Y),
                    _ => Err("axis must be x or y"),
                })
                .transpose()?;
            let modifier = value
                .get("modifier")
                .cloned()
                .map(toml::Value::try_into)
                .transpose()
                .map_err(|e| format!("invalid modifier: {e}"))?
                .unwrap_or_default();
            Ok(Binding::Keyboard {
                key,
                axis,
                scale: number("scale", 1.0),
                modifier,
            })
        }
        "mouse_button" => Ok(Binding::MouseButton {
            button: value
                .get("button")
                .and_then(toml::Value::as_integer)
                .ok_or("mouse_button binding requires button")? as u32,
        }),
        "mouse_axis" => {
            let axis = match value
                .get("axis")
                .and_then(toml::Value::as_str)
                .ok_or("mouse_axis binding requires axis")?
                .to_ascii_lowercase()
                .as_str()
            {
                "x" => MouseAxis::X,
                "y" => MouseAxis::Y,
                _ => return Err("axis must be x or y".into()),
            };
            Ok(Binding::MouseAxis {
                axis,
                scale: number("scale", 1.0),
            })
        }
        "mouse_delta" => Ok(Binding::MouseDelta {
            scale: number("scale", 1.0),
        }),
        "gamepad_button" => Ok(Binding::GamepadButton {
            id: value
                .get("id")
                .and_then(toml::Value::as_integer)
                .ok_or("gamepad_button binding requires id")? as u32,
            button: value
                .get("button")
                .and_then(toml::Value::as_integer)
                .ok_or("gamepad_button binding requires button")? as u32,
        }),
        "gamepad_axis" => Ok(Binding::GamepadAxis {
            id: value
                .get("id")
                .and_then(toml::Value::as_integer)
                .ok_or("gamepad_axis binding requires id")? as u32,
            axis: value
                .get("axis")
                .and_then(toml::Value::as_integer)
                .ok_or("gamepad_axis binding requires axis")? as u32,
            scale: number("scale", 1.0),
        }),
        other => Err(format!("unknown binding kind {other:?}")),
    }
}

fn validate_binding(action_type: &str, binding: &Binding) -> Result<(), String> {
    let valid = match action_type {
        "button" => matches!(
            binding,
            Binding::Keyboard { axis: None, .. }
                | Binding::MouseButton { .. }
                | Binding::GamepadButton { .. }
        ),
        "axis" => matches!(
            binding,
            Binding::Keyboard { axis: None, .. }
                | Binding::MouseAxis { .. }
                | Binding::MouseDelta { .. }
                | Binding::GamepadAxis { .. }
        ),
        "vec2" => matches!(
            binding,
            Binding::Keyboard { axis: Some(_), .. }
                | Binding::MouseDelta { .. }
                | Binding::GamepadAxis { .. }
        ),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "binding kind {} is incompatible with action type {action_type}",
            binding.kind()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_wasd_vec2_schema() {
        let map = parse_toml(r#"[actions.move]
type = "vec2"
bindings = [{ kind = "keyboard", key = "w", axis = "y", scale = 1.0 }, { kind = "keyboard", key = "d", axis = "x", scale = 1.0 }]
"#).expect("schema should parse");
        assert!(matches!(map.actions["move"], Action::Vec2(_)));
        assert_eq!(map.bindings["move"].len(), 2);
    }
    #[test]
    fn rejects_incompatible_binding() {
        let error = parse_toml(
            r#"[actions.jump]
type = "button"
bindings = [{ kind = "mouse_axis", axis = "x" }]
"#,
        )
        .expect_err("axis cannot drive button");
        assert!(error.to_string().contains("incompatible"));
    }
}
