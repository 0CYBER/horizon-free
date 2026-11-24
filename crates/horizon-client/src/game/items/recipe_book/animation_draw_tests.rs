//! Inventory animation hooks on real Game frames and the software Vulkan HUD painter.
use super::{BookType, Game, HudBatch, ItemStack, Screen};
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::{command::args::GameMode, gfx::entity_pixel_test::render_hud};
use mod_api::{
    ClientHost, Flow, GuiLayerPostEvent, GuiLayerPreEvent, Hooks, ItemDecorationsEvent, ItemPart,
    ModEntry, RecipeBookRenderEvent, Registrar, RegistrationContext, ScreenRenderPreEvent,
};
use mod_platform::testing::with_mods;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SIZE: [u32; 2] = [321, 240];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const CYAN: [u8; 4] = [0, 255, 255, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const HOTBAR: &str = "minecraft:hotbar";

fn render(game: &Game<'_>, batch: &HudBatch, size: [u32; 2]) -> Vec<u8> {
    render_hud(batch, &game.gui.atlas, size).expect("software Vulkan inventory draw")
}

fn frame(game: &mut Game<'_>, size: [u32; 2]) -> Vec<u8> {
    let batch = game.hud_layers(size);
    render(game, &batch, size)
}

fn pixel(pixels: &[u8], size: [u32; 2], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * size[0] + x) * 4) as usize;
    pixels[at..at + 4].try_into().unwrap()
}

#[derive(Default)]
struct BookProbe {
    paint: AtomicBool,
    cancel: AtomicBool,
    seen: Mutex<Vec<([f32; 4], bool)>>,
}

struct BookPainter(Arc<BookProbe>);

#[mod_api::__subscriptions]
impl Hooks for BookPainter {
    fn screen_render_pre(&self, _: &dyn ClientHost, _: &mut ScreenRenderPreEvent<'_>) -> Flow {
        if self.0.cancel.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn recipe_book_render(&self, _: &dyn ClientHost, event: &mut RecipeBookRenderEvent<'_>) {
        self.0
            .seen
            .lock()
            .unwrap()
            .push((event.rect, event.covers_menu));
        assert_eq!(event.draw.hotbar_origin(), None);
        if self.0.paint.load(Ordering::Relaxed) {
            let [w, h] = event.draw.size();
            let [x, y, _, _] = event.rect;
            event.draw.fill([0.0, 0.0, w, h], 0xFFFF00FF);
            event.draw.fill([x + 8.0, y + 8.0, 4.0, 4.0], 0xFF00FF00);
        }
    }
}

#[test]
#[ignore = "daily"]
fn inventory_animation_draw_book_and_controls() {
    let probe = Arc::new(BookProbe::default());
    let mods = vec![ModEntry::new(
        "recipe-panel",
        Box::new(BookPainter(probe.clone())),
    )]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            game.title = None;
            game.screen = Screen::Inventory;
            game.container = None;
            game.craft_size = 2;
            game.settings.gui_scale = 1;
            game.level.recipe_book.open = [false; 4];
            game.level.recipe_book.filtering = [false; 4];
            game.ui.hide_gui = true;
            game.inventory.fill(None);
            game.recipe_ui.test_viewport = Some(SIZE);
            let batch = HudBatch::scratch(SIZE);
            let book = BookType::Crafting;
            let seen = || std::mem::take(&mut *probe.seen.lock().unwrap());
            game.hud_layers(SIZE);
            assert!(seen().is_empty(), "a closed book does not fire");

            // The book button still opens it, and page arrows and the filter still own input.
            let (x, _) = game.inventory_origin(&batch);
            let (bx, by) = game.book_button_pos(book, x, &batch);
            game.cursor = (bx + 2.0, by + 2.0);
            assert!(game.recipe_book_click(false));
            let [x, y, w, h] = game.recipe_book_rect(&batch).unwrap();
            game.level.recipe_book.recipes.extend(
                game.recipe_ui
                    .book
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|entry| entry.id.clone()),
            );
            game.cursor = (2.0, 2.0);
            let vanilla = frame(game, SIZE);
            assert_eq!(seen(), [([x, y, w, h], true)]);
            assert!(
                game.recipe_ui.view.buttons.len() > 20,
                "prepared recipes occupy several pages"
            );
            game.cursor = (x + 94.0, y + 138.0);
            assert!(game.recipe_book_click(false));
            assert_eq!(game.recipe_ui.page, 1);

