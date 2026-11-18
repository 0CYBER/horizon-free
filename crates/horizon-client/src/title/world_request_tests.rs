//! A native mod's `request_world` as the title asks it: the question names a save that exists, No leaves
//! everything as it was, and Yes goes through the checks the Play button runs.
use super::tests::Fixture;
use super::*;
use horizon_core::nbt::Tag;
use std::fs;
use winit::keyboard::KeyCode;

/// Writes `folder`'s level.dat as a save from a newer version: Play's downgrade backup question is due.
fn write_newer(f: &Fixture, folder: &str) {
    crate::save::write_dat(
        &f.title.saves.join(folder).join("level.dat"),
        Tag::compound([(
            "Data",
            Tag::compound([
                ("LevelName", Tag::String(folder.into())),
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
                (
                    "Version",
                    Tag::compound([
                        ("Name", Tag::String("26.3".into())),
                        ("Id", Tag::Int(horizon_world::DATA_VERSION + 1)),
                    ]),
                ),
            ]),
        )]),
    )
    .unwrap();
}

#[test]
fn only_an_existing_save_is_asked_about() {
    let mut f = Fixture::new();
    f.world("Saved", "Saved");
    fs::create_dir_all(f.title.saves.join("Empty")).unwrap();
    // An empty folder is not a world: nothing is asked, and nothing is created.
    assert!(world_request_entry(&f.title.saves, "Empty").is_none());
    assert!(!f.title.ask_world_request("Empty", "Clock Mod".into()));
    assert!(matches!(f.title.mode, Mode::Main));
    assert!(
        f.title
            .saves
            .join("Empty")
            .read_dir()
            .unwrap()
            .next()
            .is_none()
    );

    assert!(f.title.ask_world_request("Saved", "Clock Mod".into()));
    match &f.title.mode {
        Mode::WorldRequest { world, mods } => {
            assert_eq!(world.folder, "Saved");
            assert_eq!(mods, "Clock Mod");
        }
        other => panic!("expected the request's question, got {other:?}"),
    }
}

#[test]
fn no_leaves_the_save_and_the_title_as_they_were() {
    let mut f = Fixture::new();
    f.world("Kept", "Kept");
    let level = f.title.saves.join("Kept/level.dat");
    let before = fs::read(&level).unwrap();
    assert!(f.title.ask_world_request("Kept", "Clock Mod".into()));
    assert_eq!(f.click("No"), Action::None);
    assert!(matches!(f.title.mode, Mode::Main));
    assert_eq!(fs::read(&level).unwrap(), before);
    assert_eq!(list_worlds(&f.title.saves).len(), 1);
}

#[test]
fn escape_answers_no_and_leaves_the_save_and_the_title_as_they_were() {
    let mut f = Fixture::new();
    f.world("Esc", "Esc");
    let level = f.title.saves.join("Esc/level.dat");
    let before = fs::read(&level).unwrap();
    assert!(f.title.ask_world_request("Esc", "Clock Mod".into()));
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Main));
    assert_eq!(f.title.take_open_request(), None);
    assert_eq!(fs::read(&level).unwrap(), before);
    assert_eq!(list_worlds(&f.title.saves).len(), 1);
}

#[test]
fn yes_asks_play_to_open_the_save() {
    let mut f = Fixture::new();
    f.world("Ready", "Ready");
    assert!(f.title.ask_world_request("Ready", "Clock Mod".into()));
    assert_eq!(
        f.click("Yes"),
        Action::Play {
            folder: "Ready".into()
        }
    );
}

#[test]
fn yes_after_the_save_went_away_opens_nothing_and_creates_nothing() {
    let mut f = Fixture::new();
    f.world("Gone", "Gone");
    assert!(f.title.ask_world_request("Gone", "Clock Mod".into()));
    fs::remove_dir_all(f.title.saves.join("Gone")).unwrap();
    assert_eq!(f.click("Yes"), Action::None);
    assert!(matches!(f.title.mode, Mode::Main));
    assert!(!f.title.saves.join("Gone").exists());
}

