//! The subtitle overlay (`SubtitleOverlay`): closed captions for the sounds
//! the player could hear, with direction arrows and a fade over the
//! notification display time.
use crate::{
    audio::Heard,
    hud::{HudAtlas, HudBatch, rgba},
    text::Text,
};

/// How long a subtitle shows before it is dropped (milliseconds).
const DISPLAY_TIME: f64 = 3000.0;
/// Looping sounds stay at full brightness this long.
const LOOPING_FULL_BRIGHTNESS: f64 = 500.0;

#[derive(Clone, Debug, PartialEq)]
struct PlayedAt {
    location: [f64; 3],
    time: u64,
}
#[derive(Clone, Debug)]
struct Subtitle {
    key: String,
    range: f32,
    played: Vec<PlayedAt>,
    looping: bool,
}
fn distance_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}
/// `Vec3.closerThan`.
fn closer_than(a: [f64; 3], b: [f64; 3], range: f32) -> bool {
    distance_sqr(a, b) < f64::from(range) * f64::from(range)
}
impl Subtitle {
    fn best_candidate(&self, position: [f64; 3]) -> Option<&PlayedAt> {
        match self.played.len() {
            0 => None,
            1 => self.played.first(),
            _ => self
                .played
                .iter()
                .filter(|p| closer_than(p.location, position, self.range))
                .max_by(|a, b| {
                    a.time.cmp(&b.time).then_with(|| {
                        distance_sqr(a.location, position)
                            .total_cmp(&distance_sqr(b.location, position))
                    })
                }),
        }
    }
    fn audible_from(&self, camera: [f64; 3]) -> bool {
        if self.range.is_infinite() {
            return true;
        }
        self.best_candidate(camera)
            .is_some_and(|p| closer_than(camera, p.location, self.range))
    }
    fn still_active(&self, multiplier: f64, now: u64) -> bool {
        let max_age = DISPLAY_TIME * multiplier;
        self.played
            .iter()
            .any(|p| (now.saturating_sub(p.time) as f64) <= max_age)
    }
    /// `getBrightness`: fades from 255 to 75 over the display time.
    fn brightness(&self, at: &PlayedAt, multiplier: f64, now: u64) -> i32 {
        let full = if self.looping {
            LOOPING_FULL_BRIGHTNESS
        } else {
            0.0
        };
        let age = (now.saturating_sub(at.time) as f64 - full).max(0.0);
        let t = (age / ((DISPLAY_TIME - full) * multiplier)) as f32;
        let t = t.clamp(0.0, 1.0);
        (255.0_f32 + (75.0_f32 - 255.0_f32) * t).floor() as i32
    }
}

/// A row to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub text: Text,
    pub brightness: i32,
    pub arrow: Arrow,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    None,
    Left,
    Right,
}

