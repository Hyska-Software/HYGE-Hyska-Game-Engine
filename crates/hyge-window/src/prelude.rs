//! Convenience re-exports for `hyge-window`.

pub use crate::config::WindowConfig;
pub use crate::events::{
    translate_window_event, DeviceEvent, DeviceEventKind, HygeWindowEvent,
    WindowCloseRequested, WindowFocused, WindowResized,
};
pub use crate::plugin::{WindowPlugin, WindowState};
pub use crate::raw_input::register_raw_input_devices;
pub use crate::window::Window;
