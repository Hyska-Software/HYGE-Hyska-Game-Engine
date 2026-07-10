//! Script runtime events.

use hyge_core::prelude::HygeError;
use hyge_ecs::prelude::*;

/// Reports a script load or execution failure without crashing the engine.
#[derive(Event, Debug)]
pub struct ScriptError {
    /// Entity whose script produced the error, or `Entity::PLACEHOLDER` for
    /// an engine-level evaluation.
    pub entity: Entity,
    /// Structured engine error.
    pub error: HygeError,
}
