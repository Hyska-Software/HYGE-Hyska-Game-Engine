//! Window and device events emitted by the window plugin.
//!
//! `winit` events are translated into Hyge events by
//! [`translate_window_event`]. `DeviceEvent`s are not translated here —
//! that happens in `hyge-input` (R-074) once the input layer is in place.

use bevy_ecs::event::Event;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent as WinitWindowEvent;

/// Emitted when the window is resized (e.g. by the user dragging the
/// border, or by the OS when the DPI changes).
#[derive(Event, Debug, Clone, Copy)]
pub struct WindowResized {
    /// New width in physical pixels.
    pub width: u32,
    /// New height in physical pixels.
    pub height: u32,
}

/// Emitted when the user requests window close (e.g. clicking the X button
/// or pressing Alt+F4). The application is responsible for actually
/// closing the window and exiting the event loop in response.
#[derive(Event, Debug, Clone, Copy)]
pub struct WindowCloseRequested;

/// Emitted when the window gains or loses focus.
#[derive(Event, Debug, Clone, Copy)]
pub struct WindowFocused {
    /// `true` if the window gained focus, `false` if it lost focus.
    pub focused: bool,
}

/// Emitted for raw device-level events: raw mouse delta, raw key state,
/// gamepad connect/disconnect. Translated from `winit::event::DeviceEvent`
/// in `hyge-input` (R-074) once the input layer is in place.
#[derive(Event, Debug, Clone)]
pub struct DeviceEvent {
    /// The kind of device event that occurred.
    pub event: DeviceEventKind,
}

/// The specific kind of device event.
#[derive(Debug, Clone)]
pub enum DeviceEventKind {
    /// Raw mouse motion delta (no OS cursor acceleration applied). `dx`
    /// and `dy` are in physical pixels.
    MouseMotion {
        /// Delta X in physical pixels.
        dx: f32,
        /// Delta Y in physical pixels.
        dy: f32,
    },

    /// Mouse wheel scroll delta. `dx` and `dy` are in physical pixels (or
    /// lines, depending on the input source).
    MouseWheel {
        /// Horizontal scroll delta.
        dx: f32,
        /// Vertical scroll delta.
        dy: f32,
    },

    /// Raw keyboard event. `scancode` is the platform-specific scan code;
    /// `pressed` is `true` for key-down, `false` for key-up.
    Key {
        /// Platform-specific scan code.
        scancode: u32,
        /// `true` for key-down, `false` for key-up.
        pressed: bool,
    },

    /// A gamepad was connected. `id` is the platform-specific device id;
    /// `name` is the human-readable device name.
    GamepadConnected {
        /// Platform-specific gamepad id.
        id: u32,
        /// Human-readable device name (from the OS).
        name: String,
    },

    /// A gamepad was disconnected. `id` is the platform-specific device id.
    GamepadDisconnected {
        /// Platform-specific gamepad id.
        id: u32,
    },
}

/// Sum type covering all events that can be produced by the window plugin.
/// Used by [`translate_window_event`] to return a homogeneous result.
#[derive(Debug, Clone)]
pub enum HygeWindowEvent {
    /// `WindowEvent::Resized` translated to [`WindowResized`].
    Resized(WindowResized),
    /// `WindowEvent::CloseRequested` translated to [`WindowCloseRequested`].
    CloseRequested(WindowCloseRequested),
    /// `WindowEvent::Focused` translated to [`WindowFocused`].
    Focused(WindowFocused),
    /// A device event (mouse motion, wheel, key, gamepad) translated to
    /// [`DeviceEvent`].
    Device(DeviceEvent),
}

/// Translates a `winit` `WindowEvent` into a Hyge event, or `None` if the
/// event does not correspond to anything the engine cares about (e.g.
/// `RedrawRequested`, `CursorMoved`, `ScaleFactorChanged`).
///
/// Device-level events (mouse motion, key, gamepad) are handled separately
/// by `hyge-input` and are not produced by this function.
pub fn translate_window_event(event: &WinitWindowEvent) -> Option<HygeWindowEvent> {
    match event {
        WinitWindowEvent::Resized(PhysicalSize { width, height }) => {
            Some(HygeWindowEvent::Resized(WindowResized {
                width: *width,
                height: *height,
            }))
        }
        WinitWindowEvent::CloseRequested => {
            Some(HygeWindowEvent::CloseRequested(WindowCloseRequested))
        }
        WinitWindowEvent::Focused(focused) => Some(HygeWindowEvent::Focused(WindowFocused {
            focused: *focused,
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::dpi::PhysicalSize;
    use winit::event::WindowEvent;

    #[test]
    fn translates_resized() {
        let event = WindowEvent::Resized(PhysicalSize::new(800, 600));
        match translate_window_event(&event) {
            Some(HygeWindowEvent::Resized(r)) => {
                assert_eq!(r.width, 800);
                assert_eq!(r.height, 600);
            }
            other => panic!("expected Resized, got {other:?}"),
        }
    }

    #[test]
    fn translates_resized_to_zero_size() {
        // Edge case: window minimized to 0x0 on Windows. Should still
        // produce a valid event with width=0, height=0.
        let event = WindowEvent::Resized(PhysicalSize::new(0, 0));
        match translate_window_event(&event) {
            Some(HygeWindowEvent::Resized(r)) => {
                assert_eq!(r.width, 0);
                assert_eq!(r.height, 0);
            }
            _ => panic!("expected Resized(0, 0)"),
        }
    }

    #[test]
    fn translates_close_requested() {
        let event = WindowEvent::CloseRequested;
        assert!(matches!(
            translate_window_event(&event),
            Some(HygeWindowEvent::CloseRequested(_))
        ));
    }

    #[test]
    fn translates_focused_true() {
        let event = WindowEvent::Focused(true);
        match translate_window_event(&event) {
            Some(HygeWindowEvent::Focused(f)) => assert!(f.focused),
            _ => panic!("expected Focused(true)"),
        }
    }

    #[test]
    fn translates_focused_false() {
        let event = WindowEvent::Focused(false);
        match translate_window_event(&event) {
            Some(HygeWindowEvent::Focused(f)) => assert!(!f.focused),
            _ => panic!("expected Focused(false)"),
        }
    }

    #[test]
    fn ignores_redraw_requested() {
        let event = WindowEvent::RedrawRequested;
        assert!(translate_window_event(&event).is_none());
    }

    #[test]
    fn ignores_cursor_moved() {
        let event = WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: PhysicalPosition::new(0.0, 0.0),
        };
        assert!(translate_window_event(&event).is_none());
    }

    #[test]
    fn all_translated_variants_are_distinct() {
        let r = HygeWindowEvent::Resized(WindowResized {
            width: 1,
            height: 1,
        });
        let c = HygeWindowEvent::CloseRequested(WindowCloseRequested);
        let f = HygeWindowEvent::Focused(WindowFocused { focused: true });
        // Just verify they are distinct enum variants (Debug, PartialEq).
        assert_ne!(format!("{r:?}"), format!("{c:?}"));
        assert_ne!(format!("{c:?}"), format!("{f:?}"));
        assert_ne!(format!("{r:?}"), format!("{f:?}"));
    }

    use winit::dpi::PhysicalPosition;
}
