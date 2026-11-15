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
        .truncate(false)
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

    #[test]
    fn swr_nlife_003_hotkey_mailbox_is_bounded_and_coalescing() {
        let signal = HotkeySignal::new();
        assert!(signal.sender.try_send(()).is_ok());
        assert!(signal.sender.try_send(()).is_err());
        assert!(signal.take());
        assert!(!signal.take());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotkeyError {
    UnsupportedPlatform,
    RegistrationFailed,
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedPlatform => "当前平台尚未验收全局 F8 快捷键",
            Self::RegistrationFailed => "无法注册全局 F8 快捷键",
        })
    }
}

impl std::error::Error for HotkeyError {}

struct HotkeySignal {
    sender: std::sync::mpsc::SyncSender<()>,
    receiver: std::sync::mpsc::Receiver<()>,
}

impl HotkeySignal {
    fn new() -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        Self { sender, receiver }
    }

    fn take(&self) -> bool {
        self.receiver.try_recv().is_ok()
    }
}

pub struct GlobalHotkey {
    signal: HotkeySignal,
    registration: PlatformHotkey,
}

impl GlobalHotkey {
    /// SWR-NLIFE-003: registration either succeeds as an owned resource or
    /// returns a fixed error. Events enter a capacity-one coalescing mailbox.
    pub fn register_f8() -> Result<Self, HotkeyError> {
        let signal = HotkeySignal::new();
        let registration = PlatformHotkey::register(signal.sender.clone())?;
        Ok(Self {
            signal,
            registration,
        })
    }

    pub fn take_pressed(&self) -> bool {
        self.signal.take()
    }
}

impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        // Keep explicit ownership visible: PlatformHotkey unregisters before
        // the signal receiver is released.
        let _ = &self.registration;
    }
}

#[cfg(target_os = "windows")]
struct PlatformHotkey {
    thread_id: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

#[cfg(target_os = "windows")]
impl PlatformHotkey {
    fn register(sender: std::sync::mpsc::SyncSender<()>) -> Result<Self, HotkeyError> {
        use std::ffi::c_void;

        const HOTKEY_ID: i32 = 0x474C_5408;
        const MOD_NOREPEAT: u32 = 0x4000;
        const VK_F8: u32 = 0x77;
        const WM_HOTKEY: u32 = 0x0312;

        #[repr(C)]
        struct Point {
            x: i32,
            y: i32,
        }
        #[repr(C)]
        struct Message {
            window: *mut c_void,
            message: u32,
            w_param: usize,
            l_param: isize,
            time: u32,
            point: Point,
            private: u32,
        }

        #[link(name = "user32")]
        #[allow(non_snake_case)]
        unsafe extern "system" {
            fn RegisterHotKey(window: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
            fn UnregisterHotKey(window: *mut c_void, id: i32) -> i32;
            fn GetMessageW(message: *mut Message, window: *mut c_void, min: u32, max: u32) -> i32;
        }
        #[link(name = "kernel32")]
        #[allow(non_snake_case)]
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
        }

        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("global-f8-hotkey".into())
            .spawn(move || {
                // SAFETY: this helper thread owns its Win32 message queue and
                // registration. Null HWND requests a thread-global hotkey.
                let thread_id = unsafe { GetCurrentThreadId() };
                let registered = unsafe {
                    RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, MOD_NOREPEAT, VK_F8) != 0
                };
                let _ = ready_sender.send((thread_id, registered));
                if !registered {
                    return;
                }
                loop {
                    let mut message = std::mem::MaybeUninit::<Message>::uninit();
                    // SAFETY: GetMessageW initializes Message before a positive
                    // return. The pointer is valid for the duration of the call.
                    let result = unsafe {
                        GetMessageW(message.as_mut_ptr(), std::ptr::null_mut(), 0, 0)
                    };
                    if result <= 0 {
                        break;
                    }
                    // SAFETY: positive GetMessageW guarantees initialization.
                    let message = unsafe { message.assume_init() };
                    if message.message == WM_HOTKEY && message.w_param == HOTKEY_ID as usize {
                        let _ = sender.try_send(());
                    }
                }
                // SAFETY: this thread owns the matching registration.
                unsafe { UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID) };
            })
            .map_err(|_| HotkeyError::RegistrationFailed)?;
        let (thread_id, registered) = ready_receiver
            .recv()
            .map_err(|_| HotkeyError::RegistrationFailed)?;
        if !registered {
            let _ = join.join();
            return Err(HotkeyError::RegistrationFailed);
        }

        Ok(Self {
            thread_id,
            join: Some(join),
        })
    }
}