#[test]
fn yes_keeps_the_backup_question_play_asks_for_a_newer_save() {
    let mut f = Fixture::new();
    f.world("Newer", "Newer");
    write_newer(&f, "Newer");
    assert!(f.title.ask_world_request("Newer", "Clock Mod".into()));
    assert_eq!(f.click("Yes"), Action::None);
    assert!(matches!(
        f.title.mode,
        Mode::BackupBeforeUpgrade {
            downgrade: Some(_),
            ..
        }
    ));
}

#[cfg(unix)]
#[test]
fn yes_keeps_the_symlink_refusal_play_gives() {
    let mut f = Fixture::new();
    f.world("Linked", "Linked");
    let outside = f.title.saves.parent().unwrap().join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.title.saves.join("Linked/link")).unwrap();
    assert!(f.title.ask_world_request("Linked", "Clock Mod".into()));
    assert_eq!(f.click("Yes"), Action::None);
    assert!(matches!(
        f.title.mode,
        Mode::Error {
            alert: Some(Alert::Symlink),
            ..
        }
    ));
}

#[test]
fn yes_after_the_save_was_replaced_asks_again_with_its_new_details_and_opens_nothing() {
    let mut f = Fixture::new();
    f.world("Swap", "Old name");
    assert!(f.title.ask_world_request("Swap", "Clock Mod".into()));
    // Another save takes the folder while the question is up: a different LevelName.
    f.world("Swap", "New name");
    assert_eq!(f.click("Yes"), Action::None);
    match &f.title.mode {
        Mode::WorldRequest { world, mods } => {
            assert_eq!(world.folder, "Swap");
            assert_eq!(world.name, "New name");
            assert_eq!(mods, "Clock Mod");
        }
        other => panic!("expected the question again, got {other:?}"),
    }
    assert_eq!(f.title.take_open_request(), None);
    // The question is the same kind of screen as before: Yes on the new save opens it.
    assert_eq!(
        f.click("Yes"),
        Action::Play {
            folder: "Swap".into()
        }
    );
}

#[test]
fn a_save_that_is_gone_after_the_question_returns_to_main_and_creates_nothing() {
    let mut f = Fixture::new();
    f.world("Vanished", "Vanished");
    assert!(f.title.ask_world_request("Vanished", "Clock Mod".into()));
    fs::remove_dir_all(f.title.saves.join("Vanished")).unwrap();
    assert_eq!(f.click("Yes"), Action::None);
    assert!(matches!(f.title.mode, Mode::Main));
    assert!(!f.title.saves.join("Vanished").exists());
}

#[cfg(unix)]
#[test]
fn an_alert_after_yes_returns_to_main_not_to_the_question_when_the_title_is_gated() {
    let mut f = Fixture::new();
    f.world("Linked", "Linked");
    let outside = f.title.saves.parent().unwrap().join("outside-gated");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, f.title.saves.join("Linked/link")).unwrap();
    assert!(f.title.ask_world_request("Linked", "Clock Mod".into()));
    // The game gates the title: Yes's screens are asked for, then applied by the game.
    f.title.gated = true;
    assert_eq!(f.click("Yes"), Action::None);
    let asked = f
        .title
        .take_requested()
        .expect("the symlink alert is asked for");
    assert!(matches!(
        &asked,
        Mode::Error {
            alert: Some(Alert::Symlink),
            return_to,
            ..
        } if matches!(**return_to, Mode::Main)
    ));
    f.title.gated = false;
    f.title.set_mode(asked);
    // Dismissing the alert lands on the main page, not on the question.
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Main));
}

