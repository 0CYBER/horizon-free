#![cfg(test)]
//! `DrawContext::fill_gradient` and `GuiDraw::FillGradient` through the game's generated
//! dispatch: both draw with the engine's `HudBatch::fill_gradient` in vanilla's colour order,
//! inside the hook call's clip. Zero area, one-axis inversion and non-finite input draw nothing;
//! both axes inverted swaps the visual top and bottom colours.
use horizon_client::hud::HudBatch;
use horizon_client::hud_layout::abgr;
use horizon_client::text::{Style, Text};
use mod_api::client::hud_layout::{GuiDraw, PoseOp};
use mod_api::{ComponentHoverEvent, Hooks, ModEntry};
use mod_platform::testing::with_mods;

const TOP: u32 = 0x8000_FF00;
const BOTTOM: u32 = 0xFF00_00FF;

/// Draws gradients that draw nothing, one that does, and a painted one under a translation.
struct Gradients;
#[mod_api::__subscriptions]
impl Hooks for Gradients {
    fn component_hover(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut ComponentHoverEvent<'_>,
    ) -> mod_api::Flow {
        let draw = &mut *event.draw;
        draw.fill_gradient([10.0, 10.0, 0.0, 5.0], TOP, BOTTOM);
        draw.fill_gradient([10.0, 10.0, 5.0, -5.0], TOP, BOTTOM);
        draw.fill_gradient([f32::NAN, 10.0, 5.0, 5.0], TOP, BOTTOM);
        draw.fill_gradient([10.0, 10.0, f32::INFINITY, 5.0], TOP, BOTTOM);
        draw.push_clip([0.0, 0.0, 100.0, 14.0]).unwrap();
        draw.fill_gradient([10.0, 10.0, 20.0, 8.0], TOP, BOTTOM);
        draw.pop_clip().unwrap();
        draw.fill_gradient([130.0, 28.0, -20.0, -8.0], 0x8011_2233, 0xFF44_5566);
        draw.gui_paint(&[
            GuiDraw::Push(vec![PoseOp::Translate(50.0, 60.0)]),
            GuiDraw::FillGradient {
                x0: 0,
                y0: 0,
                x1: 4,
                y1: 2,
                top: TOP,
                bottom: BOTTOM,
            },
            GuiDraw::FillGradient {
                x0: 4,
                y0: 0,
                x1: 0,
                y1: 2,
                top: TOP,
                bottom: BOTTOM,
            },
            GuiDraw::Pop,
        ])
        .unwrap();
        mod_api::Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_gradients_draw_top_to_bottom_in_the_clip_and_pose_and_empty_ones_draw_nothing() {
    let text = Text::from(serde_json::json!({
        "text": "x", "hover_event": { "action": "show_text", "value": "hi" }
    }));
    let style: Style = text.spans[0].style.clone();
    let mods: mod_api::Mods = vec![ModEntry::new("gradients", Box::new(Gradients))].into();
    let mut batch = HudBatch::scratch([320, 240]);
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.component_hover_effect(&mut batch, &style, (40.0, 30.0));
        });
    });
    let drawn: Vec<_> = batch.vertices.iter().map(|v| (v.pos, v.color)).collect();
    let (inverted, drawn): (Vec<_>, Vec<_>) =
        drawn.into_iter().partition(|(pos, _)| pos[0] >= 100.0);
    assert_eq!(
        inverted,
        [
            ([110.0, 20.0], 0xFF66_5544),
            ([130.0, 20.0], 0xFF66_5544),
            ([130.0, 28.0], 0x8033_2211),
            ([110.0, 20.0], 0xFF66_5544),
            ([130.0, 28.0], 0x8033_2211),
            ([110.0, 28.0], 0x8033_2211),
        ]
    );
    let (clipped, painted): (Vec<_>, Vec<_>) =
        drawn.iter().copied().partition(|(pos, _)| pos[0] < 40.0);
    assert!(!clipped.is_empty(), "{drawn:?}");
    assert_eq!(painted.len(), 6, "{drawn:?}");
    // The clipped call: cut at y 14, half way down, where the ramp is half way between.
    let mid = {
        let [t, b] = [abgr(TOP), abgr(BOTTOM)].map(u32::to_le_bytes);
        u32::from_le_bytes(std::array::from_fn(|i| {
            (u16::from(t[i]) + u16::from(b[i])).div_ceil(2) as u8
        }))
    };
    for (pos, color) in clipped {
        assert!(pos[0] == 10.0 || pos[0] == 30.0, "{pos:?}");
        match pos[1] {
            10.0 => assert_eq!(color, abgr(TOP)),
            14.0 => {
                let (got, want) = (color.to_le_bytes(), mid.to_le_bytes());
                assert!(
                    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 1),
                    "{got:?}"
                );
            }
            y => panic!("clipped vertex at y {y}"),
        }
    }
    // The painted op under its pose; the reversed one drew nothing.
    for (pos, color) in painted {
        assert!(pos[0] == 50.0 || pos[0] == 54.0, "{pos:?}");
        match pos[1] {
            60.0 => assert_eq!(color, abgr(TOP)),
            62.0 => assert_eq!(color, abgr(BOTTOM)),
            y => panic!("painted vertex at y {y}"),
        }
    }
}
