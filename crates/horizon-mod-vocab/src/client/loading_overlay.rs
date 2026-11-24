//! Client presentation values for the branded loading overlay.

/// Why the branded loading overlay is on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub enum LoadingOverlayPhase {
    /// Startup after the branded graphics and native listeners are available.
    Startup,
    /// Applying a resource-pack reload in the game.
    Reload,
    /// A client mod's temporary style preview; no resources are reloaded.
    Preview,
}

/// How to draw the two halves of the branded logo with their original positions and UVs.
// exhaustive: the renderer handles the complete set of logo draw modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum LoadingOverlayLogo {
    /// Draw the stock logo at its original color and opacity.
    Original,
    /// Skip the logo, leaving the background visible.
    Hidden,
    /// Multiply the logo by an ARGB tint and draw it with normal alpha blending.
    Tint(u32),
    /// Draw the original logo twice: subtract the first color, then add the second.
    /// Each ARGB value is scaled by the logo texel and its alpha; zero alpha skips that pass.
    ChannelDeltas {
        /// ARGB channel differences to subtract from the background.
        darken: u32,
        /// ARGB channel differences to add to the background.
        lighten: u32,
    },
}

/// The colors and logo returned by the loading overlay presentation hook. Colors are ARGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
#[non_exhaustive]
pub struct LoadingOverlayStyle {
    /// Full-screen background color.
    pub background: u32,
    /// Interior track color, or `None` to leave it transparent.
    pub bar_background: Option<u32>,
    /// Color of the completed part of the progress bar.
    pub bar_fill: u32,
    /// Progress bar border color.
    pub bar_border: u32,
    /// Control over the logo's original blits.
    pub logo: LoadingOverlayLogo,
}

impl LoadingOverlayStyle {
    /// The original logo and progress bar over `background`.
    pub const fn new(background: u32) -> Self {
        Self {
            background,
            bar_background: None,
            bar_fill: u32::MAX,
            bar_border: u32::MAX,
            logo: LoadingOverlayLogo::Original,
        }
    }
}
