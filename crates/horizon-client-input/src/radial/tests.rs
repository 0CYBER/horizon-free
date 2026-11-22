use super::*;

const UP: [f32; 2] = [0.0, 1.0];
const RIGHT: [f32; 2] = [1.0, 0.0];
const NEUTRAL: [f32; 2] = [0.0, 0.0];
/// The default pages' count.
const PAGES: usize = 2;

fn tilt() -> Tilt {
    Tilt::for_deadzone(TUNED_DEADZONE)
}
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Open with the stick at rest, then point it up.
fn pointing_up(now: Instant) -> RadialMenu {
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), now);
    menu.update(UP, tilt(), now);
    menu
}

/// A menu opened with the stick leant at `stick` since well before the press, as when it
/// was turning the camera.
fn opened_leaning(stick: [f32; 2], now: Instant) -> RadialMenu {
    let mut menu = RadialMenu::default();
    let before = now - LEAN_BEFORE_OPEN - ms(20);
    for step in [0, 10, 20, 50, 80, 110] {
        menu.sample([NEUTRAL, stick], before + ms(step));
    }
    menu.open(1, stick, tilt(), now);
    menu
}

#[test]
fn the_stick_points_at_eight_slots_clockwise_from_the_top() {
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let expected = [
        ([0.0, 1.0], 0),
        ([d, d], 1),
        ([1.0, 0.0], 2),
        ([d, -d], 3),
        ([0.0, -1.0], 4),
        ([-d, -d], 5),
        ([-1.0, 0.0], 6),
        ([-d, d], 7),
    ];
    for (stick, slot) in expected {
        assert_eq!(slot_at(stick, SELECT_TILT), Some(slot), "{stick:?}");
    }
    // Just left of straight up is still the top slot, not an out-of-range ninth.
    assert_eq!(slot_at([-0.01, 1.0], SELECT_TILT), Some(0));
    assert_eq!(slot_at([0.2, 0.2], SELECT_TILT), None);
    assert_eq!(slot_at([f32::NAN, 1.0], SELECT_TILT), None);
}

#[test]
fn a_release_commits_the_selection_once() {
    let now = Instant::now();
    let mut menu = pointing_up(now);
    assert_eq!(menu.commit(now), Some(0));
    assert!(!menu.is_open());
    assert_eq!(menu.commit(now), None, "one execution per selection");
}

#[test]
fn the_overlay_waits_for_a_held_opener_but_the_stick_counts_from_the_press() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    assert!(!menu.shown(t + DRAW_DELAY - ms(1)));
    assert!(menu.shown(t + DRAW_DELAY));
    menu.update(UP, tilt(), t + ms(10));
    assert_eq!(menu.selected(), Some(0), "aimed before it showed");
    // A quick tap: nothing shown, nothing selected, nothing run.
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.update(NEUTRAL, tilt(), t + ms(30));
    assert_eq!(menu.commit(t + ms(40)), None);
    // Latched open (toggle mode), it shows at once.
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.latch();
    assert!(menu.latched() && menu.shown(t));
}

#[test]
fn the_stick_drawn_and_the_wait_for_it_to_be_let_go() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    assert!(!menu.waiting());
    let mut menu_leaning = opened_leaning(UP, t);
    assert!(menu_leaning.waiting(), "leaning when it opened");
    menu_leaning.update([0.0, 2.0], tilt(), t);
    assert_eq!(menu_leaning.stick(), [0.0, 1.0], "no further than the edge");
    menu_leaning.update([0.3, 0.0], tilt(), t);
    assert_eq!(menu_leaning.stick(), [0.3, 0.0]);
    menu_leaning.update(NEUTRAL, tilt(), t);
    assert!(!menu_leaning.waiting());
    menu_leaning.update(UP, tilt(), t);
    menu_leaning.turn_page(true, UP, tilt(), PAGES);
    assert!(menu_leaning.waiting(), "held through the page turn");
    menu_leaning.update(RIGHT, tilt(), t);
    assert!(!menu_leaning.waiting());
    menu_leaning.cancel();
    assert!(!menu_leaning.waiting());
    menu.cancel();
}

#[test]
fn a_flick_that_starts_with_the_press_selects_and_a_lean_from_before_waits() {
    let t = Instant::now();
    // The thumb rolls onto the opener as the other flicks: leant for less than the lean.
    let mut menu = RadialMenu::default();
    menu.sample([NEUTRAL, NEUTRAL], t - ms(200));
    menu.sample([NEUTRAL, NEUTRAL], t - ms(100));
    menu.sample([NEUTRAL, UP], t - ms(40));
    menu.sample([NEUTRAL, UP], t);
    menu.open(1, UP, tilt(), t);
    assert!(
        !menu.waiting(),
        "a flick with the press is aimed at the menu"
    );
    menu.update(UP, tilt(), t + ms(10));
    assert_eq!(menu.selected(), Some(0));
    // Leant since before: turning the camera.
    let mut menu = opened_leaning(UP, t);
    menu.update(UP, tilt(), t);
    assert_eq!(menu.selected(), None);
    // A stick let go in between counts from when it leant again.
    let mut menu = RadialMenu::default();
    menu.sample([NEUTRAL, UP], t - ms(300));
    menu.sample([NEUTRAL, UP], t - ms(150));
    menu.sample([NEUTRAL, NEUTRAL], t - ms(60));
    menu.sample([NEUTRAL, UP], t - ms(30));
    menu.open(1, UP, tilt(), t);
    assert!(!menu.waiting());
    // No history (the first poll): not leaning.
    let mut menu = RadialMenu::default();
    menu.open(1, UP, tilt(), t);
    assert!(!menu.waiting());
}

