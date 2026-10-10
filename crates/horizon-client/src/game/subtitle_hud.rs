//! The subtitle overlay in the game (`Hud` -> `SubtitleOverlay`): sounds the
//! audio engine plays feed it, the HUD draws it.
use super::Game;
use horizon_client::{
    hud::HudBatch,
    subtitles::{self, SubtitleOverlay},
};

#[derive(Default)]
pub(super) struct SubtitleState {
    overlay: SubtitleOverlay,
    listening: bool,
}
impl Game<'_> {
    /// Feed the sounds played since the last tick to the overlay while the
    /// `showSubtitles` option is on.
    pub(super) fn tick_subtitles(&mut self) {
        let on = self.settings.show_subtitles;
        self.audio.record_heard = on;
        if !on {
            if self.subtitles.listening {
                self.subtitles.listening = false;
                self.subtitles.overlay.clear();
            }
            self.audio.take_heard();
            return;
        }
        self.subtitles.listening = true;
        let now = subtitles::now_ms();
        let listener = self.audio.listener;
        for heard in self.audio.take_heard() {
            self.subtitles.overlay.on_play(&heard, listener, now);
        }
        self.subtitles
            .overlay
            .purge(f64::from(self.settings.notification_display_time), now);
    }
    pub(super) fn draw_subtitles(&self, batch: &mut HudBatch) {
        if !self.settings.show_subtitles {
            return;
        }
        self.subtitles.overlay.draw(
            batch,
            &self.res.atlas,
            self.audio.listener,
            self.yaw,
            self.pitch,
            f64::from(self.settings.notification_display_time),
            subtitles::now_ms(),
        );
    }
}
