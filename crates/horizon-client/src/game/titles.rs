//! Titles, subtitles and the action bar (`Gui.setTitle`, `setSubtitle`,
//! `setTimes`, `setOverlayMessage`, `extractTitle`, `extractOverlayMessage`).
use super::Game;
use horizon_client::{
    hud::{HudBatch, rgba},
    text::Text,
};

pub(super) struct Titles {
    pub title: Option<Text>,
    pub subtitle: Option<Text>,
    pub time: i32,
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
    pub overlay: Option<Text>,
    pub overlay_time: i32,
    /// `animateOverlayMessageColor`: the "Now Playing" rainbow.
    pub overlay_animated: bool,
}
impl Default for Titles {
    fn default() -> Self {
        Self {
            title: None,
            subtitle: None,
            time: 0,
            fade_in: 10,
            stay: 70,
            fade_out: 20,
            overlay: None,
            overlay_time: 0,
            overlay_animated: false,
        }
    }
}
impl Titles {
    pub fn set_title(&mut self, title: Text) {
        self.title = Some(title);
        self.time = self.fade_in + self.stay + self.fade_out;
    }
    pub fn set_times(&mut self, fade_in: i32, stay: i32, fade_out: i32) {
        if fade_in >= 0 {
            self.fade_in = fade_in;
        }
        if stay >= 0 {
            self.stay = stay;
        }
        if fade_out >= 0 {
            self.fade_out = fade_out;
        }
        if self.time > 0 {
            self.time = self.fade_in + self.stay + self.fade_out;
        }
    }
    pub fn clear(&mut self) {
        self.title = None;
        self.subtitle = None;
        self.time = 0;
    }
    pub fn reset_times(&mut self) {
        (self.fade_in, self.stay, self.fade_out) = (10, 70, 20);
    }
    pub fn set_overlay(&mut self, text: Text) {
        self.overlay = Some(text);
        self.overlay_time = 60;
        self.overlay_animated = false;
    }
    /// Gui.tick.
    pub fn tick(&mut self) {
        if self.overlay_time > 0 {
            self.overlay_time -= 1;
        }
        if self.time > 0 {
            self.time -= 1;
            if self.time <= 0 {
                self.title = None;
                self.subtitle = None;
            }
        }
    }
}
impl Game<'_> {
    pub(super) fn draw_titles(&self, batch: &mut HudBatch) {
        let t = &self.titles;
        let partial = self.clock.fraction() as f32;
        let atlas = &self.res.atlas;
        let (w, h) = (batch.width, batch.height);
        if let Some(text) = &t.overlay
            && t.overlay_time > 0
        {
            let ticks = t.overlay_time as f32 - partial;
            let alpha = ((ticks * 255.0 / 20.0) as i32).min(255);
            if alpha > 0 {
                let width = atlas.font.width_text(text);
                let color = if t.overlay_animated {
                    let [r, g, b] = hsv_to_rgb(ticks / 50.0, 0.7, 0.6);
                    rgba(r, g, b, alpha as u8)
                } else {
                    rgba(255, 255, 255, alpha as u8)
                };
                batch.text_posed(
                    atlas,
                    text,
                    [-(width / 2.0).trunc(), -4.0],
                    color,
                    1.0,
                    [(w / 2.0).trunc(), (h - 68.0).trunc()],
                );
            }
        }
        let Some(title) = &t.title else { return };
        if t.time <= 0 {
            return;
        }
        let ticks = t.time as f32 - partial;
        let mut alpha = 255;
        if t.time > t.fade_out + t.stay {
            let time = (t.fade_in + t.stay + t.fade_out) as f32 - ticks;
            alpha = (time * 255.0 / t.fade_in.max(1) as f32) as i32;
        }
        if t.time <= t.fade_out {
            alpha = (ticks * 255.0 / t.fade_out.max(1) as f32) as i32;
        }
        let alpha = alpha.clamp(0, 255);
        if alpha == 0 {
            return;
        }
        let color = rgba(255, 255, 255, alpha as u8);
        let origin = [(w / 2.0).trunc(), (h / 2.0).trunc()];
        let width = atlas.font.width_text(title);
        batch.text_posed(
            atlas,
            title,
            [-(width / 2.0).trunc(), -10.0],
            color,
            4.0,
            origin,
        );
        if let Some(sub) = &t.subtitle {
            let width = atlas.font.width_text(sub);
            batch.text_posed(
                atlas,
                sub,
                [-(width / 2.0).trunc(), 5.0],
                color,
                2.0,
                origin,
            );
        }
    }
}
/// `Mth.hsvToRgb`.
fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let h = (hue * 6.0) as i32 % 6;
    let f = hue * 6.0 - (hue * 6.0).floor();
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - f * saturation);
    let t = value * (1.0 - (1.0 - f) * saturation);
    let (r, g, b) = match h {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        _ => (value, p, q),
    };
    let c = |v: f32| (v * 255.0).clamp(0.0, 255.0) as u8;
    [c(r), c(g), c(b)]
}
