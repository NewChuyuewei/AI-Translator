//! Audited native window operations for the Slint caption overlay.
//!
//! The desktop shell denies unsafe code. All unavoidable platform FFI is
//! isolated here behind one operation with content-free errors.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptionPlatformError {
    HandleUnavailable,
    UnsupportedPlatform,
    NativeOperationFailed,
}

impl fmt::Display for CaptionPlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::HandleUnavailable => "字幕窗口句柄尚不可用",
            Self::UnsupportedPlatform => "当前平台尚未验收鼠标穿透",
            Self::NativeOperationFailed => "字幕窗口平台操作失败",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CaptionPlatformError {}

/// Tracks the last successfully applied state so failures cannot create a
/// false UI state. New instances are fail-closed and interactive.
#[derive(Debug, Default)]
pub struct MousePassthrough {
    active: bool,
}

impl MousePassthrough {
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// SWR-NCAP-002: change only the native input-transparency bit. State is
    /// committed only after the platform operation succeeds.
    pub fn set(
        &mut self,
        provider: &impl HasWindowHandle,
        enabled: bool,
    ) -> Result<(), CaptionPlatformError> {
        if self.active == enabled {
            return Ok(());
        }
        let handle = provider
            .window_handle()
            .map_err(|_| CaptionPlatformError::HandleUnavailable)?;
        self.transition(enabled, || apply(handle.as_raw(), enabled))
    }

    fn transition(
        &mut self,
        enabled: bool,
        operation: impl FnOnce() -> Result<(), CaptionPlatformError>,
    ) -> Result<(), CaptionPlatformError> {
        operation()?;
        self.active = enabled;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn apply(handle: RawWindowHandle, enabled: bool) -> Result<(), CaptionPlatformError> {
    use std::ffi::c_void;

    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TRANSPARENT: isize = 0x0000_0020;

    #[allow(non_snake_case)]
    unsafe extern "system" {
        fn GetWindowLongPtrW(window: *mut c_void, index: i32) -> isize;
        fn SetWindowLongPtrW(window: *mut c_void, index: i32, value: isize) -> isize;
        fn GetLastError() -> u32;
        fn SetLastError(error: u32);
    }

    let RawWindowHandle::Win32(handle) = handle else {
        return Err(CaptionPlatformError::HandleUnavailable);
    };
    let window = handle.hwnd.get() as *mut c_void;

    // SAFETY: `window` is supplied by the active Slint window adapter. The
    // calls preserve every extended style except WS_EX_TRANSPARENT, and the
    // Win32 zero-return ambiguity is resolved through GetLastError.
    unsafe {
        SetLastError(0);
        let current = GetWindowLongPtrW(window, GWL_EXSTYLE);
        if current == 0 && GetLastError() != 0 {
            return Err(CaptionPlatformError::NativeOperationFailed);
        }
        let requested = if enabled {
            current | WS_EX_TRANSPARENT
        } else {
            current & !WS_EX_TRANSPARENT
        };
        if requested == current {
            return Ok(());
        }
        SetLastError(0);
        let previous = SetWindowLongPtrW(window, GWL_EXSTYLE, requested);
        if previous == 0 && GetLastError() != 0 {
            return Err(CaptionPlatformError::NativeOperationFailed);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply(handle: RawWindowHandle, enabled: bool) -> Result<(), CaptionPlatformError> {
    use std::ffi::{c_char, c_void};

    #[allow(non_snake_case)]
    unsafe extern "C" {
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }

    let RawWindowHandle::AppKit(handle) = handle else {
        return Err(CaptionPlatformError::HandleUnavailable);
    };
    let view = handle.ns_view.as_ptr();

    // SAFETY: AppKitWindowHandle guarantees `ns_view` is an NSView. Selectors
    // are static NUL-terminated strings. Both messages run on Slint's UI
    // thread, and no Objective-C object is retained beyond this call.
    unsafe {
        let window_selector = sel_registerName(c"window".as_ptr());
        let get_window: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        let window = get_window(view, window_selector);
        if window.is_null() {
            return Err(CaptionPlatformError::HandleUnavailable);
        }
        let passthrough_selector = sel_registerName(c"setIgnoresMouseEvents:".as_ptr());
        let set_passthrough: unsafe extern "C" fn(*mut c_void, *mut c_void, bool) =
            std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
        set_passthrough(window, passthrough_selector, enabled);
    }
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn apply(_handle: RawWindowHandle, enabled: bool) -> Result<(), CaptionPlatformError> {
    if enabled {
        Err(CaptionPlatformError::UnsupportedPlatform)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swr_ncap_002_failed_enable_remains_interactive() {
        let mut state = MousePassthrough::default();
        let result = state.transition(true, || Err(CaptionPlatformError::NativeOperationFailed));
        assert_eq!(result, Err(CaptionPlatformError::NativeOperationFailed));
        assert!(!state.is_active());
    }

    #[test]
    fn swr_ncap_002_failed_disable_never_reports_recovery() {
        let mut state = MousePassthrough::default();
        state.transition(true, || Ok(())).unwrap();
        let result = state.transition(false, || Err(CaptionPlatformError::NativeOperationFailed));
        assert_eq!(result, Err(CaptionPlatformError::NativeOperationFailed));
        assert!(state.is_active());
    }

    #[test]
    fn swr_ncap_002_success_commits_each_transition() {
        let mut state = MousePassthrough::default();
        state.transition(true, || Ok(())).unwrap();
        assert!(state.is_active());
        state.transition(false, || Ok(())).unwrap();
        assert!(!state.is_active());
    }
}
