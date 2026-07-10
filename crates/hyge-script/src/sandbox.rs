//! Lua standard-library policy.

use mlua::{Lua, LuaOptions, Result, StdLib, Value};

/// Creates a Lua state with the safe standard libraries and explicitly
/// removes libraries that could access the host process or filesystem.
pub fn create_sandboxed_lua() -> Result<Lua> {
    let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default())?;
    strip_forbidden_globals(&lua)?;
    Ok(lua)
}

/// Removes forbidden globals even if a future mlua safe-library set exposes
/// one of them by default.
pub fn strip_forbidden_globals(lua: &Lua) -> Result<()> {
    let globals = lua.globals();
    for name in ["os", "io", "debug", "package", "require"] {
        globals.set(name, Value::Nil)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_stdlib_is_actually_unavailable() {
        let lua = create_sandboxed_lua().expect("sandbox should initialize");
        let values: (Value, Value, Value, Value, Value) = lua
            .load("return os, io, debug, package, require")
            .eval()
            .expect("sandbox query should run");
        assert!(matches!(values.0, Value::Nil));
        assert!(matches!(values.1, Value::Nil));
        assert!(matches!(values.2, Value::Nil));
        assert!(matches!(values.3, Value::Nil));
        assert!(matches!(values.4, Value::Nil));
    }

    #[test]
    fn safe_standard_library_remains_available() {
        let lua = create_sandboxed_lua().expect("sandbox should initialize");
        let value: i64 = lua
            .load("return math.floor(3.9) + string.len('hyge')")
            .eval()
            .expect("safe libraries should remain available");
        assert_eq!(value, 7);
    }
}
