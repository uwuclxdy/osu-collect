#[cfg(windows)]
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use windows_sys::Win32::System::Console::{
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE,
    SetConsoleMode,
};

#[cfg(windows)]
pub fn enable_ansi_support() {
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return;
        }

        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return;
        }

        let new_mode = mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
        SetConsoleMode(handle, new_mode);
    }
}

#[cfg(not(windows))]
pub fn enable_ansi_support() {}
