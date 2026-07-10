//! Action state and device state used by the input translator.

use std::collections::{HashMap, HashSet};

use hyge_core::prelude::Vec2;
use hyge_ecs::Resource;

use crate::binding::{Binding, MouseAxis};

/// A named action exposed to gameplay code.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A digital action with edge information.
    Button(ButtonAction),
    /// A scalar action.
    Axis(AxisAction),
    /// A two-dimensional action.
    Vec2(Vec2Action),
}

/// State for a digital action.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ButtonAction {
    /// Whether the action is active this frame.
    pub state: bool,
    /// True only on the transition from inactive to active.
    pub just_pressed: bool,
    /// True only on the transition from active to inactive.
    pub just_released: bool,
    /// Numeric representation of `state`.
    pub value: f32,
}

/// State for a scalar action.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AxisAction {
    /// Aggregated, scaled value for this frame.
    pub value: f32,
    /// Unscaled aggregated value.
    pub raw: f32,
}

/// State for a two-dimensional action.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2Action {
    /// Aggregated, scaled value for this frame.
    pub value: Vec2,
    /// Unscaled aggregated value.
    pub raw: Vec2,
}

/// All configured actions and their bindings.
#[derive(Resource, Clone, Debug, Default)]
pub struct ActionMap {
    /// Current named action state.
    pub actions: HashMap<String, Action>,
    /// Bindings grouped by action name, preserving TOML order.
    pub bindings: HashMap<String, Vec<Binding>>,
    /// Current physical keyboard state, indexed by normalized key name.
    pub(crate) keys: HashSet<String>,
    /// Current mouse button state.
    pub(crate) mouse_buttons: HashSet<u32>,
    /// Current gamepad button state, indexed by `(gamepad, button)`.
    pub(crate) gamepad_buttons: HashSet<(u32, u32)>,
    /// Current gamepad axis values.
    pub(crate) gamepad_axes: HashMap<(u32, u32), f32>,
    /// Accumulated mouse movement for the current frame.
    pub(crate) mouse_delta: Vec2,
    /// Accumulated mouse axes for the current frame.
    pub(crate) mouse_axes: HashMap<MouseAxis, f32>,
}

impl ActionMap {
    /// Creates an empty action map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns an action by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Action> {
        self.actions.get(name)
    }

    /// Returns a mutable action by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Action> {
        self.actions.get_mut(name)
    }

    /// Replaces the configured actions and resets transient device state.
    pub fn replace(&mut self, other: Self) {
        *self = other;
    }
}
