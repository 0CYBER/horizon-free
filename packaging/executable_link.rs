//! Executable link policy shared by source executables and prebuilt SDK metadata.
#[path = "../crates/horizon-guest-stack/src/guest_stack_constants.rs"]
mod stack;

pub fn arguments(target_os: &str, target_env: &str) -> Vec<String> {
    let bytes = stack::GUEST_STACK_BYTES + stack::HOST_HEADROOM;
    match (target_os, target_env) {
        ("windows", "msvc") => vec![format!("/STACK:{bytes}")],
        ("windows", _) => vec![format!("-Wl,--stack,{bytes}")],
        // The main thread's check there can measure only the room below startup's frames, so the
        // stack holds the host headroom once more, as Linux raises its soft limit by it.
        ("macos", _) => vec![format!(
            "-Wl,-stack_size,{:#x}",
            bytes + stack::HOST_HEADROOM
        )],
        _ => Vec::new(),
    }
}