            probe.paint.store(true, Ordering::Relaxed);
            game.cursor = (2.0, 2.0);
            let painted = frame(game, SIZE);
            assert_eq!(seen(), [([x, y, w, h], true)]);
            let (mx, my) = (x as u32 + 9, y as u32 + 9);
            assert_ne!(pixel(&vanilla, SIZE, mx, my), GREEN);
            assert_eq!(pixel(&painted, SIZE, mx, my), GREEN);
            assert_eq!(pixel(&painted, SIZE, mx + 6, my), MAGENTA);

            game.cursor = (x + 111.0, y + 13.0);
            let hovered = frame(game, SIZE);
            assert_eq!(seen(), [([x, y, w, h], true)]);
            assert_ne!(
                hovered, painted,
                "the deferred filter tooltip draws above recipe_book_render"
            );
            assert!(game.recipe_book_click(false));
            assert!(game.book_filtering(book));
            assert_eq!(game.recipe_ui.page, 0);

            let wide = [500, SIZE[1]];
            game.cursor = (2.0, 2.0);
            let wide_rect = game.recipe_book_rect(&HudBatch::scratch(wide)).unwrap();
            game.hud_layers(wide);
            assert_eq!(seen(), [(wide_rect, false)]);

            probe.cancel.store(true, Ordering::Relaxed);
            game.hud_layers(SIZE);
            assert!(
                seen().is_empty(),
                "Screen Render.Pre suppresses the book stage too"
            );
            assert!(game.book_open(book), "the underlying open flag stays set");
        });
    });
}

#[derive(Default)]
struct HotbarProbe {
    paint: AtomicBool,
    cancel: AtomicBool,
    items: Mutex<Vec<(Option<[f32; 2]>, f32)>>,
    posts: Mutex<Vec<Option<[f32; 2]>>>,
}

#[derive(Clone)]
struct Decorator(Arc<HotbarProbe>);

#[mod_api::horizon_part]
impl ItemPart for Decorator {
    fn item_decorations(&self, _: &dyn ClientHost, event: &mut ItemDecorationsEvent<'_>) {
        let draw = &mut *event.draw;
        let origin = draw.hotbar_origin();
        self.0
            .items
            .lock()
            .unwrap()
            .push((origin, draw.game_time_delta_ticks()));
        if origin.is_none() || !self.0.paint.load(Ordering::Relaxed) {
            return;
        }
        assert_eq!(draw.translation(), [0.0; 2]);
        assert!(!draw.pop_translation());
        assert!(draw.push_translation());
        assert!(draw.translate([40.0, 50.0]));
        draw.push_clip([8.0, 20.0, 14.0, 16.0]).unwrap();
        assert!(draw.push_translation());
        assert!(draw.translate([2.0, 0.0]));
        assert!(draw.item(event.stack.item(), [8.0, 20.0], 16.0));
        assert!(draw.pop_translation());
        assert_eq!(draw.translation(), [40.0, 50.0]);
        draw.gui_draw_item_decorations_with(event.stack, [8.0, 20.0], Some("7"));
        draw.pop_clip().unwrap();
        draw.gui_draw_item_decorations_with(event.stack, [32.0, 20.0], Some("7"));
        // Leave a shifted clip and a pushed translation open: neither affects later hook calls.
        draw.push_clip([0.0, 0.0, 1.0, 1.0]).unwrap();
    }
}

struct HudPainter(Arc<HotbarProbe>);

#[mod_api::__subscriptions]
impl Hooks for HudPainter {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:diamond");
        r.item_part(key, Decorator(self.0.clone()));
    }

    fn gui_layer_pre(&self, _: &dyn ClientHost, event: &mut GuiLayerPreEvent<'_>) -> Flow {
        assert_eq!(event.draw.hotbar_origin(), None);
        if event.layer == HOTBAR && self.0.cancel.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn gui_layer_post(&self, _: &dyn ClientHost, event: &mut GuiLayerPostEvent<'_>) {
        let origin = event.draw.hotbar_origin();
        if event.layer != HOTBAR {
            assert_eq!(origin, None);
            return;
        }
        self.0.posts.lock().unwrap().push(origin);
        assert_eq!(event.draw.translation(), [0.0; 2]);
        if let Some([x, y]) = origin
            && self.0.paint.load(Ordering::Relaxed)
        {
            event.draw.fill([x + 80.0, y + 8.0, 2.0, 2.0], 0xFF00FFFF);
            event.draw.fill([12.0, 12.0, 2.0, 2.0], 0xFF00FF00);
        }
    }
}

