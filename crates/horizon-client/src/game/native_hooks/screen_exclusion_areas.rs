use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The hook `screen_exclusion` asks: a mod adds the rectangles it
    /// draws over `screen` (a side panel, a button column) to `areas`, in GUI
    /// pixels, `[x, y, width, height]`. NeoForge has no counterpart in the
    /// game; recipe viewers ask for it through their own API
    /// (`EmiExclusionArea`, JEI's `IGuiContainerHandler.getGuiExtraAreas`).
    /// Client-side.
    pub(in crate::game) fn native_screen_exclusion_areas(
        &self,
        screen: &ScreenView<'_>,
        areas: &mut Vec<[f32; 4]>,
    ) {
        self.fire_screen_exclusion_areas(screen, areas);
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Where the open screen draws outside its `rect`, and where mods draw
    /// over it: the recipe book's panel, the creative tab rows, the active
    /// effects beside the inventory, then whatever `native_screen_exclusion_areas`
    /// adds. Each is `[x, y, width, height]` in GUI pixels; an empty or
    /// non-finite one is dropped. A mod that lays out its own panels
    /// (a recipe viewer) keeps them clear of these. Client-side; empty with
    /// no screen up.
    pub(in crate::game) fn screen_exclusion(&self) -> Vec<[f32; 4]> {
        let Some(view) = self.observed_screen_view() else {
            return Vec::new();
        };
        let Some(renderer) = self.renderer.as_ref() else {
            return Vec::new();
        };
        let batch = HudBatch::new(
            [renderer.config.width, renderer.config.height],
            self.settings.gui_scale_inputs(),
        );
        let mut areas = self.screen_own_exclusion(&batch);
        self.native_screen_exclusion_areas(&view, &mut areas);
        areas.retain(|a| a.iter().all(|v| v.is_finite()) && a[2] > 0.0 && a[3] > 0.0);
        areas
    }
}
