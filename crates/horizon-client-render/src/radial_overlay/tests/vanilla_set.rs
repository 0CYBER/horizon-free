//! The dial on vanilla's surfaces (`radial_overlay/vanilla.rs`): the same promises as the
//! Horizon set beside it, with vanilla's sizes (its tab bar, hotbar cells, inventory slots where
//! a cell doesn't fit), no slash, and the hub's lines cut as vanilla cuts them.
use super::super::vanilla::{self as look, GREY, SHADE};
use super::super::*;
use super::{ALL, VIEWS, atlas, colours_in, dial, draw, inside, rooms, shown};
use crate::hud_kit::{icon::NoItems, room::Keep};
use crate::lang;
use crate::menu_style::{self, Surfaces};

fn english() -> lang::Language {
    lang::Language::load(&[], "en_us")
}

fn vanilla(body: fn()) {
    menu_style::with_surfaces(Surfaces::Vanilla, body)
}

/// Survival's rows above the hotbar, from their top (hearts, food and armour; absorption's
/// second row of hearts over that), as the Horizon set has them, under 0, 1 and 2 boss bars.
fn with_rows(w: f32, h: f32) -> Vec<(f32, usize, Room)> {
    [h - 49.0, h - 59.0]
        .into_iter()
        .flat_map(|top| {
            let rows = [(w - 182.0) / 2.0, top, 182.0, h - top];
            rooms(w, h)
                .into_iter()
                .enumerate()
                .map(move |(bars, room)| (top, bars, room.with(Keep::StatusRows, rows)))
        })
        .collect()
}

#[test]
fn the_layout_takes_vanilla_s_sizes() {
    vanilla(|| {
        let full = Layout::new(427.0, 240.0, &Room::new(427.0, 240.0));
        // No chips: a toggle's state is in the hub's tooltip.
        assert!(full.roomy && !full.chips && full.cancel);
        assert_eq!(
            (full.tab_h, full.tile, full.r),
            (look::TAB_H_VANILLA, look::FRAME, 64.0)
        );
        // A view whose own share would be smaller still gets a whole frame.
        let quarter = Layout::new(320.0, 180.0, &Room::new(320.0, 180.0));
        assert_eq!(
            (quarter.tab_h, quarter.tile),
            (look::TAB_H_VANILLA, look::FRAME)
        );
    });
}

#[test]
fn a_picked_toggle_shows_its_name_never_a_bare_state() {
    vanilla(|| {
        let mut pages = Pages::default();
        pages.set_slot(0, 0, Slot::Action(RadialAction::TogglePerspective));
        pages.set_slot(0, 2, Slot::Action(RadialAction::Advancements));
        let room = Room::new(427.0, 240.0);
        let lay = Layout::new(427.0, 240.0, &room);
        let lines = |selected, view| {
            let seen = Seen {
                page: 0,
                selected: Some(selected),
                waiting: false,
            };
            let mut d = dial(&pages, &room, seen, ALL);
            d.toggles.view = view;
            let (lines, _) = lang::scoped(english(), || {
                look::tooltip_lines(&atlas(), &lay, &d, pages.page(0))
            });
            lines.into_iter().map(|l| l.0).collect::<Vec<_>>()
        };
        // What it is, whatever its state: never a bare "Third Person" or "Off".
        assert_eq!(lines(0, View::ThirdPersonBack), ["Perspective"]);
        assert_eq!(lines(0, View::FirstPerson), ["Perspective"]);
        // Not a toggle: its name alone.
        assert_eq!(lines(2, View::FirstPerson), ["Advancements"]);
    });
}

#[test]
fn a_tight_view_shrinks_its_cells_rather_than_cover_the_hud_rows() {
    vanilla(|| {
        for (w, h) in [(427.0, 240.0), (320.0, 180.0)] {
            for (top, bars, room) in with_rows(w, h) {
                let lay = Layout::new(w, h, &room);
                // One bar stays clear; under two, the tabs may go over them (as the dial's own
                // look does), never the ring over the rows.
                assert!(
                    lay.header()[1] >= room.top() || bars > 1,
                    "{w}x{h}, rows from {top}: the tabs over a boss bar"
                );
                for slot in 0..SLOTS {
                    let [x, y, t, _] = grown(lay.tile_rect(slot), GROW);
                    assert!(
                        y + t <= top,
                        "{w}x{h}, rows from {top}: slot {slot} reaches {}",
                        y + t
                    );
                    assert!(x >= 0.0 && x + t <= w);
                }
            }
        }
        // 320x180 under a boss bar and both rows: vanilla's inventory slots, not hotbar cells.
        let (_, _, room) = with_rows(320.0, 180.0)
            .into_iter()
            .find(|(top, bars, _)| *bars == 1 && *top == 180.0 - 59.0)
            .unwrap();
        let lay = Layout::new(320.0, 180.0, &room);
        assert_eq!(lay.tile, look::SMALL_CELL);
        assert!(lay.header()[1] >= room.top());
    });
}

