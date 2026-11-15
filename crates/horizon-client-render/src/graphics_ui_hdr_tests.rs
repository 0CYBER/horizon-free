//! The graphics screen's HDR rows against what the renderer's output offers and its display
//! reports.
use super::tests::{CHANGED, CLICK, DRAGGED, FB, NONE, SIZE, Ui, offering, press, row, ui};
use super::*;
use crate::settings_shell::{Columns, Hosted};
use winit::keyboard::KeyCode as K;

/// On a device that can't draw HDR's frame the whole page waits: the HDR row reads Off and
/// says why, a press leaves the saved mode alone, and Auto asks for nothing.
#[test]
fn hdr_waits_for_a_device_that_draws_its_frame() {
    use crate::gfx::{HdrOutput, output::tests};
    let unsupported = "Your GPU or graphics API doesn't support it.";
    let mut s = Settings::default();
    for mode in [HdrMode::On, HdrMode::Auto] {
        s.horizon.hdr = mode;
        // The display offers HDR and reports it; the device has no layer for the HUD.
        let (mut caps, reading) = tests::window(Some(HdrOutput::Pq), Some(true));
        caps.hud_layer = false;
        let out = tests::snapshot(&s.horizon, Default::default(), caps, reading);
        assert!(out.hdr_offered() && !out.requests_hdr(mode) && !out.hdr_on());
        let hdr = Item::Horizon(H::Hdr);
        assert_eq!(hdr.blocked(&s, &out), Some(unsupported));
        assert_eq!(hdr.label(&s, &out).plain(), "HDR: Off");
        for h in [H::HdrCalibrate, H::HdrPeak, H::HdrLighting] {
            assert_eq!(Item::Horizon(h).blocked(&s, &out), Some(unsupported));
        }
        let mut ui = ui();
        ui.screen.output = out;
        let [x, y, ..] = row(&mut ui, &s, hdr);
        assert!(press(&mut ui, &mut s, (x + 8.0, y + 4.0)).is_empty());
        assert_eq!(s.horizon.hdr, mode);
    }
}

#[test]
fn hdr_lighting_is_greyed_only_while_a_pack_runs() {
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::On;
    let item = Item::Horizon(H::HdrLighting);
    let lights = Some("The shader pack lights the world itself.");
    let out = offering(true, None);
    // No pack running, as with one picked but refused, missing or its mod off: live.
    assert_eq!(item.blocked(&s, &out.with_pack(false)), None);
    assert_eq!(item.blocked(&s, &out.with_pack(true)), lights);
    // HDR's own reasons come first.
    assert_eq!(
        item.blocked(&s, &NONE.with_pack(true)),
        Some("Your display or system doesn't offer HDR output.")
    );
    s.horizon.hdr = HdrMode::Off;
    assert_eq!(
        item.blocked(&s, &out.with_pack(true)),
        Some("Turn HDR on to use it.")
    );
}

/// Under a `shaders/` pack HDR output shows the pack's picture, the HUD in it, at paper white:
/// UI Brightness waits, greyed. The rest stay live, Peak Brightness too: it is still the peak
/// the metadata declares and where the black level's lift fades out. Under no pack or one of
/// Horizon's own, on the layered frame, all of them are live.
#[test]
fn ui_brightness_is_greyed_under_a_shaders_pack() {
    use crate::gfx::{FrameRequest, HdrOutput, output::tests};
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::On;
    let (caps, reading) = tests::window(Some(HdrOutput::Pq), Some(true));
    let shown = |shaders_pack| {
        let request = FrameRequest {
            shaders_pack,
            ..Default::default()
        };
        tests::snapshot(&s.horizon, request, caps, reading).with_pack(shaders_pack)
    };
    let (layered, flat) = (shown(false), shown(true));
    assert!(layered.hdr_on() && flat.hdr_on());
    let ui_brightness = Item::Horizon(H::HdrUiBrightness);
    assert_eq!(ui_brightness.blocked(&s, &layered), None);
    assert_eq!(
        ui_brightness.blocked(&s, &flat),
        Some("The shader pack's picture is shown at Paper White.")
    );
    for h in [
        H::Hdr,
        H::HdrCalibrate,
        H::HdrBlackLevel,
        H::HdrPeak,
        H::HdrPaperWhite,
    ] {
        assert_eq!(Item::Horizon(h).blocked(&s, &flat), None, "{h:?}");
    }
    // The greyed slider takes no press, and its saved value waits; Peak Brightness's drags.
    let saved = s.horizon.hdr_ui_nits;
    let mut ui = ui();
    ui.screen.output = flat;
    let (tx, _) = Columns::of(SIZE.0, SIZE.1).track();
    let greyed = row(&mut ui, &s, ui_brightness);
    assert!(press(&mut ui, &mut s, (tx + 2.0, greyed[1] + 4.0)).is_empty());
    assert_eq!(s.horizon.hdr_ui_nits, saved);
    let peak = row(&mut ui, &s, Item::Horizon(H::HdrPeak));
    assert_eq!(press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)), [DRAGGED]);
}

