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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_error_preserves_entity_and_structured_error() {
        let entity = Entity::from_raw(7);
        let event = ScriptError {
            entity,
            error: HygeError::InvalidArgument("bad field".into()),
        };
        assert_eq!(event.entity, entity);
        assert!(
            matches!(event.error, HygeError::InvalidArgument(message) if message == "bad field")
        );
    }
}
