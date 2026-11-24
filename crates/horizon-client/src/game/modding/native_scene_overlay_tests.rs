//! A native mod's scene overlays, read back from the real HUD renderer.
use super::{Screen, native_entity_refs::tests::with_client_game};
use horizon_client::{gfx::entity_pixel_test::render_hud, hud::HudBatch};
use mod_api::{
    ClientHost, Hooks, ModEntry, Scene, SceneBlock, SceneDepth, SceneOverlay, SceneView,
    ScreenRenderForegroundEvent,
};
use mod_platform::testing::with_mods;

const SIZE: [u32; 2] = [448, 120];
const BLACK: [u8; 4] = [0, 0, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const CYAN: [u8; 4] = [0, 255, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

struct PaintScene;

#[mod_api::__subscriptions]
impl Hooks for PaintScene {
    fn screen_render_foreground(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenRenderForegroundEvent<'_>,
    ) {
        use SceneDepth::{Tested, ThroughWalls};

        let stone = host.block_key("minecraft:stone").unwrap();
        let state = host.block_state_default(stone).unwrap();
        let scene = Scene::new(vec![SceneBlock::new([0, 0, 0], state, None)]);
        let rect = |x| [x, 12.0, x + 96.0, 108.0];
        let view = |scale| SceneView::new([0.5; 3], 0.0, 0.0, scale);
        let quad = |[x0, y0]: [f32; 2], [x1, y1]: [f32; 2], z, argb, depth| {
            SceneOverlay::quad(
                [[x0, y1, z], [x1, y1, z], [x1, y0, z], [x0, y0, z]],
                argb,
                depth,
            )
        };
        let rear_line = |depth| {
            SceneOverlay::line([-0.5, 0.75, 2.0], [1.5, 0.75, 2.0], 6.0, 0xFFFF_0000, depth)
        };
        let rear_quad = |depth| quad([0.2, 0.3], [0.8, 0.5], 2.0, 0xFF00_FF00, depth);

        // Unturned, the opaque face of the stone is at z = 0, with negative z in front.
        // First paint the unaltered scene as a texture reference, then add the same rear
        // geometry to the next three panels with different depth modes and camera scales.
        event
            .draw
            .draw_scene(rect(8.0), &scene, view(64.0))
            .unwrap();
        event
            .draw
            .draw_scene_with_overlays(
                rect(120.0),
                &scene,
                view(64.0),
                &[
                    rear_line(Tested),
                    quad([0.55, 0.1], [0.9, 0.25], -2.0, 0xFFFF_00FF, Tested),
                    quad([0.2, 0.1], [0.6, 0.25], -3.0, 0xFF00_FFFF, ThroughWalls),
                    quad([0.35, 0.1], [0.75, 0.25], -0.5, 0xFFFF_FF00, Tested),
                    // Must still test against the stone after a mode switch.
                    rear_quad(Tested),
                ],
            )
            .unwrap();
        let overlays = [rear_line(ThroughWalls), rear_quad(ThroughWalls)];
        for (x, scale) in [(232.0, 64.0), (344.0, 96.0)] {
            event
                .draw
                .draw_scene_with_overlays(rect(x), &scene, view(scale), &overlays)
                .unwrap();
        }
    }
}

#[test]
#[ignore = "daily"]
fn native_scene_overlays_test_against_blocks_without_writing_depth() {
    let mods = vec![ModEntry::new("scene-overlay-pixels", Box::new(PaintScene))];
    with_mods(mods.into(), || {
        with_client_game(|game| {
            let mut batch = HudBatch::scratch(SIZE);
            let screen = game.screen_view_at(
                Screen::Inventory,
                [0.0, 0.0, SIZE[0] as f32, SIZE[1] as f32],
            );
            game.native_screen_render_foreground(&mut batch, &screen);
            let pixels =
                render_hud(&batch, &game.gui.atlas, SIZE).expect("software Vulkan HUD draw");
            let pixel = |x: u32, y: u32| -> [u8; 4] {
                pixels[((y * SIZE[0] + x) * 4) as usize..][..4]
                    .try_into()
                    .unwrap()
            };

            for (y, color) in [(44, RED), (65, GREEN)] {
                let stone = pixel(56, y);
                assert_ne!(stone, BLACK, "the reference block is visible at y={y}");
                assert_ne!(stone, color, "the overlay differs from stone at y={y}");
                assert_eq!(pixel(168, y), stone, "tested overlay behind stone at y={y}");
                assert_eq!(pixel(280, y), color, "same overlay through stone at y={y}");
            }

            assert_eq!(pixel(145, 80), MAGENTA, "front tested overlay is visible");
            assert_eq!(
                pixel(185, 80),
                CYAN,
                "front through-walls overlay is visible"
            );
            assert_eq!(
                pixel(153, 80),
                YELLOW,
                "earlier front tested overlay wrote no depth"
            );
            assert_eq!(
                pixel(169, 80),
                YELLOW,
                "earlier through-walls overlay wrote no depth"
            );

            assert_eq!(pixel(130, 44), RED, "tested line shows beside the block");
            assert_eq!(pixel(218, 44), BLACK, "tested line clipped at scene edge");
            assert_eq!(pixel(325, 44), RED, "through-walls line reaches scene edge");
            assert_eq!(
                pixel(330, 44),
                BLACK,
                "through-walls line clipped at scene edge"
            );

            // At unit GUI scale, both projected lines cover exactly six framebuffer rows.
            for (x, top) in [(280, 41), (392, 33)] {
                for y in top - 1..=top + 6 {
                    assert_eq!(
                        pixel(x, y) == RED,
                        (top..top + 6).contains(&y),
                        "line pixel at ({x}, {y})"
                    );
                }
            }
        });
    });
}
