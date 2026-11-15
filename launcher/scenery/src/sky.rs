//! The night sky behind the launcher ("Night" in `docs/design-system.md`): banded sky, stars that
//! twinkle in steps, three layers of clouds drifting a texel at a time, a sun cut in slices and two
//! skylines, all drawn by hand below as small sprites.
//!
//! It costs almost nothing at rest: the sky's bands and the stars are plain quads, the sprites are
//! baked once per appearance and scale into tiles whose size doesn't depend on the window, and a
//! frame is asked for only when something on screen changes (a cloud moves one texel, a group of
//! stars steps to its next brightness): a few frames a second. The sky stands still under reduced
//! motion, and stops 10 s after the window loses focus ([`motion`]).
use crate::art::bake;
use crate::rain::Keep;
use gpui::{
    canvas, div, fill, img, linear_color_stop, linear_gradient, point, prelude::*, px, rgb, size,
    Bounds, Div, Hsla, Pixels, RenderImage, Window,
};
use horizon_theme::night::{Land, Mood, Night, SkyArt};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// Logical pixels per texel: the sky's bands, the stars and the clouds; the sun and skylines.
const TEXEL: f32 = 4.;
const LAND: f32 = 6.;
/// Each stop of the sky's share of the window's height, top to bottom.
const SHARES: [f32; 6] = [0.2, 0.18, 0.17, 0.15, 0.15, 0.15];
/// How long the sky keeps moving after the window loses focus.
const UNFOCUSED: Duration = Duration::from_secs(10);

/// Clouds: each layer's sprites, placed `(x, y)` in texels on a strip `period` texels wide that
/// repeats across the window, the strip's top as a share of the window's height, and its speed in
/// texels a second. `#` is the cloud, `=` its lit band, `-` its shaded underside.
struct CloudLayer {
    period: usize,
    top: f32,
    speed: f32,
    clouds: &'static [(usize, usize, &'static [&'static str])],
}

const PUFF: &[&str] = &[
    "..............############..................",
    "......##########################............",
    "..##########################################",
    "============================================",
    ".....----------------------------------.....",
];
const LONG: &[&str] = &[
    "......................##############............................",
    "..............##################################................",
    "....############################################################",
    "==============================================================..",
    "..........------------------------------------------............",
    "..............................#################.................",
    "..............................=================.................",
];
const WISP: &[&str] = &[
    "......##################......",
    "==============================",
];
const BANK: &[&str] = &[
    "............##########........................................",
    "......##########################........##############........",
    "..##################################..########################",
    "====================================..========================",
    "......-------------------------.........---------------.......",
];

const CLOUDS: [CloudLayer; 3] = [
    CloudLayer {
        period: 300,
        top: 0.36,
        speed: 0.5,
        clouds: &[(20, 0, WISP), (140, 3, PUFF), (230, 1, WISP)],
    },
    CloudLayer {
        period: 380,
        top: 0.52,
        speed: 1.,
        clouds: &[(30, 1, BANK), (200, 0, PUFF), (300, 4, WISP)],
    },
    CloudLayer {
        period: 460,
        top: 0.64,
        speed: 2.,
        clouds: &[(60, 0, LONG), (290, 2, PUFF)],
    },
];
const CLOUD_ROWS: usize = 10;

/// A skyline: runs of `(width, height)` in texels, repeating every `period` (their widths' sum),
/// with sprites stood on top of the run at `x`. `#` is solid.
struct Skyline {
    runs: &'static [(usize, usize)],
    props: &'static [(usize, &'static [&'static str])],
    rows: usize,
}

const TREE: &[&str] = &[".###.", "#####", "#####", ".###.", "..#..", "..#.."];
const PINE: &[&str] = &["..#..", ".###.", "..#..", ".###.", "#####", "..#.."];
/// A taller spruce, for the taiga's woods.
const SPRUCE: &[&str] = &[
    "..#..", "..#..", ".###.", "..#..", ".###.", "#####", ".###.", "#####", "..#..",
];
const POST: &[&str] = &[".#.", "###", ".#.", ".#.", ".#."];
const TOWER: &[&str] = &["#.#", "###", "###", "###", "###", "###", "###"];

const FAR: Skyline = Skyline {
    runs: &[
        (18, 8),
        (10, 10),
        (6, 13),
        (14, 11),
        (22, 9),
        (8, 12),
        (12, 14),
        (16, 11),
        (20, 8),
        (10, 10),
        (14, 12),
        (24, 9),
        (12, 11),
        (18, 8),
    ],
    props: &[(32, TOWER), (118, PINE), (176, TOWER)],
    rows: 20,
};

const NEAR: Skyline = Skyline {
    runs: &[
        (12, 7),
        (6, 9),
        (10, 8),
        (4, 11),
        (14, 10),
        (8, 8),
        (6, 6),
        (16, 7),
        (5, 10),
        (9, 12),
        (7, 9),
        (12, 7),
        (10, 8),
        (6, 11),
        (11, 9),
        (9, 7),
        (15, 7),
    ],
    props: &[
        (5, TREE),
        (36, POST),
        (60, PINE),
        (95, TREE),
        (124, POST),
        (139, TREE),
    ],
    rows: 18,
};

/// The sun: its radius in texels, how far it stands above the near skyline's foot, and where
/// across the window (a share of its width).
const SUN_RADIUS: usize = 15;
const SUN_RISE: usize = 8;
const SUN_X: f32 = 0.72;

/// A theme's [`Mood`] moves in twelve steps a second: still stepped like the pixel art it drifts
/// over, but smooth enough that falling leaves read as falling, not ticking.
const MOOD_STEPS: f32 = 12.;

/// How much white the light look lays over the dawn's colours.
const MORNING: f32 = 0.55;

/// The stars' brightness steps and each group's period: a group steps through the levels in
/// turn, so a star brightens and dims without a frame for every shade in between.
const STAR_LEVELS: [f32; 4] = [0.95, 0.6, 0.3, 0.6];
/// The clouds' speeds and these periods keep every change on a half-second beat, so the changes of
/// different layers share frames: two a second at most.
const STAR_PERIODS: [f32; 4] = [4., 6., 8., 10.];

/// How the sky moves: not at all under reduced motion (the player's choice in Settings, else the
/// system's: Windows' "Show animations", macOS's "Reduce motion") or with
/// `HORIZON_LAUNCHER_MOTION=off`; while the window is in front (and 10 s after); or always, with
/// `HORIZON_LAUNCHER_MOTION=always`, for recordings.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Motion {
    Still,
    InFront,
    Always,
}

/// The player's choice (`Settings::reduce_motion`): 0 follows the system, 1 still, 2 moves.
static PREFERENCE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Takes the player's choice: `None` follows the system, `Some(true)` holds everything still.
pub fn set_preference(reduce: Option<bool>) {
    let v = match reduce {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    PREFERENCE.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub fn motion() -> Motion {
    static FORCED: LazyLock<Option<Motion>> =
        LazyLock::new(
            || match std::env::var("HORIZON_LAUNCHER_MOTION").as_deref() {
                Ok("off" | "reduced") => Some(Motion::Still),
                Ok("always") => Some(Motion::Always),
                Ok("full") => Some(Motion::InFront),
                _ => None,
            },
        );
    if let Some(forced) = *FORCED {
        return forced;
    }
    let still = match PREFERENCE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => system_reduces_motion(),
    };
    if still {
        Motion::Still
    } else {
        Motion::InFront
    }
}

/// The system's reduced motion, asked again at most every 2 s, so turning it on or off applies
/// without restarting the launcher.
fn system_reduces_motion() -> bool {
    static ASKED: std::sync::Mutex<Option<(Instant, bool)>> = std::sync::Mutex::new(None);
    let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    match *asked {
        Some((at, reduce)) if now.duration_since(at) < Duration::from_secs(2) => reduce,
        _ => {
            let reduce = ask_system();
            *asked = Some((now, reduce));
            reduce
        }
    }
}

#[cfg(target_os = "windows")]
fn ask_system() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };
    let mut animate: i32 = 1;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through the pointer it is given.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animate as *mut i32).cast(),
            0,
        )
    };
    ok != 0 && animate == 0
}

