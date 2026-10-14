use super::*;
use horizon_world::nbt::Tag;
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    root: PathBuf,
    title: Title,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "horizon-world-menu-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("saves")).unwrap();
        Self {
            title: Title::new(root.join("saves"), &root),
            root,
        }
    }
    fn world(&self, folder: &str, name: &str) {
        crate::save::write_dat(
            &self.title.saves.join(folder).join("level.dat"),
            Tag::compound([
                (
                    "Data",
                    Tag::compound([
                        ("LevelName", Tag::String(name.into())),
                        ("GameType", Tag::Int(1)),
                        ("allowCommands", Tag::Byte(1)),
                        (
                            "WorldGenSettings",
                            Tag::compound([("seed", Tag::Long(123456))]),
                        ),
                        (
                            "Version",
                            Tag::compound([("Name", Tag::String("26.3".into()))]),
                        ),
                        ("UnmodeledData", Tag::Long(987654321)),
                    ]),
                ),
                ("OtherRootData", Tag::String("preserve me".into())),
            ]),
        )
        .unwrap();
    }
    fn click(&mut self, label: &str) -> Action {
        self.click_with_sound(label).0
    }
    fn click_with_sound(&mut self, label: &str) -> (Action, bool) {
        let button = self
            .title
            .buttons(480.0, 262.0)
            .into_iter()
            .find(|b| b.label == label)
            .unwrap();
        self.title
            .click(button.x + button.w / 2.0, button.y + 10.0, (480.0, 262.0))
    }
    fn select(&mut self) {
        self.title.enter_singleplayer();
        self.title.click(240.0, 60.0, (480.0, 262.0));
    }
    fn finish_backup(&mut self) {
        self.title.backup_shown.set(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(self.title.mode, Mode::Backup { .. }) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.title.poll_backup();
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn empty_singleplayer_opens_creation_and_cancel_returns_to_title() {
    let mut f = Fixture::new();
    f.click("Singleplayer");
    assert!(matches!(f.title.mode, Mode::Create { .. }));
    f.click("Cancel");
    assert!(matches!(f.title.mode, Mode::Main));
    f.click("Singleplayer");
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Main));
    // Filtering an existing list down to zero results must not create a world.
    f.world("One", "One");
    f.title.enter_singleplayer();
    f.title.type_text("no match");
    assert!(matches!(f.title.mode, Mode::Select { .. }));
}

#[test]
fn selector_controls_require_selection_and_search_uses_filtered_indices() {
    let mut f = Fixture::new();
    f.world("A", "Alpha");
    f.world("B", "Beta");
    f.title.enter_singleplayer();
    let Mode::Select { worlds, .. } = &mut f.title.mode else {
        panic!("existing worlds should open the selector");
    };
    // Beta must have a different index in the full list and filtered list.
    // Filesystem timestamp precision must not decide which world comes first.
    worlds.sort_by(|a, b| a.folder.cmp(&b.folder));
    for label in ["Play Selected World", "Edit", "Delete", "Re-Create"] {
        assert!(
            !f.title
                .buttons(480.0, 262.0)
                .iter()
                .find(|b| b.label == label)
                .unwrap()
                .enabled
        );
        assert_eq!(f.click(label), Action::None);
    }
    // Clicking the disabled footer controls leaves the search box unfocused.
    f.title.click(240.0, 31.0, (480.0, 262.0));
    f.title.type_text("bEtA");
    assert_eq!(f.title.search, "bEtA");
    f.title.click(240.0, 60.0, (480.0, 262.0));
    f.click("Edit");
    assert!(matches!(&f.title.mode, Mode::Edit { world, .. } if world.folder == "B"));
    f.title.escape();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "B")
    );
    f.title.search_focused = true;
    f.title.backspace();
    assert!(matches!(f.title.mode, Mode::Select { selected: None, .. }));
}

