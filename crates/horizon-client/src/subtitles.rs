//! The subtitle overlay (`SubtitleOverlay`): closed captions for the sounds
//! the player could hear, with direction arrows and a fade over the
//! notification display time.
//!
//! Like vanilla, the overlay listens to the sound engine only from the frame
//! that renders it with the "Show Subtitles" option on until a frame renders
//! with it off; turning it off keeps the captions it has (they show again if
//! the option comes back before they expire).
use crate::{font::Font, text::Text};

/// `SubtitleOverlay.DISPLAY_TIME` (milliseconds).
const DISPLAY_TIME: i64 = 3000;
/// `LOOPING_DISPLAY_FULL_BRIGHTNESS_TIME`: looping sounds stay at full brightness this long.
const LOOPING_FULL_BRIGHTNESS: i64 = 500;

#[derive(Clone, Copy, Debug, PartialEq)]
struct PlayedAt {
    location: [f64; 3],
    time: i64,
}
#[derive(Clone, Debug)]
struct Subtitle {
    /// The translation key of the sound event's subtitle (`Component.translatable`).
    key: String,
    range: f32,
    played: Vec<PlayedAt>,
    looping: bool,
}
fn distance_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz
}
/// `Vec3.closerThan(pos, distance)`.
fn closer_than(a: [f64; 3], b: [f64; 3], range: f32) -> bool {
    let r = f64::from(range);
    distance_sqr(a, b) < r * r
}
/// `Vec3.equals`: `Double.compare` on each axis.
fn same_location(a: [f64; 3], b: [f64; 3]) -> bool {
    let bits = |v: f64| {
        if v.is_nan() {
            f64::NAN.to_bits()
        } else {
            v.to_bits()
        }
    };
    (0..3).all(|i| bits(a[i]) == bits(b[i]))
}
impl Subtitle {
    /// `getBestSubtitleCandidate`: the only play, else the latest in range
    /// (the farthest of equally recent ones; the first among full ties).
    fn best_candidate(&self, position: [f64; 3]) -> Option<PlayedAt> {
        match self.played.as_slice() {
            [] => None,
            [only] => Some(*only),
            many => many
                .iter()
                .filter(|p| closer_than(p.location, position, self.range))
                .fold(None, |best: Option<PlayedAt>, p| match best {
                    Some(b)
                        if b.time > p.time
                            || b.time == p.time
                                && distance_sqr(b.location, position).sqrt()
                                    >= distance_sqr(p.location, position).sqrt() =>
                    {
                        Some(b)
                    }
                    _ => Some(*p),
                }),
        }
    }
    fn refresh(&mut self, location: [f64; 3], now: i64) {
        self.played.retain(|p| !same_location(location, p.location));
        self.played.push(PlayedAt {
            location,
            time: now,
        });
    }
    fn audible_from(&self, camera: [f64; 3]) -> bool {
        if self.range.is_infinite() {
            return true;
        }
        self.best_candidate(camera)
            .is_some_and(|p| closer_than(camera, p.location, self.range))
    }
    fn purge_old_instances(&mut self, max_age: f64, now: i64) {
        self.played.retain(|p| ((now - p.time) as f64) <= max_age);
    }
    /// `getBrightness`: fades from 255 to 75 over the display time.
    fn brightness(&self, at: PlayedAt, multiplier: f64, now: i64) -> i32 {
        let full = if self.looping {
            LOOPING_FULL_BRIGHTNESS
        } else {
            0
        };
        let age = (now - at.time - full).max(0) as f32;
        let t = age / (((DISPLAY_TIME - full) as f64) * multiplier) as f32;
        let v = if t < 0.0 {
            255.0
        } else if t > 1.0 {
            75.0
        } else {
            255.0 + t * (75.0 - 255.0_f32)
        };
        v.floor() as i32
    }
}

/// What a frame needs to render the overlay: the sound listener, the
/// accessibility options and the GUI size.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// `options.showSubtitles`.
    pub show: bool,
    /// `SoundManager.getListenerTransform`: position, forward and up.
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
    /// `options.notificationDisplayTime`.
    pub multiplier: f64,
    /// `options.getBackgroundColor(0.8F)` (ARGB).
    pub background: u32,
    pub width: i32,
    pub height: i32,
}
/// `Options.getBackgroundColor(defaultOpacity)`: the text background opacity
/// unless the background is for chat only.
pub fn background_color(
    default_opacity: f32,
    text_background_opacity: f32,
    chat_only: bool,
) -> u32 {
    let alpha = if chat_only {
        default_opacity
    } else {
        text_background_opacity
    };
    ((alpha * 255.0).floor() as i32 as u32) << 24
}
/// One call the overlay makes, with the pose translation it is made under.
#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    /// `graphics.nextStratum()`.
    Stratum,
    Fill {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: u32,
        translate: [f32; 2],
    },
    /// Text with a drop shadow.
    Text {
        text: Text,
        x: i32,
        y: i32,
        color: u32,
        translate: [f32; 2],
    },
}