#[test]
fn a_release_without_a_selection_does_nothing() {
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), Instant::now());
    menu.update(NEUTRAL, tilt(), Instant::now());
    assert!(!menu.aimed());
    assert_eq!(menu.commit(Instant::now()), None);
}

#[test]
fn neutral_keeps_the_selection_until_the_retention_runs_out() {
    let t = Instant::now();
    let just_before = t + NEUTRAL_RETENTION - ms(1);

    let mut menu = pointing_up(t);
    menu.update(NEUTRAL, tilt(), t);
    menu.update(NEUTRAL, tilt(), just_before);
    assert_eq!(menu.selected(), Some(0));
    assert_eq!(menu.commit(just_before), Some(0));

    // Expired, seen by a frame: still aimed, for the buzz of a release that runs nothing.
    let mut menu = pointing_up(t);
    menu.update(NEUTRAL, tilt(), t);
    menu.update(NEUTRAL, tilt(), t + NEUTRAL_RETENTION);
    assert_eq!(menu.selected(), None);
    assert!(menu.aimed());
    assert_eq!(menu.commit(t + NEUTRAL_RETENTION), None);

    // Expired between frames: the release itself looks at the clock.
    let mut menu = pointing_up(t);
    menu.update(NEUTRAL, tilt(), t);
    assert_eq!(menu.commit(t + NEUTRAL_RETENTION), None);
}

#[test]
fn a_flick_out_and_back_between_two_frames_selects() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.update_with_peak(NEUTRAL, RIGHT, tilt(), t + ms(30));
    assert_eq!(menu.selected(), Some(2), "the peak since the last frame");
    assert_eq!(menu.stick(), NEUTRAL, "the stick drawn where it is");
    assert_eq!(menu.commit(t + ms(60)), Some(2));
    // A peak no further than the stick changes nothing.
    let mut menu = pointing_up(t);
    menu.update_with_peak(UP, [0.1, 0.1], tilt(), t);
    assert_eq!(menu.selected(), Some(0));
}

#[test]
fn pointing_again_restarts_the_retention() {
    let t = Instant::now();
    let mut menu = pointing_up(t);
    menu.update(NEUTRAL, tilt(), t);
    menu.update(RIGHT, tilt(), t + ms(200));
    menu.update(NEUTRAL, tilt(), t + ms(300));
    let later = t + ms(300) + NEUTRAL_RETENTION - ms(1);
    assert_eq!(menu.commit(later), Some(2));
}

#[test]
fn cancelling_commits_nothing_even_during_retention() {
    let t = Instant::now();
    let mut menu = pointing_up(t);
    menu.update(NEUTRAL, tilt(), t);
    menu.cancel();
    assert!(!menu.is_open());
    // The opener is still down when B cancels; it comes up afterwards.
    assert_eq!(menu.commit(t + ms(1)), None);
}

#[test]
fn a_page_turn_clears_the_selection_wraps_and_says_so() {
    let t = Instant::now();
    let mut menu = pointing_up(t);
    assert_eq!(menu.turn_page(false, UP, tilt(), PAGES), Some(true));
    assert_eq!((menu.page(), menu.selected()), (1, None));
    assert_eq!(menu.turn_page(true, UP, tilt(), PAGES), Some(true));
    assert_eq!(menu.page(), 0);
    assert_eq!(menu.turn_page(true, UP, tilt(), 3), Some(false));
    // During retention too.
    menu.update(RIGHT, tilt(), t);
    menu.update(NEUTRAL, tilt(), t);
    menu.turn_page(true, NEUTRAL, tilt(), 3);
    assert_eq!(menu.commit(t), None);
}

#[test]
fn a_stick_held_through_a_page_turn_or_the_opening_selects_only_once_it_moves() {
    let t = Instant::now();
    let mut menu = pointing_up(t);
    menu.turn_page(true, UP, tilt(), PAGES);
    menu.update(UP, tilt(), t);
    assert_eq!(
        menu.selected(),
        None,
        "still pointing where the old page's slot was"
    );
    menu.update(RIGHT, tilt(), t);
    assert_eq!((menu.page(), menu.commit(t)), (1, Some(2)));

    // A stick turning the camera when the opener goes down.
    let mut menu = opened_leaning(UP, t);
    menu.update(UP, tilt(), t);
    assert_eq!(menu.selected(), None);
    menu.update(NEUTRAL, tilt(), t);
    menu.update(UP, tilt(), t);
    assert_eq!(menu.selected(), Some(0), "back to neutral, then aimed");

    // Arcing round to another slot, or leaning further, isn't aiming either.
    let mut menu = opened_leaning(UP, t);
    menu.update(RIGHT, tilt(), t);
    assert_eq!(menu.selected(), None, "arced to another slot");
    let mut menu = opened_leaning([0.0, 0.4], t);
    menu.update(UP, tilt(), t);
    assert_eq!(
        menu.selected(),
        None,
        "under the tilt at open, then past it"
    );
    // A drifting stick at rest doesn't count as leaning.
    let mut menu = opened_leaning([0.1, 0.1], t);
    menu.update(UP, tilt(), t);
    assert_eq!(menu.selected(), Some(0));
}

