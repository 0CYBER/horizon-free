//! Titles, the action bar and the selected item name: the state and layout
//! live in `hud_overlays` (`Titles`, `ToolHighlight`); this feeds and draws them.
use super::Game;
pub(super) use horizon_client::hud_overlays::Titles;
use horizon_client::{
    hud::HudBatch,
    hud_layout,
    hud_overlays::HeldName,
    text::{self, Text},
};

impl Game<'_> {
    /// `Hud.extractOverlayMessage` and `extractTitle`.
    pub(super) fn draw_titles(&self, batch: &mut HudBatch) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let partial = self.clock.fraction() as f32;
        let background = self.settings.text_background(0.0);
        let font = &self.atlas.font;
        let mut draws = self.titles.overlay_message(w, h, partial, font, background);
        draws.extend(self.titles.title(w, h, partial, font, background));
        hud_layout::paint(batch, &self.atlas, &draws);
    }
    /// The selected stack as `Hud.tick` compares it for the name highlight.
    fn held_name(&self) -> Option<HeldName> {
        let stack = self.inventory.get(self.selected)?.as_ref()?;
        let custom = stack.custom_name();
        // ItemStack.getRarity: enchanting raises common and uncommon to rare
        // and rare to epic.
        let mut rarity = self.item_rarity(&stack.item);
        if !stack.enchantments().is_empty() {
            rarity = match rarity.as_str() {
                "common" | "uncommon" => "rare".into(),
                "rare" => "epic".into(),
                _ => rarity,
            };
        }
        Some(HeldName {
            item: stack.item.to_string(),
            hover_name: Text::literal(
                custom.map_or_else(|| self.pack.names.item(&stack.item), str::to_owned),
            ),
            rarity_color: match rarity.as_str() {
                "uncommon" => text::YELLOW,
                "rare" => text::AQUA,
                "epic" => text::LIGHT_PURPLE,
                _ => text::WHITE,
            },
            custom_name: custom.is_some(),
        })
    }
    /// The title, action bar and selected-item parts of `Hud.tick`.
    pub(super) fn tick_hud(&mut self) {
        self.titles.tick();
        let held = self.held_name();
        self.tool_highlight
            .tick(held, f64::from(self.settings.notification_display_time));
    }
    /// `Hud.extractSelectedItemName`.
    pub(super) fn draw_selected_name(&self, batch: &mut HudBatch, can_hurt: bool) {
        let draws = self.tool_highlight.draw(
            batch.width as i32,
            batch.height as i32,
            can_hurt,
            &self.atlas.font,
            self.settings.text_background(0.0),
        );
        hud_layout::paint(batch, &self.atlas, &draws);
    }
}