#[cfg(target_os = "macos")]
fn ask_system() -> bool {
    objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// GNOME's "Animations" switch (`org.gnome.desktop.interface enable-animations`), read by
/// `gsettings` on a worker thread: the answer lands for the next ask, and a desktop without the
/// key or the tool answers "moves".
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn ask_system() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static REDUCE: AtomicBool = AtomicBool::new(false);
    static ASKING: AtomicBool = AtomicBool::new(false);
    if !ASKING.swap(true, Ordering::AcqRel) {
        std::thread::spawn(|| {
            let off = std::process::Command::new("gsettings")
                .args(["get", "org.gnome.desktop.interface", "enable-animations"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "false");
            REDUCE.store(off, Ordering::Relaxed);
            ASKING.store(false, Ordering::Release);
        });
    }
    REDUCE.load(Ordering::Relaxed)
}

/// The animation's own time: it runs only while the sky moves, so a window that comes back
/// picks up where it stopped instead of jumping.
#[derive(Default)]
pub struct Clock {
    banked: Duration,
    running: Option<Instant>,
}

impl Clock {
    pub fn at(&self, now: Instant) -> Duration {
        self.banked + self.running.map_or(Duration::ZERO, |since| now - since)
    }

    pub fn set(&mut self, run: bool, now: Instant) {
        match (run, self.running) {
            (true, None) => self.running = Some(now),
            (false, Some(since)) => {
                self.banked += now - since;
                self.running = None;
            }
            _ => {}
        }
    }
}

struct Layers {
    clouds: [Arc<RenderImage>; 3],
    far: Arc<RenderImage>,
    near: Arc<RenderImage>,
    sun: Arc<RenderImage>,
}

#[derive(Default)]
pub struct Sky {
    baked: Option<((SkyArt, u32), Layers)>,
    /// Images replaced by a new appearance or scale, still in the window's sprite atlas.
    replaced: Vec<Arc<RenderImage>>,
    clock: Clock,
    unfocused: Option<Instant>,
}

impl Sky {
    /// The sky for a window of `w` by `h` logical pixels, and how long until it next changes
    /// (`None`: it stands still until something else redraws the window).
    pub fn render(
        &mut self,
        n: &Night,
        (w, h): (f32, f32),
        scale: f32,
        keep: Keep,
        focused: bool,
        now: Instant,
    ) -> (Div, Option<Duration>) {
        if focused {
            self.unfocused = None;
        } else {
            self.unfocused.get_or_insert(now);
        }
        let run = match motion() {
            Motion::Still => false,
            Motion::InFront => self.unfocused.is_none_or(|since| now - since < UNFOCUSED),
            Motion::Always => true,
        };
        self.clock.set(run, now);
        let t = self.clock.at(now).as_secs_f32();
        let layers = self.layers(n.art, scale);
        let snap = |v: f32| (v * scale).round() / scale;
        let art = &n.art;

        let mut sky = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .overflow_hidden();
        for (top, height, c) in bands(n, h) {
            sky = sky.child(
                div()
                    .absolute()
                    .left_0()
                    .w_full()
                    .top(px(top))
                    .h(px(height))
                    .bg(rgb(c)),
            );
        }
        for (g, period) in STAR_PERIODS.iter().enumerate() {
            let level = STAR_LEVELS[step(t, *period, g)];
            let mut group = div().absolute().top_0().left_0().size_full().opacity(level);
            for star in stars().iter().filter(|s| s.group == g) {
                let (x, y) = (star.x * TEXEL, star.y * h * 0.5);
                if x >= w {
                    continue;
                }
                let size = if star.big { TEXEL / 2. } else { TEXEL / 4. };
                group = group.child(
                    div()
                        .absolute()
                        .left(px(snap(x)))
                        .top(px(snap(y)))
                        .size(px(size.max(1. / scale)))
                        .bg(rgb(art.star)),
                );
            }
            sky = sky.child(group);
        }
        let mood = art.mood;
        let moved = (t * MOOD_STEPS).floor() / MOOD_STEPS;
        if mood != Mood::Plain {
            sky = sky.child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| paint_behind(mood, moved, b, window),
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        }
        // The light look keeps a restrained dawn: its pale clouds read as stray lines across the
        // white panels' gutters, so only the dark skies draw them.
        let cloud_layers = if clouded(n) { &layers.clouds[..] } else { &[] };
        for (layer, image) in CLOUDS.iter().zip(cloud_layers) {
            let strip = layer.period as f32 * TEXEL;
            let shift = (t * layer.speed).floor() * TEXEL % strip;
            let top = snap(h * layer.top);
            let mut x = -shift;
            while x < w {
                sky = sky.child(
                    img(image.clone())
                        .absolute()
                        .left(px(x))
                        .top(px(top))
                        .w(px(strip))
                        .h(px(CLOUD_ROWS as f32 * TEXEL)),
                );
                x += strip;
            }
        }
        let foot = h;
        let sun_w = (SUN_RADIUS * 2) as f32 * LAND;
        sky = sky.child(
            img(layers.sun.clone())
                .absolute()
                .left(px(snap(w * SUN_X - sun_w / 2.)))
                .top(px(foot - (SUN_RISE + SUN_RADIUS) as f32 * LAND))
                .w(px(sun_w))
                .h(px(SUN_RADIUS as f32 * LAND)),
        );
        for (line, image, lift) in [(&FAR, &layers.far, 2.), (&NEAR, &layers.near, 0.)] {
            let period = line.runs.iter().map(|r| r.0).sum::<usize>() as f32 * LAND;
            let top = foot - (line.rows as f32 + lift) * LAND;
            // The far line starts half a period in, so the two never repeat in step.
            let mut x = if lift > 0. { -period / 2. } else { 0. };
            while x < w {
                sky = sky.child(
                    img(image.clone())
                        .absolute()
                        .left(px(x))
                        .top(px(top))
                        .w(px(period))
                        .h(px(line.rows as f32 * LAND)),
                );
                x += period;
            }
            if lift > 0. {
                sky = sky.child(
                    div()
                        .absolute()
                        .left_0()
                        .w_full()
                        .top(px(foot - lift * LAND))
                        .h(px(lift * LAND))
                        .bg(rgb(art.far)),
                );
            }
        }

        if mood != Mood::Plain {
            sky = sky.child(
                canvas(
                    |_, _, _| {},
                    move |b, _, window, _| paint_drift(mood, moved, keep, b, window),
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
        }

        let next = self.clock.running.is_some().then(|| {
            let clouds = CLOUDS.iter().map(|l| until_next(t * l.speed) / l.speed);
            let stars = STAR_PERIODS.iter().enumerate().map(|(g, p)| {
                let quarter = p / STAR_LEVELS.len() as f32;
                until_next(t / quarter + g as f32) * quarter
            });
            let mut soonest = clouds.chain(stars).fold(f32::MAX, f32::min);
            if mood != Mood::Plain {
                soonest = soonest.min(until_next(t * MOOD_STEPS) / MOOD_STEPS);
            }
            if let (Some(since), Motion::InFront) = (self.unfocused, motion()) {
                soonest = soonest.min((UNFOCUSED - (now - since)).as_secs_f32());
            }
            // A little after the change, so the frame's clock is surely past it.
            Duration::from_secs_f32(soonest.max(0.) + 0.005)
        });
        (sky, next)
    }

    fn layers(&mut self, art: SkyArt, scale: f32) -> &Layers {
        let key = (art, (scale * 100.).round() as u32);
        if !matches!(&self.baked, Some((k, _)) if *k == key) {
            let art = &art;
            let layers = Layers {
                clouds: std::array::from_fn(|i| {
                    let l = &CLOUDS[i];
                    bake(&clouds(l, art, i), l.period, CLOUD_ROWS, TEXEL, scale)
                }),
                far: skyline(&FAR, art, 0, scale),
                near: skyline(&NEAR, art, 1, scale),
                sun: bake(&sun(art), SUN_RADIUS * 2, SUN_RADIUS, LAND, scale),
            };
            if let Some((_, old)) = self.baked.replace((key, layers)) {
                self.replaced
                    .extend(old.clouds.into_iter().chain([old.far, old.near, old.sun]));
            }
        }
        &self.baked.as_ref().expect("baked above").1
    }

    /// Frees the images replaced since the last frame (GPUI keeps them in the sprite atlas until
    /// they are dropped there).
    pub fn free_replaced(&mut self, window: &mut gpui::Window) {
        for image in self.replaced.drain(..) {
            let _ = window.drop_image(image);
        }
    }
}

fn tone(c: u32) -> Hsla {
    rgb(c).into()
}

/// A rectangle of `w` by `h` at `(x, y)` from the layer's corner, its corner on a whole texel.
fn cell(b: Bounds<Pixels>, x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    let on = |v: f32| (v / TEXEL).round() * TEXEL;
    Bounds::new(
        point(b.origin.x + px(on(x)), b.origin.y + px(on(y))),
        size(px(w), px(h)),
    )
}

/// A strip fading from `c` at `from` to `c` at `to` opacity, top to bottom.
fn fade(c: u32, from: f32, to: f32) -> gpui::Background {
    linear_gradient(
        180.,
        linear_color_stop(tone(c).opacity(from), 0.),
        linear_color_stop(tone(c).opacity(to), 1.),
    )
}

/// What a [`Mood`] paints behind the clouds and skylines: the water's light shafts and the
/// aurora's curtains. `t` is the mood's own time in seconds, in whole steps.
fn paint_behind(mood: Mood, t: f32, b: Bounds<Pixels>, window: &mut Window) {
    use crate::rain::hash;
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    match mood {
        Mood::Water([light, ..]) => {
            // Shafts from the surface, swaying a few texels and fading with depth.
            for i in 0..6u32 {
                let k = i as f32;
                let x = (k + 0.5) / 6. * w + (t * 0.4 + k * 1.9).sin() * 6. * TEXEL;
                let width = (5. + 4. * hash(i * 7 + 1)) * TEXEL;
                let a = 0.10 + 0.05 * (t * 0.7 + k).sin();
                window.paint_quad(fill(cell(b, x, 0., width, h * 0.8), fade(light, a, 0.)));
            }
        }
        Mood::Neon([glow, _]) => {
            // A soft glow rising off the horizon.
            let top = h * 0.45;
            window.paint_quad(fill(
                Bounds::new(
                    point(b.origin.x, b.origin.y + px(top)),
                    size(b.size.width, px(h - top)),
                ),
                fade(glow, 0., 0.24),
            ));
        }
        Mood::Aurora(tones) => {
            // Three curtains of 16 px strips: each strip's height and place ride slow waves, and
            // its light fades in from the top and out toward the bottom.
            const COLUMN: f32 = 16.;
            for (k, &c) in tones.iter().enumerate() {
                let k = k as f32;
                let mut x = 0.;
                while x < w {
                    let top =
                        h * (0.08 + 0.07 * k) + (x * 0.006 + t * 0.25 + k * 2.1).sin() * h * 0.07;
                    let tall = h * (0.2 + 0.1 * (x * 0.011 - t * 0.18 + k).sin().abs());
                    let a = 0.2 + 0.1 * (x * 0.004 + t * 0.4 + k * 1.3).sin();
                    window.paint_quad(fill(cell(b, x, top, COLUMN, tall * 0.4), fade(c, 0., a)));
                    window.paint_quad(fill(
                        cell(b, x, top + tall * 0.4, COLUMN, tall * 0.6),
                        fade(c, a, 0.),
                    ));
                    x += COLUMN;
                }
            }
        }
        _ => {}
    }
}

/// What a [`Mood`] paints in front of the skylines: bubbles rising, leaves and petals falling,
/// each from its own place and phase, wrapping just off the window.
fn paint_drift(mood: Mood, t: f32, keep: Keep, b: Bounds<Pixels>, window: &mut Window) {
    use crate::rain::hash;
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    if let Mood::Neon([_, grid]) = mood {
        return paint_grid(grid, t, b, window);
    }
    let run = h + 40.;
    // Snow is many small flakes; the other drifts a few larger pieces.
    let count = if matches!(mood, Mood::Snow(_)) {
        72
    } else {
        28
    };
    for i in 0..count {
        let (place, pace, phase, sway) = (
            hash(i * 5 + 1),
            hash(i * 5 + 2),
            hash(i * 5 + 3),
            hash(i * 5 + 4),
        );
        let x = place * w;
        match mood {
            Mood::Leaves(tones) => {
                let y = (t * (45. + 45. * pace) + phase * run) % run - 20.;
                let x = x + (t * 1.6 + sway * std::f32::consts::TAU).sin() * 5. * TEXEL;
                if keep.hits(x, y, TEXEL * 3., TEXEL * 3.) {
                    continue;
                }
                let c = tone(tones[i as usize % 3]);
                // A diagonal leaf of two bars, turning over now and then.
                let (a, z) = if (t * 1.5 + sway * 4.).floor() as i32 % 2 == 0 {
                    (TEXEL, 0.)
                } else {
                    (0., TEXEL)
                };
                window.paint_quad(fill(cell(b, x + a, y, TEXEL * 2., TEXEL), c));
                window.paint_quad(fill(cell(b, x + z, y + TEXEL, TEXEL * 2., TEXEL), c));
            }
            Mood::Petals(tones) => {
                let y = (t * (32. + 32. * pace) + phase * run) % run - 20.;
                let x = x + (t * 1.2 + sway * std::f32::consts::TAU).sin() * 8. * TEXEL;
                if keep.hits(x, y, TEXEL * 3., TEXEL * 3.) {
                    continue;
                }
                let c = tone(tones[i as usize % 2]).opacity(0.9);
                window.paint_quad(fill(cell(b, x, y, TEXEL * 2., TEXEL), c));
                window.paint_quad(fill(
                    cell(b, x + TEXEL * (1. + sway.round()), y + TEXEL, TEXEL, TEXEL),
                    c,
                ));
            }
            Mood::Water([_, rim, shine]) => {
                let y = h + 20. - (t * (40. + 40. * pace) + phase * run) % run;
                let x = x + (t * 1.4 + sway * std::f32::consts::TAU).sin() * 2. * TEXEL;
                if keep.hits(x, y, TEXEL * 3., TEXEL * 3.) {
                    continue;
                }
                let big = i % 3 == 0;
                let side = if big { TEXEL * 2. } else { TEXEL };
                window.paint_quad(fill(cell(b, x, y, side, side), tone(rim).opacity(0.55)));
                if big {
                    window.paint_quad(fill(cell(b, x, y, TEXEL, TEXEL), tone(shine).opacity(0.8)));
                }
            }
            Mood::Snow([flake, shade]) => {
                // Flakes of one or two texels drifting down and swaying, each with a shade below
                // it, so white snow still reads on a pale sky.
                let y = (t * (26. + 26. * pace) + phase * run) % run - 20.;
                let x = x + (t * 0.9 + sway * std::f32::consts::TAU).sin() * 3. * TEXEL;
                if keep.hits(x, y, TEXEL * 3., TEXEL * 3.) {
                    continue;
                }
                let side = if i % 4 == 0 { TEXEL * 2. } else { TEXEL };
                window.paint_quad(fill(
                    cell(b, x, y + TEXEL, side, side),
                    tone(shade).opacity(0.45),
                ));
                window.paint_quad(fill(cell(b, x, y, side, side), tone(flake)));
            }
            Mood::Plain | Mood::Aurora(_) | Mood::Neon(_) => return,
        }
    }
}

/// Neon's floor over the near skyline's foot: lines across, closer together toward the horizon and
/// scrolling toward the viewer, and lines running to a point under the sun. About 80 quads, no
/// allocation.
fn paint_grid(grid: u32, t: f32, b: Bounds<Pixels>, window: &mut Window) {
    const DEEP: f32 = 5. * LAND;
    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
    let top = h - DEEP;
    let at = |x: f32, y: f32, w: f32, h: f32| {
        Bounds::new(
            point(b.origin.x + px(x), b.origin.y + px(y)),
            size(px(w), px(h)),
        )
    };
    let scroll = (t * 0.5).fract();
    for k in 0..5 {
        let f = (k as f32 + scroll) / 5.;
        let y = top + DEEP * f * f;
        window.paint_quad(fill(at(0., y, w, 1.), tone(grid).opacity(0.12 + 0.3 * f)));
    }
    let centre = w * SUN_X;
    for k in -16i32..=16 {
        let (low, high) = (k as f32 * 64., k as f32 * 22.);
        for s in 0..3 {
            let f = (s as f32 + 0.5) / 3.;
            let x = centre + high + (low - high) * f;
            if (0. ..w).contains(&x) {
                let y = top + DEEP * s as f32 / 3.;
                window.paint_quad(fill(at(x, y, 1., DEEP / 3.), tone(grid).opacity(0.28)));
            }
        }
    }
}

/// Which of a star group's levels shows at `t` seconds.
fn step(t: f32, period: f32, group: usize) -> usize {
    // Groups start a quarter period apart, which keeps their steps on the beat.
    let phase = group as f32 * 0.25;
    ((t / period + phase).fract() * STAR_LEVELS.len() as f32) as usize % STAR_LEVELS.len()
}

/// How far `v` is from the next whole number.
fn until_next(v: f32) -> f32 {
    v.floor() + 1. - v
}

/// The sky's bands, `(top, height, colour)` in logical pixels on whole texels: each stop of the
/// sky in four steps toward the next, the stops sharing the height as [`SHARES`] says.
fn bands(n: &Night, h: f32) -> Vec<(f32, f32, u32)> {
    const STEPS: usize = 4;
    // The light look's sky is the dawn's washed with white, so the page reads white with a tint
    // of the dawn rather than lilac.
    let wash = |c: u32| {
        // A theme's own land paints its sky as given.
        if n.dark || n.art.land != Land::Plain {
            c
        } else {
            horizon_theme::night::over(0xffffff, MORNING, c)
        }
    };
    let rows = (h / TEXEL).ceil();
    let mut out = Vec::with_capacity(SHARES.len() * STEPS);
    let mut at = 0.;
    for (i, share) in SHARES.iter().enumerate() {
        let span = (share * rows).round().max(STEPS as f32);
        let Some(&next) = n.sky.get(i + 1) else {
            out.push((
                at * TEXEL,
                (h / TEXEL - at).max(span) * TEXEL,
                wash(n.sky[i]),
            ));
            break;
        };
        let mut from = at;
        for k in 0..STEPS {
            let to = (at + span * (k + 1) as f32 / STEPS as f32).round();
            let c = horizon_theme::night::over(next, k as f32 / STEPS as f32, n.sky[i]);
            out.push((from * TEXEL, (to - from) * TEXEL, wash(c)));
            from = to;
        }
        at += span;
    }
    out
}

struct Star {
    /// Column in texels, row as a share of the upper half of the window.
    x: f32,
    y: f32,
    group: usize,
    big: bool,
}

/// The stars, the same on every start: a fixed field wide enough for any window, thinned toward
/// the horizon.
fn stars() -> &'static [Star] {
    static STARS: LazyLock<Vec<Star>> = LazyLock::new(|| {
        let mut rng = 0x2545_f491u32;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            (rng >> 8) as f32 / (1 << 24) as f32
        };
        (0..240)
            .filter_map(|i| {
                let (x, y) = ((next() * 960.).floor(), next());
                let keep = next() > y * y;
                let big = next() < 0.08;
                keep.then_some(Star {
                    x,
                    y,
                    group: i % STAR_PERIODS.len(),
                    big,
                })
            })
            .collect()
    });
    &STARS
}

/// `rows` of `#`-art at `(x, y)` in `out` (`w` texels wide), each mark coloured by `paint`.
fn stamp(
    out: &mut [[u8; 4]],
    w: usize,
    (x, y): (usize, usize),
    rows: &[&str],
    paint: impl Fn(u8) -> Option<u32>,
) {
    for (dy, row) in rows.iter().enumerate() {
        for (dx, mark) in row.bytes().enumerate() {
            let (px, py) = ((x + dx) % w, y + dy);
            if let (Some(c), Some(cell)) = (paint(mark), out.get_mut(py * w + px)) {
                *cell = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
            }
        }
    }
}

fn clouds(layer: &CloudLayer, art: &SkyArt, depth: usize) -> Vec<[u8; 4]> {
    let mut out = vec![[0; 4]; layer.period * CLOUD_ROWS];
    for &(x, y, sprite) in layer.clouds {
        stamp(&mut out, layer.period, (x, y), sprite, |m| match m {
            b'#' => Some(art.cloud[depth]),
            b'=' => Some(art.lit[depth]),
            b'-' => Some(art.shade),
            _ => None,
        });
    }
    out
}

/// Whether the sky draws its clouds: every dark sky, and a blocky land's day too.
fn clouded(n: &Night) -> bool {
    n.dark || n.art.land != Land::Plain
}

fn skyline(line: &Skyline, art: &SkyArt, which: usize, scale: f32) -> Arc<RenderImage> {
    let (out, w) = skyline_texels(line, art, which);
    bake(&out, w, line.rows, LAND, scale)
}

/// A skyline's texels (`which`: 0 the far one, 1 the near), cut as `art.land` says, and how many
/// it has across.
fn skyline_texels(line: &Skyline, art: &SkyArt, which: usize) -> (Vec<[u8; 4]>, usize) {
    let color = if which == 0 { art.far } else { art.near };
    let texel = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
    let w: usize = line.runs.iter().map(|r| r.0).sum();
    let mut out = vec![[0; 4]; w * line.rows];
    let mut x = 0;
    let mut tops = vec![0; w];
    for &(run, height) in line.runs {
        for top in &mut tops[x..x + run] {
            *top = line.rows - height;
        }
        x += run;
    }
    for (x, &top) in tops.iter().enumerate() {
        for y in top..line.rows {
            out[y * w + x] = texel(color);
        }
        // Grass on the land, under the trees stood on it.
        if let Land::Blocks { grass, .. } = art.land {
            out[top * w + x] = texel(grass[which]);
        }
    }
    let prop = match art.land {
        Land::Blocks { leaves, .. } => leaves,
        _ => color,
    };
    // The taiga stands woods of spruce on its land instead of the skyline's props.
    let woods: Vec<(usize, &[&str])>;
    let props = if let Land::Taiga { .. } = art.land {
        let gap = if which == 0 { 4 } else { 6 };
        woods = (0..w.saturating_sub(5))
            .step_by(gap)
            .map(|x| (x, SPRUCE))
            .collect();
        &woods[..]
    } else {
        line.props
    };
    for &(x, sprite) in props {
        let width = sprite[0].len();
        let ground = tops[x..x + width]
            .iter()
            .copied()
            .min()
            .unwrap_or(line.rows);
        let y = ground.saturating_sub(sprite.len());
        stamp(&mut out, w, (x, y), sprite, |m| (m == b'#').then_some(prop));
    }
    // A neon line, or snow, along the whole top, towers and trees included.
    if let Land::Neon(edge) | Land::Taiga { snow: edge } = art.land {
        for x in 0..w {
            if let Some(y) = (0..line.rows).find(|y| out[y * w + x][3] > 0) {
                out[y * w + x] = texel(edge[which]);
            }
        }
    }
    (out, w)
}

/// The sky's still parts as [`Sky::render`] lays them out in a window `w` × `h` (its bands, the
/// clouds where they start, the sun and the skylines; not the stars or a mood's drift), a colour
/// (0 to 1) per [`TEXEL`]: what a painted theme's plates blur. Returns the colours, columns and
/// rows.
pub fn still_texels(n: &Night, w: f32, h: f32) -> (Vec<[f64; 3]>, usize, usize) {
    let cols = (w / TEXEL).ceil().max(1.) as usize;
    let rows = (h / TEXEL).ceil().max(1.) as usize;
    let rgbf = |c: u32| [16, 8, 0].map(|s| ((c >> s) & 255) as f64 / 255.);
    let mut out = vec![[0.; 3]; cols * rows];
    let row_of = |y: f32| ((y / TEXEL).round().max(0.) as usize).min(rows);
    for (top, height, c) in bands(n, h) {
        for row in row_of(top)..row_of(top + height) {
            out[row * cols..(row + 1) * cols].fill(rgbf(c));
        }
    }
    // `texels` (`sw` across, `unit` px each) with its corner at `(left, top)`, sampled at each
    // texel's centre; `repeat` tiles it across the window.
    let sprite = |out: &mut [[f64; 3]],
                  texels: &[[u8; 4]],
                  sw: usize,
                  unit: f32,
                  (left, top): (f32, f32),
                  repeat: bool| {
        let span = sw as f32 * unit;
        let sh = texels.len() / sw;
        for row in 0..rows {
            let y = (row as f32 + 0.5) * TEXEL - top;
            if y < 0. || y >= sh as f32 * unit {
                continue;
            }
            let sy = ((y / unit) as usize).min(sh - 1);
            for col in 0..cols {
                let mut x = (col as f32 + 0.5) * TEXEL - left;
                if repeat {
                    x = x.rem_euclid(span);
                } else if x < 0. || x >= span {
                    continue;
                }
                let [r, g, b, a] = texels[sy * sw + ((x / unit) as usize).min(sw - 1)];
                if a > 0 {
                    out[row * cols + col] = [r, g, b].map(|v| f64::from(v) / 255.);
                }
            }
        }
    };
    let art = &n.art;
    if clouded(n) {
        for (i, layer) in CLOUDS.iter().enumerate() {
            let texels = clouds(layer, art, i);
            sprite(
                &mut out,
                &texels,
                layer.period,
                TEXEL,
                (0., h * layer.top),
                true,
            );
        }
    }
    let sun_w = (SUN_RADIUS * 2) as f32 * LAND;
    let sun_top = h - (SUN_RISE + SUN_RADIUS) as f32 * LAND;
    sprite(
        &mut out,
        &sun(art),
        SUN_RADIUS * 2,
        LAND,
        (w * SUN_X - sun_w / 2., sun_top),
        false,
    );
    for (which, line, lift) in [(0, &FAR, 2.), (1, &NEAR, 0.)] {
        let (texels, sw) = skyline_texels(line, art, which);
        let period = sw as f32 * LAND;
        let top = h - (line.rows as f32 + lift) * LAND;
        let left = if lift > 0. { -period / 2. } else { 0. };
        sprite(&mut out, &texels, sw, LAND, (left, top), true);
        if lift > 0. {
            for row in row_of(h - lift * LAND)..rows {
                out[row * cols..(row + 1) * cols].fill(rgbf(art.far));
            }
        }
    }
    (out, cols, rows)
}

/// The upper half of a sun cut by the horizon, in three bands of colour, its lower half sliced by
/// rows of sky like Horizon's mark.
fn sun(art: &SkyArt) -> Vec<[u8; 4]> {
    let (r, w) = (SUN_RADIUS as f32, SUN_RADIUS * 2);
    let mut out = vec![[0; 4]; w * SUN_RADIUS];
    // A blocky land's sun or moon is a square, rim to middle in its three colours.
    if let Land::Blocks { .. } = art.land {
        const HALF: usize = 7;
        for y in SUN_RADIUS - 2 * HALF..SUN_RADIUS {
            for x in SUN_RADIUS - HALF..SUN_RADIUS + HALF {
                let ring = [
                    x + HALF - SUN_RADIUS,
                    SUN_RADIUS + HALF - 1 - x,
                    y + 2 * HALF - SUN_RADIUS,
                    SUN_RADIUS - 1 - y,
                ]
                .into_iter()
                .min()
                .unwrap_or(0);
                let c = art.sun[ring.min(2)];
                out[y * w + x] = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
            }
        }
        return out;
    }
    for y in 0..SUN_RADIUS {
        let dy = r - y as f32 - 0.5;
        let half = (r * r - dy * dy).sqrt().round() as usize;
        let depth = y as f32 / r;
        let sliced = depth > 0.5 && (SUN_RADIUS - y).is_multiple_of(3);
        if sliced {
            continue;
        }
        let c = art.sun[if depth < 0.4 {
            0
        } else if depth < 0.7 {
            1
        } else {
            2
        }];
        for x in SUN_RADIUS - half..SUN_RADIUS + half {
            out[y * w + x] = [(c >> 16) as u8, (c >> 8) as u8, c as u8, 255];
        }
    }
    out
}

/// A painted theme's plates frost its sky: its blurred copy, made once per size and blur.
/// Called by the window before its plates are built, as the sky itself is drawn in its own view.
pub fn frost(n: &Night, (w, h): (f32, f32)) {
    crate::glass::frost_sky(n, (w, h), || {
        let (texels, cols, rows) = still_texels(n, w, h);
        (texels, cols, rows, TEXEL)
    });
}

/// [`frost`] with a later blur made on a worker (`glass::frost_sky_later`): the job to run, if
/// one is due, whose result goes to `glass::frost_done` on the main thread.
pub fn frost_later(
    n: &Night,
    (w, h): (f32, f32),
) -> Option<impl FnOnce() -> crate::glass::Frosted + Send + 'static> {
    let n = *n;
    crate::glass::frost_sky_later(&n, (w, h), move || {
        let (texels, cols, rows) = still_texels(&n, w, h);
        (texels, cols, rows, TEXEL)
    })
}

