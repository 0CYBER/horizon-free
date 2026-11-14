//! Hypr Neon's night sky: thin neon rain over its black, in three depths (far drops slow, faint
//! and short; near ones fast, bright and long), with a small splash where a near drop lands.
//!
//! The rain is its own view, drawn again on every display frame (vsync) while it falls: a frame
//! of it paints a few hundred quads and doesn't build the launcher's pages again. Each drop's
//! place comes from the time, not a step per frame, so a late frame never makes it jump. It
//! stands still under reduced motion, in performance mode and while the window is in the
//! background (the pointer coming back starts it again, [`Rain::poke`]).
use crate::sky::{motion, Motion};
use gpui::{
    canvas, div, fill, linear_color_stop, linear_gradient, point, prelude::*, px, size, Bounds,
    Context, Hsla, Pixels, Window,
};
use std::time::Instant;

const DROPS: u32 = 130;
/// The rain's other two colours besides the accent: Hypr's blue and pink.
const BLUE: u32 = 0x7aa2ff;
const PINK: u32 = 0xff6ad5;

pub struct Rain {
    start: Instant,
    /// Hypr Neon's night is shown and nothing asks the rain to stand still.
    on: bool,
    accent: u32,
    /// A frame is asked for: the rain is falling.
    falling: bool,
}

impl Default for Rain {
    fn default() -> Self {
        Self::new()
    }
}

impl Rain {
    pub fn new() -> Rain {
        Rain {
            start: Instant::now(),
            on: false,
            accent: 0xb48cff,
            falling: false,
        }
    }

    /// Whether it rains (`on`), in the theme's `accent`. Redrawn only when either changes.
    pub fn set(&mut self, on: bool, accent: u32, cx: &mut Context<Self>) {
        if self.on != on || self.accent != accent {
            self.on = on;
            self.accent = accent;
            cx.notify();
        }
    }

    /// The pointer moved: rain that stopped with the window in the background starts again.
    pub fn poke(&mut self, cx: &mut Context<Self>) {
        if self.on && !self.falling {
            cx.notify();
        }
    }
}

/// A steady pseudo-random number in 0..1 for `i` (each drop keeps its column and phase).
pub fn hash(i: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x & 0xff_ffff) as f32 / 0x100_0000 as f32
}

fn color(c: u32) -> Hsla {
    gpui::rgb(c).into()
}

impl Render for Rain {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if !self.on {
            return div();
        }
        let still = motion() == Motion::Still;
        self.falling = !still && window.is_window_active();
        if self.falling {
            window.request_animation_frame();
        }
        // Still: the drops are drawn where they are at a fixed moment, a picture of rain.
        let t = if still {
            7.
        } else {
            self.start.elapsed().as_secs_f32()
        };
        let accent = self.accent;
        div().absolute().top_0().left_0().size_full().child(
            canvas(
                |_, _, _| {},
                move |b: Bounds<Pixels>, _, window, _| {
                    let w = f32::from(b.size.width);
                    let h = f32::from(b.size.height);
                    for i in 0..DROPS {
                        // Depth 0 is far, 2 near.
                        let depth = i % 3;
                        let (speed, len, alpha, thick) = match depth {
                            0 => (300., 12., 0.16, 1.),
                            1 => (520., 22., 0.28, 1.),
                            _ => (820., 36., 0.5, 1.5),
                        };
                        let c = match i % 9 {
                            4 => BLUE,
                            7 => PINK,
                            _ => accent,
                        };
                        let x = hash(i * 3 + 1) * w;
                        let travel = h + len;
                        // Each drop falls at its own speed (±20 %), from its own phase.
                        let speed = speed * (0.8 + 0.4 * hash(i * 3 + 2));
                        let q = (t * speed / travel + hash(i * 3 + 3)).fract();
                        let y = q * travel - len;
                        let streak = Bounds::new(
                            point(b.origin.x + px(x), b.origin.y + px(y)),
                            size(px(thick), px(len)),
                        );
                        window.paint_quad(fill(
                            streak,
                            linear_gradient(
                                180.,
                                linear_color_stop(color(c).opacity(0.), 0.),
                                linear_color_stop(color(c).opacity(alpha), 1.),
                            ),
                        ));
                        // A near drop's splash: a short flat flash at the bottom as it lands.
                        let landed = (y + len - h) / len;
                        if depth == 2 && (0. ..1.).contains(&landed) {
                            let wide = 4. + 10. * landed;
                            let splash = Bounds::new(
                                point(b.origin.x + px(x - wide / 2.), b.origin.y + px(h - 3.)),
                                size(px(wide), px(2.)),
                            );
                            window.paint_quad(
                                fill(splash, color(c).opacity(0.45 * (1. - landed)))
                                    .corner_radii(px(1.)),
                            );
                        }
                    }
                },
            )
            .size_full(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::hash;

    #[test]
    fn drops_spread_over_the_whole_width() {
        let xs: Vec<f32> = (0..super::DROPS).map(|i| hash(i * 3 + 1)).collect();
        assert!(xs.iter().all(|x| (0. ..1.).contains(x)));
        // Every tenth of the width gets some rain.
        for band in 0..10 {
            let lo = band as f32 / 10.;
            assert!(xs.iter().any(|&x| x >= lo && x < lo + 0.1), "band {band}");
        }
    }
}
