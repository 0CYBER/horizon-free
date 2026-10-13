//! The launcher's view of Horizon's design tokens (`crates/horizon-theme`), which the game's
//! menus share: one neutral palette in two appearances and one accent, spacing 4/8/12/16/24/32,
//! Geist / Instrument Serif / Geist Mono (bundled, SIL Open Font License), five sizes.
use gpui::{rgb, Hsla, WindowAppearance};
use horizon_theme as tokens;
use std::time::Duration;

pub const FONT: &str = tokens::FONT;
pub const DISPLAY: &str = tokens::DISPLAY;
pub const MONO: &str = tokens::MONO;

pub const SMALL: f32 = tokens::SMALL;
pub const BODY: f32 = tokens::BODY;
pub const LARGE: f32 = tokens::LARGE;
pub const TITLE: f32 = tokens::TITLE;
pub const HERO: f32 = tokens::HERO;

/// Shapes and controls, shared with the game's menus.
pub use tokens::{
    CONTROL, CONTROL_LARGE, DISABLED, PRESSED, PRESSED_SOFT, PRIMARY_HOVER, RADIUS, RADIUS_CARD,
    RADIUS_GROUP, RADIUS_SEGMENT, ROW_LABEL, SEGMENT, SIDEBAR,
};

/// Motion: short and eased out, never bouncy.
pub const QUICK: Duration = tokens::QUICK;
pub const MOVE: Duration = tokens::MOVE;

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
            bg: c(p.bg),
            sidebar: c(p.sidebar),
            raised: c(p.raised),
            hover: c(p.hover),
            border: c(p.border),
            text: c(p.text),
            muted: c(p.muted),
            faint: c(p.faint),
            accent: c(p.accent),
            on_accent: c(p.on_accent),
            danger: c(p.danger),
            dark: p.dark,
        }
    }
}
