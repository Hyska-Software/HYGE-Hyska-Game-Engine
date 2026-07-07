//! Character controller backend placeholders for the Rapier integration.

use rapier3d::control::KinematicCharacterController;

use crate::components::CharacterController;

/// Builds a Rapier kinematic character controller from Hyge tuning values.
#[must_use]
pub fn character_controller(controller: CharacterController) -> KinematicCharacterController {
    KinematicCharacterController {
        max_slope_climb_angle: controller.max_slope,
        ..KinematicCharacterController::default()
    }
}
