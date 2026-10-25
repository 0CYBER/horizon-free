//! The programs the server runs. Started by the launcher, the server has no console, and each
//! console program it started would open a terminal window on Windows: those open none. Run from
//! a terminal, they share it, so what they print still reaches it.
use std::{ffi::OsStr, process::Command};

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::{Console::GetConsoleWindow, Threading::CREATE_NO_WINDOW};
        let mut command = command;
        // SAFETY: no arguments; it returns this process's console window or null.
        if unsafe { GetConsoleWindow() }.is_null() {
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    };
    command
}
