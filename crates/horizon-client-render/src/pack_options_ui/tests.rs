use super::*;
use crate::options_ui::Event;
use crate::settings_shell::{Columns, Hosted, Screen as _, Shell, Target};
use winit::keyboard::KeyCode as K;

const SIZE: (f32, f32) = (854.0, 480.0);

/// Fog (a number) under Sky, rain (a list of values) under Weather, birds (a boolean) in no
/// section.
fn configs() -> Configs {
    let mut c = Configs::default();
    c.add_entry(
        "p:fog".into(),
        &serde_json::json!({"default_value": 0.5, "section": "sky", "section_order": 2}),
    );
    c.add_entry("p:rain".into(), &serde_json::json!({"default_value": "drops", "allowed_values": ["drops", "streaks"], "section": "weather", "section_order": 1}));
    c.add_entry(
        "p:birds".into(),
        &serde_json::json!({"default_value": true}),
    );
    c
}

/// Two entries with the pack-wide presets `fast` and `pretty`.
fn with_presets() -> Configs {
    let mut c = Configs::default();
    let entry = |default: f64, fast: f64, pretty: f64| serde_json::json!({"default_value": default, "presets": {"fast": fast, "pretty": pretty}});
    c.add_entry("p:fog".into(), &entry(0.5, 0.0, 1.0));
    c.add_entry("p:dust".into(), &entry(0.5, 0.0, 0.8));
    c
}

fn shell_of(configs: Configs) -> Shell<PackOptionsScreen> {
    let mut shell = Shell::default();
    shell.screen = PackOptionsScreen::open(configs);
    shell
}

/// The row of entry `id`, and the page it is on.
fn entry(shell: &Shell<PackOptionsScreen>, id: &str) -> (u16, Item) {
    let i = shell.screen.ids.iter().position(|e| e == id).unwrap() as u16;
    let page = shell.screen.sections.iter().position(|s| s.1.contains(&i));
    (page.unwrap() as u16, Item::Entry(i))
}

fn press(shell: &mut Shell<PackOptionsScreen>, s: &mut Settings, item: Item) -> Vec<Output> {
    let at = shell.point_at(s, SIZE, item);
    shell.click(s, at, SIZE, false, false, false)
}

fn value<'a>(shell: &'a Shell<PackOptionsScreen>, id: &str) -> Option<&'a ConfigValue> {
    shell.screen.configs.get(id)
}

#[test]
fn sections_sort_by_their_order_and_the_unsectioned_last() {
    let screen = PackOptionsScreen::open(configs());
    let order: Vec<_> = screen.sections.iter().map(|(s, _)| s.as_deref()).collect();
    assert_eq!(order, [Some("weather"), Some("sky"), None]);
    // Each section is a page, the first one holding Reset all before its entries.
    let sheets = screen.sheets(&Settings::default());
    let names: Vec<_> = sheets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Weather", "Sky", "horizon.pack.options.general"]);
    let first: Vec<Vec<Item>> = (sheets[0].groups.iter())
        .map(|g| g.rows.iter().map(|r| r.id).collect())
        .collect();
    let rain = screen.ids.iter().position(|e| e == "p:rain").unwrap() as u16;
    assert_eq!(first, [vec![Item::ResetAll], vec![Item::Entry(rain)]]);
}

#[test]
fn rows_switch_cycle_and_slide() {
    let mut s = Settings::default();
    let mut shell = shell_of(configs());
    let (page, birds) = entry(&shell, "p:birds");
    shell.show(page);
    assert_eq!(
        press(&mut shell, &mut s, birds)[0],
        Output::Event(Event::ButtonClick)
    );
    assert_eq!(value(&shell, "p:birds"), Some(&ConfigValue::Bool(false)));
    let (page, rain) = entry(&shell, "p:rain");
    shell.show(page);
    press(&mut shell, &mut s, rain);
    let streaks = ConfigValue::Text("streaks".into());
    assert_eq!(value(&shell, "p:rain"), Some(&streaks));
    // The slider's far right end is its maximum.
    let (page, fog) = entry(&shell, "p:fog");
    shell.show(page);
    let at = shell.point_at(&s, SIZE, fog);
    let (track, width) = Columns::of(SIZE.0, SIZE.1).track();
    shell.click(&mut s, (track + width, at.1), SIZE, false, false, false);
    shell.release();
    assert_eq!(value(&shell, "p:fog"), Some(&ConfigValue::Number(1.0)));
    assert_eq!(shell.screen.configs.chosen().len(), 3);
    // Reset all, on the first page, puts them all back, and is greyed once they are.
    shell.show(0);
    press(&mut shell, &mut s, Item::ResetAll);
    assert!(shell.screen.configs.chosen().is_empty());
    let sheets = shell.screen.sheets(&s);
    assert!(!sheets[0].groups[0].rows[0].live);
}