#[test]
fn hdr_auto_can_be_selected_before_querying_and_brightness_waits_for_output() {
    let mut s = Settings::default();
    let mut ui = ui();
    let at = |[x, y, ..]: [f32; 4]| (x + 8.0, y + 4.0);
    // Off has not asked the system yet: Auto must still be reachable to start the watch.
    let short = (427.0, 240.0);
    let mut at_hdr = Ui::default();
    at_hdr.show(Page::Hdr);
    at_hdr.key(&mut s, K::ArrowDown, None, false, short);
    assert_eq!(at_hdr.focused_row(), Some(Item::Horizon(H::Hdr)));
    let hdr = row(&mut ui, &s, Item::Horizon(H::Hdr));
    assert_eq!(press(&mut ui, &mut s, at(hdr)), [CLICK, CHANGED]);
    assert_eq!(s.horizon.hdr, HdrMode::Auto);
    assert_eq!(press(&mut ui, &mut s, at(hdr)), [CLICK, CHANGED]);
    assert_eq!(s.horizon.hdr, HdrMode::Off);
    // Brightness still needs an output, even with On saved from another computer.
    s.horizon.hdr = HdrMode::On;
    for h in [H::HdrCalibrate, H::HdrPeak, H::HdrPaperWhite] {
        assert!(Item::Horizon(h).blocked(&s, &NONE).is_some(), "{h:?}");
    }
    s.horizon.hdr = HdrMode::Off;
    ui.screen.output = offering(true, None);
    let (tx, _) = Columns::of(SIZE.0, SIZE.1).track();
    let peak = row(&mut ui, &s, Item::Horizon(H::HdrPeak));
    assert!(press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)).is_empty());
    // The choice turns it on, and the sliders below it work.
    let hdr = row(&mut ui, &s, Item::Horizon(H::Hdr));
    assert_eq!(press(&mut ui, &mut s, at(hdr)), [CLICK, CHANGED]);
    assert!(ui.screen.output.requests_hdr(s.horizon.hdr));
    let peak = row(&mut ui, &s, Item::Horizon(H::HdrPeak));
    assert_eq!(press(&mut ui, &mut s, (tx + 2.0, peak[1] + 4.0)), [DRAGGED]);
    ui.release();
    // Calibrate starts the calibration.
    let calibrate = row(&mut ui, &s, Item::Horizon(H::HdrCalibrate));
    assert_eq!(
        press(&mut ui, &mut s, at(calibrate)),
        [CLICK, Output::Push(OptionsPage::HdrCalibration)]
    );
    assert!(ui.changed());
}

#[test]
fn hdr_auto_is_a_choice_on_a_reporting_sdr_display() {
    let out = offering(false, Some(false));
    let mut s = Settings::default();
    let item = Item::Horizon(H::Hdr);
    assert_eq!(item.switch(&s), None);
    assert_eq!(item.blocked(&s, &out), None);
    item.cycle(&mut s, false, FB, &out);
    assert_eq!(s.horizon.hdr, HdrMode::Auto); // On requires an offered output.
    assert!(!out.requests_hdr(s.horizon.hdr));
    assert!(Item::Horizon(H::HdrPeak).blocked(&s, &out).is_some());
    item.cycle(&mut s, true, FB, &out);
    assert_eq!(s.horizon.hdr, HdrMode::Off);
    let out = offering(true, Some(false));
    for expected in [HdrMode::On, HdrMode::Auto, HdrMode::Off] {
        item.cycle(&mut s, false, FB, &out);
        assert_eq!(s.horizon.hdr, expected);
    }
    s.horizon.hdr = HdrMode::Auto;
    let out = offering(true, Some(true));
    assert!(out.requests_hdr(s.horizon.hdr));
    assert_eq!(Item::Horizon(H::HdrPeak).blocked(&s, &out), None);
}

