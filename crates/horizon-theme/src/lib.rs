//! Horizon's design tokens, the one source of truth for the launcher (`launcher/src/theme.rs`),
//! the game's own menus (`horizon-client`'s `ui_theme`) and the website (`website/src/tokens.css`,
//! generated from [`css`] by `cargo run -p xtask -- tokens`). Change a value here and all three
//! follow. The system is described in `docs/design-system.md`.
//!
//! Cool neutrals in two appearances and one violet accent; Geist and Geist Mono only, bundled
//! here under the SIL Open Font License (`fonts/OFL-Geist.txt`). Colours are `0xRRGGBB`; sizes
//! are in logical pixels.
use std::fmt::Write as _;
use std::time::Duration;

pub const FONT: &str = "Geist";
pub const MONO: &str = "Geist Mono";

/// The bundled font files.
pub mod fonts {
    pub const GEIST_REGULAR: &[u8] = include_bytes!("../fonts/Geist-Regular.ttf");
    pub const GEIST_MEDIUM: &[u8] = include_bytes!("../fonts/Geist-Medium.ttf");
    pub const GEIST_SEMIBOLD: &[u8] = include_bytes!("../fonts/Geist-SemiBold.ttf");
    pub const GEIST_MONO: &[u8] = include_bytes!("../fonts/GeistMono-Regular.ttf");
}

/// Horizon's icon at 64 px (PNG), for window title bars and task bars at run time. Rendered from
/// `docs/media/brand/icon.svg` by `cargo run -p xtask -- icons`.
pub const ICON_PNG: &[u8] = include_bytes!("../icons/horizon-64.png");

/// How a type step sizes on the web.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Web {
    /// The app size, fixed.
    Fixed,
    /// From `min` px on a 375 px phone to `max` px at 1440 px and wider.
    Fluid(f32, f32),
    /// A share of the viewport width, in `vw`.
    Viewport(f32),
}

/// One step of the type scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub name: &'static str,
    /// The size in the apps (launcher and game menus).
    pub px: f32,
    pub web: Web,
    /// Line height as a multiple of the size.
    pub line: f32,
    /// Letter spacing in em.
    pub tracking: f32,
    pub weight: u16,
    pub mono: bool,
}

impl Step {
    /// The line height in pixels at the app size.
    pub fn line_px(&self) -> f32 {
        (self.px * self.line).round()
    }
}

const fn step(
    name: &'static str,
    px: f32,
    web: Web,
    line: f32,
    tracking: f32,
    weight: u16,
    mono: bool,
) -> Step {
    Step {
        name,
        px,
        web,
        line,
        tracking,
        weight,
        mono,
    }
}

/// Indices (`01`), counts and timestamps, in mono.
pub const MICRO: Step = step("micro", 11., Web::Fixed, 1.5, 0.04, 400, true);
/// Footnotes and help under a control.
pub const CAPTION: Step = step("caption", 12., Web::Fixed, 1.5, 0., 400, false);
/// Row labels, navigation and buttons.
pub const LABEL: Step = step("label", 13., Web::Fixed, 1.5, 0., 400, false);
/// Body text in the apps.
pub const UI: Step = step("ui", 14., Web::Fixed, 1.5, 0., 400, false);
/// Paragraphs.
pub const BODY: Step = step("body", 15., Web::Fluid(15., 17.), 1.6, 0., 400, false);
/// Section heads.
pub const HEADING: Step = step("heading", 20., Web::Fluid(20., 24.), 1.3, -0.01, 500, false);
/// The names in an index list.
pub const TITLE: Step = step("title", 28., Web::Fluid(32., 48.), 1.05, -0.02, 500, false);
/// Page titles.
pub const DISPLAY: Step = step(
    "display",
    44.,
    Web::Fluid(34., 64.),
    1.05,
    -0.025,
    500,
    false,
);
/// The giant faint signature.
pub const WORDMARK: Step = step("wordmark", 220., Web::Viewport(18.), 0.8, -0.04, 500, false);

pub const TYPE: [Step; 9] = [
    MICRO, CAPTION, LABEL, UI, BODY, HEADING, TITLE, DISPLAY, WORDMARK,
];

/// The spacing scale.
pub const SPACE: [f32; 9] = [4., 8., 12., 16., 24., 32., 48., 64., 96.];
/// Page margins: wide windows and pages, and phones.
pub const GUTTER: f32 = 64.;
pub const GUTTER_NARROW: f32 = 16.;
/// The longest paragraph line, in characters.
pub const MEASURE: f32 = 56.;

