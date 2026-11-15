//! `RealmsResetWorldScreen.FrameButton`: a preview and slot frame, not a button sprite.
use crate::{
    hud::{HudAtlas, HudBatch},
    text::Text,
    widgets::{self, Kind, Narratable},
};
use std::borrow::Cow;

pub const WIDTH: f32 = 60.0;
pub const HEIGHT: f32 = 69.0;

/// `image` is the preview's HUD atlas key. The label sits above the 60 × 69 control.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    at: [f32; 2],
    label: &Text,
    image: &str,
    active: bool,
    hovered: bool,
    focused: bool,
) {
    let [x, y] = at;
    let rect = [x, y, WIDTH, HEIGHT];
    let plain = label.plain();
    widgets::record_as(&plain, rect, active, Kind::Button);
    if active {
        widgets::target_as(rect, Kind::Button);
    }
    widgets::narratable(|| {
        Narratable::button(plain, rect, active)
            .hovered(hovered)
            .focused(focused)
    });
    widgets::widget_cursor(hovered, active);

    let highlighted = hovered || focused;
    // ARGB.as8BitChannel floors 0.56 * 255 to 142.
    let color = if highlighted { 0xFF8E_8E8E } else { u32::MAX };
    super::blit_sprite(batch, atlas, image, x + 2.0, y + 2.0, 56.0, 56.0, color);
    super::blit_sprite(
        batch,
        atlas,
        "gui/sprites/widget/slot_frame",
        x,
        y,
        WIDTH,
        WIDTH,
        color,
    );

    // WithInactiveMessage grays the label, not the preview or frame.
    let label = label.resolved();
    let label = if active {
        label
    } else {
        Cow::Owned(label.into_owned().with_color(0x00A0_A0A0))
    };
    let left = x + 28.0 - (atlas.font.width_text(&label) / 2.0).floor();
    batch.text_styled(
        atlas,
        &label,
        left,
        y - 14.0,
        if highlighted { 0xFFA0_A0A0 } else { u32::MAX },
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::Rect;

    fn vertices(batch: &HudBatch) -> Vec<([f32; 2], [f32; 2], u32, u32)> {
        batch
            .vertices
            .iter()
            .map(|v| (v.pos, v.uv, v.color, v.text))
            .collect()
    }

    #[test]
    fn frame_registers_full_bounds_and_narrates_with_matching_hover_and_focus_art() {
        // Widget state is thread-local: leave the test runner's frame alone.
        std::thread::spawn(|| {
            let mut atlas =
                HudAtlas::build(std::path::Path::new("/nonexistent/frame-button-assets")).unwrap();
            for (key, u0, size) in [
                ("gui/realms/upload", 0.1, 56),
                ("gui/sprites/widget/slot_frame", 0.5, 60),
            ] {
                atlas.sprites.insert(
                    key.into(),
                    Rect {
                        u0,
                        v0: 0.2,
                        u1: u0 + 0.2,
                        v1: 0.4,
                        w: size,
                        h: size,
                    },
                );
            }
            let label = Text::literal("Upload world");
            let earlier = [1.0, 2.0, 30.0, 20.0];
            let rect = [40.0, 50.0, 60.0, 69.0];
            let mut rendered = Vec::new();
            widgets::set_narrating(true);
            for (i, (active, hovered, focused)) in [
                (true, false, false),
                (true, true, false),
                (true, false, true),
                (false, false, false),
                (false, true, false),
                (false, false, true),
            ]
            .into_iter()
            .enumerate()
            {
                widgets::begin_screen(|| widgets::ScreenInfo::new("frame", "Frame test"));
                widgets::record_as("Earlier", earlier, true, Kind::Button);
                widgets::target_as(earlier, Kind::Button);
                let mut batch = HudBatch::new([320, 240]);
                draw(
                    &mut batch,
                    &atlas,
                    [40.0, 50.0],
                    &label,
                    "gui/realms/upload",
                    active,
                    hovered,
                    focused,
                );
                let cursor = widgets::take_cursor();
                assert_eq!(
                    cursor,
                    match (hovered, active) {
                        (false, _) => widgets::Cursor::Arrow,
                        (true, true) => widgets::Cursor::PointingHand,
                        (true, false) => widgets::Cursor::NotAllowed,
                    }
                );
                widgets::end_frame(i as u64 + 1);
                let registered = widgets::last_labelled();
                assert_eq!(registered.len(), 2);
                assert_eq!(registered[0].label, "Earlier");
                let w = &registered[1];
                assert_eq!(w.label, "Upload world");
                assert_eq!((w.enabled, w.kind), (active, Kind::Button));
                assert_eq!([w.x, w.y, w.w, w.h], rect);
                let targets = widgets::last_targets();
                assert_eq!(targets.len(), 1 + usize::from(active));
                assert_eq!(targets[0].rect, earlier);
                if active {
                    assert_eq!(targets[1].rect, rect);
                    assert_eq!(targets[1].kind, Kind::Button);
                }
                let narration = widgets::take_last_narration();
                assert_eq!(narration.items.len(), 1);
                let n = &narration.items[0];
                assert_eq!(
                    (n.rect, n.hovered, n.focused),
                    (rect, Some(hovered), focused)
                );
                assert!(matches!(
                    &n.what,
                    widgets::What::Button { message, active: enabled }
                        if message == "Upload world" && *enabled == active
                ));
                // Only the 56 × 56 preview and 60 × 60 frame precede the label.
                assert!(batch.vertices.len() > 12);
                assert_eq!(batch.vertices[0].pos, [42.0, 52.0]);
                assert_eq!(batch.vertices[2].pos, [98.0, 108.0]);
                assert_eq!(batch.vertices[0].uv, [0.1, 0.2]);
                assert_eq!(batch.vertices[6].pos, [40.0, 50.0]);
                assert_eq!(batch.vertices[8].pos, [100.0, 110.0]);
                assert_eq!(batch.vertices[6].uv, [0.5, 0.2]);
                let tint = if hovered || focused {
                    0xFF8E_8E8E
                } else {
                    u32::MAX
                };
                assert!(batch.vertices[..12].iter().all(|v| v.color == tint));
                let expected_label = if active {
                    label.clone()
                } else {
                    label.clone().with_color(0x00A0_A0A0)
                };
                let mut expected = HudBatch::new([320, 240]);
                expected.text_styled(
                    &atlas,
                    &expected_label,
                    68.0 - (atlas.font.width_text(&expected_label) / 2.0).floor(),
                    36.0,
                    if hovered || focused {
                        0xFFA0_A0A0
                    } else {
                        u32::MAX
                    },
                    true,
                );
                assert_eq!(&vertices(&batch)[12..], vertices(&expected).as_slice());
                rendered.push(vertices(&batch));
            }
            assert_eq!(rendered[1], rendered[2]);
            assert_eq!(rendered[4], rendered[5]);
            assert_eq!(rendered[1], rendered[4]);
            assert_ne!(rendered[0], rendered[3]);
        })
        .join()
        .unwrap();
    }
}