/// The labels and the calibration row read the report of the renderer they are shown
/// for: two windows' screens, one whose display reports 400/1000 nits and one that
/// reports nothing, side by side.
#[test]
fn auto_levels_and_setup_read_their_own_renderers_report() {
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::Auto;
    let reporting = offering(true, Some(true));
    let silent = NONE;
    let label = |h, out| Item::Horizon(h).label(&s, out).plain();
    assert_eq!(
        label(H::HdrPaperWhite, &reporting),
        "Paper White: Auto (400 nits)"
    );
    assert_eq!(
        label(H::HdrPaperWhite, &silent),
        "Paper White: Auto (203 nits)"
    );
    assert_eq!(
        label(H::HdrPeak, &reporting),
        "Peak Brightness: Auto (1000 nits)"
    );
    assert_eq!(label(H::HdrCalibrate, &reporting), "Calibrate Anyway...");
    assert_eq!(label(H::HdrCalibrate, &silent), "Calibrate...");
}

/// The brightness rows show their unit: the nits of the signal Horizon sends (PQ, or scRGB's
/// 80-nit units), Auto with the nits it gives and a value set by hand alike.
#[test]
fn brightness_rows_read_in_nits() {
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::On;
    let label = |s: &Settings, h| Item::Horizon(h).label(s, &NONE).plain();
    assert_eq!(label(&s, H::HdrPaperWhite), "Paper White: Auto (203 nits)");
    assert_eq!(label(&s, H::HdrPeak), "Peak Brightness: Auto (1000 nits)");
    assert_eq!(
        label(&s, H::HdrUiBrightness),
        "UI Brightness: Auto (203 nits)"
    );
    assert_eq!(label(&s, H::HdrBlackLevel), "Black Level: 0.00 nits");
    s.horizon.hdr_paper_white = 160;
    s.horizon.hdr_peak = 1000;
    s.horizon.hdr_ui_nits = 250;
    s.horizon.hdr_black_level = 5;
    assert_eq!(label(&s, H::HdrPaperWhite), "Paper White: 160 nits");
    assert_eq!(label(&s, H::HdrPeak), "Peak Brightness: 1000 nits");
    assert_eq!(label(&s, H::HdrUiBrightness), "UI Brightness: 250 nits");
    assert_eq!(label(&s, H::HdrBlackLevel), "Black Level: 0.05 nits");
}

