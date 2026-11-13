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
        Opener::Program(program, path) => spawn(program, path.as_os_str()),
    }
}

/// `Blaze3D.openUri`: the desktop's handler for a link, which is handed over as it is (no path
/// is resolved). On Windows ShellExecute keeps it one Unicode value, without passing its query
/// string through a command interpreter.
pub fn open_uri(uri: &str) -> io::Result<()> {
    #[cfg(windows)]
    return open::that_detached(uri);
    #[cfg(not(windows))]
    spawn(
        if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        },
        std::ffi::OsStr::new(uri),
    )
}

/// Starts the desktop opener on `arg` and reaps it without blocking the UI thread.
#[cfg(not(windows))]
fn spawn(program: &str, arg: &std::ffi::OsStr) -> io::Result<()> {
    let mut child = std::process::Command::new(program)
        .arg(arg)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// `Path.toUri` of an absolute Windows path: `file:///C:/a%20b/` (a share is
/// `file://server/share/`), a directory with a trailing slash. The characters a URI path can't
/// hold are escaped; other characters, non-ASCII ones too, stay as they are.
#[cfg_attr(not(windows), allow(dead_code))]
fn file_uri(path: &Path, directory: bool) -> String {
    // dunce leaves a share (a mapped drive canonicalizes to one), a path past MAX_PATH or one
    // with a reserved name verbatim: `\\?\UNC\server\share` is `\\server\share`, `\\?\C:\` is `C:\`.
    let text = path.to_string_lossy();
    let text = if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{share}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
    let text = text.replace('\\', "/");
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

    /// What dunce leaves verbatim (a share or mapped drive, a path past MAX_PATH) names the
    /// same location as its plain form: the share's host, the drive's path.
    #[test]
    fn a_verbatim_path_gives_the_uri_of_its_plain_form() {
        assert_eq!(
            file_uri(Path::new(r"\\?\UNC\server\share\packs"), true),
            "file://server/share/packs/"
        );
        assert_eq!(
            file_uri(Path::new(r"\\?\C:\games\resourcepacks"), true),
            "file:///C:/games/resourcepacks/"
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