#[test]
fn a_leaning_stick_waits_until_it_is_at_rest_not_only_under_the_select_tilt() {
    let t = Instant::now();
    let mut menu = opened_leaning(UP, t);
    // Wobbling between the rest and the select tilts: still turning the camera.
    menu.update([0.0, 0.45], tilt(), t + ms(10));
    assert!(menu.waiting(), "under the select tilt is not let go");
    menu.update(UP, tilt(), t + ms(20));
    assert_eq!(menu.selected(), None);
    menu.update(NEUTRAL, tilt(), t + ms(30));
    assert!(!menu.waiting());
    menu.update(UP, tilt(), t + ms(40));
    assert_eq!(menu.selected(), Some(0));
}

#[test]
fn a_stick_leaning_since_the_opening_still_waits_after_a_page_turn() {
    let t = Instant::now();
    let mut menu = opened_leaning(UP, t);
    // Swung round to another slot and the page turned, without ever being let go.
    menu.update(RIGHT, tilt(), t);
    menu.turn_page(true, RIGHT, tilt(), PAGES);
    menu.update(UP, tilt(), t);
    assert!(menu.waiting());
    assert_eq!(menu.selected(), None, "aimed without having been let go");
    menu.update(NEUTRAL, tilt(), t);
    menu.update(UP, tilt(), t);
    assert_eq!(menu.selected(), Some(0));
}

#[test]
fn a_selected_slot_holds_past_its_wedge_and_under_the_tilt() {
    let t = Instant::now();
    let at = |degrees: f32, length: f32| {
        let r = degrees.to_radians();
        [r.sin() * length, r.cos() * length]
    };
    let mut menu = pointing_up(t);
    // The boundary with the next slot is 22.5 degrees; the margin is about 6.9.
    menu.update(at(25.0, 1.0), tilt(), t);
    assert_eq!(menu.selected(), Some(0));
    menu.update(at(-25.0, 1.0), tilt(), t);
    assert_eq!(menu.selected(), Some(0), "the same on its other side");
    menu.update(at(35.0, 1.0), tilt(), t);
    assert_eq!(menu.selected(), Some(1));
    // Back near the boundary it stays on the new slot.
    menu.update(at(20.0, 1.0), tilt(), t);
    assert_eq!(menu.selected(), Some(1));
    // A light hold: 0.4 keeps it, under 0.35 is neutral, and 0.4 doesn't select anew.
    let mut menu = pointing_up(t);
    menu.update(at(0.0, 0.4), tilt(), t);
    menu.update(at(0.0, 0.4), tilt(), t + ms(300));
    assert_eq!(menu.selected(), Some(0));
    menu.update(at(0.0, 0.3), tilt(), t + ms(300));
    menu.update(at(0.0, 0.3), tilt(), t + ms(600));
    assert_eq!(menu.selected(), None);
    menu.update(at(0.0, 0.4), tilt(), t + ms(600));
    assert_eq!(menu.selected(), None);
}

#[test]
fn the_tilts_follow_a_raised_deadzone_and_stay_in_reach() {
    let t = Instant::now();
    // A worn stick that reaches no further than this on any side.
    const WORN_REACH: f32 = 0.94;
    for deadzone in [0.1, 0.36, 0.5, 0.9] {
        let tilt = Tilt::for_deadzone(deadzone);
        assert!(
            tilt.select >= tilt.keep && tilt.keep >= deadzone + KEEP_OVER_DEADZONE,
            "{deadzone}: {tilt:?}"
        );
        assert!(tilt.select <= SELECT_TILT_MAX, "{deadzone}: {tilt:?}");
        // Every slot is reached by the worn stick.
        for slot in 0..SLOTS {
            let angle = (slot as f32 * 45.0).to_radians();
            let stick = [angle.sin() * WORN_REACH, angle.cos() * WORN_REACH];
            let mut menu = RadialMenu::default();
            menu.open(1, NEUTRAL, tilt, t);
            menu.update(stick, tilt, t);
            assert_eq!(menu.selected(), Some(slot), "{deadzone}: slot {slot}");
        }
        // A stick drifting just inside the deadzone lets go of the selection.
        let mut menu = RadialMenu::default();
        menu.open(1, NEUTRAL, tilt, t);
        menu.update(UP, tilt, t);
        let drift = [0.0, deadzone - 0.01];
        menu.update(drift, tilt, t);
        menu.update(drift, tilt, t + NEUTRAL_RETENTION);
        assert_eq!(menu.selected(), None, "{deadzone}: a drift kept it");
    }
    // At the default deadzone they are as they were tuned.
    let tuned = Tilt::for_deadzone(crate::gamepad::PadSettings::default().look_deadzone);
    assert!((tuned.select - SELECT_TILT).abs() < 1e-6);
    assert!((tuned.keep - SELECT_TILT * KEEP_TILT).abs() < 1e-6);
}