/// Milliseconds on a monotonic clock (`Util.getMillis`).
pub fn now_ms() -> i64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as i64
}
#[derive(Default)]
pub struct SubtitleOverlay {
    subtitles: Vec<Subtitle>,
    listening: bool,
}
impl SubtitleOverlay {
    /// Whether the overlay is one of the sound engine's listeners.
    pub fn listening(&self) -> bool {
        self.listening
    }
    /// `onPlaySound` for a sound the engine played while the overlay listens:
    /// a subtitle already listed (by its text) refreshes its location.
    pub fn on_play(
        &mut self,
        subtitle: Option<&str>,
        location: [f64; 3],
        range: f32,
        looping: bool,
        now: i64,
    ) {
        let Some(key) = subtitle else {
            return;
        };
        if let Some(s) = self.subtitles.iter_mut().find(|s| s.key == key) {
            s.refresh(location, now);
            return;
        }
        self.subtitles.push(Subtitle {
            key: key.to_owned(),
            range,
            played: vec![PlayedAt {
                location,
                time: now,
            }],
            looping,
        });
    }
    /// `extractRenderState`: starts or stops listening as the option says,
    /// forgets old plays and lays out the boxes at the bottom right.
    pub fn extract(&mut self, frame: &Frame, font: &Font, now: i64) -> Vec<Draw> {
        let mut out = Vec::new();
        if self.listening != frame.show {
            self.listening = frame.show;
        }
        if !self.listening {
            return out;
        }
        let position = frame.position;
        let (f, u) = (frame.forward, frame.up);
        // ListenerTransform.right: forward x up.
        let right = [
            f[1] * u[2] - f[2] * u[1],
            f[2] * u[0] - f[0] * u[2],
            f[0] * u[1] - f[1] * u[0],
        ];
        let width_of = |t: &Text| font.width_text(t) as i32;
        let mut audible = Vec::new();
        let mut width = 0;
        for (i, s) in self.subtitles.iter_mut().enumerate() {
            if !s.played.is_empty() {
                s.purge_old_instances(3000.0 * frame.multiplier, now);
            }
            if s.audible_from(position) && !s.played.is_empty() {
                width = width.max(width_of(&Text::translate(&s.key, &[])));
                audible.push(i);
            }
        }
        if audible.is_empty() {
            return out;
        }
        let w = |s: &str| font.width(s) as i32;
        width += w("<") + w(" ") + w(">") + w(" ");
        out.push(Draw::Stratum);
        let mut row = 0;
        for i in audible {
            let s = &self.subtitles[i];
            let text = Text::translate(&s.key, &[]);
            let Some(closest) = s.best_candidate(position) else {
                continue;
            };
            let mut delta = [
                closest.location[0] - position[0],
                closest.location[1] - position[1],
                closest.location[2] - position[2],
            ];
            // Vec3.normalize.
            let length = distance_sqr(delta, [0.0; 3]).sqrt();
            delta = if length < f64::from(1.0e-5_f32) {
                [0.0; 3]
            } else {
                delta.map(|v| v / length)
            };
            let dot = |a: [f64; 3]| a[0] * delta[0] + a[1] * delta[1] + a[2] * delta[2];
            let (rightness, forwardness) = (dot(right), dot(f));
            let in_view = forwardness > 0.5;
            let half_width = width / 2;
            let height = 9;
            let half_height = height / 2;
            let text_width = width_of(&text);
            let brightness = s.brightness(closest, frame.multiplier, now);
            let translate = [
                frame.width as f32 - half_width as f32 * 1.0 - 2.0,
                (frame.height - 35) as f32 - (row * (height + 1)) as f32 * 1.0,
            ];
            out.push(Draw::Fill {
                x0: -half_width - 1,
                y0: -half_height - 1,
                x1: half_width + 1,
                y1: half_height + 1,
                color: frame.background,
                translate,
            });
            let b = brightness as u32 & 0xFF;
            let color = 0xFF00_0000 | b << 16 | b << 8 | b;
            if !in_view {
                if rightness > 0.0 {
                    out.push(Draw::Text {
                        text: Text::literal(">"),
                        x: half_width - w(">"),
                        y: -half_height,
                        color,
                        translate,
                    });
                } else if rightness < 0.0 {
                    out.push(Draw::Text {
                        text: Text::literal("<"),
                        x: -half_width,
                        y: -half_height,
                        color,
                        translate,
                    });
                }
            }
            out.push(Draw::Text {
                text,
                x: -text_width / 2,
                y: -half_height,
                color,
                translate,
            });
            row += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_and_brightness() {
        let mut s = Subtitle {
            key: "a".into(),
            range: 16.0,
            played: vec![PlayedAt {
                location: [0.0, 0.0, 3.0],
                time: 1000,
            }],
            looping: false,
        };
        assert_eq!(s.brightness(s.played[0], 1.0, 1000), 255);
        assert_eq!(s.brightness(s.played[0], 1.0, 2500), 165);
        assert_eq!(s.brightness(s.played[0], 2.0, 2500), 210);
        assert_eq!(s.brightness(s.played[0], 1.0, 9000), 75);
        // Equally recent plays: the farthest in range wins.
        s.refresh([0.0, 0.0, 5.0], 1000);
        s.refresh([0.0, 0.0, 40.0], 1000);
        let best = s.best_candidate([0.0; 3]).unwrap();
        assert_eq!(best.location, [0.0, 0.0, 5.0]);
        s.purge_old_instances(3000.0, 4001);
        assert!(s.played.is_empty());
        assert!(!s.audible_from([0.0; 3]));
    }

    #[test]
    fn listening_follows_the_option() {
        let mut o = SubtitleOverlay::default();
        o.on_play(Some("a"), [0.0; 3], 16.0, false, 0);
        assert_eq!(o.subtitles.len(), 1);
        o.on_play(None, [0.0; 3], 16.0, false, 0);
        o.on_play(Some("a"), [1.0; 3], 16.0, false, 5);
        assert_eq!(o.subtitles.len(), 1);
        assert_eq!(o.subtitles[0].played.len(), 2);
        assert!(!o.listening());
    }
}
