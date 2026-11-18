//! `host.request_world(folder)`: a native mod asks to open one of its player's worlds from the title screen. The
//! request is queued, and the next title frame asks the player, naming the mod and the save; Yes opens it through
//! Play's own checks (`Title::ask_world_request`, `play`), No drops it. Nothing happens inside the mod's call.
//! Off the title, without a bundle's `world_control`, or for a folder that is not an existing save under
//! `saves/`, the answer is a code and nothing is touched.
use super::*;
use horizon_client::title::{Mode, world_request_entry};
use horizon_mod_package::mod_list;
use mod_api::NativeWorldResult;
use std::path::Path;

/// The longest folder name a request may name.
const MAX_FOLDER: usize = 64;

/// Whether `folder` is one plain path segment: 1 to 64 characters, no separator or colon, no control character,
/// and no leading dot, so no `.`, `..` or hidden folder.
pub(crate) fn valid_folder(folder: &str) -> bool {
    (1..=MAX_FOLDER).contains(&folder.chars().count())
        && !folder.starts_with('.')
        && folder
            .chars()
            .all(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
}

/// Where the title is, for a request: its main page, the question a request already put up, or anywhere else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TitleState {
    Main,
    Asking,
    Elsewhere,
}

/// A request the title's next frame takes: the folder, and the mods it is asked for, by name, for the question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) folder: String,
    pub(crate) mods: String,
}

/// The queue's decision, apart from the game: `title` where the title is, whether a world entry is already waiting
/// for the world resources (`entering`), `granted` the name the question shows when the caller may ask (None when it
/// may not; asked only once the title and the queue allow a request), the `saves` folder, and the waiting request.
/// A folder counts only when it is an existing save (`world_request_entry`): an empty folder is not one, and nothing
/// is created.
fn queue(
    title: TitleState,
    entering: bool,
    granted: impl FnOnce() -> Option<String>,
    saves: &Path,
    folder: &str,
    pending: &mut Option<Request>,
) -> NativeWorldResult {
    if title == TitleState::Asking || pending.is_some() || entering {
        return NativeWorldResult::Busy;
    }
    if title != TitleState::Main {
        return NativeWorldResult::NotTitle;
    }
    let Some(mods) = granted() else {
        return NativeWorldResult::NotPermitted;
    };
    if !valid_folder(folder) || world_request_entry(saves, folder).is_none() {
        return NativeWorldResult::InvalidRequest;
    }
    *pending = Some(Request {
        folder: folder.to_owned(),
        mods,
    });
    NativeWorldResult::Queued
}

/// The name the question shows for a request from `caller`, when the caller's own bundle may grant it: `id` and
/// `name` are that bundle's manifest identity, `declares` its `[native] world_control`, `disabled` whether the
/// player turned it off. An unidentified caller (None) is granted nothing, whatever other bundles declare, and no
/// other bundle grants it.
fn granted_name(
    caller: Option<&str>,
    id: &str,
    name: &str,
    declares: bool,
    disabled: bool,
) -> Option<String> {
    (caller == Some(id) && declares && !disabled).then(|| name.to_owned())
}

