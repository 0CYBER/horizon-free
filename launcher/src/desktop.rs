//! Linux: the launcher puts itself in the applications menu. It writes
//! `applications/horizon-launcher.desktop` (pointing at wherever the binary is now) and Horizon's
//! icon into the `hicolor` theme, under `$XDG_DATA_HOME` (`~/.local/share` by default), so no root
//! is needed; `horizon.desktop` (hidden from menus) gives the game's window its icon on Wayland.
//! A release build does this when it starts, and again whenever the binary has moved; the
//! Settings switch and `horizon-launcher desktop-entry --remove` take it all away again.
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The icon theme sizes, as `packaging/icons/hicolor` holds them (`cargo run -p xtask -- icons`).
const ICONS: [(&str, &[u8]); 9] = [
    (
        "16x16",
        include_bytes!("../../packaging/icons/hicolor/16x16/apps/horizon.png"),
    ),
    (
        "24x24",
        include_bytes!("../../packaging/icons/hicolor/24x24/apps/horizon.png"),
    ),
    (
        "32x32",
        include_bytes!("../../packaging/icons/hicolor/32x32/apps/horizon.png"),
    ),
    (
        "48x48",
        include_bytes!("../../packaging/icons/hicolor/48x48/apps/horizon.png"),
    ),
    (
        "64x64",
        include_bytes!("../../packaging/icons/hicolor/64x64/apps/horizon.png"),
    ),
    (
        "128x128",
        include_bytes!("../../packaging/icons/hicolor/128x128/apps/horizon.png"),
    ),
    (
        "256x256",
        include_bytes!("../../packaging/icons/hicolor/256x256/apps/horizon.png"),
    ),
    (
        "512x512",
        include_bytes!("../../packaging/icons/hicolor/512x512/apps/horizon.png"),
    ),
    (
        "scalable",
        include_bytes!("../../packaging/icons/hicolor/scalable/apps/horizon.svg"),
    ),
];

const LAUNCHER_ENTRY: &str = "horizon-launcher.desktop";
const GAME_ENTRY: &str = "horizon.desktop";
const GAME_ENTRY_TEXT: &str = include_str!("../../packaging/linux/horizon.desktop");

/// `$XDG_DATA_HOME`, or `~/.local/share` when it is unset or not absolute (as the spec says).
pub fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
}

/// Quotes a path for an `Exec` key: inside double quotes, `"`, `` ` ``, `$` and `\` are escaped.
fn quote(path: &Path) -> String {
    let mut out = String::from("\"");
    for c in path.to_string_lossy().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The launcher's entry, starting the binary at `exe`.
pub fn launcher_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Horizon\n\
         GenericName=Game launcher\n\
         Comment=Install, prepare and play Horizon, Minecraft: Java Edition 26.3 rewritten in Rust\n\
         Exec={}\n\
         TryExec={}\n\
         Icon=horizon\n\
         Terminal=false\n\
         Categories=Game;\n\
         StartupWMClass=horizon-launcher\n",
        quote(exe),
        exe.display()
    )
}

/// Every file this module writes, with its contents, under `data`.
fn files(data: &Path, exe: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = vec![
        (
            data.join("applications").join(LAUNCHER_ENTRY),
            launcher_entry(exe).into_bytes(),
        ),
        (
            data.join("applications").join(GAME_ENTRY),
            GAME_ENTRY_TEXT.as_bytes().to_vec(),
        ),
    ];
    for (size, bytes) in ICONS {
        let name = if size == "scalable" {
            "horizon.svg"
        } else {
            "horizon.png"
        };
        out.push((
            data.join("icons/hicolor")
                .join(size)
                .join("apps")
                .join(name),
            bytes.to_vec(),
        ));
    }
    out
}

/// Writes whatever is missing or out of date (a moved binary changes the entry). Returns
/// whether anything was written.
pub fn install_in(data: &Path, exe: &Path) -> io::Result<bool> {
    let mut wrote = false;
    for (path, bytes) in files(data, exe) {
        if fs::read(&path).ok().as_deref() == Some(&bytes[..]) {
            continue;
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Written beside and renamed, so a menu never reads half an entry.
        let temp = path.with_extension("horizon-tmp");
        fs::write(&temp, &bytes)?;
        fs::rename(&temp, &path)?;
        wrote = true;
    }
    Ok(wrote)
}

/// Removes every file `install_in` writes (the folders stay: other apps share them).
pub fn remove_in(data: &Path) -> io::Result<()> {
    for (path, _) in files(data, Path::new("")) {
        match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Registers the launcher, using a distributor's wrapper when one is configured.
/// Both GUI registration (including the settings switch) and the CLI use this path.
pub fn install(exe: &Path) -> Result<bool, String> {
    let data = data_home().ok_or("No home folder to put the menu entry in.")?;
    let wrapper = std::env::var_os("HORIZON_LAUNCHER_DESKTOP_EXEC")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    install_in(&data, wrapper.as_deref().unwrap_or(exe))
        .map_err(|e| format!("{}: {e}", data.display()))
}

pub fn remove() -> Result<(), String> {
    let data = data_home().ok_or("No home folder.")?;
    remove_in(&data).map_err(|e| format!("{}: {e}", data.display()))
}

/// Whether the launcher's entry is there.
pub fn installed() -> bool {
    data_home().is_some_and(|d| d.join("applications").join(LAUNCHER_ENTRY).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_paths_are_quoted() {
        let entry = launcher_entry(Path::new("/home/a b/Horizon \"x\"/horizon-launcher"));
        assert!(entry.contains("Exec=\"/home/a b/Horizon \\\"x\\\"/horizon-launcher\"\n"));
        assert!(entry.contains("TryExec=/home/a b/Horizon \"x\"/horizon-launcher\n"));
        assert!(entry.contains("Icon=horizon\n"));
    }

    #[test]
    fn installs_follows_a_moved_binary_and_removes() {
        let data = std::env::temp_dir().join(format!("horizon-desktop-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        assert!(install_in(&data, Path::new("/opt/a/horizon-launcher")).unwrap());
        assert!(!install_in(&data, Path::new("/opt/a/horizon-launcher")).unwrap());
        let png = data.join("icons/hicolor/48x48/apps/horizon.png");
        assert!(fs::read(&png).unwrap().starts_with(b"\x89PNG"));
        assert!(data
            .join("icons/hicolor/scalable/apps/horizon.svg")
            .exists());
        assert!(install_in(&data, Path::new("/opt/b/horizon-launcher")).unwrap());
        let entry = fs::read_to_string(data.join("applications").join(LAUNCHER_ENTRY)).unwrap();
        assert!(entry.contains("Exec=\"/opt/b/horizon-launcher\""));
        remove_in(&data).unwrap();
        assert!(!data.join("applications").join(LAUNCHER_ENTRY).exists());
        assert!(!png.exists());
        remove_in(&data).unwrap();
        let _ = fs::remove_dir_all(&data);
    }
}