/// The setup rows' label and description for each kind of report: none, a compositor's
/// (Wayland), and Windows', whose peak may be the display's or a calibration profile's.
#[test]
fn the_setup_rows_ask_to_calibrate_unless_the_system_set_the_display_up() {
    let trusted = offering(true, Some(true));
    let mut guessed = trusted;
    guessed.reported = guessed.reported.map(|r| crate::graphics::Reported {
        peak_guessed: true,
        ..r
    });
    let rows = |s: &Settings, out: &EffectiveOutput| {
        [H::HdrCalibrate, H::HdrPeak, H::HdrPaperWhite].map(|h| {
            let item = Item::Horizon(h);
            (item.label(s, out).plain(), description(item, s, out, "-"))
        })
    };
    let calibrate = "Calibrate...";
    let set_up = "Set up for your display. Leave on Auto unless it looks wrong.";
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::On;
    let none = (
        calibrate.into(),
        "Your display didn't say how bright it gets: calibrate to set it up.",
    );
    assert_eq!(
        rows(&s, &NONE),
        [
            none.clone(),
            ("Peak Brightness: Auto (1000 nits)".into(), "-"),
            ("Paper White: Auto (203 nits)".into(), "-"),
        ]
    );
    assert_eq!(
        rows(&s, &guessed),
        [
            (
                calibrate.into(),
                "Auto uses the peak Windows reports. Calibrate if highlights look clipped."
            ),
            ("Peak Brightness: Auto (1000 nits)".into(), "-"),
            ("Paper White: Auto (400 nits)".into(), "-"),
        ]
    );
    assert_eq!(
        rows(&s, &trusted),
        [
            (
                "Calibrate Anyway...".into(),
                "Already set up for your display. Only calibrate if HDR looks wrong."
            ),
            ("Peak Brightness: Auto (1000 nits)".into(), set_up),
            ("Paper White: Auto (400 nits)".into(), set_up),
        ]
    );
    // Calibrated or set by hand, either report offers its automatic setup back.
    s.horizon.hdr_peak = 800;
    // Peak's Auto has no known source on Windows: only Paper White is said to be set up there.
    let changed = |peak| {
        [
            (
                "Use Automatic Setup".into(),
                "Puts Peak Brightness and Paper White back to your display's setup.",
            ),
            ("Peak Brightness: 800 nits".into(), peak),
            ("Paper White: Auto (400 nits)".into(), set_up),
        ]
    };
    assert_eq!(rows(&s, &guessed), changed("-"));
    assert_eq!(rows(&s, &trusted), changed(set_up));
    assert_eq!(rows(&s, &NONE)[0], none);
    let mut ui = ui();
    ui.screen.output = guessed;
    let at = |[x, y, ..]: [f32; 4]| (x + 8.0, y + 4.0);
    let calibrate = row(&mut ui, &s, Item::Horizon(H::HdrCalibrate));
    assert_eq!(press(&mut ui, &mut s, at(calibrate)), [CLICK, CHANGED]);
    assert_eq!((s.horizon.hdr_peak, s.horizon.hdr_paper_white), (0, 0));
    assert_eq!(
        press(&mut ui, &mut s, at(calibrate)),
        [CLICK, Output::Push(OptionsPage::HdrCalibration)]
    );
}

/// UI Brightness reads as Paper White does, in nits apart from it: Auto shows the paper white
/// in use, and a value set by hand its own nits, whatever Paper White is. Its slider has Paper
/// White's steps: Auto, then 10 to 480 nits by 10.
#[test]
fn ui_brightness_reads_in_nits_on_paper_whites_steps() {
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::On;
    let reporting = offering(true, Some(true));
    let label = |s: &Settings, out| Item::Horizon(H::HdrUiBrightness).label(s, out).plain();
    assert_eq!(label(&s, &reporting), "UI Brightness: Auto (400 nits)");
    assert_eq!(label(&s, &NONE), "UI Brightness: Auto (203 nits)");
    s.horizon.hdr_paper_white = 250;
    assert_eq!(label(&s, &reporting), "UI Brightness: Auto (250 nits)");
    s.horizon.hdr_ui_nits = 160;
    for paper_white in [0, 80, 250] {
        s.horizon.hdr_paper_white = paper_white;
        assert_eq!(label(&s, &reporting), "UI Brightness: 160 nits");
    }
    let item = Item::Horizon(H::HdrUiBrightness);
    assert_eq!(
        H::HdrUiBrightness.slider_steps(),
        H::HdrPaperWhite.slider_steps()
    );
    H::HdrUiBrightness.set_slider(&mut s, 0.0);
    let mut reached = vec![s.horizon.hdr_ui_nits];
    for _ in 0..H::HdrUiBrightness.slider_steps().unwrap() {
        item.step_slider(&mut s, true, 200.0, &reporting);
        reached.push(s.horizon.hdr_ui_nits);
    }
    let paper: Vec<u32> = std::iter::once(0).chain((10..=480).step_by(10)).collect();
    assert_eq!(reached, paper);
    // Each value sits where Paper White's same value does.
    for nits in paper {
        s.horizon.hdr_ui_nits = nits;
        s.horizon.hdr_paper_white = nits;
        assert_eq!(item.slider(&s), Item::Horizon(H::HdrPaperWhite).slider(&s));
    }
}

