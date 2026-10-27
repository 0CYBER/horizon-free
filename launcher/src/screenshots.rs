//! An instance's screenshots: the pictures F2 saves into `screenshots/` in its game folder
//! (vanilla's `Screenshot.grab`, named `<yyyy-MM-dd_HH.mm.ss>.png`).
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub path: PathBuf,
    /// Its file name, the id commands take.
    pub name: String,
    /// When the file was written.
    pub taken: SystemTime,
    pub size: u64,
}

/// The folder F2 saves into for the game folder `home`.
pub fn folder(home: &Path) -> PathBuf {
    home.join("screenshots")
}

/// The pictures in `home`'s screenshots folder, the newest first.
pub fn list(home: &Path) -> Vec<Shot> {
    let mut shots: Vec<Shot> = fs::read_dir(folder(home))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let picture = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| ["png", "jpg", "jpeg"].contains(&x.to_ascii_lowercase().as_str()));
            let meta = e.metadata().ok().filter(|m| m.is_file() && picture)?;
            Some(Shot {
                name: e.file_name().to_string_lossy().into_owned(),
                taken: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                size: meta.len(),
                path,
            })
        })
        .collect();
    // Same second: the game names the next ones `_2`, `_3`..., which sort after the first.
    shots.sort_by(|a, b| b.taken.cmp(&a.taken).then_with(|| b.name.cmp(&a.name)));
    shots
}

/// The screenshot a command names: its file name, with or without the extension.
pub fn find(home: &Path, query: &str) -> Result<Shot, String> {
    list(home)
        .into_iter()
        .find(|s| s.name == query || Path::new(&s.name).file_stem().is_some_and(|n| n == query))
        .ok_or_else(|| format!("No screenshot {query} in {}", folder(home).display()))
}

/// Moves the screenshot to the system's trash.
pub fn delete(shot: &Shot) -> Result<(), String> {
    crate::desktop_seams::to_trash(&shot.path)
        .map_err(|e| format!("Could not move {} to the trash: {e}", shot.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn lists_pictures_newest_first() {
        let home =
            std::env::temp_dir().join(format!("horizon-launcher-shots-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        let dir = folder(&home);
        fs::create_dir_all(dir.join("not-a-picture.png")).unwrap();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        for (name, age) in [
            ("2026-10-25_20.00.01.png", 60),
            ("2026-10-27_09.15.42.png", 0),
            ("2026-10-27_09.15.42_2.png", 0),
            ("notes.txt", 0),
            ("Edited.JPG", 30),
        ] {
            let path = dir.join(name);
            fs::write(&path, b"x").unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(base - Duration::from_secs(age)).unwrap();
        }
        let names: Vec<_> = list(&home).into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "2026-10-27_09.15.42_2.png",
                "2026-10-27_09.15.42.png",
                "Edited.JPG",
                "2026-10-25_20.00.01.png"
            ]
        );
        assert_eq!(find(&home, "2026-10-25_20.00.01").unwrap().size, 1);
        assert_eq!(find(&home, "Edited.JPG").unwrap().name, "Edited.JPG");
        assert!(find(&home, "notes.txt").is_err());
        assert!(list(&home.join("none")).is_empty());
        fs::remove_dir_all(&home).unwrap();
    }
}
