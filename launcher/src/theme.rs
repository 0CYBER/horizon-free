//! One neutral palette in two appearances and one accent. Spacing follows 4/8/12/16/24/32.
use gpui::{rgb, Hsla, WindowAppearance};

pub const FONT: &str = "Inter";
#[cfg(target_os = "macos")]
pub const MONO: &str = "Menlo";
#[cfg(windows)]
pub const MONO: &str = "Consolas";
#[cfg(not(any(target_os = "macos", windows)))]
pub const MONO: &str = "DejaVu Sans Mono";

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
            accent: c(0x4c9a64),
            on_accent: c(0xffffff),
            danger: c(0xd9705f),
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
            accent: c(0x3b8753),
            on_accent: c(0xffffff),
            danger: c(0xb84a3a),
        }
    }
}