/// Corner radii: fields and small controls; plates (hover rows, console, dialogs); pills
/// (buttons, segmented controls, switches).
pub const RADIUS: f32 = 8.;
pub const RADIUS_PLATE: f32 = 12.;
pub const RADIUS_PILL: f32 = 999.;
/// Rules between rows are one pixel; there are no other borders.
pub const RULE: f32 = 1.;

/// Heights: buttons, the one large action, a segment, a field, an index row in the apps and
/// on the web.
pub const CONTROL: f32 = 32.;
pub const CONTROL_LARGE: f32 = 40.;
pub const SEGMENT: f32 = 28.;
pub const FIELD: f32 = 36.;
pub const ROW: f32 = 64.;
pub const ROW_WEB: f32 = 86.;
/// The index column of a list (where `01` sits) and how far the hover plate reaches past a row.
pub const INDEX: f32 = 40.;
pub const PLATE_BLEED: f32 = 16.;
/// A pressed primary button's opacity, and disabled controls'.
pub const PRESSED: f32 = 0.85;
pub const DISABLED: f32 = 0.4;

/// A duration and its easing curve (CSS `cubic-bezier` control points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub name: &'static str,
    pub ms: u64,
    pub curve: [f32; 4],
}

impl Motion {
    pub const fn duration(&self) -> Duration {
        Duration::from_millis(self.ms)
    }
    pub fn secs(&self) -> f32 {
        self.ms as f32 / 1000.
    }
    /// The eased progress for a linear `t` in 0..1.
    pub fn ease(&self, t: f32) -> f32 {
        bezier(self.curve, t)
    }
}

/// Standard ease, for hovers, presses and colour.
pub const QUICK: Motion = Motion {
    name: "quick",
    ms: 150,
    curve: [0.4, 0., 0.2, 1.],
};
/// Ease-out with a long tail, for selection, switches and page changes.
pub const MOVE: Motion = Motion {
    name: "move",
    ms: 240,
    curve: [0.23, 1., 0.32, 1.],
};
/// Content arriving: a fade and a [`RISE`].
pub const ENTER: Motion = Motion {
    name: "enter",
    ms: 600,
    curve: [0.23, 1., 0.32, 1.],
};
pub const MOTION: [Motion; 3] = [QUICK, MOVE, ENTER];
/// The delay between list rows arriving, and how far content rises as it arrives.
pub const STAGGER: Duration = Duration::from_millis(40);
pub const RISE: f32 = 8.;

/// The launcher's opening: the wordmark holds, then the interface arrives.
pub const OPENING_HOLD: Duration = Duration::from_millis(1400);
pub const OPENING_MAX: Duration = Duration::from_millis(2500);
pub const OPENING_REVEAL: Duration = Duration::from_millis(800);

/// A CSS `cubic-bezier(x1, y1, x2, y2)` at `t`: solves x for the curve parameter, returns y.
pub fn bezier([x1, y1, x2, y2]: [f32; 4], t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if t == 0. || t == 1. {
        return t;
    }
    let at = |a: f32, b: f32, s: f32| {
        let u = 1. - s;
        3. * u * u * s * a + 3. * u * s * s * b + s * s * s
    };
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if at(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(y1, y2, (lo + hi) / 2.)
}

pub fn ease_out_quint(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(5)
}
pub fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The background.
    pub page: u32,
    /// The hover plate, fields, the segmented track, the console.
    pub plate_soft: u32,
    /// Pressed plates, a switch that is off, the faint wordmark.
    pub plate: u32,
    /// Dialogs and menus above the page.
    pub surface: u32,
    /// One-pixel rules between rows, slider and progress tracks.
    pub rule: u32,
    /// Text, selection, the slider's fill.
    pub ink: u32,
    /// Secondary text and values.
    pub ink_soft: u32,
    /// Labels, descriptions, indices.
    pub muted: u32,
    /// The main action, links, focus, progress.
    pub accent: u32,
    pub on_accent: u32,
    pub danger: u32,
    pub success: u32,
    pub dark: bool,
}

pub const LIGHT: Palette = Palette {
    page: 0xfafafc,
    plate_soft: 0xf3f3f8,
    plate: 0xebecf4,
    surface: 0xffffff,
    rule: 0xe2e3ea,
    ink: 0x16181f,
    ink_soft: 0x3b3e49,
    muted: 0x646875,
    accent: 0x5b3fd6,
    on_accent: 0xffffff,
    danger: 0xb4372a,
    success: 0x1d7a48,
    dark: false,
};