#[test]
fn cells_sit_on_their_rays_clear_of_each_other_the_hub_and_the_tabs() {
    vanilla(|| {
        for (w, h) in VIEWS {
            for room in rooms(w, h) {
                let lay = Layout::new(w, h, &room);
                let header = lay.header();
                assert!(
                    header[1] >= room.top() || h < ROOMY_H,
                    "{w}x{h}: the tabs under a boss bar"
                );
                for slot in 0..SLOTS {
                    let a = grown(lay.tile_rect(slot), GROW);
                    assert!(inside(a, w, h), "{w}x{h} slot {slot}: {a:?}");
                    assert!(!overlaps(a, header), "{w}x{h}: slot {slot} meets the tabs");
                    if slot % 2 == 0 {
                        assert!(
                            !meets_disc(a, [lay.cx, lay.cy], lay.hub),
                            "{w}x{h}: slot {slot} reaches into the hub"
                        );
                    }
                    for other in slot + 1..SLOTS {
                        let b = grown(lay.tile_rect(other), GROW);
                        assert!(!overlaps(a, b), "{w}x{h}: slots {slot} and {other} overlap");
                    }
                }
            }
        }
    });
}

#[test]
fn everything_drawn_stays_inside_the_view() {
    vanilla(|| {
        let pages = Pages::default();
        for size in [
            [1280, 720],
            [854, 480],
            [640, 360],
            [427, 240],
            [320, 180],
            [213, 120],
            [427, 120],
        ] {
            let (w, h) = (size[0] as f32, size[1] as f32);
            let room = Room::new(w, h);
            let seen = Seen {
                page: 1,
                selected: Some(2),
                waiting: false,
            };
            let batch = draw(size, &dial(&pages, &room, seen, ALL), &shown(seen));
            // The shade's soft rim may run past the view, where nothing draws; nothing else may.
            let shade = |c: u32| c & 0x00FF_FFFF == 0 && (c >> 24) as u8 <= SHADE[0];
            for v in batch.vertices.iter().filter(|v| !shade(v.color)) {
                let [x, y] = v.pos;
                assert!(
                    (-0.01..=w + 0.01).contains(&x) && (-0.01..=h + 0.01).contains(&y),
                    "{size:?}: a vertex at {x} {y}"
                );
            }
        }
    });
}

#[test]
fn an_action_that_can_t_run_is_dimmed_not_slashed_and_the_tooltip_names_it() {
    vanilla(|| {
        let pages = Pages::default();
        let page = pages.page(0);
        let slot = (0..SLOTS).find(|i| !page.slot(*i).is_empty()).unwrap();
        let room = Room::new(427.0, 240.0);
        let seen = Seen {
            page: 0,
            selected: Some(slot),
            waiting: false,
        };
        let mut available = ALL;
        available[slot] = false;
        let mut blocked = dial(&pages, &room, seen, available);
        blocked.why[slot] = Some("horizon.radial.why.cosmetics");
        let batch = draw([427, 240], &blocked, &shown(seen));
        let lay = Layout::new(427.0, 240.0, &room);
        assert!(!colours_in(&batch, lay.tile_rect(slot)).contains(&SLASH));
        // Its name alone: the dimmed slot says it can't run; the hub gives no reason.
        let Slot::Action(action) = page.slot(slot) else {
            unreachable!()
        };
        let (lines, _) = lang::scoped(english(), || {
            let name = action_label(*action);
            (look::tooltip_lines(&atlas(), &lay, &blocked, page), name)
        });
        assert_eq!(lines.0, [(lines.1, look::WHITE)]);
    });
}

