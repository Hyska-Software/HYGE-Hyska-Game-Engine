//! Transactional undo/redo history for editor commands.

use bevy_ecs::world::World;

use crate::commands::{Command, CommandEffect, CommandFailure, EditorCommand};

/// Engine-owned undo and redo stacks.
#[derive(Clone, Debug, Default)]
pub struct CommandHistory {
    undo: Vec<EditorCommand>,
    redo: Vec<EditorCommand>,
}

impl CommandHistory {
    /// Applies a new command and clears redo history.
    pub fn apply(
        &mut self,
        mut command: EditorCommand,
        world: &mut World,
    ) -> Result<CommandEffect, CommandFailure> {
        let effect = command.apply(world)?;
        self.undo.push(command);
        self.redo.clear();
        Ok(effect)
    }

    /// Reverts the latest command.
    pub fn undo(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let mut command = self
            .undo
            .pop()
            .ok_or_else(|| CommandFailure::new("history_empty", "undo history is empty"))?;
        match command.revert(world) {
            Ok(effect) => {
                self.redo.push(command);
                Ok(effect)
            }
            Err(error) => {
                self.undo.push(command);
                Err(error)
            }
        }
    }

    /// Reapplies the latest reverted command.
    pub fn redo(&mut self, world: &mut World) -> Result<CommandEffect, CommandFailure> {
        let mut command = self
            .redo
            .pop()
            .ok_or_else(|| CommandFailure::new("history_empty", "redo history is empty"))?;
        match command.apply(world) {
            Ok(effect) => {
                self.undo.push(command);
                Ok(effect)
            }
            Err(error) => {
                self.redo.push(command);
                Err(error)
            }
        }
    }

    /// Clears both history stacks.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Returns whether undo is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Returns whether redo is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
