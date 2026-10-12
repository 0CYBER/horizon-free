//! The subtitle overlay in the game (`Hud` -> `SubtitleOverlay`): the sound
//! engine reports the sounds it plays while the overlay listens, the HUD
//! renders it (which starts or stops the listening as the "Show Subtitles"
//! option says).
use super::Game;
use horizon_client::{
    hud::{HudBatch, rgba},
    subtitles::{self, Draw, Frame, SubtitleOverlay},
};
use std::cell::RefCell;

#[derive(Default)]
pub(super) struct SubtitleState {
    overlay: RefCell<SubtitleOverlay>,
}
impl Game<'_> {
    /// Feed the sounds the engine reported since the last tick
    /// (`SoundEventListener.onPlaySound`) while the overlay listens.
    pub(super) fn tick_subtitles(&mut self) {
        let mut overlay = self.subtitles.overlay.borrow_mut();
        let listening = overlay.listening();
        self.audio.record_heard = listening;
        let heard = self.audio.take_heard();
        if !listening {
            return;
        }
        let now = subtitles::now_ms();
        for h in heard {
            let at = h.at.unwrap_or(self.audio.listener);
            overlay.on_play(Some(&h.subtitle), at, h.range, h.looping, now);
        }
    }
    pub(super) fn draw_subtitles(&self, batch: &mut HudBatch) {
        // Hud.extractRenderState: hidden with the rest of the HUD (F1).
        if self.ui.hide_gui {
            return;
        }
        let v = &self.view;
        let frame = Frame {
            show: self.settings.show_subtitles,
            position: v.position,
            forward: v.forwards.map(f64::from),
            up: v.up.map(f64::from),
            multiplier: f64::from(self.settings.notification_display_time),
            background: subtitles::background_color(
                0.8,
                self.settings.text_background_opacity,
                self.settings.file.bool("backgroundForChatOnly"),
            ),
            width: batch.width as i32,
            height: batch.height as i32,
        };
        let atlas = &self.res.atlas;
        let draws =
            self.subtitles
                .overlay
                .borrow_mut()
                .extract(&frame, &atlas.font, subtitles::now_ms());
        let argb = |c: u32| {
            let [a, r, g, b] = c.to_be_bytes();
            rgba(r, g, b, a)
        };
        for d in draws {
            match d {
                Draw::Stratum => (),
                Draw::Fill {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                    translate: [tx, ty],
                } => {
                    batch.fill(
                        atlas,
                        tx + x0 as f32,
                        ty + y0 as f32,
                        (x1 - x0) as f32,
                        (y1 - y0) as f32,
                        argb(color),
                    );
                }
                Draw::Text {
                    text,
                    x,
                    y,
                    color,
                    translate: [tx, ty],
                } => {
                    batch.text_styled(
                        atlas,
                        &text,
                        tx + x as f32,
                        ty + y as f32,
                        argb(color),
                        true,
                    );
                }
            }
        }
    }
}
