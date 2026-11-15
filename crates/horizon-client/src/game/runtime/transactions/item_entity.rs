impl Game<'_> {
    pub(super) fn abort_tick_item_transactions(&mut self, depth: usize) {
        while self.transactions.len() > depth {
            self.close_storage_transaction(false);
        }
    }

    /// Sets item entity `id`'s stack, keeping it as it was if a transaction is open and hasn't
    /// yet. Emptied in one, it stays, empty, until the root commit removes it, so an abort can
    /// put its stack back. False where no such item entity is.
    pub(super) fn journal_set_item_entity(&mut self, id: u32, stack: ItemStack) -> bool {
        if self.block_entity_tick_key.is_some() {
            return self
                .stage_tick_item(id, (stack.count != 0).then_some(stack))
                .unwrap_or(false);
        }
        let Some(level) = self.transactions.last_mut() else {
            let dimension = self.dim.id();
            return self.native_set_item_entity(dimension, id, stack);
        };
        let Some(e) = self.dropped.iter_mut().find(|e| e.id == id) else {
            return false;
        };
        level
            .item_entities
            .entry(id)
            .or_insert_with(|| e.stack.clone());
        if stack.count == 0 {
            level.after.push(After::Emptied(id));
        }
        e.stack = stack;
        true
    }
}
