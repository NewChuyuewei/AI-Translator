//! Minimal, fail-closed single-instance ownership for the desktop process.

use std::{fmt, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceError {
    AlreadyRunning,
    NativeOperationFailed,
}

impl fmt::Display for InstanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyRunning => "应用已在运行",
            Self::NativeOperationFailed => "无法建立单实例保护",
        })
    }
}

impl std::error::Error for InstanceError {}

pub struct InstanceGuard {
    #[cfg(windows)]
    mutex: *mut std::ffi::c_void,
    #[cfg(unix)]
    _file: std::fs::File,
}

impl InstanceGuard {
    /// SWR-NLIFE-002: ownership is acquired before any UI, audio, or network
    /// resource. Ambiguous native failures deny startup instead of allowing a
    /// second process.
    pub fn acquire(lock_path: &Path) -> Result<Self, InstanceError> {
        acquire(lock_path)
    }
}

#[cfg(windows)]
fn acquire(_lock_path: &Path) -> Result<InstanceGuard, InstanceError> {
    use std::ffi::c_void;

    const ERROR_ALREADY_EXISTS: u32 = 183;
    #[link(name = "kernel32")]
    #[allow(non_snake_case)]
    unsafe extern "system" {
        fn CreateMutexW(
            attributes: *const c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    let mut name: Vec<u16> = "Local\\com.gemini.live.translator.instance"
        .encode_utf16()
        .collect();
    name.push(0);
    // SAFETY: pointers are valid for the duration of the call, the name is
    // NUL-terminated, and every successful handle is owned by InstanceGuard.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if mutex.is_null() {
        return Err(InstanceError::NativeOperationFailed);
    }
    // SAFETY: GetLastError immediately observes CreateMutexW's result.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        // SAFETY: mutex is a non-null handle returned by CreateMutexW.
        unsafe { CloseHandle(mutex) };
        return Err(InstanceError::AlreadyRunning);
    }
    Ok(InstanceGuard { mutex })
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        // SAFETY: this guard uniquely owns the non-null mutex handle.
        unsafe { CloseHandle(self.mutex) };
    }
}

#[cfg(unix)]
fn acquire(lock_path: &Path) -> Result<InstanceGuard, InstanceError> {
    use std::fs::OpenOptions;
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};

    const LOCK_EX: i32 = 2;
    const LOCK_NB: i32 = 4;
    unsafe extern "C" {
        fn flock(file_descriptor: i32, operation: i32) -> i32;
    }

    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(lock_path)
        .map_err(|_| InstanceError::NativeOperationFailed)?;
    // SAFETY: the descriptor remains open in InstanceGuard for the full lock
    // lifetime. LOCK_EX | LOCK_NB does not access Rust-managed memory.
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
        return Err(InstanceError::AlreadyRunning);
    }
    Ok(InstanceGuard { _file: file })
}

#[cfg(not(any(unix, windows)))]
fn acquire(_lock_path: &Path) -> Result<InstanceGuard, InstanceError> {
    Err(InstanceError::NativeOperationFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn swr_nlife_002_lock_is_exclusive_and_released_by_guard() {
        let path = std::env::temp_dir().join(format!(
            "translator-instance-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let first = InstanceGuard::acquire(&path).unwrap();
        assert!(matches!(
            InstanceGuard::acquire(&path),
            Err(InstanceError::AlreadyRunning)
        ));
        drop(first);
        InstanceGuard::acquire(&path).unwrap();
        let _ = std::fs::remove_file(path);
    }
}
