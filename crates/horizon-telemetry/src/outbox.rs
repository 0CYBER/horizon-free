//! Items waiting to be sent: one small JSON file each in `telemetry/outbox/`, written by the game
//! and the launcher, sent and removed by the launcher's [`crate::upload::flush`]. Only kinds the
//! player allows get in, each within its daily cap, and the folder never holds more than
//! [`MAX_FILES`] items or [`MAX_BYTES`].
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Consent, Item, Kind};

pub const MAX_FILES: usize = 200;
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub fn dir(home: &Path) -> PathBuf {
    crate::dir(home).join("outbox")
}

/// How many items of each kind were kept today, so a crash loop doesn't fill the outbox.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Kept {
    day: u32,
    counts: [u32; 4],
}

fn kept_path(home: &Path) -> PathBuf {
    crate::dir(home).join("kept.json")
}

/// Keeps `item` to be sent later if the player allows its kind and today's cap for it isn't
/// reached; returns whether it was kept. An invalid item (too large…) is an error.
pub fn push(home: &Path, item: &Item) -> std::io::Result<bool> {
    let kind = item.kind();
    if !Consent::load(home).allows(kind) {
        return Ok(false);
    }
    item.validate().map_err(std::io::Error::other)?;
    let today = crate::today();
    let mut kept: Kept = std::fs::read(kept_path(home))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|k: &Kept| k.day == today)
        .unwrap_or(Kept {
            day: today,
            ..Kept::default()
        });
    let slot = Kind::ALL.iter().position(|&k| k == kind).unwrap_or(0);
    if kept.counts[slot] >= kind.daily_cap() {
        return Ok(false);
    }
    let json = serde_json::to_vec(item).map_err(std::io::Error::other)?;
    let dir = dir(home);
    std::fs::create_dir_all(&dir)?;
    make_room(&dir, json.len() as u64)?;
    // Names sort by day, so the oldest go first when there is no room.
    let name = format!(
        "{:06}-{}-{}.json",
        item.day,
        kind.name(),
        crate::random_hex(6)
    );
    crate::write_atomic(&dir.join(name), &json)?;
    kept.counts[slot] += 1;
    let kept = serde_json::to_vec(&kept).map_err(std::io::Error::other)?;
    crate::write_atomic(&kept_path(home), &kept)?;
    Ok(true)
}

fn files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut files: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| (e.path(), e.metadata().map_or(0, |m| m.len())))
        .collect();
    files.sort();
    files
}

/// Removes the oldest items until one more of `incoming` bytes fits.
fn make_room(dir: &Path, incoming: u64) -> std::io::Result<()> {
    let files = files(dir);
    let mut count = files.len();
    let mut bytes: u64 = files.iter().map(|f| f.1).sum();
    for (path, size) in files {
        if count < MAX_FILES && bytes + incoming <= MAX_BYTES {
            break;
        }
        std::fs::remove_file(path)?;
        count -= 1;
        bytes -= size;
    }
    Ok(())
}

/// The items waiting, oldest first, with their files. Unreadable files are removed.
pub fn pending(home: &Path) -> Vec<(PathBuf, Item)> {
    files(&dir(home))
        .into_iter()
        .filter_map(|(path, _)| {
            let item = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Item>(&b).ok());
            if item.is_none() {
                let _ = std::fs::remove_file(&path);
            }
            item.map(|i| (path, i))
        })
        .collect()
}

/// The waiting items the player still allows, oldest first; the others are deleted, never sent.
pub fn retain_allowed(home: &Path) -> Vec<(PathBuf, Item)> {
    let consent = Consent::load(home);
    pending(home)
        .into_iter()
        .filter(|(path, item)| {
            let allowed = consent.allows(item.kind());
            if !allowed {
                let _ = std::fs::remove_file(path);
            }
            allowed
        })
        .collect()
}

/// Removes every waiting item ("Delete my data", or everything turned off).
pub fn clear(home: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir(home)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{Bug, Crash, Payload, Source};
    use crate::{Choices, test_home};

    fn allow(home: &Path, choices: Choices) {
        let mut consent = Consent::load(home);
        consent.answer(choices);
        consent.save(home).unwrap();
    }

    fn crash() -> Item {
        Item::new(Source::Game, "v1", Payload::Crash(Crash::default()))
    }

    #[test]
    fn nothing_is_kept_without_consent_for_its_kind() {
        let home = test_home::new("outbox-consent");
        assert!(!push(&home, &crash()).unwrap());
        allow(
            &home,
            Choices {
                bugs: true,
                ..Choices::NONE
            },
        );
        assert!(!push(&home, &crash()).unwrap());
        assert!(pending(&home).is_empty());
        assert!(!dir(&home).exists());
        allow(
            &home,
            Choices {
                crashes: true,
                ..Choices::NONE
            },
        );
        assert!(push(&home, &crash()).unwrap());
        assert_eq!(pending(&home).len(), 1);
        assert_eq!(pending(&home)[0].1, crash());
    }

    #[test]
    fn each_kind_has_a_daily_cap() {
        let home = test_home::new("outbox-cap");
        allow(&home, Choices::ALL);
        let kept = (0..20).filter(|_| push(&home, &crash()).unwrap()).count();
        assert_eq!(kept as u32, Kind::Crash.daily_cap());
        // Another kind still gets in.
        let bug = Item::new(
            Source::Launcher,
            "0.1.5",
            Payload::Bug(Bug {
                text: "x".into(),
                ..Bug::default()
            }),
        );
        assert!(push(&home, &bug).unwrap());
    }

    #[test]
    fn the_oldest_items_make_room() {
        let home = test_home::new("outbox-room");
        allow(&home, Choices::ALL);
        let dir = dir(&home);
        std::fs::create_dir_all(&dir).unwrap();
        let old = serde_json::to_vec(&crash()).unwrap();
        for i in 0..MAX_FILES {
            std::fs::write(dir.join(format!("000001-crash-{i:04}.json")), &old).unwrap();
        }
        assert!(push(&home, &crash()).unwrap());
        let files = files(&dir);
        assert_eq!(files.len(), MAX_FILES);
        assert!(!dir.join("000001-crash-0000.json").exists());
        assert!(
            files
                .last()
                .unwrap()
                .0
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("{:06}", crate::today()))
        );
    }

    #[test]
    fn invalid_items_and_broken_files_are_refused() {
        let home = test_home::new("outbox-invalid");
        allow(&home, Choices::ALL);
        let empty_bug = Item::new(Source::Game, "v1", Payload::Bug(Bug::default()));
        assert!(push(&home, &empty_bug).is_err());
        std::fs::create_dir_all(dir(&home)).unwrap();
        std::fs::write(dir(&home).join("000001-bug-x.json"), b"{").unwrap();
        assert!(pending(&home).is_empty());
        assert!(files(&dir(&home)).is_empty());
        clear(&home).unwrap();
        clear(&home).unwrap();
    }
}
