//! Placement state is discarded before a new catalogue or menu can be shown.
use super::*;
pub(super) struct PlacementOverlay {
    pub(super) view: OverlayView,
    pub(super) recipe: String,
    pub(super) sources: Vec<u32>,
}
impl Game<'_> {
    pub(super) fn request_mod_book_recipe(&mut self, recipe: String, source: u32) {
        if !self
            .native_net
            .placement
            .client
            .window()
            .is_some_and(|window| self.native_book_placement_window_matches(window))
        {
            return;
        }
        let Ok(recipe) = recipe.try_into() else {
            return;
        };
        let use_max = self.has_shift_down();
        if let Some(frame) = self.native_net.placement.client.select(
            horizon_client::recipe_book::placement_sync::DisplayId { recipe, source },
            use_max,
        ) {
            self.send_mod_book_placement(frame);
            if self
                .batch_for_size()
                .is_some_and(|batch| self.mod_book_overlay(batch.width))
                && let Some((_, filtering)) = self.mod_book_flags()
                && let Some(book) = self.native_screen_book().cloned()
            {
                self.request_custom_book_settings(
                    &book,
                    mod_api::RecipeBookSettings {
                        open: false,
                        filtering,
                    },
                );
            }
        }
    }
    pub(super) fn consume_mod_book_overlay(&mut self, right: bool) -> bool {
        let Some(overlay) = self.recipe_ui.custom.overlay.take() else {
            return false;
        };
        if !right {
            let columns = if overlay.view.recipes.len() <= 16 {
                4
            } else {
                5
            };
            for (n, source) in overlay.sources.iter().enumerate() {
                let x = overlay.view.x as f32 + 4.0 + 25.0 * (n % columns) as f32;
                let y = overlay.view.y as f32 + 5.0 + 25.0 * (n / columns) as f32;
                if inside(self.cursor, x, y, 24.0, 24.0) {
                    self.request_mod_book_recipe(overlay.recipe, *source);
                    break;
                }
            }
        }
        true
    }
    pub(in crate::game) fn resolve_mod_book_declaration_slot(
        &self,
        slot: &mod_api::RecipeSlotDisplay,
    ) -> Vec<ItemStack> {
        let stocks = horizon_client::menu::place::Stacked::default();
        resolve::Resolver::new(self, &stocks, &[]).resolve(slot)
    }
    pub(in crate::game) fn install_mod_book_ghost(
        &mut self,
        selection: &horizon_client::recipe_book::placement_sync::Selection,
    ) {
        let Some(view) = self
            .native_net
            .placement
            .view
            .as_ref()
            .filter(|view| view.identity == selection.catalog)
        else {
            return;
        };
        let Some(layout) = view.index.layout(selection.catalog.menu.as_str()).cloned() else {
            return;
        };
        let Some(display) = view
            .index
            .categories()
            .flat_map(|category| &category.displays)
            .find(|entry| entry.recipe == selection.display.recipe.as_str())
            .and_then(|entry| entry.displays.get(selection.display.source as usize))
            .cloned()
        else {
            return;
        };
        let result = match layout.declaration() {
            mod_api::RecipeBookLayout::Crafting { result, .. }
            | mod_api::RecipeBookLayout::Furnace { result, .. }
            | mod_api::RecipeBookLayout::Stonecutter { result, .. }
            | mod_api::RecipeBookLayout::Smithing { result, .. } => *result,
        };
        let fuel_empty = match layout.declaration() {
            mod_api::RecipeBookLayout::Furnace { fuel, .. } => {
                self.slot_mut(300 + usize::from(*fuel)).is_none()
            }
            _ => true,
        };
        let Ok(mapped) = layout.map(&display, fuel_empty) else {
            return;
        };
        let mut ghosts = Vec::with_capacity(mapped.len());
        for (slot, display) in mapped {
            if self.native_screen_slot(usize::from(slot)).is_none() {
                return;
            }
            let items = self.resolve_mod_book_declaration_slot(&display);
            if !items.is_empty() {
                ghosts.push(super::super::Ghost {
                    slot: 300 + usize::from(slot),
                    items,
                    result: slot == result,
                });
            }
        }
        self.recipe_ui.ghost = ghosts;
    }
    pub(in crate::game) fn reset_mod_book_placement_ui(&mut self) {
        self.recipe_ui.ghost.clear();
        self.recipe_ui.custom.prepared = None;
        self.recipe_ui.custom.presentation = None;
        self.recipe_ui.custom.overlay = None;
    }
}
