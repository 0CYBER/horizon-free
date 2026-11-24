use super::*;

impl Game<'_> {
    /// `HopperScreen.extractBackground` / `DispenserScreen.extractBackground` and their labels.
    pub(in crate::game) fn draw_machine_screen(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) -> bool {
        let Some(screen) = self.machine_screen() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        let key_is_cart = self
            .container
            .is_some_and(|k| k.1 == crate::game::minecarts::CART_KEY_Y);
        let (texture, height, key, fallback) = match screen {
            Screen::Hopper if key_is_cart => (
                "gui/container/hopper",
                133.0,
                "entity.minecraft.hopper_minecart",
                "Minecart with Hopper",
            ),
            Screen::Hopper => (
                "gui/container/hopper",
                133.0,
                "container.hopper",
                "Item Hopper",
            ),
            Screen::Dispenser => (
                "gui/container/dispenser",
                166.0,
                "container.dispenser",
                "Dispenser",
            ),
            Screen::Dropper => (
                "gui/container/dispenser",
                166.0,
                "container.dropper",
                "Dropper",
            ),
        };
        batch.texture_blit_baked(
            atlas,
            texture,
            [ox, oy, 176.0, height],
            [0.0; 2],
            [256.0; 2],
        );
        self.draw_screen_phase(batch, horizon_client::screen_draw::RenderPhase::Background);
        let grey = horizon_client::hud::rgba(64, 64, 64, 255);
        let title = &self.remote_menu_title().unwrap_or_else(|| {
            horizon_client::text::Text::literal(
                self.container_title(self.translate_with_fallback(key, fallback)),
            )
            .resolve_legacy()
        });
        crate::game::screen_info::name_container(|| title.plain());
        let title_x = if screen == Screen::Hopper {
            8.0
        } else {
            (176.0 - atlas.font.width_text(title)) / 2.0
        };
        batch.text_styled(atlas, title, ox + title_x, oy + 6.0, grey, false);
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + height - 94.0, grey, false);
        true
    }
}
