//! The own name tag extra (`docs/extras.md`): the player's name over their own head in third
//! person, as other players see it.
//!
//! A Horizon crate (docs/layers.md): the engine asks the hooks whether to show it ([`HOOKS`],
//! `own_name_tag`) and still hides it in first person, asleep, as a spectator or while invisible.
//! Its setting, `ownNameTag`, keeps its line in `horizon-extras.txt` ([`SETTING`]) and its row in
//! the Extras page's Third person group ([`SECTION`]). `horizon-official` registers all three.
use horizon_platform::hooks::{Hooks, seat};
use horizon_platform::pages::{Act, Label, Page, Place, Row};
use horizon_platform::settings::Store;
use std::sync::Mutex;

/// The setting's key in `horizon-extras.txt`, as the engine's extras wrote it.
const KEY: &str = "ownNameTag";
const LABEL: &str = "horizon.extras.own_name_tag";
const ABOUT: &str = "horizon.extras.own_name_tag.tooltip";

/// The switch, one per splitscreen seat (`horizon_platform::hooks::seat`), as each player's game
/// had its own before.
pub static HOOKS: OwnNameTag = OwnNameTag {
    seats: Mutex::new(Vec::new()),
};

pub struct OwnNameTag {
    seats: Mutex<Vec<bool>>,
}

impl OwnNameTag {
    /// The `ownNameTag` setting of the player whose turn it is.
    pub fn set_enabled(&self, on: bool) {
        self.with(|s| *s = on);
    }
    /// Whether the player whose turn it is has it on.
    pub fn enabled(&self) -> bool {
        self.with(|s| *s)
    }
    fn with<T>(&self, f: impl FnOnce(&mut bool) -> T) -> T {
        let mut seats = self.seats.lock().unwrap_or_else(|e| e.into_inner());
        let i = seat();
        if seats.len() <= i {
            seats.resize(i + 1, false);
        }
        f(&mut seats[i])
    }
}

impl Hooks for OwnNameTag {
    fn own_name_tag(&self) -> bool {
        self.enabled()
    }
}

/// `ownNameTag` in `horizon-extras.txt`, the player's own file in splitscreen: the game loads and
/// saves it on the player's turn, so it is that seat's.
pub static SETTING: Setting = Setting;

pub struct Setting;

impl Store for Setting {
    fn file(&self) -> &'static str {
        "horizon-extras.txt"
    }
    fn owns(&self, key: &str) -> bool {
        key == KEY
    }
    fn defaults(&self) {
        HOOKS.set_enabled(false);
    }
    fn load(&self, _key: &str, value: &str) {
        if let Ok(on) = value.parse() {
            HOOKS.set_enabled(on);
        }
    }
    fn save(&self, line: &mut dyn FnMut(&str, &str)) {
        line(KEY, if HOOKS.enabled() { "true" } else { "false" });
    }
    fn is_default(&self) -> bool {
        !HOOKS.enabled()
    }
}

/// Its row on the Extras page, in the Third person group it always had.
pub static SECTION: Section = Section;

pub struct Section;

impl Page for Section {
    fn id(&self) -> &'static str {
        "horizon-name-tag.extras"
    }
    fn title(&self) -> &'static str {
        "horizon.extras.header.third_person"
    }
    fn place(&self) -> Place {
        Place::Extras
    }
    fn rows(&self, rows: &mut Vec<Row>) {
        rows.push(Row::Toggle {
            label: LABEL,
            on: HOOKS.enabled(),
        });
    }
    fn act(&self, row: usize, act: Act) {
        if row == 0 && matches!(act, Act::Press | Act::PressBack) {
            HOOKS.set_enabled(!HOOKS.enabled());
        }
    }
    fn tooltip(&self, _row: usize) -> Option<Label> {
        Some(Label::Key(ABOUT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_hooks::engine::set_seat;

    /// The player's line loads into its seat's setting, the hook and the row show it, a press
    /// flips it, and the line saved says what it is now; the other seats keep theirs.
    #[test]
    fn the_setting_loads_shows_flips_and_saves_per_seat() {
        set_seat(1);
        assert!(SETTING.is_default() && SETTING.owns("ownNameTag") && !SETTING.owns("ownNameTags"));
        assert!(!HOOKS.own_name_tag());
        SETTING.load("ownNameTag", "true");
        assert!(HOOKS.own_name_tag());
        let mut rows = Vec::new();
        SECTION.rows(&mut rows);
        let shown = Row::Toggle {
            label: "horizon.extras.own_name_tag",
            on: true,
        };
        assert_eq!(rows, [shown]);
        SECTION.act(0, Act::Press);
        assert!(!HOOKS.own_name_tag());
        let mut saved = Vec::new();
        SETTING.save(&mut |key, value| saved.push(format!("{key}:{value}")));
        assert_eq!(saved, ["ownNameTag:false"]);
        SETTING.load("ownNameTag", "true");
        set_seat(0);
        assert!(!HOOKS.enabled(), "P1 keeps its own");
        set_seat(1);
        assert!(HOOKS.enabled() && !SETTING.is_default());
        // A value that isn't a switch changes nothing.
        SETTING.load("ownNameTag", "yes");
        assert!(HOOKS.enabled());
        // Seat 1 switches to a profile whose file has no `ownNameTag`: the load starts from the
        // default, and seat 0 keeps its own.
        set_seat(0);
        SETTING.load("ownNameTag", "true");
        set_seat(1);
        SETTING.defaults();
        assert!(!HOOKS.own_name_tag() && SETTING.is_default());
        set_seat(0);
        assert!(HOOKS.enabled(), "P1's stays");
        SETTING.defaults();
    }
}
