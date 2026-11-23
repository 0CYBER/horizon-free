//! Resource-backed Game screen hooks rendered by the real software-Vulkan HUD pipeline.
use super::{Game, Screen};
use horizon_client::{
    chunk::Chunk,
    containers::BlockEntity,
    gfx::entity_pixel_test::render_hud,
    hud::{HudBatch, rgba},
    items::ItemStack,
};
use mod_api::{
    ClientHost, Hooks, ModEntry, ScreenRenderBackgroundEvent, ScreenRenderForegroundEvent,
};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

const SIZE: [u32; 2] = [320, 240];
const POS: (i32, i32, i32) = (0, 0, 0);
const GREEN: [u8; 4] = [0, 255, 0, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];

struct PaintPhases(Arc<AtomicU8>);

#[mod_api::__subscriptions]
impl Hooks for PaintPhases {
    fn screen_render_background(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenRenderBackgroundEvent<'_>,
    ) {
        if self.0.load(Ordering::Relaxed) == 1 {
            let [x, y, _, _] = event.screen.rect;
            // ContainerScreen's first slot: its panel edge surrounds the item.
            event.draw.fill([x + 7.0, y + 17.0, 18.0, 18.0], 0xFF00FF00);
        }
    }

    fn screen_render_foreground(
        &self,
        _: &dyn ClientHost,
        event: &mut ScreenRenderForegroundEvent<'_>,
    ) {
        if self.0.load(Ordering::Relaxed) == 2 {
            event.draw.fill([0.0, 0.0, 320.0, 240.0], 0xFFFF00FF);
        }
    }
}

