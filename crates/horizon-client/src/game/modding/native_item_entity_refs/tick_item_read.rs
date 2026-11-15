impl Game<'_> {
    pub(super) fn unscoped_item_entity_stack(&self, item: ItemEntityRef) -> Option<ItemStack> {
        Some(
            self.dropped
                .get(self.unscoped_item_entity_index(item)?)?
                .stack
                .clone(),
        )
    }

    fn unscoped_item_entity_index(&self, item: ItemEntityRef) -> Option<usize> {
        let Target::Item(id) = self.unscoped_native_entity_target(item.entity())? else {
            return None;
        };
        self.dropped.iter().position(|entity| entity.id == id)
    }

    #[cold]
    #[inline(never)]
    fn scoped_item_entity_stack(&self, item: ItemEntityRef) -> Option<ItemStack> {
        self.unscoped_item_entity_index(item)?;
        if let Some(stack) = self.tick_item_view(item.entity().id()) {
            return stack.cloned();
        }
        self.unscoped_item_entity_stack(item)
    }
}
