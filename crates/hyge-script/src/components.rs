//! Script-related ECS components.

use hyge_ecs::prelude::*;

/// Reference to a Lua script attached to an entity.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Default)]
#[reflect(Component)]
pub struct ScriptRef {
    /// Asset path to the Lua source file.
    pub path: String,
    /// Optional named table/module inside the script.
    pub table: Option<String>,
    /// Whether this script is enabled.
    pub enabled: bool,
}

impl ScriptRef {
    /// Builds an enabled script reference.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            table: None,
            enabled: true,
        }
    }

    /// Sets the optional table/module name.
    #[must_use]
    pub fn table(mut self, table: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::ScriptRef;

    #[test]
    fn script_ref_builder_defaults_to_enabled_without_table() {
        let script = ScriptRef::new("assets/player.lua");
        assert_eq!(script.path, "assets/player.lua");
        assert!(script.enabled);
        assert_eq!(script.table, None);
    }

    #[test]
    fn script_ref_builder_sets_table_and_preserves_path() {
        let script = ScriptRef::new("assets/player.lua").table("player");
        assert_eq!(script.path, "assets/player.lua");
        assert_eq!(script.table.as_deref(), Some("player"));
        assert!(script.enabled);
    }
}