#[test]
fn a_bumper_on_a_single_page_keeps_the_selection() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.update(UP, tilt(), t);
    assert_eq!(menu.turn_page(true, UP, tilt(), 1), None);
    assert_eq!((menu.page(), menu.selected()), (0, Some(0)));
}

#[test]
fn every_hold_opens_on_the_first_page() {
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), Instant::now());
    menu.turn_page(true, NEUTRAL, tilt(), PAGES);
    assert_eq!(menu.page(), 1);
    menu.cancel();
    menu.open(1, NEUTRAL, tilt(), Instant::now());
    assert_eq!(menu.page(), 0);
}

#[test]
fn a_d_pad_press_just_after_the_close_is_a_rolled_thumb_but_not_the_opener() {
    let t = Instant::now();
    let mut settle = DpadSettle::default();
    assert!(!settle.quiet(false, false, t));
    settle.closed(t);
    assert!(
        settle.quiet(false, false, t + ms(10)),
        "rolled onto the next button"
    );
    assert!(
        !settle.quiet(true, false, t + ms(10)),
        "the opener again opens the menu"
    );
    assert!(
        !settle.quiet(false, false, t + DPAD_SETTLE),
        "a press of its own"
    );
}

#[test]
fn the_d_pad_settles_from_the_openers_release_after_a_cancel_with_it_held() {
    // B cancels at t with the opener held; the thumb lets it go 250 ms later and rolls on.
    let t = Instant::now();
    let mut settle = DpadSettle::default();
    settle.closed(t);
    let up = t + ms(250);
    assert!(settle.quiet(false, true, up - ms(1)), "still held");
    settle.opener_up(up);
    assert!(
        settle.quiet(false, false, up + ms(30)),
        "rolled off the opener"
    );
    assert!(!settle.quiet(false, false, up + DPAD_SETTLE));
    // A release with no close before it settles nothing.
    let mut fresh = DpadSettle::default();
    fresh.opener_up(up);
    assert!(!fresh.quiet(false, false, up + ms(1)));
}

#[test]
fn latched_open_the_selection_outlives_the_stick() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.latch();
    menu.update(UP, tilt(), t);
    menu.update(NEUTRAL, tilt(), t + ms(10));
    let later = t + NEUTRAL_RETENTION * 4;
    menu.update(NEUTRAL, tilt(), later);
    assert_eq!(
        menu.selected(),
        Some(0),
        "let go, then A pressed a while later"
    );
    // Another slot, or a page turn, still changes it.
    menu.update(RIGHT, tilt(), later);
    assert_eq!(menu.selected(), Some(2));
    menu.turn_page(true, NEUTRAL, tilt(), PAGES);
    assert_eq!(menu.selected(), None);
    menu.update(UP, tilt(), later);
    menu.update(NEUTRAL, tilt(), later + ms(10));
    assert_eq!(menu.commit(later + NEUTRAL_RETENTION * 2), Some(0));
    // Selected while held, then latched: kept too.
    let mut menu = pointing_up(t);
    menu.update(NEUTRAL, tilt(), t + ms(5));
    menu.latch();
    assert_eq!(menu.commit(t + NEUTRAL_RETENTION * 2), Some(0));
}

#[test]
fn each_stick_leans_on_its_own() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    // The left stick walked all along; the right moved only with the press.
    for step in [200, 150, 100, 50] {
        menu.sample([UP, NEUTRAL], t - ms(step));
    }
    menu.sample([UP, UP], t);
    menu.open(1, UP, tilt(), t);
    assert!(!menu.waiting(), "the right stick's own history");
    menu.cancel();
    menu.open(0, UP, tilt(), t);
    assert!(menu.waiting(), "the left had been leaning");
}

#[test]
fn a_stick_that_takes_over_selects_at_once() {
    // Either: opened on the walking left stick, which waits; the right flicks up and takes over.
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    for step in [200, 150, 100, 50] {
        menu.sample([UP, NEUTRAL], t - ms(step));
    }
    menu.open(0, UP, tilt(), t);
    assert!(menu.waiting());
    menu.took_over();
    menu.update(RIGHT, tilt(), t + ms(30));
    assert_eq!(menu.selected(), Some(2));
    assert_eq!(menu.commit(t + ms(40)), Some(2));
}