#[test]
fn the_presets_row_cycles_the_pack_wide_presets_and_shows_custom_otherwise() {
    let mut screen = PackOptionsScreen::open(with_presets());
    // The defaults are no preset.
    assert_eq!(screen.current_preset(), None);
    screen.cycle_preset(false);
    assert_eq!(screen.current_preset().as_deref(), Some("fast"));
    assert_eq!(screen.configs.get("p:fog"), Some(&ConfigValue::Number(0.0)));
    screen.cycle_preset(false);
    assert_eq!(screen.current_preset().as_deref(), Some("pretty"));
    screen.cycle_preset(true);
    assert_eq!(screen.current_preset().as_deref(), Some("fast"));
    // One value off the preset: custom again.
    screen.set("p:dust", ConfigValue::Number(0.3));
    assert_eq!(screen.current_preset(), None);
    let presets = &screen.sheets(&Settings::default())[0].groups[0].rows[0];
    assert_eq!(presets.id, Item::Presets);
    let custom = lang::get("horizon.pack.options.presets.custom");
    assert_eq!(presets.value, Value::Choice(custom));
}

#[test]
fn left_and_right_step_the_presets() {
    let mut s = Settings::default();
    let mut shell = shell_of(with_presets());
    shell.set_focus(Some(Target::Row(Item::Presets)));
    shell.key(&mut s, K::ArrowRight, None, false, SIZE);
    assert_eq!(shell.screen.current_preset().as_deref(), Some("fast"));
    shell.key(&mut s, K::ArrowRight, None, false, SIZE);
    assert_eq!(shell.screen.current_preset().as_deref(), Some("pretty"));
    shell.key(&mut s, K::ArrowLeft, None, false, SIZE);
    assert_eq!(shell.screen.current_preset().as_deref(), Some("fast"));
    assert_eq!(shell.focused_row(), Some(Item::Presets));
}

#[test]
fn sliders_stop_at_the_entrys_step() {
    let mut c = Configs::default();
    let json = serde_json::json!({"default_value": 4.0, "min": 0.0, "max": 10.0, "step": 2.0});
    c.add_entry("p:clouds".into(), &json);
    let mut s = Settings::default();
    let mut shell = shell_of(c);
    let row = &shell.screen.sheets(&s)[0].groups[1].rows[0];
    let stops = Value::Slider {
        position: 0.4,
        steps: Some(5),
        text: "4".into(),
    };
    assert_eq!(row.value, stops);
    // A press a little past the middle of the track lands on the step nearest it.
    let at = shell.point_at(&s, SIZE, Item::Entry(0));
    let (track, width) = Columns::of(SIZE.0, SIZE.1).track();
    shell.click(
        &mut s,
        (track + width * 0.55, at.1),
        SIZE,
        false,
        false,
        false,
    );
    shell.release();
    assert_eq!(value(&shell, "p:clouds"), Some(&ConfigValue::Number(6.0)));
    // Held by the press, Right goes one step on and Left one back.
    shell.set_focus(Some(Target::Row(Item::Entry(0))));
    shell.key(&mut s, K::ArrowRight, None, false, SIZE);
    assert_eq!(value(&shell, "p:clouds"), Some(&ConfigValue::Number(8.0)));
    shell.key(&mut s, K::ArrowLeft, None, false, SIZE);
    shell.key(&mut s, K::ArrowLeft, None, false, SIZE);
    assert_eq!(value(&shell, "p:clouds"), Some(&ConfigValue::Number(4.0)));
}

