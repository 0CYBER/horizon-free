//! Background commands keep their configured streams without opening Windows consoles.
use std::ffi::OsStr;
use std::process::Command;

/// Builds a background command. Callers still choose how to capture its input and output.
pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    hide_console(&mut command);
    command
}

/// Hides a background command's console on Windows, preserving its configured standard streams.
pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}
