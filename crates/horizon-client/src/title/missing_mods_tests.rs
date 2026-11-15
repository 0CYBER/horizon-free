use super::super::tests::{Fixture, only_world};
use super::*;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;

/// A mod the fixture's world was saved with and no test loads.
fn gone() -> MissingMod {
    MissingMod {
        id: "gone".into(),
        saved_version: "1.0.0".into(),
        code: true,
        content: false,
        blocks: true,
    }
}

#[test]
fn entries_preserve_affected_kinds_and_known_or_unknown_versions() {
    crate::lang::scoped(crate::lang::Language::load(&[], "en_us"), || {
        assert_eq!(entry(&gone()), "gone, version 1.0.0 (code mod, its blocks)");
        assert_eq!(
            entry(&MissingMod {
                code: false,
                blocks: false,
                ..gone()
            }),
            "gone, version 1.0.0"
        );
        assert_eq!(
            entry(&MissingMod {
                saved_version: String::new(),
                code: false,
                content: true,
                blocks: false,
                ..gone()
            }),
            "gone, version unknown (content mod)"
        );
    });
}

/// Every file under `dir`, by its path relative to `dir`, with its bytes.
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let name = path.strip_prefix(root).unwrap().to_string_lossy();
                out.insert(name.replace('\\', "/"), fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// A world with a region file beside its level.dat, asked about `gone`.
fn asked(f: &mut Fixture) -> (WorldEntry, PathBuf) {
    f.world("Gone", "Gone");
    let dir = f.title.saves.join("Gone");
    fs::create_dir_all(dir.join("region")).unwrap();
    fs::write(dir.join("region/r.0.0.mca"), b"chunk bytes").unwrap();
    // What the game keeps when it asks: the hook's answer for this world.
    level_mods::remember_answer(&dir, Vec::new(), Vec::new());
    f.title.ask_missing_mods("Gone", vec![gone()]);
    (only_world(f), dir)
}

fn finish_backup(f: &mut Fixture) {
    f.title.backup_shown.set(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while matches!(f.title.mode, Mode::Backup { .. }) {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
        f.title.poll_backup();
    }
}

/// The question lists the mod; Don't Open, the default, leaves every file of the world as it
/// was and keeps it closed, and the next open asks again.
#[test]
fn dont_open_leaves_the_world_closed_and_unchanged() {
    let _answer = level_mods::TEST_ANSWER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    let (_, dir) = asked(&mut f);
    let before = files(&dir);
    assert!(matches!(
        &f.title.mode,
        Mode::MissingMods { missing, .. } if *missing == [gone()]
    ));
    assert_eq!(
        f.click(&lang::get("horizon.missing_mods.dont_open")),
        Action::None
    );
    assert!(matches!(f.title.mode, Mode::Select { .. }));
    assert!(!level_mods::is_confirmed(&dir));
    assert_eq!(f.title.take_open_request(), None);
    assert_eq!(files(&dir), before);
    // Escape answers the same.
    level_mods::remember_answer(&dir, Vec::new(), Vec::new());
    f.title.ask_missing_mods("Gone", vec![gone()]);
    f.title.escape();
    assert!(matches!(f.title.mode, Mode::Select { .. }));
    assert!(!level_mods::is_confirmed(&dir));
    assert_eq!(files(&dir), before);
}

/// Open Without a Backup opens the world, and its open goes ahead without asking again.
#[test]
fn open_without_a_backup_opens_at_once() {
    let _answer = level_mods::TEST_ANSWER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    let (_, dir) = asked(&mut f);
    assert_eq!(
        f.click(&lang::get("horizon.missing_mods.open")),
        Action::Play {
            folder: "Gone".into()
        }
    );
    assert!(level_mods::is_confirmed(&dir));
    level_mods::forget_answer();
}

/// Back Up and Open copies the whole world before anything is written, then opens it and says
/// where the copy is.
#[test]
fn back_up_and_open_copies_the_world_first() {
    let _answer = level_mods::TEST_ANSWER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    let (_, dir) = asked(&mut f);
    let before = files(&dir);
    assert_eq!(
        f.click(&lang::get("horizon.missing_mods.backup_open")),
        Action::None
    );
    finish_backup(&mut f);
    assert_eq!(f.title.take_open_request(), Some("Gone".into()));
    assert!(level_mods::is_confirmed(&dir));
    let Some(WorldToast::Created { folder: path, .. }) = &f.title.pending_toast else {
        panic!("no backup toast: {:?}", f.title.pending_toast.is_some());
    };
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut copied = BTreeMap::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .unwrap()
            .strip_prefix("Gone/")
            .unwrap()
            .to_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        copied.insert(name, bytes);
    }
    assert_eq!(copied, before);
    assert_eq!(files(&dir), before);
    level_mods::forget_answer();
}

/// A backup that cannot be written cancels the open and says why.
#[test]
fn a_failed_backup_cancels_the_open() {
    let _answer = level_mods::TEST_ANSWER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    let (_, dir) = asked(&mut f);
    // The backups folder's place is taken by a file, so it cannot be made.
    let backups = world_management::backups_path(&f.title.saves);
    fs::write(&backups, b"not a folder").unwrap();
    f.click(&lang::get("horizon.missing_mods.backup_open"));
    finish_backup(&mut f);
    assert_eq!(f.title.take_open_request(), None);
    assert!(!level_mods::is_confirmed(&dir));
    assert!(matches!(&f.title.mode, Mode::Error { message, .. }
        if message.starts_with(&lang::tr("horizon.missing_mods.backup_failed", &[&""]))));
}

/// Button hit targets stay entirely inside a narrow viewport, regardless of text length.
#[test]
fn long_text_keeps_all_three_actions_inside_narrow_viewport() {
    let mut f = Fixture::new();
    f.title.ask_missing_mods("Gone", vec![gone(); 30]);
    for lines in [3, 14, 100] {
        f.title.message_lines.set(lines);
        let buttons = f.title.buttons(240.0, 240.0);
        assert_eq!(buttons.len(), 3);
        for (button, y) in buttons.iter().zip([212.0, 164.0, 188.0]) {
            assert_eq!(button.x, 10.0);
            assert_eq!(button.w, 220.0);
            assert_eq!(button.y, y);
            assert!(button.y + 20.0 <= 240.0);
        }
    }
}

#[test]
fn page_keys_scroll_independently_of_action_focus() {
    let mut f = Fixture::new();
    f.title.ask_missing_mods("Gone", vec![gone()]);
    f.title.message_lines.set(100);
    f.title.focused_button = Some(101);
    assert!(
        f.title
            .page_missing_mods(winit::keyboard::KeyCode::PageDown, (240.0, 240.0))
    );
    assert_eq!(f.title.scroll.get().amount, 108.0);
    assert_eq!(f.title.focused_button, Some(101));
    f.title.focused_button = None;
    f.title.scroll.set(Default::default());
    f.title.message_lines.set(3);
    f.title
        .arrow_key(winit::keyboard::KeyCode::ArrowDown, (240.0, 240.0), false);
    assert!(f.title.focused_button.is_some());
}

#[test]
fn opening_prompt_replaces_stale_destructive_focus_with_cancel() {
    let _answer = level_mods::TEST_ANSWER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut f = Fixture::new();
    f.world("Gone", "Gone");
    f.title.focused_button = Some(103);
    f.title.scroll.get_mut().amount = 400.0;
    f.title.ask_missing_mods("Gone", vec![gone()]);
    assert_eq!(f.title.focused_button, Some(101));
    assert_eq!(f.title.scroll.get().amount, 0.0);
    for size in [(240.0, 240.0), (640.0, 360.0)] {
        f.title
            .arrow_key(winit::keyboard::KeyCode::ArrowDown, size, false);
        assert_eq!(f.title.focused_button, Some(101));
    }
    // Untouched Enter must choose Cancel too, not a world-opening action.
    f.title.ask_missing_mods("Gone", vec![gone()]);
    assert_eq!(f.title.confirm_key_size((240.0, 240.0)).0, Action::None);
    assert!(matches!(f.title.mode, Mode::Select { .. }));
}

#[test]
fn controller_down_past_visible_rows_scrolls_then_reaches_cancel_at_end() {
    use crate::gamepad::{Button, ScrollArea, list_step};
    let mut f = Fixture::new();
    f.title.ask_missing_mods("Gone", vec![gone(); 30]);
    f.title.message_lines.set(100);
    let size = (240.0, 240.0);
    let (top, bottom, _) = layout(size.0, size.1);
    let row = [16.0, bottom - 9.0, 208.0, 9.0];
    let buttons = f.title.buttons(size.0, size.1);
    let targets: Vec<_> = buttons.iter().map(|b| [b.x, b.y, b.w, 20.0]).collect();
    let mut area = ScrollArea {
        rect: [0.0, top, 236.0, bottom - top],
        amount: 0.0,
        up: false,
        down: true,
    };
    assert_eq!(list_step(row, Button::DPadDown, &targets, &[area]), None);
    // The existing pad focus path emits this step-wheel when list_step returns None.
    f.title.step_scroll(-1.0, (120.0, bottom - 4.0), size);
    assert_eq!(f.title.scroll.get().amount, 18.0);
    area.down = false;
    let mut from = row;
    for expected_id in [102, 103, 101] {
        let next = list_step(from, Button::DPadDown, &targets, &[area]).unwrap();
        assert_eq!(buttons[next].id, expected_id);
        from = targets[next];
    }
}

#[test]
fn missing_mod_scrollbar_uses_existing_press_drag_release() {
    let mut f = Fixture::new();
    f.title.ask_missing_mods("Gone", vec![gone(); 30]);
    f.title.message_lines.set(100);
    assert_eq!(f.title.click(233.0, 40.0, (240.0, 240.0)).0, Action::None);
    f.title.drag((233.0, 130.0), (240.0, 240.0));
    assert!(f.title.scroll.get().amount > 0.0);
    f.title.release();
    let released = f.title.scroll.get().amount;
    f.title.drag((233.0, 50.0), (240.0, 240.0));
    assert_eq!(f.title.scroll.get().amount, released);
}

#[test]
fn controller_rows_group_wrapped_mods_and_exclude_the_description() {
    let mut f = Fixture::new();
    let mut wrapped_mod = gone();
    wrapped_mod.id = "first_line\nsecond_line".into();
    f.title.ask_missing_mods("Gone", vec![wrapped_mod, gone()]);
    let atlas = HudAtlas::build(&horizon_assets::resource_source::ResourceSource::directory(
        "/nonexistent/horizon-test-assets".into(),
    ))
    .unwrap();
    let mut batch = HudBatch::new(
        [240, 480],
        crate::hud::GuiScale {
            requested: 1,
            unicode: false,
        },
    );
    crate::widgets::end_frame(100);
    let english = crate::lang::Language::load(&[], "en_us");
    let mut phases = Vec::new();
    crate::lang::scoped(english, || {
        f.title.draw(
            &atlas,
            &mut batch,
            (-1.0, -1.0),
            true,
            &mut |phase, batch| phases.push((phase, batch.vertices.len())),
        );
    });
    crate::widgets::end_frame(101);
    use crate::screen_draw::RenderPhase::{Background, Foreground};
    assert_eq!(
        phases.iter().map(|(phase, _)| *phase).collect::<Vec<_>>(),
        [Background, Foreground],
    );
    assert!(phases[0].1 > 0, "menu background precedes the hook");
    assert!(phases[1].1 > phases[0].1, "prompt contents follow the hook");
    let rows: Vec<_> = crate::widgets::last_targets()
        .into_iter()
        .filter(|t| t.kind == crate::widgets::Kind::Row)
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].rect[3] > 9.0);
    assert_eq!(f.title.focused_button, Some(101));
    f.title.missing_mods_pad_focus();
    assert_eq!(f.title.focused_button, None);
}