/// While HDR output is held back on this window's driver, the HDR row says so and keeps
/// the mode; any change of the mode drops the hold, and HDR is asked for again.
#[test]
fn a_held_hdr_start_is_said_on_the_page_and_a_mode_change_tries_again() {
    use crate::gfx::{HdrOutput, output::tests};
    let mut s = Settings::default();
    s.horizon.hdr = HdrMode::Auto;
    s.horizon.hdr_failed_start = Some("Vulkan 10de:2684 GPU; driver; 1.0".into());
    let (mut caps, reading) = tests::window(Some(HdrOutput::Pq), Some(true));
    caps.held = true;
    let out = tests::snapshot(&s.horizon, Default::default(), caps, reading);
    let hdr = Item::Horizon(H::Hdr);
    let setting = h(
        H::Hdr,
        Cost::None,
        "Lava, fire and the sun shine brighter than white.",
    );
    let held = super::row(setting, &s, &out);
    assert_eq!(held.chosen.as_deref(), Some(HDR_HELD));
    assert_eq!(held.why, None);
    assert_eq!(hdr.label(&s, &out).plain(), "HDR: Auto");
    assert_eq!(Item::Horizon(H::HdrPeak).blocked(&s, &out), Some(HDR_HELD));
    // Not held, the row says what HDR does.
    caps.held = false;
    let free = tests::snapshot(&s.horizon, Default::default(), caps, reading);
    assert_eq!(super::row(setting, &s, &free).chosen, None);
    // Any step of the mode, either way, tries again.
    for back in [false, true] {
        let mut s = s.clone();
        hdr.cycle(&mut s, back, FB, &out);
        assert_ne!(s.horizon.hdr, HdrMode::Auto);
        assert_eq!(s.horizon.hdr_failed_start, None);
    }
}

/// Undo never brings the hold back: held Auto, changed to On (HDR tried again), then
/// undone, is Auto without the hold. With the mode left alone, it keeps the hold as it is.
#[test]
fn undo_never_restores_a_held_hdr_start() {
    let mut screen = GraphicsScreen {
        framebuffer: FB,
        output: NONE,
    };
    let mut opened = Settings::default();
    opened.horizon.hdr = HdrMode::Auto;
    opened.horizon.hdr_failed_start = Some("Vulkan 10de:2684 GPU; driver; 1.0; os".into());
    let mut s = opened.clone();
    let out = offering(true, Some(true));
    Item::Horizon(H::Hdr).cycle(&mut s, true, FB, &out);
    assert_eq!(
        (s.horizon.hdr, &s.horizon.hdr_failed_start),
        (HdrMode::On, &None)
    );
    screen.undo(&mut s, &opened);
    assert_eq!(
        (s.horizon.hdr, &s.horizon.hdr_failed_start),
        (HdrMode::Auto, &None)
    );
    // Another row changed and undone: the hold stays.
    let mut s = opened.clone();
    s.horizon.fxaa = !s.horizon.fxaa;
    screen.undo(&mut s, &opened);
    assert_eq!(s.horizon, opened.horizon);
}

/// The Calibrate row's descriptions fit the strip's two lines at vanilla's 427-wide GUI (a
/// 1080p window at scale 4 is wider): a longer one would end in "…".
#[test]
fn the_calibrate_descriptions_fit_the_strip() {
    let atlas = crate::hud::HudAtlas::build(std::path::Path::new("/nonexistent")).unwrap();
    let mut guessed = offering(true, Some(true));
    guessed.reported = guessed.reported.map(|r| crate::graphics::Reported {
        peak_guessed: true,
        ..r
    });
    let mut changed = Settings::default();
    changed.horizon.hdr_peak = 800;
    let item = Item::Horizon(H::HdrCalibrate);
    let width = Columns::of(427.0, 240.0).dock[2] - 24.0;
    for (s, out) in [
        (Settings::default(), NONE),
        (Settings::default(), guessed),
        (Settings::default(), offering(true, Some(true))),
        (changed, guessed),
    ] {
        let text = description(item, &s, &out, "-");
        assert_ne!(text, "-");
        let lines = atlas.font.split(&Text::literal(text.to_owned()), width);
        assert!(lines.len() <= 2, "{text}");
    }
}
