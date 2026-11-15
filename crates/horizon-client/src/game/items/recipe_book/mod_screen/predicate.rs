//! Predicate stock is rebuilt only when the panel's component-aware stock cache changes.
use super::*;
impl Game<'_> {
    pub(super) fn mod_book_placement_stocks(&mut self) -> Vec<(ItemStack, i32)> {
        let Some(menu) = self.native_screen_menu() else {
            return Vec::new();
        };
        if mod_api::RecipeBookLayout::declaring_mod(menu).is_none() {
            return Vec::new();
        }
        let Some(layout) = self
            .native_net
            .placement
            .view
            .as_ref()
            .and_then(|view| view.index.layout(menu))
        else {
            return Vec::new();
        };
        let slots = match layout.declaration() {
            mod_api::RecipeBookLayout::Crafting { slots, .. } => slots.clone(),
            mod_api::RecipeBookLayout::Furnace { input, .. }
            | mod_api::RecipeBookLayout::Stonecutter { input, .. } => vec![*input],
            mod_api::RecipeBookLayout::Smithing {
                template,
                base,
                addition,
                ..
            } => vec![*template, *base, *addition],
        };
        let mut stacks: Vec<_> = self
            .inventory
            .iter()
            .flatten()
            .filter(|stack| horizon_client::menu::place::usable_for_crafting(stack))
            .cloned()
            .collect();
        for slot in slots {
            if let Some(stack) = self.slot_mut(300 + usize::from(slot)).clone() {
                stacks.push(stack);
            }
        }
        stacks
            .into_iter()
            .map(|mut stack| {
                let amount = i32::try_from(
                    stack
                        .count
                        .min(horizon_client::menu::item_max_stack(self.items(), &stack)),
                )
                .unwrap_or(i32::MAX);
                stack.count = 1;
                (stack, amount)
            })
            .collect()
    }
}