#[test]
#[ignore = "daily"]
fn title_post_hook_draws_a_packaged_wide_gui_sprite_before_world_open() {
    use crate::game::{UiResources, client_reload::fixture_png};
    use horizon_client::packs::{Pack, Source, resources};
    use mod_api::ScreenRenderPostEvent;
    use std::io::Write;

    const SPRITE: &str = "title_fixture:gui/title/panorama";
    const TINT: u32 = 0xFF1379D3;
    struct PaintTitle(Arc<AtomicU8>);

    #[mod_api::__subscriptions]
    impl Hooks for PaintTitle {
        fn screen_render_post(&self, _: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
            if event.screen.id == "title" {
                assert!(event.draw.sprite(
                    SPRITE,
                    [0.0, 0.0, 1230.0, 216.0],
                    [7.0, 11.0, 41.0, 23.0],
                    Some(TINT),
                ));
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("title_fixture.hmod");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    zip.start_file(
        "assets/title_fixture/textures/gui/title/panorama.png",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&fixture_png(1230, 216, 77)).unwrap();
    zip.finish().unwrap();
    let pack = Pack::mod_layer("title_fixture".into(), Source::Zip(archive));
    let calls = Arc::new(AtomicU8::new(0));

    with_mods(
        vec![ModEntry::new(
            "title_fixture",
            Box::new(PaintTitle(calls.clone())),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let source = resources(game.pack.source.clone(), &[pack]).unwrap();
                game.gui = UiResources::load(&source).unwrap();
                let rect = game.gui.atlas.sprites[SPRITE];
                assert_eq!((rect.w, rect.h), (1230, 216));

                let mut title = game.new_title(game.save.root.clone());
                title.mode = horizon_client::title::Mode::Main;
                game.title = Some(title);
                game.screen = Screen::Playing;
                let batch = game.hud_layers(SIZE);
                assert_eq!(calls.load(Ordering::Relaxed), 1);

                let tinted = batch
                    .vertices
                    .iter()
                    .filter(|v| v.color == rgba(19, 121, 211, 255));
                let quad: Vec<_> = tinted.collect();
                assert_eq!(quad.len(), 6);
                for (vertex, (pos, uv)) in quad.into_iter().zip([
                    ([7.0, 11.0], [rect.u0, rect.v0]),
                    ([48.0, 11.0], [rect.u1, rect.v0]),
                    ([48.0, 34.0], [rect.u1, rect.v1]),
                    ([7.0, 11.0], [rect.u0, rect.v0]),
                    ([48.0, 34.0], [rect.u1, rect.v1]),
                    ([7.0, 34.0], [rect.u0, rect.v1]),
                ]) {
                    assert_eq!(vertex.pos, pos);
                    for (actual, expected) in vertex.uv.into_iter().zip(uv) {
                        assert!((actual - expected).abs() <= f32::EPSILON);
                    }
                }
            });
        },
    );
}

fn chest(game: &mut Game<'_>) -> (u32, u32) {
    game.title = None;
    game.screen = Screen::Inventory;
    game.ui.hide_gui = true;
    game.container = Some(POS);
    game.craft_size = 2;
    game.level.recipe_book.open = [false; 4];
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:chest")
        .unwrap();
    game.world.set(POS.0, POS.1, POS.2, state).unwrap();
    let mut items = vec![None; 27];
    items[0] = Some(ItemStack::new("minecraft:stone", 1));
    game.world
        .block_entities
        .insert(POS, BlockEntity::Chest(items));
    let batch = HudBatch::scratch(SIZE);
    let (x, y) = game.inventory_origin(&batch);
    let (sx, sy) = game.slot_position(300);
    assert_eq!((sx, sy), (8.0, 18.0));
    game.cursor = (x + sx + 8.0, y + sy + 8.0);
    ((x + sx) as u32, (y + sy) as u32)
}

fn frame(game: &mut Game<'_>) -> Vec<u8> {
    // Exercise the actual Game dispatch, including its Pre/Post orchestration,
    // rather than manufacturing an equivalent sequence of render operations.
    let batch = game.hud_layers(SIZE);
    render_hud(&batch, &game.gui.atlas, SIZE).expect("software Vulkan screen draw")
}

fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    pixels[((y * SIZE[0] + x) * 4) as usize..][..4]
        .try_into()
        .unwrap()
}

fn overlay_reference(game: &mut Game<'_>, carried: bool) -> Vec<u8> {
    let mut batch = HudBatch::scratch(SIZE);
    batch.fill(
        &game.gui.atlas,
        0.0,
        0.0,
        320.0,
        240.0,
        rgba(255, 0, 255, 255),
    );
    if carried {
        game.draw_carried(&mut batch);
    } else {
        game.draw_slot_tooltip(&mut batch);
    }
    render_hud(&batch, &game.gui.atlas, SIZE).expect("software Vulkan overlay reference")
}

#[test]
#[ignore = "daily"]
fn chest_render_phases_cover_panel_but_preserve_items_tooltips_and_carried_stack() {
    let mut no_subscriber = None;
    with_mods(Vec::new().into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            chest(game);
            no_subscriber = Some(frame(game));
        });
    });
    let no_subscriber = no_subscriber.unwrap();
    let mode = Arc::new(AtomicU8::new(0));
    with_mods(
        vec![ModEntry::new(
            "phase-pixels",
            Box::new(PaintPhases(mode.clone())),
        )]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                let (x, y) = chest(game);
                let baseline = frame(game);
                assert_eq!(
                    baseline, no_subscriber,
                    "an idle listener leaves the default picture unchanged"
                );
                mode.store(1, Ordering::Relaxed);
                let background = frame(game);
                assert_ne!(pixel(&baseline, x - 1, y - 1), GREEN);
                assert_eq!(
                    pixel(&background, x - 1, y - 1),
                    GREEN,
                    "Background covers the panel's slot edge"
                );
                assert_ne!(pixel(&baseline, x + 8, y + 8), GREEN);
                assert_eq!(
                    pixel(&background, x + 8, y + 8),
                    pixel(&baseline, x + 8, y + 8),
                    "the actual slot item covers Background"
                );
                mode.store(2, Ordering::Relaxed);
                let foreground = frame(game);
                let tooltip = overlay_reference(game, false);
                assert!(
                    tooltip.as_chunks::<4>().0.iter().any(|p| *p != MAGENTA),
                    "the real tooltip rendered pixels"
                );
                assert_eq!(foreground, tooltip, "the real tooltip covers Foreground");
                game.carried = Some(ItemStack::new("minecraft:stone", 1));
                let foreground = frame(game);
                let carried = overlay_reference(game, true);
                assert_ne!(
                    pixel(&carried, x + 8, y + 8),
                    MAGENTA,
                    "the real carried item rendered pixels"
                );
                assert_eq!(
                    foreground, carried,
                    "the actual dragged stack covers Foreground"
                );
            });
        },
    );
}
