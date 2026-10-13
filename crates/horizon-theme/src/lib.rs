//! Horizon's design tokens, shared by the launcher (`launcher/src/theme.rs`)
//! and the game's menus (`horizon-client`'s `ui_theme`), so the two read as
//! one product: change a value here and both follow.
//!
//! One neutral palette in two appearances and one accent. Spacing follows
//! 4/8/12/16/24/32. Type: Geist for the interface, Instrument Serif for
//! titles, Geist Mono for consoles and commands, all bundled here under the
//! SIL Open Font License (`fonts/OFL-*.txt`). Five sizes: 12, 13, 15, 28 and 56.
//! Colours are `0xRRGGBB`; sizes are in logical pixels.
use std::time::Duration;

pub const FONT: &str = "Geist";
pub const DISPLAY: &str = "Instrument Serif";
pub const MONO: &str = "Geist Mono";

/// The bundled font files.
pub mod fonts {
    pub const GEIST_REGULAR: &[u8] = include_bytes!("../fonts/Geist-Regular.ttf");
    pub const GEIST_MEDIUM: &[u8] = include_bytes!("../fonts/Geist-Medium.ttf");
    pub const GEIST_SEMIBOLD: &[u8] = include_bytes!("../fonts/Geist-SemiBold.ttf");
    pub const GEIST_MONO: &[u8] = include_bytes!("../fonts/GeistMono-Regular.ttf");
    pub const INSTRUMENT_SERIF: &[u8] = include_bytes!("../fonts/InstrumentSerif-Regular.ttf");
}

pub const SMALL: f32 = 12.;
pub const BODY: f32 = 13.;
pub const LARGE: f32 = 15.;
pub const TITLE: f32 = 28.;
pub const HERO: f32 = 56.;
/// The opening's wordmark.
pub const WORDMARK: f32 = 96.;

/// The spacing scale.
pub const SPACE: [f32; 6] = [4., 8., 12., 16., 24., 32.];

/// Corner radii: buttons, fields, nav items and rows; segments inside a
/// segmented control; the segmented control's frame; menus and cards.
pub const RADIUS: f32 = 6.;
pub const RADIUS_SEGMENT: f32 = 5.;
pub const RADIUS_GROUP: f32 = 7.;
pub const RADIUS_CARD: f32 = 8.;
/// Dividers and borders are one pixel.
pub const HAIRLINE: f32 = 1.;

/// Control heights: buttons and nav items, segments, the large Play button.
pub const CONTROL: f32 = 30.;
pub const SEGMENT: f32 = 26.;
pub const CONTROL_LARGE: f32 = 40.;
/// A pressed primary button's opacity (secondary and ghost: `PRESSED_SOFT`).
pub const PRESSED: f32 = 0.82;
pub const PRESSED_SOFT: f32 = 0.8;
/// Disabled controls' opacity.
pub const DISABLED: f32 = 0.4;
/// The hover layer of a primary button: white at this opacity.
pub const PRIMARY_HOVER: f32 = 0.10;
/// Sidebar width and the label column of settings rows.
pub const SIDEBAR: f32 = 200.;
pub const ROW_LABEL: f32 = 150.;

/// Motion: short and eased out, never bouncy.
pub const QUICK: Duration = Duration::from_millis(160);
pub const MOVE: Duration = Duration::from_millis(240);
/// The opening: the wordmark holds, then the interface appears.
pub const OPENING_HOLD: Duration = Duration::from_millis(1400);
pub const OPENING_MAX: Duration = Duration::from_millis(2500);
pub const OPENING_REVEAL: Duration = Duration::from_millis(800);

/// GPUI's `ease_out_quint`, used for hovers, the sliding selection and page changes.
pub fn ease_out_quint(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(5)
}
pub fn ease_out_cubic(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: u32,
    pub sidebar: u32,
    pub raised: u32,
    pub hover: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub faint: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub danger: u32,
    pub dark: bool,
}

pub const DARK: Palette = Palette {
    bg: 0x111113,
    sidebar: 0x0c0c0d,
    raised: 0x1a1a1d,
    hover: 0x202024,
    border: 0x26262a,
    text: 0xe8e8ea,
    muted: 0x9a9aa1,
    faint: 0x626269,
    accent: 0x7c5cc4,
    on_accent: 0xffffff,
    danger: 0xd9705f,
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: 0xfcfcfb,
    sidebar: 0xf4f4f2,
    raised: 0xf2f2f0,
    hover: 0xebebe8,
    border: 0xe3e3df,
    text: 0x19191b,
    muted: 0x63636a,
    faint: 0x9a9a9f,
    accent: 0x6a4bb3,
    on_accent: 0xffffff,
    danger: 0xb84a3a,
    dark: false,
};
