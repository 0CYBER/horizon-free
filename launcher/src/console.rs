//! A desktop launch has no console; CLI commands can use the invoking terminal or pipes.
#[cfg(windows)]
pub fn attach_for_cli() -> std::io::Result<()> {
    use std::ffi::c_void;

    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_INPUT_HANDLE: u32 = -10_i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
        fn GetStdHandle(kind: u32) -> *mut c_void;
        fn SetStdHandle(kind: u32, handle: *mut c_void) -> i32;
    }

    if std::env::args_os().nth(1).is_none() {
        return Ok(());
    }
    // Attaching can replace the standard handles. Keep inherited pipes/files intact for
    // JSON output, MCP and redirected logs, while filling missing handles from the terminal.
    // SAFETY: These APIs take numeric constants and opaque handles returned by Windows.
    // No handles are closed or dereferenced, and this runs before any worker starts.
    unsafe {
        let handles = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|kind| (kind, GetStdHandle(kind)));
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            // No parent console (for example, an MCP client), or already attached.
            return Ok(());
        }
        for (kind, handle) in handles {
            if handle.is_null() || handle as isize == INVALID_HANDLE_VALUE {
                continue;
            }
            if SetStdHandle(kind, handle) == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn attach_for_cli() -> std::io::Result<()> {
    Ok(())
}