pub const DARK: Palette = Palette {
    page: 0x0d0e13,
    plate_soft: 0x181a22,
    plate: 0x20222c,
    surface: 0x16181f,
    rule: 0x24262f,
    ink: 0xecedf3,
    ink_soft: 0xc5c7d1,
    muted: 0x8b8f9c,
    accent: 0x9a86ff,
    on_accent: 0x0d0e13,
    danger: 0xf07a68,
    success: 0x45c27a,
    dark: true,
};

/// How strongly the game's menus veil the live backdrop (the title panorama, or the world behind
/// the pause and options screens) with the page colour. Every text colour the menus use keeps
/// WCAG AA over it, even on a selected row's plate, with a backdrop as bright as
/// [`BRIGHT_BACKDROP`] (a test checks it).
pub const VEIL: f32 = 0.90;
/// The title's home page shows more of the world; its text is `ink` and `ink_soft` only.
pub const HOME_VEIL: f32 = 0.80;
/// Muted text over the veiled backdrop: the dark palette's `muted`, lifted so it holds AA there.
pub const BACKDROP_MUTED: u32 = 0x9fa3ae;
/// The brightest backdrop the veils are measured against: a light grey of luminance 0.77,
/// brighter than the snowiest title panorama measured (0.49) and a sunlit world.
pub const BRIGHT_BACKDROP: u32 = 0xe3e3e3;

