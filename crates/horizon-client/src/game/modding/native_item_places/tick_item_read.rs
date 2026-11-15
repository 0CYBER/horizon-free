impl Game<'_> {
    pub(super) fn unscoped_game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
        if !self.places_simulated() {
            return None;
        }
        if let ItemPlace::ItemEntity { id } = place {
            return self
                .dropped
                .iter()
                .find(|e| e.id == *id)
                .map(|e| e.stack.clone());
        }
        if let ItemPlace::EntitySlot { entity, slot } = place {
            return self
                .edit_entity_slots(*entity, |slots, _| {
                    slots.get(*slot).cloned().flatten().filter(|s| s.count > 0)
                })
                .flatten();
        }
        if let ItemPlace::Equipment { entity, slot } = place {
            return self
                .equipment_stack(*entity, *slot)
                .flatten()
                .filter(|s| s.count > 0);
        }
        let mut stack = None;
        self.native_as_player(self.place_player(place)?, |g| {
            stack = g.player_place(place).and_then(|slot| slot.clone());
        });
        stack.filter(|s| s.count > 0)
    }
    #[cold]
    #[inline(never)]
    fn scoped_game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
        if !self.places_simulated() {
            return None;
        }
        if let ItemPlace::ItemEntity { id } = place
            && let Some(stack) = self.tick_item_view(*id)
        {
            return stack.cloned();
        }
        self.unscoped_game_stack(place)
    }
}
