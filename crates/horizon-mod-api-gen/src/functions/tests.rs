use super::*;
use std::fs;

const GAME: &str = r#"use std::fmt;
pub struct Game<'a> {
    ticks: &'a u32,
}
pub trait Tick {
    fn tick(&self) -> u32;
}
impl Game<'_> {
    /// Kept.
    pub fn ticks(&self) -> u32 {
        *self.ticks
    }
    pub fn generic<T>(&self, t: T) -> T {
        t
    }
}
impl Tick for Game<'_> {
    fn tick(&self) -> u32 {
        1
    }
}
impl fmt::Display for Game<'_> {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}
#[cfg(unix)]
pub fn twice() {}
#[cfg(not(unix))]
pub fn twice() {}
#[cfg(test)]
mod tests {
    fn hidden() {}
}
mod inner {
    pub fn nested() -> u8 {
        2
    }
}
"#;

/// A game crate and a crate below it with one marked function.
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let write = |path: &str, text: &str| {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "crates/horizon-client/Cargo.toml",
        "[package]\nname = 'horizon-client'\n",
    );
    write("crates/horizon-client/src/lib.rs", "pub mod game;\n");
    write("crates/horizon-client/src/game.rs", GAME);
    write(
        "crates/horizon-low/Cargo.toml",
        "[package]\nname = 'horizon-low'\n[lib]\nname = 'low'\n",
    );
    write(
        "crates/horizon-low/src/lib.rs",
        "pub struct Clock;\nimpl Clock {\n    #[horizon_mod_api_macros::replaceable]\n    pub fn advance(&self) {}\n}\npub fn unmarked() {}\n",
    );
    root
}

#[test]
fn the_index_lists_each_function_with_its_text_or_its_refusal() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let listed: Vec<&str> = index.functions.keys().map(String::as_str).collect();
    assert_eq!(
        listed,
        [
            "horizon_client::game::<Game as Tick>::tick",
            "horizon_client::game::Game::ticks",
            "horizon_client::game::inner::nested",
            "horizon_low::Clock::advance",
        ]
    );
    let ticks = &index.functions["horizon_client::game::Game::ticks"];
    assert_eq!(ticks.file, "crates/horizon-client/src/game.rs");
    let [start, end] = ticks.span;
    assert_eq!(
        &GAME[start..end],
        "/// Kept.\n    pub fn ticks(&self) -> u32 {\n        *self.ticks\n    }"
    );
    assert_eq!(ticks.sha256, sha256(&GAME.as_bytes()[start..end]));
    let refused = |key: &str| index.refused[key].clone();
    assert_eq!(refused("horizon_client::game::twice").code, Code::Ambiguous);
    let generic = refused("horizon_client::game::Game::generic");
    assert_eq!(generic.code, Code::NotReplaceable);
    assert!(
        generic.why.contains("it is not replaceable"),
        "{}",
        generic.why
    );
    // The stager's own trait rule: a trait of another crate has no installer.
    let display = refused("horizon_client::game::<Game as Display>::fmt");
    assert_eq!(
        display.why,
        "its trait implementation has no replacement installer (foreign trait)"
    );
    // Test modules and unmarked functions below the game crate are not listed at all.
    assert!(
        !index
            .functions
            .contains_key("horizon_client::game::tests::hidden")
    );
    assert!(
        !index
            .refused
            .contains_key("horizon_client::game::tests::hidden")
    );
    assert!(!index.functions.contains_key("horizon_low::unmarked"));
    assert_eq!(index.crates["horizon_low"].library, "low");
    let game = &index.crates["horizon_client"].modules["horizon_client::game"];
    assert_eq!(
        game,
        &[ModuleFile {
            path: "crates/horizon-client/src/game.rs".into(),
            sha256: sha256(GAME.as_bytes()),
        }]
    );
    assert_eq!(ticks.module, "horizon_client::game");
    assert_eq!(
        index.functions["horizon_client::game::inner::nested"].module,
        "horizon_client::game"
    );
}

