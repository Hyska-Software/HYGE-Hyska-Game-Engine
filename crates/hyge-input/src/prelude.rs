//! Public input API.

pub use crate::action::{Action, ActionMap, AxisAction, ButtonAction, Vec2Action};
pub use crate::binding::{
    load_file, parse_toml, Axis2, Binding, BindingError, Modifiers, MouseAxis,
};
pub use crate::plugin::{GamepadInput, InputConfig, InputHotReload, InputPlugin};
pub use crate::translate::{flush_system, translate_system};