#[test]
fn a_backup_question_after_yes_escapes_to_main_not_to_the_world_list() {
    let mut f = Fixture::new();
    f.world("Newer", "Newer");
    write_newer(&f, "Newer");
    assert!(f.title.ask_world_request("Newer", "Clock Mod".into()));
    assert_eq!(f.click("Yes"), Action::None);
    assert!(matches!(f.title.mode, Mode::BackupBeforeUpgrade { .. }));
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Main));
}

#[test]
fn the_entry_of_one_save_is_the_one_the_list_shows_including_a_level_dat_old_alone() {
    let f = Fixture::new();
    f.world("Listed", "Listed");
    f.world("Other", "Other");
    f.world("Old", "Old");
    // Only level.dat_old is left: the list still shows the folder, as a corrupted save named after it.
    fs::rename(
        f.title.saves.join("Old/level.dat"),
        f.title.saves.join("Old/level.dat_old"),
    )
    .unwrap();
    for folder in ["Listed", "Old"] {
        let listed = list_worlds(&f.title.saves)
            .into_iter()
            .find(|w| w.folder == folder)
            .unwrap();
        let one = world_request_entry(&f.title.saves, folder).unwrap();
        assert_eq!(one, listed, "{folder}");
    }
    let old = world_request_entry(&f.title.saves, "Old").unwrap();
    assert!(old.corrupted);
    assert_eq!(old.name, "Old");
}

#[test]
fn the_question_keeps_yes_and_no_on_screen_whatever_the_message_holds() {
    let mut f = Fixture::new();
    let long_save = "S".repeat(200);
    f.world("Long", &long_save);
    assert!(f.title.ask_world_request("Long", "M".repeat(120)));
    for h in [160.0_f32, 200.0, 240.0, 262.0, 300.0] {
        for lines in 1..=200usize {
            f.title.message_lines.set(lines);
            let buttons = f.title.buttons(320.0, h);
            for id in [104u8, 105] {
                let button = buttons.iter().find(|b| b.id == id).unwrap();
                assert!(
                    button.y <= h - 28.0,
                    "height {h}, {lines} lines: y {}",
                    button.y
                );
            }
        }
    }
}

#[test]
fn a_long_question_scrolls_inside_its_area_with_the_page_keys() {
    let mut f = Fixture::new();
    f.world("Long", &"S".repeat(200));
    assert!(f.title.ask_world_request("Long", "M".repeat(120)));
    let (w, h) = (320.0, 240.0);
    let area = world_request_layout((w, h));
    let page = area.bottom - area.top;
    // 200 lines of 9 pixels: the area shows `page` pixels of them at a time.
    f.title.message_lines.set(200);
    assert!(f.title.page_missing_mods(KeyCode::PageDown, (w, h)));
    assert_eq!(f.title.scroll.get().amount, page);
    assert!(f.title.page_missing_mods(KeyCode::End, (w, h)));
    assert_eq!(f.title.scroll.get().amount, 200.0 * 9.0 - page);
    assert!(f.title.page_missing_mods(KeyCode::Home, (w, h)));
    assert_eq!(f.title.scroll.get().amount, 0.0);
}

#[test]
fn yes_and_no_narrow_to_stay_inside_a_narrow_viewport() {
    for w in (60..=640).step_by(2) {
        let w = w as f32;
        let area = world_request_layout((w, 240.0));
        // Both buttons lie inside the viewport and do not overlap.
        assert!(area.yes_x >= 0.0, "width {w}: yes at {}", area.yes_x);
        assert!(
            area.no_x + area.button_w <= w,
            "width {w}: no ends at {}",
            area.no_x + area.button_w
        );
        assert!(
            area.yes_x + area.button_w <= area.no_x,
            "width {w}: the buttons overlap"
        );
        assert!(area.button_w <= 150.0);
    }
    // Room for two full buttons keeps them full width, as the other confirm screens have them.
    assert_eq!(world_request_layout((320.0, 240.0)).button_w, 150.0);
    // The 240 wide viewport that clipped them before: 114 wide each, 4 apart around the centre.
    let narrow = world_request_layout((240.0, 160.0));
    assert_eq!(
        (narrow.yes_x, narrow.no_x, narrow.button_w),
        (4.0, 122.0, 114.0)
    );
}