/// A language with the pack's own strings: a name, a section and a tooltip.
struct PackLang;
impl lang::ResourceSource for PackLang {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        (path == "assets/minecraft/lang/en_us.json").then(|| {
            let strings = serde_json::json!({
                "config.minecraft.haze": "Haze",
                "config.minecraft.haze.tooltip": "Thickens the distant air.",
                "config.minecraft.section.sky": "Sky and fog",
            });
            serde_json::to_vec(&strings).unwrap()
        })
    }
}

fn haze() -> Configs {
    let mut c = configs();
    let json =
        serde_json::json!({"default_value": false, "section": "sky", "performance_impact": "high"});
    c.add_entry("minecraft:haze".into(), &json);
    c
}

#[test]
fn the_search_finds_an_entry_by_its_tooltip_and_the_strip_has_its_impact() {
    let language = lang::Language::load(&[&PackLang], "en_us");
    lang::scoped(language, || {
        let s = Settings::default();
        let mut shell = shell_of(haze());
        let (page, haze) = entry(&shell, "minecraft:haze");
        shell.set_search("distant air");
        assert_eq!(shell.listed(&s, SIZE), [(page, haze)]);
        let sheets = shell.screen.sheets(&s);
        assert_eq!(sheets[usize::from(page)].name, "Sky and fog");
        let row = sheets.iter().flat_map(|p| &p.groups).flat_map(|g| &g.rows);
        let row = row.into_iter().find(|r| r.id == haze).unwrap();
        assert_eq!(row.name, "Haze");
        assert_eq!(
            row.about,
            "Thickens the distant air. Performance impact: high"
        );
    });
}

#[test]
fn tab_reaches_every_row_of_every_page() {
    let mut s = Settings::default();
    let mut shell = shell_of(haze());
    let pages = shell.screen.sheets(&s).len() as u16;
    let mut seen = Vec::new();
    for page in 0..pages {
        shell.show(page);
        shell.set_focus(None);
        for _ in 0..8 {
            shell.key(&mut s, K::Tab, None, false, SIZE);
            if let Some(row) = shell.focused_row()
                && !seen.contains(&row)
            {
                seen.push(row);
            }
        }
    }
    // Reset all is greyed at the defaults; every entry is reached.
    let mut entries: Vec<Item> = (0..shell.screen.ids.len() as u16)
        .map(Item::Entry)
        .collect();
    seen.sort_by_key(|item| format!("{item:?}"));
    entries.sort_by_key(|item| format!("{item:?}"));
    assert_eq!(seen, entries);
}

#[test]
fn undo_puts_the_entries_back_and_then_leaving_sends_nothing() {
    let mut s = Settings::default();
    let mut shell = shell_of(configs());
    let (page, birds) = entry(&shell, "p:birds");
    shell.show(page);
    press(&mut shell, &mut s, birds);
    assert!(shell.foot_at(&s, SIZE).iter().any(|f| f.0 == Target::Undo));
    assert!(shell.screen.finished().is_some());
    let undo = shell
        .foot_at(&s, SIZE)
        .into_iter()
        .find(|f| f.0 == Target::Undo);
    let [x, y, w, h] = undo.unwrap().1;
    shell.click(
        &mut s,
        (x + w / 2.0, y + h / 2.0),
        SIZE,
        false,
        false,
        false,
    );
    assert_eq!(value(&shell, "p:birds"), Some(&ConfigValue::Bool(true)));
    assert!(!shell.foot_at(&s, SIZE).iter().any(|f| f.0 == Target::Undo));
    assert_eq!(shell.screen.finished(), None);
    // Stepped there and back by hand, it sends nothing either.
    press(&mut shell, &mut s, birds);
    press(&mut shell, &mut s, birds);
    assert_eq!(shell.screen.finished(), None);
    press(&mut shell, &mut s, birds);
    let Some(Event::PackOptions { entries, chosen }) = shell.screen.finished() else {
        panic!("a change is sent");
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(chosen.get("p:birds"), Some(&ConfigValue::Bool(false)));
}
