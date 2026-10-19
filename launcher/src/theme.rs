//! The launcher's view of Horizon's design tokens (`crates/horizon-theme`, described in
//! `docs/design-system.md`), which the game's menus and the website share: cool neutrals in two
//! appearances and one violet accent, Geist and Geist Mono (bundled, SIL Open Font License).
use gpui::{rgb, Hsla, WindowAppearance};
use horizon_theme as tokens;
use std::time::Duration;

pub use tokens::{Step, BODY, CAPTION, DISPLAY, HEADING, LABEL, MICRO, TITLE, UI, WORDMARK};
pub const FONT: &str = tokens::FONT;
pub const MONO: &str = tokens::MONO;

/// Shapes and controls, shared with the game's menus and the website.
pub use tokens::{
    CONTROL, CONTROL_LARGE, DISABLED, FIELD, GUTTER, INDEX, PLATE_BLEED, PRESSED, RADIUS,
    RADIUS_PILL, RADIUS_PLATE, RISE, ROW, SEGMENT,
};

/// Motion: short and eased out, never bouncy.
pub const QUICK: Duration = tokens::QUICK.duration();
pub const MOVE: Duration = tokens::MOVE.duration();
pub const ENTER: Duration = tokens::ENTER.duration();
pub const STAGGER: Duration = tokens::STAGGER;

#[derive(Clone, Copy)]
pub struct Theme {
    pub page: Hsla,
    pub plate_soft: Hsla,
    pub plate: Hsla,
    pub surface: Hsla,
    pub rule: Hsla,
    pub ink: Hsla,
    pub ink_soft: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub dark: bool,
}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

impl Theme {
    /// The system's appearance; `HORIZON_LAUNCHER_APPEARANCE=light` or `dark` overrides it (for
    /// screenshots and tests).
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match std::env::var("HORIZON_LAUNCHER_APPEARANCE").as_deref() {
            Ok("light") => return Self::light(),
            Ok("dark") => return Self::dark(),
            _ => {}
        }
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
            page: c(p.page),
            plate_soft: c(p.plate_soft),
            plate: c(p.plate),
            surface: c(p.surface),
            rule: c(p.rule),
            ink: c(p.ink),
            ink_soft: c(p.ink_soft),
            muted: c(p.muted),
            accent: c(p.accent),
            on_accent: c(p.on_accent),
            danger: c(p.danger),
            success: c(p.success),
            dark: p.dark,
        }
    }
}

/// How strongly a selected suggestion in a text field is tinted with the accent.
pub const SELECTION: f32 = 0.22;

#[cfg(test)]
mod tests {
    use super::{tokens, SELECTION};

    /// `top` at `alpha` over `under`, as the GPU blends them.
    fn blend(top: u32, under: u32, alpha: f32) -> u32 {
        let channel = |shift: u32| {
            let (a, b) = ((top >> shift & 0xff) as f32, (under >> shift & 0xff) as f32);
            ((a * alpha + b * (1. - alpha)).round() as u32) << shift
        };
        channel(16) | channel(8) | channel(0)
    }

    /// What the Servers page draws besides text (whose colours horizon-theme's own test holds to
    /// WCAG AA on every background): the keyboard focus ring, the borders of a focused or refused
    /// field (WCAG 1.4.11 asks 3:1 for those), and text over a selected suggestion.
    #[test]
    fn marks_and_selections_read_clearly() {
        for p in [tokens::LIGHT, tokens::DARK] {
            let theme = if p.dark { "dark" } else { "light" };
            for (name, bg) in [
                ("page", p.page),
                ("plate_soft", p.plate_soft),
                ("plate", p.plate),
                ("surface", p.surface),
            ] {
                let ring = tokens::contrast(p.accent, bg);
                assert!(ring >= 3.0, "{theme} focus ring on {name}: {ring:.2}");
                let refused = tokens::contrast(p.danger, bg);
                assert!(
                    refused >= 3.0,
                    "{theme} refused field on {name}: {refused:.2}"
                );
            }
            let selection = blend(p.accent, p.surface, SELECTION);
            let ratio = tokens::contrast(p.ink, selection);
            assert!(ratio >= 4.5, "{theme} text on a selection: {ratio:.2}");
        }
    }
}