/// The painted sky as a view of its own: its drift wakes only this view, so a sky frame draws
/// the sky again and the rest of the window from its last frame (GPUI Fast retains views).
pub struct SkyView {
    sky: Sky,
    shown: Option<(Night, (f32, f32), f32, Keep)>,
    wake: Option<(Instant, gpui::Task<()>)>,
    _activation: gpui::Subscription,
}

impl SkyView {
    pub fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        Self {
            sky: Sky::default(),
            shown: None,
            wake: None,
            // Back in front, a sky held while the window was behind others drifts again.
            _activation: cx.observe_window_activation(window, |_, _, cx| cx.notify()),
        }
    }

    /// The sky to draw for `n` in a window of `size` logical pixels at `scale`.
    pub fn set(
        &mut self,
        n: Night,
        size: (f32, f32),
        scale: f32,
        keep: Keep,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some((n, size, scale, keep));
        if self.shown != next {
            self.shown = next;
            cx.notify();
        }
    }

    /// Draws the sky again in `after`, when it next changes, unless a frame comes sooner.
    fn wake(&mut self, after: Duration, cx: &mut gpui::Context<Self>) {
        let at = Instant::now() + after;
        if self.wake.as_ref().is_some_and(|(when, _)| *when <= at) {
            return;
        }
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                this.wake = None;
                cx.notify();
            });
        });
        self.wake = Some((at, timer));
    }
}

