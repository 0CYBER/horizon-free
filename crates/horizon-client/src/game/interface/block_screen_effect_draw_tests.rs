//! `block_screen_effect_draw` in a real game: what a frame's screen quads hold for each thing a
//! mod can do with an overlay, and where the quads land in a drawn frame.
use super::Game;
use super::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use BlockScreenEffect::{Block, Fire, Water};
use horizon_client::{
    camera::hand_projection,
    chunk::Chunk,
    gfx::ScreenshotRequest,
    motion::PlayerMotion,
    screen_effects::{Blend, ScreenQuad},
};
use horizon_core::{geometry::Vec3, position::BlockPos};
use mod_api::{
    BlockScreenEffect, BlockScreenEffectDrawEvent, ClientHost, ExtractBlockScreenEffectEvent, Flow,
    GuiRenderPostEvent, Hooks, ModEntry, Mods,
};
use std::sync::{Arc, Mutex};

const RED: u32 = 0xFFFF0000;
/// Half-transparent blue: it shows what is under it.
const CLEAR_BLUE: u32 = 0x800000FF;
/// A quad a quarter of a block from the view's centre, at vanilla's depth.
const QUAD: [[f32; 3]; 4] = [
    [-0.25, -0.25, -0.5],
    [0.25, -0.25, -0.5],
    [0.25, 0.25, -0.5],
    [-0.25, 0.25, -0.5],
];

/// What the mod does with each overlay.
#[derive(Clone, Copy)]
struct Plan {
    custom: &'static [BlockScreenEffect],
    cancel: &'static [BlockScreenEffect],
    replace: &'static [BlockScreenEffect],
}
const LEAVE: Plan = Plan {
    custom: &[],
    cancel: &[],
    replace: &[],
};
/// One ask of the draw hook.
#[derive(Debug, PartialEq)]
struct Asked {
    effect: BlockScreenEffect,
    state: mod_api::BlockState,
    pos: BlockPos,
    partial_tick: f32,
    hide_gui: bool,
}
static PLAN: Mutex<Plan> = Mutex::new(LEAVE);
static ASKED: Mutex<Vec<Asked>> = Mutex::new(Vec::new());
/// The partial tick each extract event held.
static EXTRACTED: Mutex<Vec<(BlockScreenEffect, f32)>> = Mutex::new(Vec::new());

/// The mod's colour for what it draws over `effect`.
fn marker(effect: BlockScreenEffect) -> u32 {
    match effect {
        Block => 0xFF010000,
        Water => 0xFF020000,
        Fire => 0xFF030000,
    }
}
struct Planned;
#[mod_api::__subscriptions]
impl Hooks for Planned {
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent,
    ) -> Flow {
        let plan = *PLAN.lock().unwrap();
        EXTRACTED
            .lock()
            .unwrap()
            .push((event.effect, event.partial_tick));
        event.custom_draw = plan.custom.contains(&event.effect);
        if plan.cancel.contains(&event.effect) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn block_screen_effect_draw(
        &self,
        _: &dyn ClientHost,
        event: &mut BlockScreenEffectDrawEvent<'_>,
    ) {
        ASKED.lock().unwrap().push(Asked {
            effect: event.effect,
            state: event.state,
            pos: event.pos,
            partial_tick: event.partial_tick,
            hide_gui: event.hide_gui,
        });
        assert!(event.draw.quad(QUAD, marker(event.effect)));
        event.replace_vanilla = PLAN.lock().unwrap().replace.contains(&event.effect);
    }
}

