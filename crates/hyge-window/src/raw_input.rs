//! Raw input device registration (Windows-specific; no-op on other platforms).
//!
//! On Windows, this calls `RegisterRawInputDevices` to enable raw input mode
//! for keyboard and mouse. Raw input bypasses the OS's mouse acceleration
//! curve and is essential for FPS-style controls.
//!
//! On non-Windows platforms the function is a no-op (Linux/macOS handle raw
//! input through their own mechanisms, integrated by `winit`).
//!
//! Note: `winit` 0.30 already registers raw input devices by default on
//! Windows. Calling this function is therefore usually redundant; it is
//! provided for completeness and for advanced use cases where the user
//! wants to register additional device types (e.g. game controllers) under
//! the raw input umbrella.

use raw_window_handle::HasWindowHandle;

use hyge_core::result::HygeError;

/// Registers raw input devices (mouse, keyboard) for the given window.
///
/// On Windows, this calls `RegisterRawInputDevices` with `RIDEV_INPUTSINK`
/// (input is received even when the window is not in the foreground,
/// which is what game windows want). On other platforms, this is a no-op.
///
/// # Errors
///
/// Returns `HygeError::Unsupported` if the platform-specific registration
/// fails or the window handle is not a Win32 handle (when compiled for
/// Windows).
pub fn register_raw_input_devices(window: &impl HasWindowHandle) -> Result<(), HygeError> {
    #[cfg(windows)]
    {
        windows_impl::register(window)?;
    }
    // Non-Windows: no-op.
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_impl {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK};

    use hyge_core::result::HygeError;

    /// HID usage page: "Generic Desktop" (mouse, keyboard, game controllers).
    const USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
    /// HID usage: mouse.
    const USAGE_MOUSE: u16 = 0x02;
    /// HID usage: keyboard.
    const USAGE_KEYBOARD: u16 = 0x06;

    /// Calls `RegisterRawInputDevices` to enable raw input for the given
    /// window.
    pub fn register(window: &(impl HasWindowHandle + ?Sized)) -> Result<(), HygeError> {
        let hwnd = hwnd_from(window)?;
        let devices = [device(hwnd, USAGE_MOUSE), device(hwnd, USAGE_KEYBOARD)];
        // SAFETY: `RegisterRawInputDevices` is a Win32 API; we pass a valid
        // pointer to a stack-allocated array of two `RAWINPUTDEVICE`s and
        // the correct count/size. The HWND is derived from a live
        // `HasWindowHandle` and remains valid for the duration of the call.
        let ok = unsafe {
            RegisterRawInputDevices(
                devices.as_ptr(),
                devices.len() as u32,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        if ok == 0 {
            return Err(HygeError::Unsupported(
                "RegisterRawInputDevices returned 0 (failure)".into(),
            ));
        }
        Ok(())
    }

    fn device(hwnd: HWND, usage: u16) -> RAWINPUTDEVICE {
        RAWINPUTDEVICE {
            usUsagePage: USAGE_PAGE_GENERIC_DESKTOP,
            usUsage: usage,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        }
    }

    fn hwnd_from(window: &(impl HasWindowHandle + ?Sized)) -> Result<HWND, HygeError> {
        let handle = window
            .window_handle()
            .map_err(|e| HygeError::Unsupported(format!("window_handle: {e}")))?;
        match handle.as_raw() {
            RawWindowHandle::Win32(h) => Ok(h.hwnd.get() as HWND),
            _ => Err(HygeError::Unsupported(
                "raw input registration requires a Win32 window handle".into(),
            )),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The function compiles and is callable. We cannot easily create
        /// a real Win32 window in a unit test (requires an `EventLoop`),
        /// so the actual end-to-end behavior is verified by integration
        /// tests. This test asserts that the function exists and returns
        /// `Ok` for the (admittedly trivial) no-handle case — but the
        /// function signature requires a `HasWindowHandle`, so the
        /// simplest assertion is the compile-time check.
        #[test]
        fn function_is_defined() {
            // Compile-time check: the function signature exists.
            fn _check(f: &dyn HasWindowHandle) -> Result<(), HygeError> {
                register(f)
            }
        }
    }
}
