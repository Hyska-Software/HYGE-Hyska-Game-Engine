//! Windows named shared-memory handles for the Hyge editor viewport.
//!
//! This crate deliberately confines the Win32 mapping FFI approved by
//! ADR-0017. Higher layers interact with a mapping only through safe byte
//! closures and own the ring-buffer protocol separately.

#![allow(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

/// Failures while creating or opening a named shared-memory region.
#[derive(Debug)]
pub enum SharedMemoryError {
    /// The current platform does not provide the Windows mapping backend.
    UnsupportedPlatform,
    /// The caller supplied an invalid mapping name or length.
    InvalidArgument(&'static str),
    /// A Win32 call failed.
    Os(std::io::Error),
}

impl fmt::Display for SharedMemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("Windows named shared memory is unavailable")
            }
            Self::InvalidArgument(message) => formatter.write_str(message),
            Self::Os(error) => write!(formatter, "Windows shared-memory operation failed: {error}"),
        }
    }
}

impl std::error::Error for SharedMemoryError {}

/// An owned mapping that provides safe scoped access to its bytes.
pub struct SharedMapping {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    view: *mut u8,
    len: usize,
}

impl SharedMapping {
    /// Creates a pagefile-backed named mapping of exactly `len` bytes.
    pub fn create(name: &str, len: usize) -> Result<Self, SharedMemoryError> {
        platform::create(name, len)
    }

    /// Opens an existing named mapping using the declared byte length.
    pub fn open(name: &str, len: usize) -> Result<Self, SharedMemoryError> {
        platform::open(name, len)
    }

    /// Returns the mapping length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Reads mapping bytes while keeping raw pointer access internal.
    pub fn with_bytes<R>(&self, read: impl FnOnce(&[u8]) -> R) -> R {
        platform::with_bytes(self, read)
    }

    /// Mutates mapping bytes while keeping raw pointer access internal.
    pub fn with_bytes_mut<R>(&mut self, write: impl FnOnce(&mut [u8]) -> R) -> R {
        platform::with_bytes_mut(self, write)
    }
}

#[cfg(windows)]
impl Drop for SharedMapping {
    fn drop(&mut self) {
        // SAFETY: `view` and `handle` are created together by this crate and
        // are released only here after all scoped byte access has returned.
        unsafe {
            let _ = windows_sys::Win32::System::Memory::UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.view.cast(),
                },
            );
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
impl Drop for SharedMapping {
    fn drop(&mut self) {}
}

#[cfg(windows)]
mod platform {
    use super::{SharedMapping, SharedMemoryError};
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, OpenFileMappingW, FILE_MAP_ALL_ACCESS, PAGE_READWRITE,
    };

    fn wide(name: &str) -> Result<Vec<u16>, SharedMemoryError> {
        if name.is_empty() || name.len() > 240 || !name.is_ascii() {
            return Err(SharedMemoryError::InvalidArgument(
                "mapping name must be non-empty ASCII up to 240 bytes",
            ));
        }
        Ok(name.encode_utf16().chain(std::iter::once(0)).collect())
    }

    fn split_len(len: usize) -> Result<(u32, u32), SharedMemoryError> {
        if len == 0 || len > u32::MAX as usize * 2 {
            return Err(SharedMemoryError::InvalidArgument(
                "mapping length is outside the supported range",
            ));
        }
        Ok(((len >> 32) as u32, len as u32))
    }

    pub(super) fn create(name: &str, len: usize) -> Result<SharedMapping, SharedMemoryError> {
        let name = wide(name)?;
        let (high, low) = split_len(len)?;
        // SAFETY: pagefile-backed mapping parameters are validated above and
        // the NUL-terminated name remains live for the duration of the call.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                high,
                low,
                name.as_ptr(),
            )
        };
        if handle.is_null() {
            return Err(SharedMemoryError::Os(std::io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32,
            )));
        }
        map(handle, len)
    }

    pub(super) fn open(name: &str, len: usize) -> Result<SharedMapping, SharedMemoryError> {
        if len == 0 {
            return Err(SharedMemoryError::InvalidArgument(
                "mapping length must be non-zero",
            ));
        }
        let name = wide(name)?;
        // SAFETY: `name` is NUL terminated and valid for the call.
        let handle = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(SharedMemoryError::Os(std::io::Error::from_raw_os_error(
                unsafe { GetLastError() } as i32,
            )));
        }
        map(handle, len)
    }

    fn map(handle: HANDLE, len: usize) -> Result<SharedMapping, SharedMemoryError> {
        // SAFETY: `handle` is a valid mapping handle owned by this function.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, len) };
        if view.Value.is_null() {
            // SAFETY: close the handle created/opened immediately above.
            let error = unsafe { GetLastError() };
            // SAFETY: see above.
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(SharedMemoryError::Os(std::io::Error::from_raw_os_error(
                error as i32,
            )));
        }
        Ok(SharedMapping {
            handle,
            view: view.Value.cast::<u8>(),
            len,
        })
    }

    pub(super) fn with_bytes<R>(mapping: &SharedMapping, read: impl FnOnce(&[u8]) -> R) -> R {
        // SAFETY: `view` is live for `mapping` and no mutable byte closure can
        // coexist because it requires `&mut SharedMapping`.
        read(unsafe { std::slice::from_raw_parts(mapping.view, mapping.len) })
    }

    pub(super) fn with_bytes_mut<R>(
        mapping: &mut SharedMapping,
        write: impl FnOnce(&mut [u8]) -> R,
    ) -> R {
        // SAFETY: `&mut SharedMapping` guarantees exclusive scoped access.
        write(unsafe { std::slice::from_raw_parts_mut(mapping.view, mapping.len) })
    }

    #[allow(dead_code)]
    fn _assert_void(_: *mut c_void) {}
}

#[cfg(not(windows))]
mod platform {
    use super::{SharedMapping, SharedMemoryError};

    pub(super) fn create(_: &str, _: usize) -> Result<SharedMapping, SharedMemoryError> {
        Err(SharedMemoryError::UnsupportedPlatform)
    }
    pub(super) fn open(_: &str, _: usize) -> Result<SharedMapping, SharedMemoryError> {
        Err(SharedMemoryError::UnsupportedPlatform)
    }
    pub(super) fn with_bytes<R>(_: &SharedMapping, _: impl FnOnce(&[u8]) -> R) -> R {
        panic!("Windows shared memory is unavailable")
    }
    pub(super) fn with_bytes_mut<R>(_: &mut SharedMapping, _: impl FnOnce(&mut [u8]) -> R) -> R {
        panic!("Windows shared memory is unavailable")
    }
}