#[test]
fn toggle_s_a_runs_the_slot_settled_on_not_the_one_the_thumb_crossed_on_its_way() {
    let t = Instant::now();
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.latch();
    menu.update(RIGHT, tilt(), t + ms(10));
    // Held on Right a while, then the thumb slides off down-right towards A.
    let down_right = [0.7, -0.7];
    menu.update(down_right, tilt(), t + ms(400));
    assert_eq!(menu.selected(), Some(3));
    assert_eq!(
        menu.commit_settled(t + ms(430)),
        Some(2),
        "the slide crossed Down-Right"
    );
    // Aimed and left there, A runs it.
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.latch();
    menu.update(RIGHT, tilt(), t + ms(10));
    menu.update(down_right, tilt(), t + ms(400));
    assert_eq!(menu.commit_settled(t + ms(400) + A_SETTLE), Some(3));
    // A first selection, however quick, is the one run.
    let mut menu = RadialMenu::default();
    menu.open(1, NEUTRAL, tilt(), t);
    menu.latch();
    menu.update(RIGHT, tilt(), t + ms(10));
    assert_eq!(menu.commit_settled(t + ms(20)), Some(2));
}

#[test]
fn a_selection_is_said_once_it_settles() {
    let t = Instant::now();
    let mut settle = Settle::default();
    assert_eq!(settle.poll(Some(1), t), None);
    // Swept on before it settled: only where it stops is said.
    assert_eq!(settle.poll(Some(2), t + ms(50)), None);
    assert_eq!(settle.poll(Some(2), t + ms(50) + NARRATE_SETTLE), Some(2));
    assert_eq!(settle.poll(Some(2), t + ms(500)), None, "said once");
    // Let go, then aimed there again: said again.
    settle.poll(None, t + ms(600));
    settle.poll(Some(2), t + ms(700));
    assert_eq!(settle.poll(Some(2), t + ms(700) + NARRATE_SETTLE), Some(2));
}

#[test]
fn the_default_pages_hold_only_actions_without_a_button_on_the_straight_directions() {
    let pages = Pages::default();
    assert_eq!(pages.len(), PAGES);
    for i in 0..pages.len() {
        for (slot, cell) in pages.page(i).slots().into_iter().enumerate() {
            let Some(action) = cell.action() else {
                assert!(cell.is_empty());
                continue;
            };
            assert_eq!(
                crate::gamepad::PadSettings::default_of(action.action()),
                None,
                "{action:?} has a button of its own"
            );
            assert_eq!(slot % 2, 0, "{action:?} on a diagonal");
        }
    }
    use RadialAction as A;
    let actions = |page: usize| pages.page(page).slots().map(Slot::action);
    assert_eq!(
        actions(0),
        [
            Some(A::Advancements),
            None,
            Some(A::Command),
            None,
            Some(A::SocialInteractions),
            None,
            None,
            None,
        ]
    );
    // Emote runs only with cosmetics on: the picker has it, the defaults don't.
    assert!(
        (0..pages.len()).all(|i| !actions(i).contains(&Some(A::Emote))),
        "a default slot that may not run"
    );
    assert!(RadialAction::ALL.contains(&A::Emote));
}

#[test]
fn every_action_reaches_the_game_and_has_one_id_in_its_namespace() {
    for action in RadialAction::ALL {
        assert!(
            crate::gamepad::route(action.action()).is_some(),
            "{action:?}"
        );
        assert_eq!(RadialAction::from_id(action.id()), Some(action));
        // Vanilla's by their key mapping, Horizon's own in its namespace.
        if !action.is_horizons() {
            assert_eq!(action.id(), action.action(), "{action:?}");
            assert!(action.id().starts_with("key."), "{action:?}");
        }
    }
    use RadialAction as A;
    let horizons: Vec<_> = RadialAction::ALL
        .into_iter()
        .filter(|action| action.is_horizons())
        .collect();
    assert_eq!(horizons, [A::Emote, A::HotbarPrevious, A::HotbarNext]);
    // Held actions, the opener itself and anything else aren't slot actions.
    for id in [
        "key.attack",
        crate::gamepad::RADIAL,
        "/op someone",
        "horizon:",
    ] {
        assert_eq!(RadialAction::from_id(id), None, "{id}");
    }
}

#[test]
fn ids_saved_before_the_namespaces_read_as_their_actions() {
    use RadialAction as A;
    let read = Pages::parse(
        [(
            "radial_0_slots",
            "key.emote,pad.hotbarPrevious,pad.hotbarNext,,,,,",
        )]
        .into_iter(),
    )
    .unwrap();
    let actions = read.page(0).slots().map(Slot::action);
    assert_eq!(
        actions[..3],
        [Some(A::Emote), Some(A::HotbarPrevious), Some(A::HotbarNext)]
    );
    // Written back under their new ids, which read the same.
    let text = read.serialize();
    assert!(
        text.contains("horizon:emote,horizon:hotbar_previous,horizon:hotbar_next"),
        "{text}"
    );
    assert_eq!(Pages::parse(lines(&text)), Some(read));
}

fn lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| l.split_once(':'))
}