/// A player on fire in first person, with their eye in block (0, 65, 0).
fn burning_player(game: &mut Game<'_>) {
    game.world.chunks.insert(
        (0, 0),
        Arc::new(Chunk::with_layout(0, 0, game.world.layout())),
    );
    let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
    player.position = Vec3::new(0.5, 64.0, 0.5);
    player.fire_ticks = 100;
    game.player = Some(player);
}
/// The frame's quads at 2:1 with the hand's usual 70 degrees.
fn quads(game: &Game<'_>) -> Vec<ScreenQuad> {
    let projection = hand_projection(70.0, 1000.0, 500.0, 1000.0);
    game.screen_quads(0.25, [1000, 500], 1.0, projection)
}
/// Names the block overlays' quads in draw order, the HUD's camera overlays left out.
fn names(quads: &[ScreenQuad]) -> Vec<&'static str> {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    quads
        .iter()
        .filter(|quad| quad.blend == Blend::Alpha)
        .filter_map(|quad| {
            let [r, g, b, a] = quad.vertices[0].color;
            Some(match (r, g, b, a) {
                _ if close(r, 1.0 / 255.0) && g == 0.0 => "mod block",
                _ if close(r, 2.0 / 255.0) && g == 0.0 => "mod water",
                _ if close(r, 3.0 / 255.0) && g == 0.0 => "mod fire",
                _ if close(r, 25.0 / 255.0) && close(a, 1.0) => "block",
                _ if close(a, 0.1) => "water",
                _ if close(a, 229.0 / 255.0) => "fire",
                _ => return None,
            })
        })
        .collect()
}
fn asked() -> Vec<BlockScreenEffect> {
    let mut asked = ASKED.lock().unwrap();
    asked.drain(..).map(|ask| ask.effect).collect()
}
fn close_to(actual: [f32; 4], expected: [f32; 4]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!((a - e).abs() < 1e-4, "{actual:?} is not {expected:?}");
    }
}

/// Each thing a mod can do with an overlay, in a frame with stone at the eye and fire on the
/// player, then with water at the eye. The expected names are `ScreenEffectRenderer.submit`'s
/// order, each mod's quads right before the overlay they belong to.
#[test]
#[ignore = "daily"]
fn a_mod_draws_prepends_replaces_and_cancels_overlays() {
    with_client_game(|game| {
        burning_player(game);
        let registry = &game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        let (stone, water, fire) = (
            state("minecraft:stone"),
            state("minecraft:water"),
            state("minecraft:fire"),
        );
        game.world.set(0, 65, 0, stone).unwrap();
        // No listener: vanilla's overlays, at vanilla's place. `submitBlockSprite`'s (-1, -1) at
        // depth 0.5 under 70 degrees at 2:1 is -1 / (0.5 * tan 35 * 2) and -1 / (0.5 * tan 35).
        let vanilla = quads(game);
        assert_eq!(names(&vanilla), ["block", "fire", "fire"]);
        close_to(vanilla[0].vertices[0].pos, [-1.42815, -2.85630, 0.0, 1.0]);
        close_to(vanilla[0].vertices[2].pos, [1.42815, 2.85630, 0.0, 1.0]);
        let bytes = |quads: &[ScreenQuad]| {
            let vertices = quads.iter().flat_map(|quad| quad.vertices);
            let vertices: Vec<_> = vertices.collect();
            bytemuck::cast_slice::<_, u8>(&vertices).to_vec()
        };
        let mods: Mods = vec![ModEntry::new("screen-effect", Box::new(Planned))].into();
        mod_platform::testing::with_mods(mods, || {
            let frame = |game: &Game<'_>, plan: Plan| {
                *PLAN.lock().unwrap() = plan;
                let quads = quads(game);
                (quads, asked())
            };
            // A listener that claims nothing leaves every vertex and is never asked to draw.
            let (left, draws) = frame(game, LEAVE);
            assert_eq!(bytes(&left), bytes(&vanilla));
            assert_eq!(draws, []);
            // `quads` draws at partial tick 0.25: `getWorldPartialTick` of each shown overlay.
            assert_eq!(
                std::mem::take(&mut *EXTRACTED.lock().unwrap()),
                [(Block, 0.25), (Fire, 0.25)]
            );

            // Prepending: the mod's quad, then vanilla's.
            let plan = Plan {
                custom: &[Block],
                ..LEAVE
            };
            *PLAN.lock().unwrap() = plan;
            game.ui.hide_gui = true;
            let prepended = quads(game);
            game.ui.hide_gui = false;
            assert_eq!(names(&prepended), ["mod block", "block", "fire", "fire"]);
            assert_eq!(
                ASKED.lock().unwrap().drain(..).collect::<Vec<_>>(),
                [Asked {
                    effect: Block,
                    state: crate::game::api_values::TryIntoApi::try_into_api(&stone).unwrap(),
                    pos: BlockPos::new(0, 65, 0),
                    partial_tick: 0.25,
                    hide_gui: true,
                }]
            );
            // Camera space under the hand's projection: x * 1 / (tan 35 * 2), y / tan 35, and
            // w is the depth. Reversed depth from 0.05 puts z at about 0.05 for any depth.
            let corner = prepended[0].vertices[2].pos;
            close_to(corner, [0.17852, 0.35704, 0.04998, 0.5]);

            // Replacing: only the mod's quad, and the other overlays stay.
            let (replaced, draws) = frame(
                game,
                Plan {
                    custom: &[Block],
                    replace: &[Block],
                    ..LEAVE
                },
            );
            assert_eq!(names(&replaced), ["mod block", "fire", "fire"]);
            assert_eq!(draws, [Block]);

            // Canceled with custom_draw: the hook is still asked, vanilla's overlay is gone.
            let (canceled, draws) = frame(
                game,
                Plan {
                    custom: &[Block],
                    cancel: &[Block],
                    ..LEAVE
                },
            );
            assert_eq!(names(&canceled), ["mod block", "fire", "fire"]);
            assert_eq!(draws, [Block]);

            // Canceled alone is NO_OP: nothing drawn, nothing asked.
            let (no_op, draws) = frame(
                game,
                Plan {
                    cancel: &[Block],
                    ..LEAVE
                },
            );
            assert_eq!(names(&no_op), ["fire", "fire"]);
            assert_eq!(draws, []);

            // Fire's own state and place, replaced while the block's overlay stays.
            *PLAN.lock().unwrap() = Plan {
                custom: &[Fire],
                replace: &[Fire],
                ..LEAVE
            };
            assert_eq!(names(&quads(game)), ["block", "mod fire"]);
            assert_eq!(
                ASKED.lock().unwrap().drain(..).collect::<Vec<_>>(),
                [Asked {
                    effect: Fire,
                    state: crate::game::api_values::TryIntoApi::try_into_api(&fire).unwrap(),
                    pos: BlockPos::new(0, 64, 0),
                    partial_tick: 0.25,
                    hide_gui: false,
                }]
            );

            // Under water: every overlay claimed, in vanilla's order.
            game.world.set(0, 65, 0, water).unwrap();
            let (under, draws) = frame(
                game,
                Plan {
                    custom: &[Water, Fire],
                    replace: &[Water],
                    ..LEAVE
                },
            );
            assert_eq!(names(&under), ["mod water", "mod fire", "fire", "fire"]);
            assert_eq!(draws, [Water, Fire]);
        });
    });
}