impl gpui::Render for SkyView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        self.sky.free_replaced(window);
        let Some((n, size, scale, keep)) = self.shown else {
            return div();
        };
        let (sky, next) = self.sky.render(
            &n,
            size,
            scale,
            keep,
            window.is_window_active(),
            Instant::now(),
        );
        if let Some(next) = next {
            self.wake(next, cx);
        }
        sky
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_theme::night::{DAWN, NIGHT, UNCARDED_STOPS};

    /// The sky's upper bands, behind text without a plate, hold the night tokens' text colours.
    fn uncarded_bands(n: &Night, h: f32) -> Vec<u32> {
        let limit: f32 = SHARES[..UNCARDED_STOPS].iter().sum::<f32>() * h;
        bands(n, h)
            .into_iter()
            .filter(|(top, _, _)| *top < limit)
            .map(|(_, _, c)| c)
            .collect()
    }

    #[test]
    fn the_blurred_copy_holds_the_sky_sun_and_skyline() {
        let (w, h) = (1000., 600.);
        let (texels, cols, rows) = still_texels(&NIGHT, w, h);
        assert_eq!((cols, rows), (250, 150));
        let at = |x: f32, y: f32| texels[(y / TEXEL) as usize * cols + (x / TEXEL) as usize];
        let rgbf = |c: u32| [16, 8, 0].map(|s| ((c >> s) & 255) as f64 / 255.);
        // The top band, the near skyline at the bottom left, the sun's rim above the skyline.
        assert_eq!(at(0., 0.), rgbf(NIGHT.sky[0]));
        assert_eq!(at(2., h - 2.), rgbf(NIGHT.art.near));
        let sun_top = h - (SUN_RISE + SUN_RADIUS) as f32 * LAND;
        assert_eq!(at(w * SUN_X, sun_top + 2.), rgbf(NIGHT.art.sun[0]));
    }

    #[test]
    fn the_bands_cover_the_window_on_whole_texels() {
        for h in [600., 777., 1080.] {
            let bands = bands(&NIGHT, h);
            let mut at = 0.;
            for (top, height, _) in &bands {
                assert_eq!(*top, at);
                assert_eq!(top % TEXEL, 0.);
                at += height;
            }
            assert!(at >= h, "{at} < {h}");
        }
    }

    #[test]
    fn text_without_a_plate_reads_on_the_upper_bands() {
        for n in [NIGHT, DAWN] {
            for c in uncarded_bands(&n, 800.) {
                for text in [n.ink, n.ink_soft, n.muted] {
                    let ratio = horizon_theme::contrast(text, c);
                    assert!(ratio >= 4.5, "#{text:06x} on #{c:06x}: {ratio:.2}");
                }
            }
        }
    }

    #[test]
    fn the_art_fits_its_tiles() {
        for layer in &CLOUDS {
            for (x, y, sprite) in layer.clouds {
                assert!(y + sprite.len() <= CLOUD_ROWS);
                assert!(
                    x + sprite[0].len() <= layer.period,
                    "a cloud crosses its strip's end"
                );
                assert!(sprite.iter().all(|r| r.len() == sprite[0].len()));
            }
        }
        for line in [&FAR, &NEAR] {
            let w: usize = line.runs.iter().map(|r| r.0).sum();
            assert!(line.runs.iter().all(|r| r.1 < line.rows));
            // The tile's ends meet: the line repeats without a step.
            assert_eq!(line.runs[0].1, line.runs[line.runs.len() - 1].1);
            for (x, sprite) in line.props {
                assert!(x + sprite[0].len() <= w);
            }
        }
    }

    #[test]
    fn every_change_falls_on_the_beat() {
        const BEAT: f32 = 0.5;
        let on_beat = |v: f32| ((v / BEAT).round() * BEAT - v).abs() < 1e-4;
        for layer in &CLOUDS {
            assert!(
                on_beat(1. / layer.speed),
                "a cloud layer steps every {} s",
                1. / layer.speed
            );
        }
        for (g, p) in STAR_PERIODS.iter().enumerate() {
            let quarter = p / STAR_LEVELS.len() as f32;
            assert!(
                on_beat(quarter) && on_beat(g as f32 * 0.25 * p),
                "star group {g}"
            );
        }
    }

    #[test]
    fn the_clock_stops_and_resumes_without_a_jump() {
        let start = Instant::now();
        let mut clock = Clock::default();
        clock.set(true, start);
        let s = Duration::from_secs(1);
        clock.set(false, start + 2 * s);
        assert_eq!(clock.at(start + 10 * s), 2 * s);
        clock.set(true, start + 10 * s);
        assert_eq!(clock.at(start + 11 * s), 3 * s);
    }

    #[test]
    fn frames_come_only_when_something_moves() {
        assert!((until_next(0.) - 1.).abs() < 1e-6);
        let mut sky = Sky::default();
        let now = Instant::now();
        let (_, next) = sky.render(&NIGHT, (1280., 800.), 2., Keep::default(), true, now);
        if motion() != Motion::Still {
            let next = next.expect("a moving sky asks for its next frame");
            assert!(next <= Duration::from_secs(1), "{next:?}");
        }
        // Unfocused for longer than UNFOCUSED: no more frames.
        if motion() != Motion::InFront {
            return;
        }
        sky.render(&NIGHT, (1280., 800.), 2., Keep::default(), false, now);
        let (_, next) = sky.render(
            &NIGHT,
            (1280., 800.),
            2.,
            Keep::default(),
            false,
            now + UNFOCUSED,
        );
        assert_eq!(next, None);
    }
}