#[test]
fn pages_survive_their_text_with_empty_slots_where_they_were() {
    let mut pages = Pages::default();
    pages.rename(1, "Build: mine");
    pages.set_slot(1, 0, Slot::Empty);
    pages.set_slot(1, 7, Slot::Action(RadialAction::Drop));
    let added = pages.add().unwrap();
    pages.set_slot(added, 4, Slot::Action(RadialAction::Chat));
    // A player's text that looks like a key stays their text.
    let typed = pages.add().unwrap();
    pages.rename(typed, CAMERA_PAGE_NAME);
    let text = pages.serialize();
    assert_eq!(Pages::parse(lines(&text)), Some(pages));
}

#[test]
fn what_this_game_doesnt_know_survives_a_save() {
    let file = [
        ("radialLayout", "1"),
        // A slot from a newer Horizon or a mod, a field and a kind it doesn't read.
        ("radial_0_label", "Mine"),
        ("radial_0_slots", "key.chat,mymod:dash,,,,,,"),
        ("radial_0_icon", "minecraft:diamond"),
        ("radial_1_kind", "items"),
        ("radial_x_name", "not a number"),
        ("radial_2_kind", "hotbar"),
        ("radial_3_preset", "combat"),
    ];
    let pages = Pages::parse(file.into_iter()).unwrap();
    let page = pages.page(0);
    assert_eq!(page.slot(0), &Slot::Action(RadialAction::Chat));
    assert_eq!(page.slot(1), &Slot::Missing("mymod:dash".to_owned()));
    assert_eq!(page.slot(1).action(), None, "never runs");
    assert_eq!(pages.page(2).kind, PageKind::Hotbar);
    assert!(pages.page(2).slots().iter().all(|slot| slot.is_empty()));
    let text = pages.serialize();
    for (key, value) in file {
        assert!(
            text.contains(&format!("{key}:{value}\n")),
            "{key}:{value} lost from\n{text}"
        );
    }
    assert_eq!(Pages::parse(lines(&text)), Some(pages));
}

#[test]
fn a_kind_this_game_doesnt_know_keeps_its_lines_and_gains_none() {
    let file = [
        ("radial_0_label", "Loadout"),
        ("radial_0_kind", "items"),
        ("radial_0_slots", "minecraft:torch,minecraft:bread"),
    ];
    let mut pages = Pages::parse(file.into_iter()).unwrap();
    assert!(pages.page(0).slots().iter().all(|slot| slot.is_empty()));
    let text = pages.serialize();
    let written: Vec<_> = lines(&text)
        .filter(|(k, _)| k.starts_with("radial_"))
        .collect();
    assert_eq!(written, file, "nothing added, nothing lost:\n{text}");
    // Filled in the editor, it is a page of slots.
    pages.set_slot(0, 2, Slot::Action(RadialAction::Chat));
    let text = pages.serialize();
    assert!(!text.contains("_kind:"), "{text}");
    assert!(text.contains("radial_0_slots:,,key.chat,,,,,\n"), "{text}");
}

#[test]
fn slot_ids_past_the_eighth_are_kept() {
    let ids = "key.chat,,,,,,,key.drop,mymod:ninth,mymod:tenth";
    let pages =
        Pages::parse([("radial_0_label", "Wide"), ("radial_0_slots", ids)].into_iter()).unwrap();
    assert_eq!(pages.page(0).slot(7), &Slot::Action(RadialAction::Drop));
    let text = pages.serialize();
    assert!(text.contains(&format!("radial_0_slots:{ids}\n")), "{text}");
    // Trailing commas are no slots.
    let pages = Pages::parse(
        [
            ("radial_0_label", "W"),
            ("radial_0_slots", "key.chat,,,,,,,,,"),
        ]
        .into_iter(),
    )
    .unwrap();
    assert!(
        pages
            .serialize()
            .contains("radial_0_slots:key.chat,,,,,,,\n")
    );
}

#[test]
fn the_layout_is_read_and_a_newer_ones_kept() {
    // This game's layout, and a file from before the line: a page saved by its slots that is a
    // default page follows the defaults again.
    let general_by_slots = [
        ("radial_0_name", GENERAL_PAGE_NAME),
        (
            "radial_0_slots",
            "key.advancements,,key.command,,key.socialInteractions,,,",
        ),
    ];
    for layout in [None, Some("1")] {
        let file = layout
            .map(|l| (LAYOUT_KEY, l))
            .into_iter()
            .chain(general_by_slots);
        let pages = Pages::parse(file).unwrap();
        assert_eq!(pages.page(0).preset(), Some(Preset::General), "{layout:?}");
        assert!(
            pages
                .serialize()
                .starts_with(&format!("{LAYOUT_KEY}:{RADIAL_LAYOUT}\n"))
        );
    }
    // A newer Horizon's: its number stays, and its pages stay as saved, never turned into
    // this game's presets (which may hold something else there).
    let newer = RADIAL_LAYOUT + 1;
    let newer_text = newer.to_string();
    let file = [(LAYOUT_KEY, newer_text.as_str())]
        .into_iter()
        .chain(general_by_slots);
    let mut pages = Pages::parse(file).unwrap();
    assert_eq!(pages.page(0).preset(), None);
    pages.set_slot(0, 6, Slot::Action(RadialAction::Chat));
    pages.set_slot(0, 6, Slot::Empty);
    assert_eq!(pages.page(0).preset(), None, "edited back, still its own");
    let text = pages.serialize();
    assert!(
        text.starts_with(&format!("{LAYOUT_KEY}:{newer}\n")),
        "{text}"
    );
    assert!(!text.contains("_preset:"), "{text}");
    assert_eq!(Pages::parse(lines(&text)), Some(pages));
    // A newer layout alone (its pages were its defaults) is kept too.
    let only = Pages::parse([(LAYOUT_KEY, newer_text.as_str())].into_iter()).unwrap();
    assert!(
        only.serialize()
            .starts_with(&format!("{LAYOUT_KEY}:{newer}\n"))
    );
    assert_ne!(only, Pages::default(), "so the settings file writes it");
    // Today's layout alone is the defaults.
    assert_eq!(Pages::parse([(LAYOUT_KEY, "1")].into_iter()), None);
}

