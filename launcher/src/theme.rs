//! One neutral palette in two appearances and one accent. Spacing follows 4/8/12/16/24/32.
//! Type: Geist for the interface, Instrument Serif for titles, Geist Mono for consoles and
//! commands, all bundled (SIL Open Font License). Five sizes: 12, 13, 15, 28 and 56.
use gpui::{rgb, Hsla, WindowAppearance};
use std::time::Duration;

pub const FONT: &str = "Geist";
pub const DISPLAY: &str = "Instrument Serif";
pub const MONO: &str = "Geist Mono";

pub const SMALL: f32 = 12.;
pub const BODY: f32 = 13.;
pub const LARGE: f32 = 15.;
pub const TITLE: f32 = 28.;
pub const HERO: f32 = 56.;

/// Motion: short and eased out, never bouncy.
pub const QUICK: Duration = Duration::from_millis(160);
pub const MOVE: Duration = Duration::from_millis(240);

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
        Self {
            bg: c(0x111113),
            sidebar: c(0x0c0c0d),
            raised: c(0x1a1a1d),
            hover: c(0x202024),
            border: c(0x26262a),
            text: c(0xe8e8ea),
            muted: c(0x9a9aa1),
            faint: c(0x626269),
            accent: c(0x7c5cc4),
            on_accent: c(0xffffff),
            danger: c(0xd9705f),
            dark: true,
        }
    }

    pub fn light() -> Self {
        Self {
            bg: c(0xfcfcfb),
            sidebar: c(0xf4f4f2),
            raised: c(0xf2f2f0),
            hover: c(0xebebe8),
            border: c(0xe3e3df),
            text: c(0x19191b),
            muted: c(0x63636a),
            faint: c(0x9a9a9f),
            accent: c(0x6a4bb3),
            on_accent: c(0xffffff),
            danger: c(0xb84a3a),
            dark: false,
        }
    }
}