/// A frame's pixels, RGBA.
fn drawn_frame(game: &mut Game<'_>) -> (Vec<u8>, usize) {
    game.renderer.as_mut().unwrap().screenshot = Some(ScreenshotRequest {
        path: "unused.png".into(),
        size: None,
        scaled: false,
        background: false,
    });
    game.render_with(|renderer, frame| renderer.render(frame))
        .unwrap();
    let result = game.renderer.as_mut().unwrap().screenshot_result.take();
    let mut pending = result.unwrap().unwrap();
    let image = pending.image_mut();
    (std::mem::take(&mut image.rgba), image.width as usize)
}

/// Whether the frame's mod draws: off for the frame to compare with.
static DRAWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct Painter;
#[mod_api::__subscriptions]
impl Hooks for Painter {
    fn extract_block_screen_effect(
        &self,
        _: &dyn ClientHost,
        event: &mut ExtractBlockScreenEffectEvent,
    ) -> Flow {
        event.custom_draw = event.effect == Fire;
        Flow::Continue
    }
    fn block_screen_effect_draw(
        &self,
        _: &dyn ClientHost,
        event: &mut BlockScreenEffectDrawEvent<'_>,
    ) {
        // Leaning away to the right: the right edge is twice as deep, so half as far out.
        let leaning = [
            [-0.25, -0.25, -0.5],
            [0.25, -0.25, -1.0],
            [0.25, 0.25, -1.0],
            [-0.25, 0.25, -0.5],
        ];
        assert!(event.draw.quad(leaning, RED));
        let clear = [
            [0.33, -0.1, -0.5],
            [0.44, -0.1, -0.5],
            [0.44, 0.1, -0.5],
            [0.33, 0.1, -0.5],
        ];
        assert!(event.draw.quad(clear, CLEAR_BLUE));
        event.replace_vanilla = true;
    }
    fn gui_render_post(&self, _: &dyn ClientHost, event: &mut GuiRenderPostEvent<'_>) {
        if DRAWING.load(std::sync::atomic::Ordering::Relaxed) {
            let [width, height] = event.batch.size();
            event
                .batch
                .fill([0.0, 0.0, width / 2.0, height], 0xFF00FF00);
        }
    }
}