/// Milliseconds on a monotonic clock (`Util.getMillis`).
pub fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
#[derive(Default)]
pub struct SubtitleOverlay {
    subtitles: Vec<Subtitle>,
}
impl SubtitleOverlay {
    /// `onPlaySound`: a repeated subtitle refreshes its location.
    pub fn on_play(&mut self, heard: &Heard, listener: [f64; 3], now: u64) {
        let location = heard.at.unwrap_or(listener);
        if let Some(s) = self.subtitles.iter_mut().find(|s| s.key == heard.subtitle) {
            s.played.retain(|p| p.location != location);
            s.played.push(PlayedAt {
                location,
                time: now,
            });
            return;
        }
        self.subtitles.push(Subtitle {
            key: heard.subtitle.clone(),
            range: heard.range,
            played: vec![PlayedAt {
                location,
                time: now,
            }],
            looping: heard.looping,
        });
    }
    pub fn clear(&mut self) {
        self.subtitles.clear();
    }
    /// `purgeOldInstances`: forget sounds older than the display time.
    pub fn purge(&mut self, multiplier: f64, now: u64) {
        let max_age = DISPLAY_TIME * multiplier;
        for s in &mut self.subtitles {
            s.played
                .retain(|p| (now.saturating_sub(p.time) as f64) <= max_age);
        }
    }
    /// The audible subtitles for a listener looking along `forward` with
    /// `right` to the right, and the width the box needs.
    pub fn rows(
        &self,
        position: [f64; 3],
        forward: [f64; 3],
        right: [f64; 3],
        multiplier: f64,
        now: u64,
    ) -> Vec<Row> {
        let mut out = Vec::new();
        for s in &self.subtitles {
            if !(s.audible_from(position) && s.still_active(multiplier, now)) {
                continue;
            }
            let Some(at) = s.best_candidate(position) else {
                continue;
            };
            let mut delta = [
                at.location[0] - position[0],
                at.location[1] - position[1],
                at.location[2] - position[2],
            ];
            let length = distance_sqr(delta, [0.0; 3]).sqrt();
            if length >= 1.0e-4 {
                delta = delta.map(|v| v / length);
            } else {
                delta = [0.0; 3];
            }
            let dot = |a: [f64; 3]| a[0] * delta[0] + a[1] * delta[1] + a[2] * delta[2];
            let (rightness, forwardness) = (dot(right), dot(forward));
            let arrow = if forwardness > 0.5 {
                Arrow::None
            } else if rightness > 0.0 {
                Arrow::Right
            } else if rightness < 0.0 {
                Arrow::Left
            } else {
                Arrow::None
            };
            out.push(Row {
                text: Text::translate(&s.key, &[]),
                brightness: s.brightness(at, multiplier, now),
                arrow,
            });
        }
        out
    }
    /// `extractRenderState`: the boxes at the bottom right.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
        multiplier: f64,
        now: u64,
    ) {
        let (forward, right) = listener_vectors(yaw, pitch);
        let rows = self.rows(position, forward, right, multiplier, now);
        if rows.is_empty() {
            return;
        }
        let font = &atlas.font;
        let mut width = rows
            .iter()
            .map(|r| font.width_text(&r.text))
            .fold(0.0_f32, f32::max);
        width += font.width("<") + font.width(" ") + font.width(">") + font.width(" ");
        let half_width = (width / 2.0).trunc();
        let background = rgba(0, 0, 0, (0.8 * 255.0) as u8);
        for (row, r) in rows.iter().enumerate() {
            let bx = batch.width - half_width - 2.0;
            let by = batch.height - 35.0 - row as f32 * 10.0;
            batch.fill(
                atlas,
                bx - half_width - 1.0,
                by - 4.0 - 1.0,
                half_width * 2.0 + 2.0,
                10.0,
                background,
            );
            let b = r.brightness.clamp(0, 255) as u8;
            let color = rgba(b, b, b, 255);
            match r.arrow {
                Arrow::Right => {
                    batch.text(
                        atlas,
                        ">",
                        bx + half_width - font.width(">"),
                        by - 4.0,
                        color,
                        true,
                    );
                }
                Arrow::Left => {
                    batch.text(atlas, "<", bx - half_width, by - 4.0, color, true);
                }
                Arrow::None => {}
            }
            let text_width = font.width_text(&r.text);
            batch.text_styled(
                atlas,
                &r.text,
                bx - (text_width / 2.0).trunc(),
                by - 4.0,
                color,
                true,
            );
        }
    }
}
/// The listener's forward and right vectors for a Minecraft yaw and pitch
/// (degrees): `ListenerTransform.forward()` and `.right()`.
pub fn listener_vectors(yaw: f32, pitch: f32) -> ([f64; 3], [f64; 3]) {
    let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    let (sy, cy, sp, cp) = (yaw.sin(), yaw.cos(), pitch.sin(), pitch.cos());
    ([-sy * cp, -sp, cy * cp], [-cy, 0.0, -sy])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(key: &str, at: Option<[f64; 3]>, range: f32) -> Heard {
        Heard {
            subtitle: key.into(),
            at,
            range,
            looping: false,
        }
    }
    const ORIGIN: [f64; 3] = [0.0; 3];

    #[test]
    fn arrows_follow_the_sound_direction() {
        // Looking south (yaw 0): forward +z, right -x.
        let (forward, right) = listener_vectors(0.0, 0.0);
        assert_eq!(forward, [0.0, 0.0, 1.0]);
        assert!((right[0] + 1.0).abs() < 1e-9);
        let mut o = SubtitleOverlay::default();
        o.on_play(&heard("a", Some([0.0, 0.0, 5.0]), 16.0), ORIGIN, 0);
        o.on_play(&heard("b", Some([-5.0, 0.0, 0.0]), 16.0), ORIGIN, 0);
        o.on_play(&heard("c", Some([5.0, 0.0, 0.0]), 16.0), ORIGIN, 0);
        let rows = o.rows(ORIGIN, forward, right, 1.0, 0);
        let arrows: Vec<Arrow> = rows.iter().map(|r| r.arrow).collect();
        // in front, to the right (west is -x), to the left.
        assert_eq!(arrows, vec![Arrow::None, Arrow::Right, Arrow::Left]);
    }

    #[test]
    fn range_fade_and_expiry() {
        let mut o = SubtitleOverlay::default();
        let (f, r) = listener_vectors(0.0, 0.0);
        o.on_play(&heard("near", Some([0.0, 0.0, 3.0]), 16.0), ORIGIN, 1000);
        o.on_play(&heard("far", Some([0.0, 0.0, 40.0]), 16.0), ORIGIN, 1000);
        let rows = o.rows(ORIGIN, f, r, 1.0, 1000);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].brightness, 255);
        // Halfway through the display time the colour is halfway to 75.
        let rows = o.rows(ORIGIN, f, r, 1.0, 2500);
        assert_eq!(rows[0].brightness, 165);
        // A longer notification time stretches the fade.
        let rows = o.rows(ORIGIN, f, r, 2.0, 2500);
        assert_eq!(rows[0].brightness, 210);
        // Gone after three seconds.
        assert!(o.rows(ORIGIN, f, r, 1.0, 4001).is_empty());
    }

    #[test]
    fn repeats_refresh_instead_of_stacking() {
        let mut o = SubtitleOverlay::default();
        let (f, r) = listener_vectors(0.0, 0.0);
        o.on_play(&heard("a", Some([0.0, 0.0, 3.0]), 16.0), ORIGIN, 0);
        o.on_play(&heard("a", Some([0.0, 0.0, 3.0]), 16.0), ORIGIN, 2000);
        o.on_play(&heard("a", Some([0.0, 0.0, 4.0]), 16.0), ORIGIN, 2500);
        let rows = o.rows(ORIGIN, f, r, 1.0, 3000);
        assert_eq!(rows.len(), 1);
        // The newest candidate in range is the reference for the fade.
        assert_eq!(rows[0].brightness, 225);
    }

    #[test]
    fn looping_sounds_stay_bright_longer() {
        let mut o = SubtitleOverlay::default();
        let (f, r) = listener_vectors(0.0, 0.0);
        let mut h = heard("loop", None, f32::INFINITY);
        h.looping = true;
        o.on_play(&h, ORIGIN, 0);
        assert_eq!(o.rows(ORIGIN, f, r, 1.0, 500)[0].brightness, 255);
        // Non-positional sounds are always audible and never show an arrow.
        assert_eq!(o.rows(ORIGIN, f, r, 1.0, 500)[0].arrow, Arrow::None);
    }
}