#[test]
fn edit_layout_disables_unsupported_optimization_and_missing_icon() {
    let mut f = Fixture::new();
    f.world("World", "New World");
    f.select();
    f.click("Edit");
    let buttons = f.title.buttons(480.0, 262.0);
    let reset = buttons.iter().find(|b| b.label == "Reset Icon").unwrap();
    assert_eq!(
        (reset.x, reset.y, reset.w, reset.enabled),
        (140.0, 78.0, 200.0, false)
    );
    // A 1920x1046 client area has a fractional height at GUI scale 4;
    // vanilla rounds up to 262 GUI pixels before positioning the layout.
    assert_eq!(f.title.buttons(480.0, 261.5)[0].y, 78.0);
    assert!(
        !buttons
            .iter()
            .find(|b| b.label == "Optimize World")
            .unwrap()
            .enabled
    );
    f.click("Optimize World");
    assert!(matches!(f.title.mode, Mode::Edit { .. }));
    fs::write(f.title.saves.join("World/icon.png"), b"icon").unwrap();
    assert!(
        f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.label == "Reset Icon")
            .unwrap()
            .enabled
    );
    f.click("Reset Icon");
    assert!(!f.title.saves.join("World/icon.png").exists());
    assert!(
        !f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.label == "Reset Icon")
            .unwrap()
            .enabled
    );
    f.title.click(240.0, 60.0, (480.0, 262.0));
    for _ in 0..32 {
        f.title.backspace();
    }
    assert!(
        !f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.label == "Save")
            .unwrap()
            .enabled
    );
}

#[test]
fn rename_preserves_unknown_metadata_and_cancel_discards_edits() {
    let mut f = Fixture::new();
    f.world("World", "Original");
    let path = f.title.saves.join("World/level.dat");
    let original = crate::save::read_dat(&path).unwrap();
    f.select();
    f.click("Edit");
    f.title.type_text(" discarded");
    f.click("Cancel");
    assert_eq!(crate::save::read_dat(&path).unwrap(), original);
    f.click("Edit");
    for _ in 0..32 {
        f.title.backspace();
    }
    f.title.type_text("  Renamed 世界  ");
    f.click("Save");
    let mut expected = original;
    let Some(Tag::Compound(data)) = expected.get_mut("Data") else {
        panic!()
    };
    data.insert("LevelName".into(), Tag::String("Renamed 世界".into()));
    assert_eq!(crate::save::read_dat(&path).unwrap(), expected);
    assert!(path.with_file_name("level.dat_old").is_file());
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].name == "Renamed 世界")
    );
    f.click("Edit");
    f.title.type_text(" via Enter");
    f.title.confirm_edit();
    assert_eq!(
        Level::read(&f.title.saves.join("World")).unwrap().name,
        "Renamed 世界 via Enter"
    );
}

#[test]
fn delete_requires_confirmation_and_both_cancel_paths_preserve_world() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Delete");
    assert!(matches!(f.title.mode, Mode::Delete { .. }));
    assert!(f.title.saves.join("World/level.dat").is_file());
    f.click("Cancel");
    f.click("Delete");
    f.title.escape();
    assert!(f.title.saves.join("World/level.dat").is_file());
    f.click("Delete");
    let delete = f
        .title
        .buttons(480.0, 262.0)
        .into_iter()
        .find(|b| b.label == "Delete")
        .unwrap();
    assert_eq!((delete.x, delete.y, delete.w), (88.0, 146.0, 150.0));
    f.click("Delete");
    assert!(!f.title.saves.join("World").exists());
    assert!(matches!(&f.title.mode, Mode::Select { worlds, selected: None } if worlds.is_empty()));
    f.click("Back");
    f.click("Singleplayer");
    assert!(matches!(f.title.mode, Mode::Create { .. }));
}

#[test]
fn backup_zip_preserves_nested_files_excludes_lock_and_keeps_duplicate_backups() {
    let f = Fixture::new();
    f.world("World", "World");
    let world = f.title.saves.join("World");
    fs::create_dir_all(world.join("dimensions/minecraft/overworld/region")).unwrap();
    fs::write(world.join("session.lock"), b"locked").unwrap();
    fs::write(
        world.join("dimensions/minecraft/overworld/region/r.0.0.mca"),
        b"chunk bytes",
    )
    .unwrap();
    for _ in 0..2 {
        assert!(world_management::backup(&f.title.saves, "World").unwrap() > 0);
    }
    let archives: Vec<_> = fs::read_dir(f.root.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(archives.len(), 2);
    for archive in archives {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        assert!(zip.by_name("World/session.lock").is_err());
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name("World/level.dat").unwrap(), &mut bytes)
            .unwrap();
        assert_eq!(bytes, fs::read(world.join("level.dat")).unwrap());
        bytes.clear();
        std::io::Read::read_to_end(
            &mut zip
                .by_name("World/dimensions/minecraft/overworld/region/r.0.0.mca")
                .unwrap(),
            &mut bytes,
        )
        .unwrap();
        assert_eq!(bytes, b"chunk bytes");
    }
}

