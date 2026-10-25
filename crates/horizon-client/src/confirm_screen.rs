//! `ConfirmScreen`: a title, a message wrapped to `width - 50` (15 lines at
//! most) and two 150-wide buttons 4 apart and 16 below it, the whole column
//! centred; Escape answers no.
use crate::{
    font::Font,
    hud::{HudAtlas, HudBatch},
    text::Text,
    ui,
};

pub struct Confirm {
    /// Each line and where it is drawn.
    pub lines: Vec<(Text, f32, f32)>,
    /// The yes and no buttons' corners.
    pub buttons: [(f32, f32); 2],
}
impl Confirm {
    pub fn new(font: &Font, (w, h): (f32, f32), title: &Text, message: &Text) -> Self {
        let message: Vec<Text> = font
            .split_lines(message, w - 50.0)
            .into_iter()
            .map(|(line, _)| line)
            .take(15)
            .collect();
        let tw = font.width_text(title);
        let mw = message
            .iter()
            .map(|l| font.width_text(l))
            .fold(0.0, f32::max);
        let mh = 9.0 * message.len() as f32;
        let col = tw.max(mw).max(304.0);
        let total = 9.0 + 8.0 + mh + 8.0 + 36.0;
        let (lx, ly) = (((w - col) / 2.0).trunc(), ((h - total) / 2.0).trunc());
        let cell = |cw: f32| lx + ((col - cw) / 2.0).trunc();
        let mid = cell(mw) + (mw / 2.0).trunc();
        let mut lines = vec![(title.clone(), cell(tw), ly)];
        for (i, line) in message.into_iter().enumerate() {
            let x = mid - (font.width_text(&line) / 2.0).trunc();
            lines.push((line, x, ly + 17.0 + 9.0 * i as f32));
        }
        let (bx, by) = (cell(304.0), ly + 9.0 + 8.0 + mh + 8.0 + 16.0);
        Self {
            lines,
            buttons: [(bx, by), (bx + 154.0, by)],
        }
    }
    /// The answer a click at `cursor` gives: true for yes, false for no.
    pub fn answer_at(&self, cursor: (f32, f32)) -> Option<bool> {
        let [yes, no] = self.buttons;
        let on = |(x, y): (f32, f32)| ui::hovered(cursor, x, y, 150.0, 20.0);
        if on(yes) {
            Some(true)
        } else if on(no) {
            Some(false)
        } else {
            None
        }
    }
    /// The lines and the buttons (yes first), over whatever is behind.
    pub fn draw(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        labels: [&Text; 2],
    ) {
        for (line, x, y) in &self.lines {
            batch.text_styled(atlas, line, *x, *y, u32::MAX, true);
        }
        for ((bx, by), label) in self.buttons.into_iter().zip(labels) {
            let hover = ui::hovered(cursor, bx, by, 150.0, 20.0);
            ui::button(batch, atlas, bx, by, 150.0, 20.0, label, true, hover);
        }
    }
}
