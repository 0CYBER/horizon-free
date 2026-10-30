//! The packs in the game's `shaderpacks` folder: every folder and `.zip` in it (spec 1.1).

use std::fs;
use std::path::{Path, PathBuf};

use crate::Pack;

/// One entry of the `shaderpacks` folder that is or looks like a pack.
#[derive(Debug, PartialEq)]
pub struct Found {
    /// The folder or file name, which is how players and settings name the pack.
    pub name: String,
    pub path: PathBuf,
    /// Why the entry can't be used, said to the player (a zip that doesn't open, a folder with
    /// no `shaders` folder at its root). `None` for a usable pack.
    pub problem: Option<String>,
}

/// The packs in `dir`, sorted by name ignoring case. Hidden entries and files that are not
/// `.zip` (the game's own option files live beside the packs) are skipped.
pub fn discover(dir: &Path) -> Vec<Found> {
    let mut found: Vec<Found> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let is_zip = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"));
            if name.starts_with('.') || !(path.is_dir() || is_zip) {
                return None;
            }
            let problem = Pack::open(&path).err().map(|e| e.to_string());
            Some(Found {
                name,
                path,
                problem,
            })
        })
        .collect();
    found.sort_by_cached_key(|f| (f.name.to_lowercase(), f.name.clone()));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::tests::{temp_dir, zip};

    #[test]
    fn lists_folders_and_zips_with_their_problems() {
        let dir = temp_dir("discover");
        fs::create_dir_all(dir.join("Zeta/shaders")).unwrap();
        fs::create_dir_all(dir.join("Extracted wrong/Inner/shaders")).unwrap();
        fs::create_dir_all(dir.join(".hidden/shaders")).unwrap();
        fs::write(dir.join("alpha.ZIP"), zip(&[("shaders/final.fsh", "")])).unwrap();
        fs::write(dir.join("broken.zip"), "not a zip").unwrap();
        fs::write(dir.join("alpha.ZIP.txt"), "OPTION=1").unwrap();
        let found = discover(&dir);
        let summary: Vec<(&str, bool)> = found
            .iter()
            .map(|f| (f.name.as_str(), f.problem.is_none()))
            .collect();
        assert_eq!(
            summary,
            [
                ("alpha.ZIP", true),
                ("broken.zip", false),
                ("Extracted wrong", false),
                ("Zeta", true)
            ]
        );
        assert_eq!(
            found[2].problem.as_deref(),
            Some("no shaders folder at the root of the pack")
        );
        assert!(discover(&dir.join("missing")).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