#[cfg(target_os = "windows")]
impl Drop for PlatformHotkey {
    fn drop(&mut self) {
        const WM_QUIT: u32 = 0x0012;
        #[link(name = "user32")]
        #[allow(non_snake_case)]
        unsafe extern "system" {
            fn PostThreadMessageW(thread_id: u32, message: u32, w_param: usize, l_param: isize)
                -> i32;
        }
        // SAFETY: thread_id belongs to the live helper thread. Failure is
        // harmless during process teardown; join still preserves ownership.
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(target_os = "macos")]
struct PlatformHotkey {
    hotkey: *mut std::ffi::c_void,
    handler: *mut std::ffi::c_void,
    sender: Box<std::sync::mpsc::SyncSender<()>>,
}

#[cfg(target_os = "macos")]
impl PlatformHotkey {
    fn register(sender: std::sync::mpsc::SyncSender<()>) -> Result<Self, HotkeyError> {
        use std::ffi::c_void;

        type Status = i32;
        type Handler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> Status;
        #[repr(C)]
        struct EventType {
            class: u32,
            kind: u32,
        }
        #[repr(C)]
        struct HotkeyId {
            signature: u32,
            id: u32,
        }

        const EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
        const EVENT_HOTKEY_PRESSED: u32 = 6;
        const KEY_F8: u32 = 100;
        const SIGNATURE: u32 = u32::from_be_bytes(*b"GLTR");

        #[link(name = "Carbon", kind = "framework")]
        #[allow(non_snake_case)]
        unsafe extern "C" {
            fn GetApplicationEventTarget() -> *mut c_void;
            fn InstallEventHandler(
                target: *mut c_void,
                handler: Handler,
                count: u32,
                events: *const EventType,
                user_data: *mut c_void,
                installed: *mut *mut c_void,
            ) -> Status;
            fn RegisterEventHotKey(
                key_code: u32,
                modifiers: u32,
                identifier: HotkeyId,
                target: *mut c_void,
                options: u32,
                registered: *mut *mut c_void,
            ) -> Status;
            fn GetEventParameter(
                event: *mut c_void,
                name: u32,
                desired_type: u32,
                actual_type: *mut u32,
                buffer_size: usize,
                actual_size: *mut usize,
                data: *mut c_void,
            ) -> Status;
            fn UnregisterEventHotKey(hotkey: *mut c_void) -> Status;
            fn RemoveEventHandler(handler: *mut c_void) -> Status;
        }

        unsafe extern "C" fn handle(
            _call: *mut c_void,
            event: *mut c_void,
            user_data: *mut c_void,
        ) -> Status {
            let mut identifier = HotkeyId {
                signature: 0,
                id: 0,
            };
            // SAFETY: Carbon writes exactly one EventHotKeyID into the provided
            // stack value. Null optional size/type outputs are permitted.
            let status = unsafe {
                GetEventParameter(
                    event,
                    u32::from_be_bytes(*b"----"),
                    u32::from_be_bytes(*b"hkid"),
                    std::ptr::null_mut(),
                    std::mem::size_of::<HotkeyId>(),
                    std::ptr::null_mut(),
                    (&mut identifier as *mut HotkeyId).cast(),
                )
            };
            if status != 0
                || identifier.signature != u32::from_be_bytes(*b"GLTR")
                || identifier.id != 8
            {
                return 0;
            }
            // SAFETY: user_data points to the boxed sender retained by
            // PlatformHotkey until after handler removal.
            let sender = unsafe { &*(user_data.cast::<std::sync::mpsc::SyncSender<()>>()) };
            let _ = sender.try_send(());
            0
        }

        let mut sender = Box::new(sender);
        let mut handler = std::ptr::null_mut();
        let event = EventType {
            class: EVENT_CLASS_KEYBOARD,
            kind: EVENT_HOTKEY_PRESSED,
        };
        // SAFETY: all pointers are valid, the sender remains boxed at a stable
        // address, and registration occurs on Slint's AppKit UI thread.
        let handler_status = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                handle,
                1,
                &event,
                (&mut *sender as *mut std::sync::mpsc::SyncSender<()>).cast(),
                &mut handler,
            )
        };
        if handler_status != 0 || handler.is_null() {
            return Err(HotkeyError::RegistrationFailed);
        }
        let mut hotkey = std::ptr::null_mut();
        // SAFETY: target and output pointers are valid; identifier is fixed and
        // contains no user-controlled data.
        let hotkey_status = unsafe {
            RegisterEventHotKey(
                KEY_F8,
                0,
                HotkeyId {
                    signature: SIGNATURE,
                    id: 8,
                },
                GetApplicationEventTarget(),
                0,
                &mut hotkey,
            )
        };
        if hotkey_status != 0 || hotkey.is_null() {
            // SAFETY: handler was successfully installed above.
            unsafe { RemoveEventHandler(handler) };
            return Err(HotkeyError::RegistrationFailed);
        }
        Ok(Self {
            hotkey,
            handler,
            sender,
        })
    }
}

#[cfg(target_os = "macos")]
impl Drop for PlatformHotkey {
    fn drop(&mut self) {
        #[link(name = "Carbon", kind = "framework")]
        #[allow(non_snake_case)]
        unsafe extern "C" {
            fn UnregisterEventHotKey(hotkey: *mut std::ffi::c_void) -> i32;
            fn RemoveEventHandler(handler: *mut std::ffi::c_void) -> i32;
        }
        // SAFETY: both opaque references are uniquely owned and valid until
        // their corresponding unregister operation.
        unsafe {
            UnregisterEventHotKey(self.hotkey);
            RemoveEventHandler(self.handler);
        }
        let _ = &self.sender;
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
struct PlatformHotkey;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl PlatformHotkey {
    fn register(_sender: std::sync::mpsc::SyncSender<()>) -> Result<Self, HotkeyError> {
        Err(HotkeyError::UnsupportedPlatform)
    }
}
