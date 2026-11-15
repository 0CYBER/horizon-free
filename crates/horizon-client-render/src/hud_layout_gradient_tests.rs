//! `GuiDraw::FillGradient` through [`paint`]: colours tied to their input y edges, alpha
//! kept per corner, and the current pose and clip applied. Zero area and one-axis inversion
//! draw nothing; both axes inverted swaps the visual top and bottom colours.
use super::{GuiDraw, PoseOp, abgr, paint};
use crate::hud::{HudAtlas, HudBatch};

fn atlas() -> HudAtlas {
    HudAtlas::build(&crate::resource_source::ResourceSource::directory(
        "/nonexistent/horizon-test-assets".into(),
    ))
    .unwrap()
}

fn gradient(x0: i32, y0: i32, x1: i32, y1: i32, top: u32, bottom: u32) -> GuiDraw {
    GuiDraw::FillGradient {
        x0,
        y0,
        x1,
        y1,
        top,
        bottom,
    }
}

/// Each vertex's position and packed colour.
fn vertices(batch: &HudBatch) -> Vec<([f32; 2], u32)> {
    batch.vertices.iter().map(|v| (v.pos, v.color)).collect()
}

#[test]
fn the_top_colour_is_on_the_top_edge_and_the_bottom_colour_on_the_bottom_edge() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let (top, bottom) = (0x00FF_0000, 0xFF00_00FF);
    paint(&mut batch, &atlas, &[gradient(10, 20, 30, 28, top, bottom)]);
    let drawn = vertices(&batch);
    assert_eq!(drawn.len(), 6);
    for (pos, color) in drawn {
        assert!(pos[0] == 10.0 || pos[0] == 30.0, "{pos:?}");
        match pos[1] {
            20.0 => assert_eq!(color, abgr(top)),
            28.0 => assert_eq!(color, abgr(bottom)),
            y => panic!("vertex off the rectangle at y {y}"),
        }
    }
    // ARGB in, the batch's packed RGBA out, alpha included: transparent red to opaque blue.
    assert_eq!(abgr(top) >> 24, 0x00);
    assert_eq!(abgr(bottom) >> 24, 0xFF);
}

#[test]
fn both_inverted_axes_keep_the_first_colour_on_the_lower_edge() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    paint(
        &mut batch,
        &atlas,
        &[gradient(30, 28, 10, 20, 0x8011_2233, 0xFF44_5566)],
    );
    assert_eq!(
        vertices(&batch),
        [
            ([10.0, 20.0], 0xFF66_5544),
            ([30.0, 20.0], 0xFF66_5544),
            ([30.0, 28.0], 0x8033_2211),
            ([10.0, 20.0], 0xFF66_5544),
            ([30.0, 28.0], 0x8033_2211),
            ([10.0, 28.0], 0x8033_2211),
        ]
    );
}

#[test]
fn a_clip_through_the_middle_keeps_the_ramp_between_the_two_colours() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let (top, bottom) = (0xFFFF_0000, 0x0000_00FF);
    paint(&mut batch, &atlas, &[gradient(0, 0, 10, 8, top, bottom)]);
    batch.clip_since(0, 0.0, 2.0, 10.0, 6.0);
    // A quarter and three quarters of the way down: every channel, alpha too, is mixed.
    let at = |y: f32| {
        let colors: Vec<u32> = batch
            .vertices
            .iter()
            .filter(|v| v.pos[1] == y)
            .map(|v| v.color)
            .collect();
        assert!(!colors.is_empty(), "nothing on the cut at y {y}");
        assert!(colors.iter().all(|c| *c == colors[0]));
        colors[0].to_le_bytes()
    };
    let near = |got: [u8; 4], want: [u8; 4]| {
        for (g, w) in got.into_iter().zip(want) {
            assert!(g.abs_diff(w) <= 1, "{got:?} is not {want:?}");
        }
    };
    // Packed RGBA bytes are r, g, b, a.
    near(at(2.0), [191, 0, 64, 191]);
    near(at(6.0), [64, 0, 191, 64]);
}

#[test]
fn an_empty_or_reversed_rectangle_draws_nothing() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let (top, bottom) = (0xFFFF_FFFF, 0xFF00_0000);
    paint(
        &mut batch,
        &atlas,
        &[
            gradient(5, 5, 5, 20, top, bottom),
            gradient(5, 5, 20, 5, top, bottom),
            gradient(20, 5, 5, 20, top, bottom),
            gradient(5, 20, 20, 5, top, bottom),
            gradient(i32::MAX, 0, i32::MIN, 1, top, bottom),
        ],
    );
    assert!(batch.vertices.is_empty());
}

#[test]
fn the_current_pose_moves_and_scales_the_gradient_and_pop_restores_it() {
    let atlas = atlas();
    let mut batch = HudBatch::scratch([854, 480]);
    let (top, bottom) = (0xFF11_2233, 0xFF44_5566);
    paint(
        &mut batch,
        &atlas,
        &[
            GuiDraw::Push(vec![
                PoseOp::Translate(100.0, 50.0),
                PoseOp::Scale(2.0, 3.0),
            ]),
            gradient(1, 2, 4, 6, top, bottom),
            GuiDraw::Pop,
            gradient(1, 2, 4, 6, top, bottom),
        ],
    );
    let drawn = vertices(&batch);
    assert_eq!(drawn.len(), 12);
    for (pos, color) in &drawn[..6] {
        assert!(pos[0] == 102.0 || pos[0] == 108.0, "{pos:?}");
        match pos[1] {
            56.0 => assert_eq!(*color, abgr(top)),
            68.0 => assert_eq!(*color, abgr(bottom)),
            y => panic!("posed vertex off the rectangle at y {y}"),
        }
    }
    for (pos, _) in &drawn[6..] {
        assert!(pos[0] == 1.0 || pos[0] == 4.0, "{pos:?}");
        assert!(pos[1] == 2.0 || pos[1] == 6.0, "{pos:?}");
    }
}