impl Palette {
    /// Every colour with its token name, in the order the docs list them.
    pub fn named(&self) -> [(&'static str, u32); 12] {
        [
            ("page", self.page),
            ("plate-soft", self.plate_soft),
            ("plate", self.plate),
            ("surface", self.surface),
            ("rule", self.rule),
            ("ink", self.ink),
            ("ink-soft", self.ink_soft),
            ("muted", self.muted),
            ("accent", self.accent),
            ("on-accent", self.on_accent),
            ("danger", self.danger),
            ("success", self.success),
        ]
    }
}

/// The WCAG 2 contrast ratio of two colours.
pub fn contrast(a: u32, b: u32) -> f32 {
    fn luminance(c: u32) -> f32 {
        let ch = |s: u32| {
            let v = ((c >> s) & 255) as f32 / 255.;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
    }
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `website/src/tokens.css`: every token as a CSS custom property, the light palette by
/// default, the dark one when the system asks for it or the page sets `data-theme="dark"`.
pub fn css() -> String {
    let mut s = String::new();
    let num = |v: f32| {
        let t = format!("{v:.3}");
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let colors = |s: &mut String, p: &Palette| {
        for (name, c) in p.named() {
            let _ = writeln!(s, "  --{name}: #{c:06x};");
        }
        let _ = writeln!(
            s,
            "  color-scheme: {};",
            if p.dark { "dark" } else { "light" }
        );
    };
    s.push_str(
        "/* Generated from crates/horizon-theme by `cargo run -p xtask -- tokens`; do not edit.\n   \
         docs/design-system.md describes the tokens. */\n\n:root {\n",
    );
    colors(&mut s, &LIGHT);
    let _ = writeln!(
        s,
        "  --font: \"{FONT}\", ui-sans-serif, system-ui, sans-serif;\n  --mono: \"{MONO}\", ui-monospace, monospace;"
    );
    for step in TYPE {
        let size = match step.web {
            Web::Fixed => format!("{}px", num(step.px)),
            Web::Fluid(min, max) => format!(
                "clamp({}px, calc({}px + {} * (100vw - 375px) / 1065), {}px)",
                num(min),
                num(min),
                num(max - min),
                num(max)
            ),
            Web::Viewport(vw) => format!("{}vw", num(vw)),
        };
        let n = step.name;
        let _ = writeln!(s, "  --text-{n}: {size};");
        let _ = writeln!(s, "  --text-{n}-line: {};", num(step.line));
        let _ = writeln!(s, "  --text-{n}-tracking: {}em;", num(step.tracking));
        let _ = writeln!(s, "  --text-{n}-weight: {};", step.weight);
    }
    for (i, v) in SPACE.iter().enumerate() {
        let _ = writeln!(s, "  --space-{}: {}px;", i + 1, num(*v));
    }
    for (name, v) in [
        ("gutter", GUTTER),
        ("gutter-narrow", GUTTER_NARROW),
        ("radius", RADIUS),
        ("radius-plate", RADIUS_PLATE),
        ("radius-pill", RADIUS_PILL),
        ("rule-width", RULE),
        ("control", CONTROL),
        ("control-large", CONTROL_LARGE),
        ("segment", SEGMENT),
        ("field", FIELD),
        ("row", ROW_WEB),
        ("index", INDEX),
        ("plate-bleed", PLATE_BLEED),
        ("rise", RISE),
    ] {
        let _ = writeln!(s, "  --{name}: {}px;", num(v));
    }
    let _ = writeln!(s, "  --measure: {}ch;", num(MEASURE));
    for m in MOTION {
        let [a, b, c, d] = m.curve;
        let _ = writeln!(s, "  --{}: {}ms;", m.name, m.ms);
        let _ = writeln!(
            s,
            "  --ease-{}: cubic-bezier({}, {}, {}, {});",
            m.name,
            num(a),
            num(b),
            num(c),
            num(d)
        );
    }
    let _ = writeln!(s, "  --stagger: {}ms;", STAGGER.as_millis());
    s.push_str(
        "}\n\n@media (prefers-color-scheme: dark) {\n  :root:not([data-theme=\"light\"]) {\n",
    );
    let mut dark = String::new();
    colors(&mut dark, &DARK);
    for line in dark.lines() {
        let _ = writeln!(s, "  {line}");
    }
    s.push_str("  }\n}\n\n:root[data-theme=\"dark\"] {\n");
    s.push_str(&dark);
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every text colour on every background it is drawn on reaches WCAG AA (4.5:1).
    #[test]
    fn text_meets_wcag_aa_in_both_palettes() {
        for p in [LIGHT, DARK] {
            let backgrounds = [
                ("page", p.page),
                ("plate_soft", p.plate_soft),
                ("plate", p.plate),
                ("surface", p.surface),
            ];
            let texts = [
                ("ink", p.ink),
                ("ink_soft", p.ink_soft),
                ("muted", p.muted),
                ("accent", p.accent),
                ("danger", p.danger),
                ("success", p.success),
            ];
            for (bn, bg) in backgrounds {
                for (tn, fg) in texts {
                    let ratio = contrast(fg, bg);
                    assert!(
                        ratio >= 4.5,
                        "{} {tn} on {bn}: {ratio:.2}",
                        if p.dark { "dark" } else { "light" }
                    );
                }
            }
            // Text on the accent, and page-coloured text on an ink segment.
            assert!(contrast(p.on_accent, p.accent) >= 4.5);
            assert!(contrast(p.page, p.ink) >= 4.5);
        }
    }

    /// The game's menus over the brightest backdrop: the page colour veiled over it, and a
    /// selected row's plate (ink at 8 %) on top.
    #[test]
    fn menu_text_meets_wcag_aa_over_a_bright_backdrop() {
        let mix = |a: u32, b: u32, t: f32| {
            [16, 8, 0].iter().fold(0, |out, s| {
                let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
                out | ((x * (1. - t) + y * t).round() as u32) << s
            })
        };
        let p = DARK;
        for (veil, texts) in [
            (
                VEIL,
                &[p.ink, p.ink_soft, BACKDROP_MUTED, p.danger, p.success][..],
            ),
            (HOME_VEIL, &[p.ink, p.ink_soft][..]),
        ] {
            let bg = mix(BRIGHT_BACKDROP, p.page, veil);
            let plate = mix(bg, p.ink, 0.08);
            for &text in texts {
                for under in [bg, plate] {
                    let ratio = contrast(text, under);
                    assert!(ratio >= 4.5, "#{text:06x} over veil {veil}: {ratio:.2}");
                }
            }
        }
        assert!(contrast(BACKDROP_MUTED, p.page) >= 4.5);
    }

    #[test]
    fn contrast_matches_known_values() {
        assert!((contrast(0x000000, 0xffffff) - 21.).abs() < 0.01);
        assert!((contrast(0x777777, 0xffffff) - 4.48).abs() < 0.01);
    }

    #[test]
    fn bezier_eases_out() {
        assert_eq!(bezier(MOVE.curve, 0.), 0.);
        assert!((bezier(MOVE.curve, 1.) - 1.).abs() < 1e-4);
        assert!(MOVE.ease(0.25) > 0.6, "{}", MOVE.ease(0.25));
        assert!((bezier([0., 0., 1., 1.], 0.3) - 0.3).abs() < 1e-3);
    }

    /// `website/src/tokens.css` is what [`css`] writes; regenerate it with
    /// `cargo run -p xtask -- tokens`.
    #[test]
    fn website_tokens_are_up_to_date() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../website/src/tokens.css");
        let written = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            written == css(),
            "website/src/tokens.css is out of date: run `cargo run -p xtask -- tokens`"
        );
    }
}