#[test]
fn the_end_of_a_long_question_scrolls_into_view_whatever_the_height() {
    let mut f = Fixture::new();
    f.world("Long", &"S".repeat(200));
    assert!(f.title.ask_world_request("Long", "M".repeat(120)));
    for h in [160.0_f32, 200.0, 240.0, 300.0] {
        let (w, area) = (240.0, world_request_layout((240.0, h)));
        for lines in [3usize, 7, 11, 20, 33, 101] {
            f.title.message_lines.set(lines);
            f.title.scroll.set(Default::default());
            assert!(f.title.page_missing_mods(KeyCode::End, (w, h)));
            let offset = world_request_offset(f.title.scroll.get().amount);
            // The last line ends inside the area at the farthest scroll, and the first line is still where it was.
            let last_bottom = area.top + lines as f32 * 9.0 - offset;
            assert!(
                last_bottom <= area.bottom + 0.001,
                "height {h}, {lines} lines: the last line ends at {last_bottom}, the area at {}",
                area.bottom
            );
            if lines as f32 * 9.0 > area.bottom - area.top {
                assert!(
                    last_bottom > area.bottom - 9.0,
                    "height {h}, {lines} lines: scrolled past the end"
                );
            }
        }
    }
}

#[test]
fn a_request_keeps_main_as_the_return_target_through_a_recovery() {
    let mut f = Fixture::new();
    f.world("Rec", "Rec");
    let dir = f.title.saves.join("Rec");
    // level.dat is broken; level.dat_old is a save from another series, whose open is an alert.
    fs::rename(dir.join("level.dat"), dir.join("level.dat_old")).unwrap();
    let mut old = crate::save::read_dat(&dir.join("level.dat_old")).unwrap();
    if let Some(Tag::Compound(data)) = old.get_mut("Data") {
        data.insert(
            "Version".into(),
            Tag::compound([
                ("Name", Tag::String("1.0".into())),
                ("Series", Tag::String("snapshot".into())),
            ]),
        );
    }
    crate::save::write_dat(&dir.join("level.dat_old"), old).unwrap();
    fs::write(dir.join("level.dat"), b"truncated").unwrap();
    assert!(f.title.ask_world_request("Rec", "Clock Mod".into()));
    assert_eq!(f.click("Yes"), Action::None);
    assert!(matches!(f.title.mode, Mode::Recover { .. }));
    f.click("Attempt to Restore");
    f.finish_backup();
    assert!(matches!(
        f.title.mode,
        Mode::Recovered { success: true, .. }
    ));
    f.click("Continue");
    assert!(
        matches!(&f.title.mode, Mode::Error { alert: Some(Alert::Incompatible), return_to, .. }
            if matches!(**return_to, Mode::Main)),
        "got {:?}",
        f.title.mode
    );
}

#[cfg(unix)]
#[test]
fn ordinary_play_reaches_the_symlink_alert_for_a_listed_save_that_links_elsewhere() {
    let f = Fixture::new();
    f.world("Linked", "Linked");
    let outside = f.title.saves.parent().unwrap().join("outside-plain");
    fs::create_dir_all(&outside).unwrap();
    let kept = fs::read(f.title.saves.join("Linked/level.dat")).unwrap();
    fs::remove_dir_all(f.title.saves.join("Linked")).unwrap();
    // The whole save folder is a link to a save elsewhere: the list shows it, a mod's request refuses it.
    fs::create_dir_all(outside.join("real")).unwrap();
    fs::write(outside.join("real/level.dat"), kept).unwrap();
    std::os::unix::fs::symlink(outside.join("real"), f.title.saves.join("Linked")).unwrap();
    assert!(is_save_folder(&f.title.saves, "Linked"));
    assert!(world_request_entry(&f.title.saves, "Linked").is_none());
}
