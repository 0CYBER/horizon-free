//! The launcher's view of Horizon's design tokens (`crates/horizon-theme`, described in
//! `docs/design-system.md`), which the game's menus and the website share: cool neutrals in two
//! appearances and one violet accent, Geist and Geist Mono (bundled, SIL Open Font License).
use gpui::{rgb, Hsla, WindowAppearance};
use horizon_theme as tokens;
use std::time::Duration;

pub const FONT: &str = tokens::FONT;
/// Display type is Geist too, at medium weight.
pub const DISPLAY: &str = tokens::FONT;
pub const MONO: &str = tokens::MONO;

pub const SMALL: f32 = tokens::CAPTION.px;
pub const BODY: f32 = tokens::LABEL.px;
pub const LARGE: f32 = tokens::BODY.px;
pub const TITLE: f32 = tokens::TITLE.px;
pub const HERO: f32 = tokens::DISPLAY.px;

/// Shapes and controls, shared with the game's menus.
pub use tokens::{CONTROL, DISABLED, PRESSED, RADIUS, SEGMENT};
pub const PRESSED_SOFT: f32 = tokens::PRESSED;
pub const PRIMARY_HOVER: f32 = 0.10;
pub const RADIUS_SEGMENT: f32 = tokens::RADIUS;
pub const RADIUS_GROUP: f32 = tokens::RADIUS;
pub const ROW_LABEL: f32 = 150.;
pub const SIDEBAR: f32 = 200.;

/// Motion: short and eased out, never bouncy.
pub const QUICK: Duration = tokens::QUICK.duration();
pub const MOVE: Duration = tokens::MOVE.duration();

#[derive(Clone, Copy)]
pub struct Theme {
    pub bg: Hsla,
    pub sidebar: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub dark: bool,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self::light(),
            _ => Self::dark(),
        }
    }

    pub fn dark() -> Self {
        Self::from(tokens::DARK)
    }

    pub fn light() -> Self {
        Self::from(tokens::LIGHT)
    }

    fn from(p: tokens::Palette) -> Self {
        Self {
            bg: c(p.page),
            sidebar: c(p.plate_soft),
            raised: c(p.plate_soft),
            hover: c(p.plate),
            border: c(p.rule),
            text: c(p.ink),
            muted: c(p.ink_soft),
            faint: c(p.muted),
            accent: c(p.accent),
            on_accent: c(p.on_accent),
            danger: c(p.danger),
            dark: p.dark,
        }
    }
}