#[test]
fn a_missing_slot_shows_its_mark_and_the_tooltip_says_missing() {
    vanilla(|| {
        let pages = Pages::parse([("radial_0_slots", "mymod:dash,,,,,,,")].into_iter()).unwrap();
        let room = Room::new(427.0, 240.0);
        let seen = Seen {
            page: 0,
            selected: Some(0),
            waiting: false,
        };
        let mut available = ALL;
        available[0] = false;
        let d = dial(&pages, &room, seen, available);
        let lay = Layout::new(427.0, 240.0, &room);
        let english = lang::Language::load(&[], "en_us");
        let (lines, _) = lang::scoped(english, || {
            look::tooltip_lines(&atlas(), &lay, &d, pages.page(0))
        });
        let lines: Vec<&str> = lines.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(lines, ["Missing"]);
        let colours = colours_in(&draw([427, 240], &d, &shown(seen)), lay.tile_rect(0));
        assert!(!colours.contains(&SLASH), "a slash on a missing slot");
        assert!(
            colours.contains(&shapes::faded(GREY, DIM)),
            "no mark in the missing slot"
        );
    });
}

#[test]
fn long_names_are_cut_as_vanilla_cuts_them() {
    vanilla(|| {
        let atlas = atlas();
        let ((), _) = lang::scoped(english(), || {
            for (w, h) in VIEWS {
                let room = Room::new(w, h);
                let lay = Layout::new(w, h, &room);
                let width = look::tooltip_room(&lay);
                for action in RadialAction::ALL {
                    let mut pages = Pages::default();
                    pages.set_slot(0, 0, Slot::Action(action));
                    let seen = Seen {
                        page: 0,
                        selected: Some(0),
                        waiting: false,
                    };
                    let d = dial(&pages, &room, seen, ALL);
                    let lines = look::tooltip_lines(&atlas, &lay, &d, pages.page(0));
                    let name = action_label(action);
                    assert!(
                        atlas.font.width(&lines[0].0) <= width,
                        "{w}x{h}: {name} too wide"
                    );
                    if atlas.font.width(&name) > width {
                        let kept = lines[0].0.strip_suffix(fit::ELLIPSIS).expect("an ellipsis");
                        assert!(
                            name.starts_with(kept),
                            "{w}x{h}: {name} cut to {}",
                            lines[0].0
                        );
                    } else {
                        assert_eq!(lines[0].0, name);
                    }
                }
            }
        });
    });
}

#[test]
fn the_tooltip_stays_clear_of_the_cells() {
    vanilla(|| {
        let english = lang::Language::load(&[], "en_us");
        let atlas = atlas();
        let ((), _) = lang::scoped(english, || {
            let mut views: Vec<(f32, f32, Room)> = VIEWS
                .iter()
                .flat_map(|&(w, h)| rooms(w, h).into_iter().map(move |r| (w, h, r)))
                .collect();
            for (w, h) in [(427.0, 240.0), (320.0, 180.0)] {
                views.extend(with_rows(w, h).into_iter().map(|(_, _, r)| (w, h, r)));
            }
            for (w, h, room) in views {
                let lay = Layout::new(w, h, &room);
                for action in RadialAction::ALL {
                    let mut pages = Pages::default();
                    pages.set_slot(0, 0, Slot::Action(action));
                    let seen = Seen {
                        page: 0,
                        selected: Some(0),
                        waiting: false,
                    };
                    let mut available = ALL;
                    available[0] = false;
                    let mut d = dial(&pages, &room, seen, available);
                    d.why[0] = Some("horizon.radial.why.cosmetics");
                    let lines = look::tooltip_lines(&atlas, &lay, &d, pages.page(0));
                    let Some(rect) = look::tooltip_rect(&atlas, &lay, &lines) else {
                        continue;
                    };
                    let shown = look::tooltip_shown(rect);
                    for slot in 0..SLOTS {
                        let cell = grown(look::cell_rect(&lay, slot), GROW);
                        assert!(
                            !overlaps(shown, cell),
                            "{w}x{h}: {action:?}'s tooltip {shown:?} over cell {slot} {cell:?}"
                        );
                    }
                }
            }
        });
    });
}

#[test]
fn the_tabs_are_equal_and_the_open_one_sits_on_the_dial_s_centre() {
    vanilla(|| {
        let atlas = atlas();
        let mut three = Pages::default();
        let build = three.add().unwrap();
        three.rename(build, "Build");
        let english = english();
        let ((), _) = lang::scoped(english, || {
            for (w, h) in [
                (1280.0, 720.0),
                (640.0, 360.0),
                (427.0, 240.0),
                (480.0, 270.0),
                (320.0, 180.0),
            ] {
                let room = Room::new(w, h);
                let lay = Layout::new(w, h, &room);
                for page in 0..three.len() {
                    let seen = Seen {
                        page,
                        selected: None,
                        waiting: false,
                    };
                    let d = dial(&three, &room, seen, ALL);
                    let tabs = look::tab_rects(&atlas, &lay, &d);
                    assert_eq!(tabs.len(), 3, "{w}x{h}");
                    let tw = tabs[0].1[2];
                    assert!(
                        tabs.iter().all(|(_, r)| r[2] == tw),
                        "{w}x{h}: unequal tabs"
                    );
                    let (_, open) = tabs.iter().find(|(i, _)| *i == page).unwrap();
                    assert_eq!(
                        open[0] + open[2] / 2.0,
                        lay.cx,
                        "{w}x{h} page {page}: the open tab"
                    );
                    // The row is symmetric about the dial.
                    let (l, r) = (tabs[0].1[0], tabs[2].1[0] + tw);
                    assert_eq!(l + r, 2.0 * lay.cx, "{w}x{h}: the row");
                }
            }
        });
    });
}

