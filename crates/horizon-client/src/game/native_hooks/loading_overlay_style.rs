use super::*;
use horizon_client::hud::HudAtlas;
use horizon_client::level_loading;
use mod_api::{LoadingOverlayPhase, LoadingOverlayStyle};

impl Game<'_> {
    /// A branded loading overlay is about to paint its background, bar and logo.
    pub(in crate::game) fn native_loading_overlay_style(
        phase: LoadingOverlayPhase,
        progress: f32,
        style: &mut LoadingOverlayStyle,
    ) {
        Self::fire_loading_overlay_style(phase, progress, style);
    }

    pub(in crate::game) fn draw_native_loading_overlay(
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        line: &str,
        progress: f32,
        dark: bool,
        phase: LoadingOverlayPhase,
    ) {
        let mut style = level_loading::stock_style(dark);
        Self::native_loading_overlay_style(phase, progress, &mut style);
        level_loading::draw_loading_overlay_with_style(batch, atlas, line, progress, &style);
    }
}