#[test]
fn a_target_is_checked_by_its_key_and_its_text() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |target: &str| index.check(root.path(), target);
    check("horizon_client::game::Game<'_>::ticks").unwrap();
    check("horizon_client::game::<Game<'_> as crate::game::Tick>::tick").unwrap();
    check("horizon_low::Clock::advance").unwrap();
    assert_eq!(
        check("horizon_client::game::Game<'_>::no_such").unwrap_err(),
        (
            "E_MIXIN_TARGET_MISSING",
            "no function `horizon_client::game::Game::no_such` in this game build's function index (functions.json)".to_owned()
        )
    );
    // A crate the source does not have is not a crate to mark.
    assert_eq!(
        check("horizon_nowhere::f").unwrap_err(),
        (
            "E_MIXIN_TARGET_MISSING",
            "no crate `horizon_nowhere` in the game's source".to_owned()
        )
    );
    assert_eq!(
        check("horizon_low::unmarked").unwrap_err(),
        (
            "E_MIXIN_NOT_REPLACEABLE",
            crate::replace::UNMARKED.to_owned()
        )
    );
    assert_eq!(
        check("horizon_client::game::twice").unwrap_err().0,
        "E_MIXIN_TARGET_AMBIGUOUS"
    );
    // A source that is not the build's: refused by name, not searched.
    let game = root.path().join("crates/horizon-client/src/game.rs");
    fs::write(&game, GAME.replace("*self.ticks", "*self.ticks + 1")).unwrap();
    let (code, why) = check("horizon_client::game::Game::ticks").unwrap_err();
    assert_eq!(code, "E_MIXIN_SOURCE");
    assert!(
        why.contains("differ from this game build's functions.json"),
        "{why}"
    );
    fs::write(&game, "").unwrap();
    let (code, why) = check("horizon_client::game::Game::ticks").unwrap_err();
    assert_eq!(code, "E_MIXIN_SOURCE");
    assert!(why.contains("has no bytes"), "{why}");
    // An index naming a file outside the source is refused before anything is read.
    let mut escaping = index.clone();
    escaping
        .functions
        .get_mut("horizon_client::game::inner::nested")
        .unwrap()
        .file = "../outside.rs".into();
    let (code, why) = escaping
        .check(root.path(), "horizon_client::game::inner::nested")
        .unwrap_err();
    assert_eq!(code, "E_MIXIN_SOURCE");
    assert!(why.contains("not a path inside the game's source"), "{why}");
}

#[test]
fn the_copy_reads_the_index_and_refuses_a_stale_source() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let targets = ["horizon_client::game::Game<'_>::ticks".to_owned()];
    let generated = crate::patch::generate(
        root.path(),
        "horizon_patch",
        &targets,
        &Default::default(),
        &index,
    )
    .unwrap();
    assert_eq!(
        generated.patched[0].id,
        "horizon_patch::game::<Game<'_> as __Fnr_Game_ticks>::ticks"
    );
    assert!(generated.libs[0].1.contains("*self.ticks"));
    fs::write(
        root.path().join("crates/horizon-client/src/game.rs"),
        GAME.replace("*self.ticks", "0"),
    )
    .unwrap();
    let refused = crate::patch::generate(
        root.path(),
        "horizon_patch",
        &targets,
        &Default::default(),
        &index,
    )
    .unwrap_err();
    assert_eq!(refused[0].code, "E_MIXIN_SOURCE");
}

#[test]
fn every_file_the_copy_reads_is_checked_not_only_the_span() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let targets = ["horizon_client::game::Game::ticks".to_owned()];
    // The target's span is unchanged; the crate root the mirror reads is not the build's.
    let lib = root.path().join("crates/horizon-client/src/lib.rs");
    fs::write(&lib, "pub mod game;\npub fn added() {}\n").unwrap();
    let refused = crate::patch::generate(
        root.path(),
        "horizon_patch",
        &targets,
        &Default::default(),
        &index,
    )
    .unwrap_err();
    assert_eq!(refused[0].code, "E_MIXIN_SOURCE");
    assert!(
        refused[0]
            .why
            .contains("lib.rs differs from this game build's functions.json"),
        "{}",
        refused[0].why
    );
}

#[test]
fn a_function_staging_gave_no_slot_is_refused_unless_a_loop_runs_it() {
    let root = fixture();
    let mut index = index(root.path(), &BTreeSet::new()).unwrap();
    let slots = BTreeSet::from(["horizon_client::game::Game::ticks".to_owned()]);
    let generic = BTreeSet::from(["horizon_client::game::inner::nested".to_owned()]);
    index.retain_slotted(&slots, &generic);
    let listed: Vec<&str> = index.functions.keys().map(String::as_str).collect();
    assert_eq!(
        listed,
        [
            "horizon_client::game::Game::ticks",
            "horizon_client::game::inner::nested"
        ]
    );
    assert_eq!(
        index.refused["horizon_low::Clock::advance"].why,
        "the game build gives it no replacement slot"
    );
}

#[test]
fn a_target_key_drops_generics_spaces_and_the_traits_path() {
    assert_eq!(
        key(
            "horizon_client::game::level_access::<GameLevel<'_> as crate::LevelAccess>::block_state"
        ),
        "horizon_client::game::level_access::<GameLevel as LevelAccess>::block_state"
    );
    assert_eq!(
        key("horizon_client::game::autosave::Game<'_>::next_autosave_interval"),
        "horizon_client::game::autosave::Game::next_autosave_interval"
    );
}
