//! `horizon-controller.txt` through the settings store (`horizon_hooks::settings`, docs/layers.md
//! "Platform registries"): the file keeps its name and its keys, and each splitscreen player
//! (`horizon_hooks::seat`) reads and writes its own. The engine hands the store the file's lines
//! as it loads a player's settings and asks for them back as it saves.
//!
//! The store owns every line of the file. [`PadSettings::parse`] reads the file as a whole (its
//! layout version, the radial menu's pages), so the store collects the lines and the player's
//! settings are parsed from them once the load ends ([`loaded`]). A line no setting here reads
//! stays in [`PadSettings`]'s own list and is written back.
use std::sync::Mutex;

use horizon_hooks::settings::Store;

use super::{FILE, PadSettings};

/// One player's side of the file.
#[derive(Default)]
struct Seat {
    /// The lines read since the last [`loaded`]; `None` when the file wasn't there.
    read: Option<String>,
    /// The settings the next save writes ([`keep`]).
    saving: PadSettings,
}

/// Every player's side, by seat.
static SEATS: Mutex<Vec<Seat>> = Mutex::new(Vec::new());

fn with_seat<T>(seat: usize, f: impl FnOnce(&mut Seat) -> T) -> T {
    let mut seats = SEATS.lock().unwrap_or_else(|e| e.into_inner());
    if seats.len() <= seat {
        seats.resize_with(seat + 1, Seat::default);
    }
    f(&mut seats[seat])
}

/// The controller settings' store, registered in the game's entry before it starts.
pub struct PadStore;

/// The one store of `horizon-controller.txt`.
pub static STORE: PadStore = PadStore;

impl Store for PadStore {
    fn file(&self) -> &'static str {
        FILE
    }
    fn owns(&self, _key: &str) -> bool {
        true
    }
    fn load(&self, key: &str, value: &str) {
        with_seat(horizon_hooks::seat(), |seat| {
            let read = seat.read.get_or_insert_with(String::new);
            read.push_str(key);
            read.push(':');
            read.push_str(value);
            read.push('\n');
        });
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        let text = with_seat(horizon_hooks::seat(), |seat| seat.saving.serialize());
        for (key, value) in text.lines().filter_map(|l| l.split_once(':')) {
            line(key, value);
        }
    }
    fn is_default(&self) -> bool {
        with_seat(horizon_hooks::seat(), |seat| {
            seat.saving == PadSettings::default()
        })
    }
}

/// The settings of the player whose turn it is, from the lines the stores loaded since the last
/// call: the defaults when the player has no file.
pub fn loaded() -> PadSettings {
    let read = with_seat(horizon_hooks::seat(), |seat| seat.read.take());
    let settings = read.map_or_else(PadSettings::default, |text| PadSettings::parse(&text));
    keep(&settings);
    settings
}

/// What the next save writes for the player whose turn it is.
pub fn keep(settings: &PadSettings) {
    with_seat(horizon_hooks::seat(), |seat| {
        seat.saving.clone_from(settings)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Registers the store once, as the game's entry does.
    fn registered() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| horizon_hooks::settings::register(&STORE).unwrap());
    }

    /// Loads `dir`'s file into `seat` through the engine's side of the stores.
    fn load(dir: &Path, seat: usize) -> PadSettings {
        registered();
        horizon_hooks::engine::set_seat(seat);
        horizon_hooks::settings::engine::load(&mut |file| fs::read_to_string(dir.join(file)).ok());
        loaded()
    }

    /// Saves `settings` for `seat` into `dir` through the engine's side of the stores.
    fn save(dir: &Path, seat: usize, settings: &PadSettings) {
        registered();
        horizon_hooks::engine::set_seat(seat);
        keep(settings);
        horizon_hooks::settings::engine::save_own(dir, &[]);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("horizon-pad-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Each test takes seats of its own: the seats are statics the test threads share.
    #[test]
    fn a_players_file_reads_back_as_it_was_written() {
        let dir = scratch("round-trip");
        // A file as an older build wrote it, with a line this build doesn't read (another
        // branch's crafting screen keeps its views there).
        let before = "bindLayout:1\nbind_key.jump:east\nlookSensitivity:0.7\nvibration:false\n\
                      legacyViews:inventory=console\n";
        fs::write(dir.join(FILE), before).unwrap();
        let read = load(&dir, 4);
        assert_eq!(read, PadSettings::parse(before));
        assert_eq!(read.binding("key.jump"), Some(super::super::Button::East));
        save(&dir, 4, &read);
        let written = fs::read_to_string(dir.join(FILE)).unwrap();
        assert_eq!(
            written,
            read.serialize(),
            "the file is what the settings write"
        );
        assert!(written.contains("legacyViews:inventory=console\n"));
        assert_eq!(load(&dir, 4), read, "and reads back the same");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn each_seat_keeps_its_own_settings() {
        let p1 = scratch("seat-p1");
        let p2 = scratch("seat-p2");
        let theirs = PadSettings {
            invert_y: true,
            ..PadSettings::default()
        };
        save(&p2, 3, &theirs);
        assert_eq!(load(&p1, 2), PadSettings::default());
        save(&p1, 2, &PadSettings::default());
        assert!(!p1.join(FILE).exists(), "defaults make no file");
        assert_eq!(load(&p2, 3), theirs, "P1's turn didn't touch P2's");
        // A seat whose next player has no file starts from the defaults, not the last one's.
        let empty = scratch("seat-empty");
        assert_eq!(load(&empty, 3), PadSettings::default());
        for dir in [p1, p2, empty] {
            let _ = fs::remove_dir_all(dir);
        }
    }
}
