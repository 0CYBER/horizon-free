use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `Screen.renderTransparentBackground`: draws the translucent dark background
    /// behind an in-game screen (inventory, container, creative, book, sign, editor).
    /// Set `canceled` (or return `Flow::Cancel`) to suppress the vanilla dark gradient,
    /// allowing a mod to draw its own background or apply menu background blur
    /// via `Host::apply_screen_blur`. API @API:screen_transparent_background@.
    pub(in crate::game) fn native_screen_transparent_background(
        &self,
        screen: &str,
        batch: &mut HudBatch,
        canceled: &mut bool,
    ) {
        self.fire_screen_transparent_background(screen, batch, canceled);
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Requests vanilla's menu background blur for this frame, with a per-frame guard
    /// so blur is applied at most once per frame. Returns true if blur was newly
    /// requested this frame, or false if it was already applied/requested this frame.
    /// API @API:screen_transparent_background@.
    pub(in crate::game) fn apply_screen_blur(&self) -> bool {
        if self.screen_blur_requested.get() {
            return false;
        }
        self.screen_blur_requested.set(true);
        true
    }
    /// The user's configured menu background blur radius (`menuBackgroundBlurriness` in options.txt,
    /// usually 0.0 to 10.0; default 5.0). 0.0 means blur is turned off. API @API:screen_transparent_background@.
    pub(in crate::game) fn menu_blur_radius(&self) -> f32 {
        self.settings.menu_blur as f32
    }
    /// `Screen.renderMenuBackground`: draws the tiled menu background (32x32) across `rect`
    /// (`[x, y, w, h]` in GUI pixels). If `in_world` is true, uses the dark translucent
    /// texture (`gui/inworld_menu_background`); otherwise uses the opaque dirt-like
    /// texture (`gui/menu_background`). API @API:screen_transparent_background@.
    pub(in crate::game) fn native_draw_menu_background(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        in_world: bool,
    ) {
        let [x, y, w, h] = rect;
        horizon_client::ui::menu_background(batch, &self.gui.atlas, x, y, w, h, in_world);
    }
    /// Repeats sprite `sprite` across `rect` (`[x, y, w, h]` in GUI pixels) with each tile
    /// sized `size` GUI pixels square, tinted by `argb`. False when the sprite is not in the HUD atlas.
    /// API @API:screen_transparent_background@.
    pub(in crate::game) fn native_tiled_sprite(
        &self,
        batch: &mut HudBatch,
        sprite: &str,
        rect: [f32; 4],
        size: f32,
        argb: u32,
    ) -> bool {
        let atlas = &self.gui.atlas;
        if !atlas.sprites.contains_key(sprite) {
            return false;
        }
        let [x, y, w, h] = rect;
        let [a, r, g, b] = argb.to_be_bytes();
        horizon_client::ui::tiled(batch, atlas, sprite, x, y, w, h, size, rgba(r, g, b, a));
        true
    }
}