#[test]
#[ignore = "daily"]
fn inventory_animation_draw_hotbar_and_translated_items() {
    crate::game::in_own_process(
        "game::recipe_book::animation_draw_tests::inventory_animation_draw_hotbar_and_translated_items",
        || {
            let probe = Arc::new(HotbarProbe::default());
            let mods: mod_api::Mods = vec![ModEntry::new(
                "hotbar-paint",
                Box::new(HudPainter(probe.clone())),
            )]
            .into();
            with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                with_client_game(|game| {
                    game.title = None;
                    game.screen = Screen::Playing;
                    game.mode = GameMode::Survival;
                    game.settings.gui_scale = 1;
                    game.ui.hide_gui = false;
                    game.inventory.fill(None);
                    let stack = ItemStack::new("minecraft:diamond", 5);
                    game.inventory[0] = Some(stack.clone());
                    game.selected = 0;
                    game.clock.reset();
                    assert_eq!(game.clock.advance(Duration::from_millis(125), 100.0), 1);
                    game.frame_ticks = 2.5;
                    let items = || std::mem::take(&mut *probe.items.lock().unwrap());
                    let posts = || std::mem::take(&mut *probe.posts.lock().unwrap());
                    let origin = Some([69.0, 218.0]);
                    let normal = frame(game, SIZE);
                    assert_eq!(items(), [(origin, 1.25)]);
                    assert_eq!(posts(), [origin]);

                    probe.paint.store(true, Ordering::Relaxed);
                    let painted = frame(game, SIZE);
                    assert_eq!(
                        items(),
                        [(origin, 1.25)],
                        "decoration-only does not re-enter the hook"
                    );
                    assert_eq!(posts(), [origin]);
                    assert_ne!(
                        pixel(&normal, SIZE, 149, 226),
                        BLACK,
                        "marker is on the stock panel"
                    );
                    assert_eq!(pixel(&painted, SIZE, 149, 226), CYAN);
                    assert_eq!(pixel(&painted, SIZE, 12, 12), GREEN);
                    assert_eq!(
                        pixel(&painted, SIZE, 80, 229),
                        pixel(&normal, SIZE, 80, 229)
                    );

                    let mut reference = HudBatch::scratch(SIZE);
                    game.draw_item_model(&mut reference, &stack, 50.0, 70.0);
                    game.paint_item_decorations(
                        &mut reference,
                        &stack,
                        48.0,
                        70.0,
                        Some("7".into()),
                    );
                    reference.clip_since(0, 48.0, 70.0, 62.0, 86.0);
                    game.paint_item_decorations(
                        &mut reference,
                        &stack,
                        72.0,
                        70.0,
                        Some("7".into()),
                    );
                    let reference = render(game, &reference, SIZE);
                    let mut count_pixels = 0;
                    for y in 70..86 {
                        for x in 48..89 {
                            let expected = pixel(&reference, SIZE, x, y);
                            assert_eq!(pixel(&painted, SIZE, x, y), expected, "pixel {x}, {y}");
                            if x >= 72 && expected != BLACK {
                                assert!(y >= 79, "the decoration-only pass never draws the icon");
                                count_pixels += 1;
                            }
                        }
                    }
                    assert!(count_pixels > 0, "the count text reached the actual batch");
                    assert_eq!(
                        pixel(&painted, SIZE, 30, 28),
                        BLACK,
                        "no draw at the untranslated slot"
                    );

                    probe.cancel.store(true, Ordering::Relaxed);
                    game.hud_layers(SIZE);
                    assert_eq!(posts(), [None]);
                    assert!(items().is_empty());
                    probe.cancel.store(false, Ordering::Relaxed);

                    game.mode = GameMode::Spectator;
                    game.hud_layers(SIZE);
                    assert_eq!(posts(), [None]);
                    assert!(items().is_empty());

                    game.mode = GameMode::Survival;
                    game.ui.hide_gui = true;
                    game.screen = Screen::Inventory;
                    game.container = None;
                    game.level.recipe_book.open = [false; 4];
                    game.hud_layers(SIZE);
                    assert!(posts().is_empty());
                    assert_eq!(
                        items(),
                        [(None, 1.25)],
                        "container slots do not inherit the HUD origin"
                    );
                });
            });
        },
    );
}