#[test]
fn backup_screen_blocks_navigation_and_returns_to_the_selected_world() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    f.title.type_text(" unsaved");
    f.click("Make Backup");
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    assert!(f.title.buttons(480.0, 262.0).is_empty());
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    // The first frame must be shown even when the backup finishes immediately.
    f.title.poll_backup();
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    f.finish_backup();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "World")
    );
    assert!(
        matches!(&f.title.toast, Some((_, BackupToast::Created { folder, bytes })) if folder == "World" && *bytes > 0)
    );
    assert_eq!(
        Level::read(&f.title.saves.join("World")).unwrap().name,
        "World"
    );
}

#[test]
fn failed_backup_returns_to_the_selected_world_with_an_error_toast() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    f.title.type_text(" unsaved");
    // A file at the backup-directory path makes the real worker fail reliably.
    fs::write(f.root.join("backups"), b"not a directory").unwrap();
    f.click("Make Backup");
    f.finish_backup();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "World")
    );
    let Some((_, BackupToast::Failed(error))) = &f.title.toast else {
        panic!("backup failures should produce an error toast");
    };
    assert!(!error.is_empty());
    assert_eq!(
        Level::read(&f.title.saves.join("World")).unwrap().name,
        "World"
    );
    assert_eq!(
        fs::read(f.root.join("backups")).unwrap(),
        b"not a directory"
    );
}

#[test]
fn stopped_backup_worker_returns_to_the_list_with_an_error_toast() {
    let mut f = Fixture::new();
    f.world("World", "World");
    let world = list_worlds(&f.title.saves).remove(0);
    let (send, receive) = mpsc::channel();
    f.title.mode = Mode::Backup { world };
    f.title.backup = Some(receive);
    f.title.backup_shown.set(true);
    drop(send);
    f.title.poll_backup();
    assert!(
        matches!(&f.title.mode, Mode::Select { worlds, selected: Some(i) } if worlds[*i].folder == "World")
    );
    assert!(
        matches!(&f.title.toast, Some((_, BackupToast::Failed(error))) if error == "Backup worker stopped")
    );
}

#[test]
fn failed_operations_show_error_and_leave_world_intact() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    fs::remove_file(f.title.saves.join("World/level.dat")).unwrap();
    f.click("Save");
    assert!(matches!(f.title.mode, Mode::Error { .. }));
    f.click("Back");
    assert!(matches!(f.title.mode, Mode::Edit { .. }));
    assert!(f.title.saves.join("World").is_dir());
    for bad in ["../World", "..", ".", "/World"] {
        assert!(world_management::delete(&f.title.saves, bad).is_err());
    }
}

#[cfg(unix)]
#[test]
fn backup_rejects_symlinks_and_removes_incomplete_archive() {
    let f = Fixture::new();
    f.world("World", "World");
    std::os::unix::fs::symlink(f.root.join("outside"), f.title.saves.join("World/link")).unwrap();
    assert!(world_management::backup(&f.title.saves, "World").is_err());
    assert_eq!(fs::read_dir(f.root.join("backups")).unwrap().count(), 0);
}

#[test]
fn recreate_uses_existing_name_seed_and_mode_and_scroll_reaches_later_rows() {
    let mut f = Fixture::new();
    for i in 0..10 {
        f.world(&format!("World{i}"), &format!("World {i}"));
    }
    f.select();
    f.click("Re-Create");
    assert!(
        matches!(&f.title.mode,Mode::Create { name,seed,creative:true,.. } if name.starts_with("World ") && seed == "123456")
    );
    f.click("Cancel");
    f.title
        .scroll_worlds(-100.0, (240.0, 100.0), (480.0, 262.0));
    assert!(f.title.scroll.get() > 0.0);
    let Mode::Select { worlds, .. } = &f.title.mode else {
        panic!()
    };
    let last = worlds.last().unwrap().folder.clone();
    f.title.click(240.0, 180.0, (480.0, 262.0));
    assert!(
        matches!(&f.title.mode,Mode::Select { worlds,selected:Some(i) } if worlds[*i].folder == last)
    );
}