#[test]
fn pages_past_the_editors_most_are_read_and_those_past_the_files_most_kept() {
    let file: Vec<_> = (0..PARSE_MAX + 2)
        .map(|i| (format!("radial_{i}_label"), format!("Page {i}")))
        .collect();
    let pages = Pages::parse(file.iter().map(|(k, v)| (k.as_str(), v.as_str()))).unwrap();
    assert_eq!(
        pages.len(),
        MAX_PAGES,
        "the menu and the editor show the first ones"
    );
    let text = pages.serialize();
    for (key, value) in &file {
        assert!(text.contains(&format!("{key}:{value}\n")), "{key} lost");
    }
    // A page that goes brings the next one forward.
    let mut fewer = pages.clone();
    fewer.remove(0);
    assert_eq!(
        page_label(fewer.page(MAX_PAGES - 1)),
        format!("Page {MAX_PAGES}")
    );
    // The editor adds none while more are kept.
    assert_eq!(fewer.clone().add(), None);
    assert_eq!(Pages::parse(lines(&text)), Some(pages));
}

fn page_label(page: &Page) -> String {
    match &page.name {
        PageName::Builtin(key) => (*key).to_owned(),
        PageName::Custom(name) => name.clone(),
    }
}

#[test]
fn an_untouched_default_page_is_saved_as_its_preset_and_follows_the_defaults() {
    let mut pages = Pages::default();
    pages.set_slot(0, 1, Slot::Action(RadialAction::Chat));
    let text = pages.serialize();
    assert!(text.contains("radial_1_preset:camera\n"), "{text}");
    assert!(!text.contains("radial_1_slots"), "{text}");
    assert!(
        text.contains("radial_0_slots:"),
        "the changed page by its slots"
    );
    // Read back, the preset is the default page as it is now, whatever was saved beside it.
    let read = Pages::parse(
        [
            ("radial_0_preset", "camera"),
            ("radial_0_slots", "key.drop,,,,,,,"),
        ]
        .into_iter(),
    )
    .unwrap();
    assert_eq!(read.page(0), Pages::default().page(1));
    assert_eq!(read.page(0).preset(), Some(Preset::Camera));
    // Renamed, it is the player's.
    pages.rename(1, "Shots");
    assert_eq!(pages.page(1).preset(), None);
    assert!(pages.serialize().contains("radial_1_slots:key.screenshot"));
}

#[test]
fn an_edit_back_to_the_default_keeps_the_page_following_the_defaults() {
    let mut pages = Pages::default();
    let chat = Slot::Action(RadialAction::Chat);
    // The same action again, or the same name, is no change.
    pages.set_slot(0, 0, Slot::Action(RadialAction::Advancements));
    assert_eq!(pages.page(0).preset(), Some(Preset::General));
    pages.rename(1, "Shots");
    let renamed = pages.clone();
    pages.rename(1, "Shots");
    assert_eq!(pages, renamed);
    // Changed, then changed back.
    pages.set_slot(0, 1, chat.clone());
    assert_eq!(pages.page(0).preset(), None);
    pages.set_slot(0, 1, Slot::Empty);
    assert_eq!(pages.page(0).preset(), Some(Preset::General));
    assert_eq!(pages.page(0), Pages::default().page(0));
    // Read back from its slots too (a file from before presets).
    let read = Pages::parse(
        [
            ("radial_0_name", GENERAL_PAGE_NAME),
            (
                "radial_0_slots",
                "key.advancements,,key.command,,key.socialInteractions,,,",
            ),
        ]
        .into_iter(),
    )
    .unwrap();
    assert_eq!(read.page(0).preset(), Some(Preset::General));
}

#[test]
fn a_touched_page_drops_a_preset_it_cant_read() {
    let mut pages =
        Pages::parse([("radial_0_preset", "combat"), ("radial_0_label", "Fight")].into_iter())
            .unwrap();
    assert!(pages.serialize().contains("radial_0_preset:combat"));
    pages.set_slot(0, 0, Slot::Action(RadialAction::Chat));
    let text = pages.serialize();
    assert!(!text.contains("preset"), "{text}");
    assert!(text.contains("radial_0_slots:key.chat"), "{text}");
}