/// On a real renderer: the quads go over the level, perspective included, and the GUI goes over
/// them. At 320x240 the hand's 70 degrees put a corner `(x, y, z)` at pixel
/// `160 + 160 * x * 1.07111 / -z`, `120 - 120 * y * 1.42815 / -z`, so the leaning quad's left edge
/// is at x 74 and its right edge at x 203, and the clear quad covers x 273 to 311.
#[test]
#[ignore = "daily"]
fn screen_effect_quads_draw_over_the_level_and_under_the_gui() {
    let name = "game::block_screen_effect_draw_tests::screen_effect_quads_draw_over_the_level_and_under_the_gui";
    if run_fresh_cases(name, "HORIZON_SCREEN_EFFECT_PASS_CHILD", &["pixels"]) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("painter", Box::new(Painter))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let mut builder = winit::event_loop::EventLoop::builder();
            #[cfg(target_os = "linux")]
            {
                use winit::platform::x11::EventLoopBuilderExtX11;
                builder.with_x11().with_any_thread(true);
            }
            let event_loop = builder.build().unwrap_or_else(|error| {
                eprintln!("screen-effect event loop error: {error}");
                panic!("screen-effect event loop creation failed")
            });
            #[allow(deprecated)]
            let window = Arc::new(
                event_loop
                    .create_window(
                        winit::window::Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
                    )
                    .unwrap(),
            );
            game.options.offscreen = true;
            game.renderer = Some(game.new_renderer(window).unwrap());
            game.title = None;
            burning_player(game);
            game.previous_position = game.player.as_ref().unwrap().position;
            // A black vignette is no quad: the HUD's camera overlays would darken the colours.
            game.screen_fx.vignette_brightness = 0.0;
            game.player.as_mut().unwrap().fire_ticks = 0;
            let (level, width) = drawn_frame(game);
            game.player.as_mut().unwrap().fire_ticks = 100;
            DRAWING.store(true, std::sync::atomic::Ordering::Relaxed);
            let (frame, _) = drawn_frame(game);
            let pixel = |pixels: &[u8], x: usize, y: usize| {
                let at = (y * width + x) * 4;
                [pixels[at], pixels[at + 1], pixels[at + 2]]
            };
            let green = [0, 255, 0];
            let red = [255, 0, 0];
            assert_eq!(pixel(&frame, 190, 120), red, "inside the leaning quad");
            // The top edge runs from (74, 34) to (203, 77), so it is at y 66 for x 170.
            assert_eq!(pixel(&frame, 170, 70), red, "under the sloping top edge");
            assert_eq!(
                pixel(&frame, 170, 60),
                pixel(&level, 170, 60),
                "over the sloping top edge: perspective, not a rectangle"
            );
            assert_eq!(
                pixel(&frame, 208, 120),
                pixel(&level, 208, 120),
                "past its right edge"
            );
            assert_eq!(pixel(&frame, 100, 120), green, "the GUI is over the quad");
            // The clear quad blends with the level under it.
            let under = pixel(&level, 290, 120);
            let over = pixel(&frame, 290, 120);
            assert_ne!(under, [0, 0, 0], "the level draws something here");
            let blue = [0.0, 0.0, 255.0];
            for i in 0..3 {
                let kept = f32::from(under[i]) * (127.0 / 255.0);
                let expected = kept + blue[i] * (128.0 / 255.0);
                let off = (f32::from(over[i]) - expected).abs();
                assert!(off <= 1.0, "{over:?} is not blue at half over {under:?}");
            }
        });
    });
}
