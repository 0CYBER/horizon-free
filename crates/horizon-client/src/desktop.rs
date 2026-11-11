//! Opening a folder or file in the desktop's own handler, as `Blaze3D.openPath` does: vanilla
//! hands `path.normalize().toUri()` to `SDL_OpenURL`, which on Windows is ShellExecute's `open`
//! verb, so a folder opens in the user's default file manager (File Explorer when none is set).
//! Starting `explorer` directly, or `SHOpenFolderAndSelectItems` (which the `open` crate uses for
//! a directory), always opens File Explorer (#15259).
//!
//! Paths are resolved in the game's filesystem context before they are handed over: a desktop
//! process may not inherit Windows AppData virtualization.
#[cfg(not(windows))]
use std::path::PathBuf;
use std::{io, path::Path};

/// How a path is handed to the desktop.
#[derive(Debug, PartialEq, Eq)]
pub enum Opener {
    /// ShellExecute's default verb on this `file:` URI (Windows).
    #[cfg(windows)]
    Shell(String),
    /// The desktop opener (`open` on macOS, `xdg-open` elsewhere) with this path.
    #[cfg(not(windows))]
    Program(&'static str, PathBuf),
}

/// The opener for `path`, which must exist. It names the path's physical location; on Windows
/// without the verbatim (`\\?\`) prefix where a plain path names it, as `Path.toUri` would.
pub fn opener(path: &Path) -> io::Result<Opener> {
    #[cfg(windows)]
    {
        let physical = dunce::canonicalize(path)?;
        Ok(Opener::Shell(file_uri(&physical, physical.is_dir())))
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        Ok(Opener::Program(program, std::fs::canonicalize(path)?))
    }
}

/// Opens `path` without waiting for its handler.
pub fn open_path(path: &Path) -> io::Result<()> {
    match opener(path)? {
        // ShellExecuteExW with the default verb: `open` only takes its folder shortcut
        // (SHOpenFolderAndSelectItems) for a path that is a directory, never for a URI.
        #[cfg(windows)]
        Opener::Shell(uri) => open::that_detached(uri),
        #[cfg(not(windows))]
        Opener::Program(program, path) => {
            let mut child = std::process::Command::new(program)
                .arg(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            // Reap the desktop opener without blocking the UI thread.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
    }
}

/// `Path.toUri` of an absolute Windows path: `file:///C:/a%20b/` (a share is
/// `file://server/share/`), a directory with a trailing slash. The characters a URI path can't
/// hold are escaped; other characters, non-ASCII ones too, stay as they are.
#[cfg_attr(not(windows), allow(dead_code))]
fn file_uri(path: &Path, directory: bool) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut uri = String::from(if text.starts_with("//") {
        "file:"
    } else {
        "file:///"
    });
    for c in text.chars() {
        match c {
            ' ' | '"' | '#' | '%' | '<' | '>' | '?' | '[' | ']' | '^' | '`' | '{' | '|' | '}' => {
                uri.push_str(&format!("%{:02X}", c as u32));
            }
            c if c.is_ascii_control() => uri.push_str(&format!("%{:02X}", c as u32)),
            c => uri.push(c),
        }
    }
    if directory && !uri.ends_with('/') {
        uri.push('/');
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "horizon-desktop-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn a_uri_escapes_what_a_path_cannot_hold_and_keeps_the_rest() {
        let path = Path::new("C:\\Users\\a b\\packs & textures #1 50% [x] \u{2603}");
        assert_eq!(
            file_uri(path, true),
            "file:///C:/Users/a%20b/packs%20&%20textures%20%231%2050%25%20%5Bx%5D%20\u{2603}/"
        );
        assert_eq!(
            file_uri(Path::new(r"C:\shots\2026-11-11_10.00.00.png"), false),
            "file:///C:/shots/2026-11-11_10.00.00.png"
        );
        assert_eq!(
            file_uri(Path::new(r"\\server\share\packs"), true),
            "file://server/share/packs/"
        );
    }

    /// The folder goes to the shell as a `file:` URI (ShellExecute's association), not to
    /// `explorer`, and names the folder's one physical path.
    #[cfg(windows)]
    #[test]
    fn windows_hands_the_shell_the_folder_as_a_file_uri() {
        let scratch = Scratch::new();
        let folder = scratch.0.join("packs & textures \u{2603}");
        fs::create_dir(&folder).unwrap();
        let logical = folder.join("..").join("packs & textures \u{2603}");
        let Opener::Shell(uri) = opener(&logical).unwrap();
        let physical = dunce::canonicalize(&folder).unwrap();
        assert_eq!(uri, file_uri(&physical, true));
        assert!(uri.starts_with("file:///"));
        assert!(!uri.contains("/../"));
        assert!(!uri.contains("?"));
        assert!(uri.ends_with("packs%20&%20textures%20\u{2603}/"));
    }

    #[cfg(not(windows))]
    #[test]
    fn the_desktop_opener_gets_the_folder_s_physical_path() {
        let scratch = Scratch::new();
        let folder = scratch.0.join("packs");
        fs::create_dir(&folder).unwrap();
        let logical = folder.join("..").join("packs");
        let Opener::Program(_, path) = opener(&logical).unwrap();
        assert_eq!(path, fs::canonicalize(&folder).unwrap());
    }

    #[test]
    fn a_missing_folder_is_not_handed_over() {
        let scratch = Scratch::new();
        assert_eq!(
            opener(&scratch.0.join("missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
}
