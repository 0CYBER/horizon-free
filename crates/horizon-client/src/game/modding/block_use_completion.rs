use super::*;

impl Game<'_> {
    /// The game mode's item-phase trigger is separate from item-internal triggers.
    pub(super) fn finish_item_block_use(
        &mut self,
        hit: Option<&Hit>,
        original: &ItemStack,
    ) -> horizon_client::Result<bool> {
        if let Some(hit) = hit {
            self.trigger_block("minecraft:item_used_on_block", hit.pos, original);
        }
        Ok(true)
    }

    /// Run one block use with an isolated original stack, restoring the outer
    /// operation even when a listener unwinds. Replacements never edit this value.
    pub(super) fn with_block_use_original<R>(
        &mut self,
        hand: Hand,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let original = self.with_hand(hand, |g| g.held().cloned());
        let depth = self.block_use_original.len();
        self.block_use_original.push((hand, original));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self)));
        self.block_use_original.truncate(depth);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Apply an engine in-place edit to the acting hand and its operation original.
    pub(super) fn edit_block_use_stack(&mut self, edit: impl Fn(&mut Option<ItemStack>)) {
        edit(self.held_mut());
        if let Some((hand, original)) = self.block_use_original.last_mut()
            && *hand == self.hand
        {
            edit(original);
        }
    }

    /// Complete a consuming use with the stack held when block use started,
    /// including the action's own in-place consumption, damage, component edits,
    /// and creative-mode count restore. An exhausted stack is reported empty.
    /// Listener hand replacements and whole replacements by the action (such as
    /// a bucket becoming a water bucket) do not change the reported original.
    /// This matches vanilla passing the same stack object it read before the action.
    pub(super) fn finish_block_use(&mut self, pos: [i32; 3], _hand: Hand) {
        let stack = self
            .block_use_original
            .last_mut()
            .and_then(|(_, stack)| stack.take())
            .unwrap_or_else(|| ItemStack::new("minecraft:air", 0));
        horizon_client::spans::time("use_advancements", || {
            self.trigger_any_block_use(pos, &stack);
        });
    }
}