impl Game<'_> {
    fn title_state(&self) -> TitleState {
        if self.screen != Screen::Playing {
            return TitleState::Elsewhere;
        }
        match self.title.as_ref().map(|title| &title.mode) {
            Some(Mode::Main) => TitleState::Main,
            Some(Mode::WorldRequest { .. }) => TitleState::Asking,
            _ => TitleState::Elsewhere,
        }
    }

    /// The name the question shows for the calling mod, when its own bundle grants `world_control`: it asks by its
    /// manifest name. None when the caller is unidentified, or its bundle does not grant it. Only the manifests of
    /// the bundles are read (never their code or resources), and only until the caller's is found.
    fn world_control_mod(&self) -> Option<String> {
        let caller = mod_api::content::crash::current_native_mod()?;
        let game_dir = Self::game_dir();
        [mod_list::DIR, mod_list::NATIVE_DIR]
            .iter()
            .filter_map(|dir| std::fs::read_dir(game_dir.join(dir)).ok())
            .flatten()
            .flatten()
            .filter_map(|entry| {
                horizon_mod_package::index::native::read_manifest(&entry.path()).ok()
            })
            .find_map(|manifest| {
                let declares = manifest
                    .native
                    .as_ref()
                    .is_some_and(|native| native.world_control);
                granted_name(
                    Some(caller),
                    &manifest.id,
                    &manifest.name,
                    declares,
                    self.settings.mods.disabled.contains(&manifest.id),
                )
            })
    }

    /// For a native mod: queue asking the player to open `saves/<folder>` on the title screen's next frame.
    pub(in crate::game) fn request_world(&mut self, folder: &str) -> NativeWorldResult {
        let saves = Self::game_dir().join("saves");
        let title = self.title_state();
        let entering = self.pending_entry.is_some();
        // The queue slot is taken out so the grant can read the game beside it.
        let mut pending = self.native_world_request.take();
        let answer = queue(
            title,
            entering,
            || self.world_control_mod(),
            &saves,
            folder,
            &mut pending,
        );
        self.native_world_request = pending;
        answer
    }

    /// The title frame's turn for a queued request: the title's main page asks the player (`Mode::WorldRequest`).
    /// A request that outlives the main page, or that meets an entry already waiting for the world resources, is
    /// dropped, not kept for later: it never replaces that entry. A save that has gone away is dropped too, with
    /// nothing created. Nothing opens here: Yes does, through Play's checks.
    pub(super) fn drain_native_world_request(&mut self) -> Result<()> {
        let Some(request) = self.native_world_request.take() else {
            return Ok(());
        };
        if self.title_state() == TitleState::Main
            && self.pending_entry.is_none()
            && let Some(title) = self.title.as_mut()
        {
            title.ask_world_request(&request.folder, request.mods);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::Tag;
    use std::path::PathBuf;

    /// A scratch `saves/` holding one world `w` (a level.dat that reads) and an empty folder `empty`, removed on
    /// drop.
    struct Saves(PathBuf);

    impl Saves {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "horizon-world-request-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("empty")).unwrap();
            write_save(&root.join("w"), "World");
            Self(root)
        }

        fn entries(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Saves {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A folder holding a level.dat that reads, as a world saved under the title's list.
    fn write_save(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        horizon_client::save::write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "Data",
                Tag::compound([("LevelName", Tag::String(name.into()))]),
            )]),
        )
        .unwrap();
    }

    #[test]
    fn world_request_names_the_callers_own_bundle_when_it_grants_world_control() {
        assert_eq!(
            granted_name(Some("clock"), "clock", "Clock Mod", true, false).as_deref(),
            Some("Clock Mod")
        );
        // Another mod's declaration does not grant the caller's request.
        assert_eq!(
            granted_name(Some("other"), "clock", "Clock Mod", true, false),
            None
        );
        // Without the declaration, or with the mod turned off, nothing is granted.
        assert_eq!(
            granted_name(Some("clock"), "clock", "Clock Mod", false, false),
            None
        );
        assert_eq!(
            granted_name(Some("clock"), "clock", "Clock Mod", true, true),
            None
        );
    }

    #[test]
    fn world_request_from_an_unidentified_caller_is_refused_whatever_bundles_declare() {
        // The bundle declares world_control and is enabled, yet no caller is named: nothing is granted.
        assert_eq!(granted_name(None, "clock", "Clock Mod", true, false), None);
        let saves = Saves::new("unknown-caller");
        let mut pending = None;
        let answer = queue(
            TitleState::Main,
            false,
            || granted_name(None, "clock", "Clock Mod", true, false),
            &saves.0,
            "w",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::NotPermitted);
        assert_eq!(pending, None);
        assert_eq!(saves.entries(), ["empty", "w"]);
    }

    #[test]
    fn world_request_refused_without_world_control() {
        let saves = Saves::new("refused");
        let mut pending = None;
        let answer = queue(
            TitleState::Main,
            false,
            || None,
            &saves.0,
            "w",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::NotPermitted);
        assert_eq!(pending, None);
        assert_eq!(saves.entries(), ["empty", "w"]);
    }

    #[test]
    fn world_request_refused_off_title() {
        let saves = Saves::new("off-title");
        let mut pending = None;
        let answer = queue(
            TitleState::Elsewhere,
            false,
            || Some("Mod".into()),
            &saves.0,
            "w",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::NotTitle);
        assert_eq!(pending, None);
        assert_eq!(saves.entries(), ["empty", "w"]);
    }

    #[test]
    fn world_request_is_busy_while_the_question_is_up() {
        let saves = Saves::new("asking");
        let mut pending = None;
        let answer = queue(
            TitleState::Asking,
            false,
            || Some("Mod".into()),
            &saves.0,
            "w",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::Busy);
        assert_eq!(pending, None);
    }

    #[test]
    fn world_request_rejects_path_escape() {
        let saves = Saves::new("escape");
        let long = "x".repeat(65);
        for folder in [
            "../w",
            "w/../w",
            "a/b",
            "a\\b",
            ".hidden",
            "",
            "c:w",
            long.as_str(),
        ] {
            let mut pending = None;
            let answer = queue(
                TitleState::Main,
                false,
                || Some("Mod".into()),
                &saves.0,
                folder,
                &mut pending,
            );
            assert_eq!(answer, NativeWorldResult::InvalidRequest, "{folder:?}");
            assert_eq!(pending, None, "{folder:?}");
        }
        // A folder that is one segment but not there is refused too, and nothing is created.
        let mut pending = None;
        let answer = queue(
            TitleState::Main,
            false,
            || Some("Mod".into()),
            &saves.0,
            "missing",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::InvalidRequest);
        assert_eq!(saves.entries(), ["empty", "w"]);
    }

    #[test]
    fn world_request_names_one_plain_segment() {
        assert!(valid_folder("w"));
        assert!(valid_folder("My World 2"));
        assert!(valid_folder(&"x".repeat(64)));
        assert!(!valid_folder(&"x".repeat(65)));
        assert!(!valid_folder(".."));
        assert!(!valid_folder("a/b"));
    }

    #[test]
    fn world_request_refuses_an_empty_folder_as_a_world() {
        let saves = Saves::new("empty");
        let mut pending = None;
        let answer = queue(
            TitleState::Main,
            false,
            || Some("Mod".into()),
            &saves.0,
            "empty",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::InvalidRequest);
        assert_eq!(pending, None);
        assert_eq!(saves.entries(), ["empty", "w"]);
        assert_eq!(std::fs::read_dir(saves.0.join("empty")).unwrap().count(), 0);
    }

    #[test]
    fn world_request_queues_the_save_once_with_the_mod_that_asked() {
        let saves = Saves::new("open");
        let mut pending = None;
        assert_eq!(
            queue(
                TitleState::Main,
                false,
                || Some("Clock Mod".into()),
                &saves.0,
                "w",
                &mut pending
            ),
            NativeWorldResult::Queued
        );
        assert_eq!(
            pending,
            Some(Request {
                folder: "w".into(),
                mods: "Clock Mod".into(),
            })
        );
        // A second request before the first is taken is refused, and the first stays queued.
        assert_eq!(
            queue(
                TitleState::Main,
                false,
                || Some("Clock Mod".into()),
                &saves.0,
                "w",
                &mut pending
            ),
            NativeWorldResult::Busy
        );
        assert_eq!(pending.as_ref().map(|r| r.folder.as_str()), Some("w"));
    }

    #[test]
    fn world_request_is_busy_while_an_entry_waits_for_resources() {
        let saves = Saves::new("entering");
        let mut pending = None;
        let answer = queue(
            TitleState::Main,
            true,
            || Some("Mod".into()),
            &saves.0,
            "w",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::Busy);
        assert_eq!(pending, None);
    }

    #[cfg(unix)]
    #[test]
    fn world_request_refuses_a_symlink_that_leaves_saves() {
        let saves = Saves::new("symlink");
        let outside = std::env::temp_dir().join(format!(
            "horizon-world-request-outside-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&outside);
        write_save(&outside, "Outside");
        std::os::unix::fs::symlink(&outside, saves.0.join("escape")).unwrap();
        // A link to another save inside `saves` is still a world under it.
        std::os::unix::fs::symlink(saves.0.join("w"), saves.0.join("alias")).unwrap();

        let mut pending = None;
        let answer = queue(
            TitleState::Main,
            false,
            || Some("Mod".into()),
            &saves.0,
            "escape",
            &mut pending,
        );
        assert_eq!(answer, NativeWorldResult::InvalidRequest);
        assert_eq!(pending, None);
        assert_eq!(
            queue(
                TitleState::Main,
                false,
                || Some("Mod".into()),
                &saves.0,
                "alias",
                &mut pending
            ),
            NativeWorldResult::Queued
        );
        let _ = std::fs::remove_dir_all(&outside);
    }
}