#[test]
fn every_default_item_shows_its_name_in_the_hub_whatever_the_toggles() {
    vanilla(|| {
        let atlas = atlas();
        let pages = Pages::default();
        let room = Room::new(427.0, 240.0);
        let lay = Layout::new(427.0, 240.0, &room);
        let ((), _) = lang::scoped(english(), || {
            for page in 0..pages.len() {
                for slot in 0..SLOTS {
                    let Slot::Action(action) = pages.page(page).slot(slot) else {
                        continue;
                    };
                    let seen = Seen {
                        page,
                        selected: Some(slot),
                        waiting: false,
                    };
                    let mut d = dial(&pages, &room, seen, ALL);
                    // Whatever the toggles are, the hub names the item.
                    d.toggles.cinematic = true;
                    d.toggles.hud_hidden = true;
                    d.toggles.view = View::ThirdPersonFront;
                    let lines = look::tooltip_lines(&atlas, &lay, &d, pages.page(page));
                    let name = action_label(*action);
                    let room = look::tooltip_room(&lay);
                    let cut = fit::clip_text(&atlas, &name, room).into_owned();
                    assert_eq!(lines, [(cut, look::WHITE)], "{action:?}");
                }
            }
        });
    });
}

/// The test atlas with stand-ins for the slots' sprites, so their quads (and tints) are drawn.
fn atlas_with_slots() -> HudAtlas {
    let mut atlas = atlas();
    let rect = crate::hud::Rect {
        u0: 0.0,
        u1: 1.0,
        v0: 0.0,
        v1: 1.0,
        w: 18,
        h: 18,
    };
    for sprite in [look::SLOT, look::TASK_FRAME[0], look::TASK_FRAME[1]] {
        atlas.sprites.insert(sprite.to_owned(), rect);
    }
    atlas
}

#[test]
fn an_empty_or_unavailable_slot_is_faded_in_frames_and_inventory_slots_alike() {
    vanilla(|| {
        let atlas = atlas_with_slots();
        let pages = Pages::default();
        let page = pages.page(0);
        let live = (0..SLOTS).find(|s| !page.slot(*s).is_empty()).unwrap();
        let empty = (0..SLOTS).find(|s| page.slot(*s).is_empty()).unwrap();
        let blocked = (live + 1..SLOTS)
            .find(|s| !page.slot(*s).is_empty())
            .unwrap();
        // Roomy: frames. 320x180 under a boss bar with both HUD rows: inventory slots.
        let (_, _, tight) = with_rows(320.0, 180.0)
            .into_iter()
            .find(|(top, bars, _)| *bars == 1 && *top == 180.0 - 59.0)
            .unwrap();
        for (size, room, small) in [
            ([427, 240], Room::new(427.0, 240.0), false),
            ([320, 180], tight, true),
        ] {
            let lay = Layout::new(size[0] as f32, size[1] as f32, &room);
            assert_eq!(lay.tile < look::least_tile(), small, "{size:?}");
            let seen = Seen {
                page: 0,
                selected: None,
                waiting: false,
            };
            let mut available = ALL;
            available[blocked] = false;
            let d = dial(&pages, &room, seen, available);
            let mut batch = HudBatch::scratch(size);
            let clock = Clock::new(Instant::now(), true);
            draw_overlay(&mut batch, &atlas, &NoItems, &d, &shown(seen), clock);
            let tints = |slot| colours_in(&batch, look::cell_rect(&lay, slot));
            assert!(
                !tints(live).contains(&look::FADED_FRAME),
                "{size:?}: a live slot faded"
            );
            assert!(
                tints(blocked).contains(&look::FADED_FRAME),
                "{size:?}: an unavailable slot not faded"
            );
            assert!(
                tints(empty).contains(&look::FADED_FRAME),
                "{size:?}: an empty slot not faded"
            );
        }
    });
}
