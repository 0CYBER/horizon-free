//! Starts each test binary in a scratch working directory of its own so tests that
//! default their game folder to `std::env::current_dir()` never write stray files
//! into the crate source tree or share files across test binaries.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static ORIGINAL_DIR: OnceLock<PathBuf> = OnceLock::new();
static ISOLATED_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The per-binary scratch game directory set at process start, if this process is a
/// test binary started from its crate's `CARGO_MANIFEST_DIR`.
pub fn isolated_dir() -> Option<&'static Path> {
    ISOLATED_DIR.get().map(PathBuf::as_path)
}

/// References the startup hook so the linker retains this module's init entry.
#[inline(never)]
pub fn ensure_linked() {
    #[cfg(any(
        all(unix, not(target_vendor = "apple")),
        target_vendor = "apple",
        target_os = "windows"
    ))]
    let _ = std::hint::black_box(INIT_TEST_GAME_DIR as usize);
}

fn same_directory(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

fn is_cargo_test_binary(exe: &Path) -> bool {
    if exe
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "examples")
    {
        return false;
    }
    exe.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.rsplit_once('-'))
        .is_some_and(|(_, hash)| hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn should_isolate(exe: &Path, cwd: &Path, manifest_dir: Option<&Path>) -> bool {
    if std::env::var_os("HORIZON_KEEP_TEST_CWD").is_some() {
        return false;
    }
    let Some(manifest_dir) = manifest_dir else {
        return false;
    };
    is_cargo_test_binary(exe) && same_directory(cwd, manifest_dir)
}

fn dir_name_for(exe: &Path, pid: u32) -> String {
    let stem = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("test")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("horizon-test-cwd-{stem}-{pid}")
}

extern "C" fn cleanup_test_game_dir() {
    let _ = std::panic::catch_unwind(|| {
        if let Some(dir) = ISOLATED_DIR.get() {
            if let Some(original) = ORIGINAL_DIR.get() {
                let _ = std::env::set_current_dir(original);
            } else {
                let _ = std::env::set_current_dir(std::env::temp_dir());
            }
            let _ = std::fs::remove_dir_all(dir);
        }
    });
}

fn register_cleanup() {
    #[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
    {
        // SAFETY: `cleanup_test_game_dir` is an `extern "C" fn()` that catches panics
        // and only performs fallible filesystem cleanup on process exit.
        unsafe {
            libc::atexit(cleanup_test_game_dir);
        }
    }
    #[cfg(target_os = "windows")]
    {
        unsafe extern "C" {
            fn atexit(func: extern "C" fn()) -> i32;
        }
        // SAFETY: `cleanup_test_game_dir` is an `extern "C" fn()` that catches panics
        // and only performs fallible filesystem cleanup on process exit.
        unsafe {
            atexit(cleanup_test_game_dir);
        }
    }
}

extern "C" fn init_test_game_dir() {
    let _ = std::panic::catch_unwind(|| {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let Ok(cwd) = std::env::current_dir() else {
            return;
        };
        let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from);
        if !should_isolate(&exe, &cwd, manifest_dir.as_deref()) {
            return;
        }
        let dir = std::env::temp_dir().join(dir_name_for(&exe, std::process::id()));
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        if std::env::set_current_dir(&dir).is_ok() {
            let _ = ORIGINAL_DIR.set(cwd);
            let _ = ISOLATED_DIR.set(dir);
            register_cleanup();
        }
    });
}

#[cfg(all(unix, not(target_vendor = "apple")))]
#[used]
#[unsafe(link_section = ".init_array")]
static INIT_TEST_GAME_DIR: extern "C" fn() = init_test_game_dir;

#[cfg(target_vendor = "apple")]
#[used]
#[unsafe(link_section = "__DATA,__mod_init_func")]
static INIT_TEST_GAME_DIR: extern "C" fn() = init_test_game_dir;

#[cfg(target_os = "windows")]
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static INIT_TEST_GAME_DIR: extern "C" fn() = init_test_game_dir;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolates_only_cargo_test_binaries_in_manifest_dir() {
        let manifest = Path::new("/repo/crates/horizon-client");
        assert!(should_isolate(
            Path::new("/repo/target/debug/deps/horizon_client-0123456789abcdef"),
            manifest,
            Some(manifest),
        ));
        assert!(should_isolate(
            Path::new(
                "/home/runner/horizon-target/debug/build/horizon-client/c8c62f4a6e80b8cf/out/horizon_client-c8c62f4a6e80b8cf"
            ),
            manifest,
            Some(manifest),
        ));
        // Examples and normal binaries keep their caller's working directory.
        assert!(!should_isolate(
            Path::new("/repo/target/debug/examples/client_probe"),
            manifest,
            Some(manifest),
        ));
        assert!(!should_isolate(
            Path::new("/repo/target/debug/examples/client_probe-0123456789abcdef"),
            manifest,
            Some(manifest),
        ));
        assert!(!should_isolate(
            Path::new("/repo/target/debug/horizon-ctl"),
            manifest,
            Some(manifest),
        ));
        // Explicit working directory outside CARGO_MANIFEST_DIR is preserved.
        assert!(!should_isolate(
            Path::new("/repo/target/debug/deps/horizon_client-0123456789abcdef"),
            Path::new("/tmp/custom-game-dir"),
            Some(manifest),
        ));
    }
}
