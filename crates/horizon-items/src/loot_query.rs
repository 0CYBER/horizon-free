use super::*;
impl<R: RandomSource> LootContext<'_, R> {
    pub(super) fn stock_condition(&self, c: &Value) -> Option<bool> {
        if !matches!(
            c["type"].as_str(),
            Some("neoforge:loot_table_id" | "neoforge:can_item_perform_ability")
        ) {
            return None;
        }
        let tool = (c["type"].as_str() == Some("neoforge:can_item_perform_ability"))
            .then(|| {
                self.held
                    .filter(|s| s.count > 0 && &*s.item != "minecraft:air")
                    .map(loot_of)
            })
            .flatten();
        horizon_world::loot::query::condition(c, &self.query, tool.as_ref(), |item, ability| {
            self.items.vanilla_can_perform(item, ability)
        })
    }
    pub(super) fn evaluate_named_root(
        &self,
        table: &Value,
        random: &mut R,
        id: &str,
    ) -> Vec<ItemStack> {
        let items = self.evaluate(table, random);
        self.query.finish(Some(id));
        items
    }
    pub(super) fn evaluate_root(&self, table: &Value, random: &mut R) -> Vec<ItemStack> {
        let items = self.evaluate(table, random);
        self.query.finish(None);
        items
    }
    /// The context a mod's own loot kind sees in this engine: the block and
    /// the tool it has, and a seed from the roll's random. The entity, the
    /// damage, the player and the biome belong to the main loot engine's
    /// rolls; they are None here (docs/modding/src/how-to/native-mods.md).
    pub(super) fn native_loot_context(
        &self,
        random: &mut R,
    ) -> horizon_world::loot::LootContext<'_> {
        horizon_world::loot::LootContext {
            queried_table_id: self.query.id(),
            queried_stack: self.query.queried_stack.clone(),
            entity: None,
            damage: None,
            player: None,
            block: (!self.block.is_empty()).then_some(self.block),
            tool: self.held.map(|h| horizon_world::loot::LootTool {
                item: h.item.to_string(),
                enchantments: h
                    .enchantments()
                    .iter()
                    .map(|(id, lvl)| (id.to_string(), *lvl as i32))
                    .collect(),
            }),
            luck: 0.0,
            biome: None,
            seed: random.next_long(),
            place: horizon_world::loot::current_native_loot_place(),
        }
    }
}

impl ItemStack {
    /// Convert loot facts using the same component write path as native loot functions.
    pub fn from_loot(value: &horizon_world::loot::LootStack) -> Self {
        let mut stack = Self {
            item: Arc::from(value.item.as_str()),
            count: 1,
            damage: 0,
            extra: None,
        };
        let before = loot_of(&stack);
        apply_loot(&before, value, &mut stack);
        stack
    }
}

impl Items {
    pub(super) fn block_drops_raw(
        &self,
        registry: &BlockRegistry,
        state: BlockStateId,
        roll: BlockLootRoll<'_>,
        random: &mut impl RandomSource,
        sequences: Option<(&mut crate::random_sequences::RandomSequences, i64)>,
    ) -> Vec<ItemStack> {
        let Some(block) = registry.state(state) else {
            return vec![];
        };
        let table = self.block_loot_table(registry, state);
        let Some(id) = table else {
            return vec![];
        };
        let Some(table) = self.loot.get(&id) else {
            return vec![];
        };
        horizon_world::loot::query::root(&id, || {
            // LootTable.getRandomItems draws from the table's random sequence.
            if let (Some(seq), Some(sequences)) = (
                table.get("random_sequence").and_then(Value::as_str),
                sequences,
            ) {
                let random = sequences.0.get(seq, sequences.1);
                return self.roll_block_table(table, &block.name, &block.properties, roll, random);
            }
            self.roll_block_table(table, &block.name, &block.properties, roll, random)
        })
    }
    /// `Block.dropFromBlockInteractLootTable`: the `block_interact` table `id`
    /// for a right click on `state` (its `match_block` conditions see the
    /// state), from the table's random sequence when it has one.
    pub fn interact_loot(
        &self,
        id: &str,
        registry: &BlockRegistry,
        state: BlockStateId,
        random: &mut impl RandomSource,
        sequences: &mut crate::random_sequences::RandomSequences,
        seed: i64,
    ) -> Vec<ItemStack> {
        let (Some(table), Some(block)) = (self.loot.get(id), registry.state(state)) else {
            return vec![];
        };
        horizon_world::loot::query::root(id, || {
            match table.get("random_sequence").and_then(Value::as_str) {
                Some(sequence) => {
                    let random = sequences.get(sequence, seed);
                    self.interact_roll(table, &block.name, &block.properties, random)
                }
                None => self.interact_roll(table, &block.name, &block.properties, random),
            }
        })
    }
}

#[cfg(test)]
#[path = "loot_query_tests.rs"]
mod tests;