#[test]
fn a_damaged_file_loses_only_the_damaged_parts() {
    let text = [
        ("radial_0_label", "Mine\u{7}"),
        // An unknown action keeps its position; the action after it stays seventh.
        ("radial_0_slots", "key.chat,/stop,,,,,key.drop,"),
        (
            "radial_1_slots",
            "key.drop,key.drop,key.drop,key.drop,key.drop,key.drop,key.drop,key.drop,key.chat",
        ),
        // Only Horizon's keys go in `name`: anything else leaves a new page's name.
        ("radial_2_name", "Building"),
    ];
    let pages = Pages::parse(text.into_iter()).unwrap();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages.page(0).name, PageName::Custom("Mine".to_owned()));
    assert_eq!(pages.page(0).slot(0), &Slot::Action(RadialAction::Chat));
    assert_eq!(pages.page(0).slot(1), &Slot::Missing("/stop".to_owned()));
    assert_eq!(pages.page(0).slot(6), &Slot::Action(RadialAction::Drop));
    assert_eq!(pages.page(1).name, PageName::Builtin(NEW_PAGE_NAME));
    assert_eq!(pages.page(1).slot(7), &Slot::Action(RadialAction::Drop));
    assert_eq!(pages.page(2).name, PageName::Builtin(NEW_PAGE_NAME));
    assert_eq!(Pages::parse(std::iter::empty()), None);
}

#[test]
fn page_names_keep_whose_they_are() {
    let read = [
        ("radial_0_name", GENERAL_PAGE_NAME),
        ("radial_1_label", "Building"),
    ];
    let pages = Pages::parse(read.into_iter()).unwrap();
    assert_eq!(pages.page(0).name, PageName::Builtin(GENERAL_PAGE_NAME));
    assert_eq!(pages.page(1).name, PageName::Custom("Building".to_owned()));
    // A renamed default page stays the player's, even renamed to a key.
    let mut pages = Pages::default();
    pages.rename(0, GENERAL_PAGE_NAME);
    assert_eq!(
        pages.page(0).name,
        PageName::Custom(GENERAL_PAGE_NAME.to_owned())
    );
    let read = Pages::parse(lines(&pages.serialize())).unwrap();
    assert_eq!(
        read.page(0).name,
        PageName::Custom(GENERAL_PAGE_NAME.to_owned())
    );
    assert_eq!(read.page(1).name, PageName::Builtin(CAMERA_PAGE_NAME));
    // A name with nothing in it is a new page's.
    assert_eq!(
        PageName::custom(" \u{7} "),
        PageName::Builtin(NEW_PAGE_NAME)
    );
}

#[test]
fn removing_an_action_leaves_the_others_in_place() {
    let mut pages = Pages::default();
    let before = pages.page(0).clone();
    pages.set_slot(0, 2, Slot::Empty);
    let after = pages.page(0);
    assert!(after.slot(2).is_empty());
    for i in (0..SLOTS).filter(|i| *i != 2) {
        assert_eq!(after.slot(i), before.slot(i));
    }
}

#[test]
fn a_slot_set_on_a_hotbar_page_gives_it_slots() {
    let mut pages = Pages::parse([("radial_0_kind", "hotbar")].into_iter()).unwrap();
    pages.set_slot(0, 3, Slot::Action(RadialAction::Chat));
    assert_eq!(pages.page(0).slot(3), &Slot::Action(RadialAction::Chat));
    assert!(matches!(pages.page(0).kind, PageKind::Fixed(_)));
}

#[test]
fn the_last_page_cant_be_deleted_and_the_count_is_bounded() {
    let mut pages = Pages::default();
    pages.remove(1);
    pages.remove(0);
    assert_eq!(pages.len(), 1);
    while pages.add().is_some() {}
    assert_eq!(pages.len(), MAX_PAGES);
}

#[test]
fn page_indices_past_the_end_mean_the_last_page() {
    let pages = Pages::default();
    assert_eq!(pages.clamp(5), 1);
    assert_eq!(pages.page(9), pages.page(1));
}

#[test]
fn pages_numbered_with_gaps_close_up_in_order() {
    let label = |i: usize| (format!("radial_{i}_label"), format!("Page {i}"));
    let read = |numbers: &[usize]| {
        let lines: Vec<_> = numbers.iter().map(|i| label(*i)).collect();
        let pages = Pages::parse(lines.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let pages = pages.unwrap();
        (0..pages.len())
            .map(|i| pages.page(i).name.clone())
            .collect::<Vec<_>>()
    };
    let named = |numbers: &[usize]| {
        let names = numbers
            .iter()
            .map(|i| PageName::Custom(format!("Page {i}")));
        names.collect::<Vec<_>>()
    };
    // Page 7 was past the most pages, yet only the third: it stays.
    assert_eq!(read(&[7, 0, 2]), named(&[0, 2, 7]));
    // Eight pages: the first six by number show.
    let eight = [9, 1, 30, 4, 12, 0, 6, 2];
    assert_eq!(read(&eight), named(&[0, 1, 2, 4, 6, 9]));
}