#[test]
fn classic_navigation_buttons_sound_even_without_a_game_action() {
    let mut f = Fixture::new();
    f.world("World", "World");
    assert_eq!(
        f.title.click(0.0, 0.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert_eq!(
        f.click_with_sound("Minecraft Realms"),
        (Action::None, false)
    );
    assert_eq!(f.click_with_sound("Singleplayer"), (Action::None, true));
    assert_eq!(
        f.click_with_sound("Play Selected World"),
        (Action::None, false)
    );
    assert_eq!(f.click_with_sound("Create New World"), (Action::None, true));
    assert_eq!(
        f.title.click(240.0, 55.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert_eq!(
        f.click_with_sound("Game Mode: Survival"),
        (Action::None, true)
    );
    assert_eq!(f.click_with_sound("Cancel"), (Action::None, true));
    assert_eq!(f.click_with_sound("Back"), (Action::None, true));
    assert_eq!(f.click_with_sound("Options..."), (Action::Options, true));
}

#[test]
fn classic_world_selection_is_silent_until_the_entry_launches() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.title.enter_singleplayer();
    assert_eq!(
        f.title.click(240.0, 60.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert_eq!(
        f.title.click(240.0, 60.0, (480.0, 262.0)),
        (
            Action::Play {
                folder: "World".into()
            },
            true
        )
    );
}

#[test]
fn world_management_buttons_sound_when_enabled() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    assert_eq!(f.click_with_sound("Edit"), (Action::None, true));
    assert_eq!(f.click_with_sound("Reset Icon"), (Action::None, false));
    assert_eq!(f.click_with_sound("Optimize World"), (Action::None, false));
    fs::write(f.title.saves.join("World/icon.png"), b"icon").unwrap();
    assert_eq!(f.click_with_sound("Reset Icon"), (Action::None, true));
    let Mode::Edit { name, .. } = &mut f.title.mode else {
        panic!("edit screen")
    };
    name.clear();
    assert_eq!(f.click_with_sound("Save"), (Action::None, false));
    let Mode::Edit { name, .. } = &mut f.title.mode else {
        panic!("edit screen")
    };
    *name = "Renamed".into();
    assert_eq!(f.click_with_sound("Save"), (Action::None, true));
    assert_eq!(f.click_with_sound("Edit"), (Action::None, true));
    assert_eq!(f.click_with_sound("Cancel"), (Action::None, true));
    assert_eq!(f.click_with_sound("Delete"), (Action::None, true));
    assert_eq!(f.click_with_sound("Cancel"), (Action::None, true));
    assert_eq!(f.click_with_sound("Delete"), (Action::None, true));
    assert_eq!(f.click_with_sound("Delete"), (Action::None, true));
    assert!(matches!(f.title.mode, Mode::Select { .. }));
}

#[test]
fn backup_button_sounds_and_the_progress_screen_is_silent() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    assert_eq!(f.click_with_sound("Make Backup"), (Action::None, true));
    assert_eq!(
        f.title.click(240.0, 120.0, (480.0, 262.0)),
        (Action::None, false)
    );
    assert!(matches!(f.title.mode, Mode::Backup { .. }));
    f.finish_backup();
    assert!(matches!(f.title.mode, Mode::Select { .. }));
}

#[test]
fn failed_world_operation_back_button_plays_the_click_sound() {
    let mut f = Fixture::new();
    f.world("World", "World");
    f.select();
    f.click("Edit");
    fs::remove_file(f.title.saves.join("World/level.dat")).unwrap();
    assert_eq!(f.click_with_sound("Save"), (Action::None, true));
    assert!(matches!(f.title.mode, Mode::Error { .. }));
    assert_eq!(f.click_with_sound("Back"), (Action::None, true));
    assert!(matches!(f.title.mode, Mode::Edit { .. }));
}

#[test]
fn delete_refuses_an_open_world_then_removes_it_with_its_lock() {
    let f = Fixture::new();
    f.world("World", "World");
    let open = crate::save::SessionLock::acquire(&f.title.saves.join("World")).unwrap();
    assert!(world_management::delete(&f.title.saves, "World").is_err());
    assert!(f.title.saves.join("World/level.dat").is_file());
    drop(open);
    world_management::delete(&f.title.saves, "World").unwrap();
    assert!(!f.title.saves.join("World").exists());
}

#[cfg(unix)]
#[test]
fn delete_removes_links_without_following_them() {
    let f = Fixture::new();
    f.world("World", "World");
    let outside = f.root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    std::os::unix::fs::symlink(&outside, f.title.saves.join("World/link")).unwrap();
    std::os::unix::fs::symlink(&outside, f.title.saves.join("Linked")).unwrap();
    assert!(world_management::delete(&f.title.saves, "Linked").is_err());
    world_management::delete(&f.title.saves, "World").unwrap();
    assert!(!f.title.saves.join("World").exists());
    assert!(outside.join("keep.txt").is_file());
}
