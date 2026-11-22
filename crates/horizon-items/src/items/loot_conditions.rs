//! A block loot table's conditions. `LootItemCondition.CODEC` is a
//! `RegistryFileCodec` over the reloadable `Registries.PREDICATE`, so a
//! `condition` string is a predicate id — the file
//! `data/<ns>/predicate/<path>.json`, which a pack replaces
//! (`minecraft:tool/can_silk_touch`) or adds (`mypack:my_silk`) — and an
//! inline object is the condition itself (`LootItemCondition.DIRECT_CODEC`,
//! dispatched on its `type`).
use super::LootContext;
use horizon_core::random::RandomSource;
use serde_json::Value;

impl<R: RandomSource> LootContext<'_, R> {
    /// The condition `id` names in `Registries.PREDICATE`. A root whose
    /// `predicate/` files were never loaded (`Items::for_tests`, a scratch
    /// root) keeps the two ids vanilla's own block tables name readable as
    /// their own kinds, as this engine read every string before the registry.
    pub(super) fn named_condition(&self, id: &str, random: &mut R) -> bool {
        match self.items.predicates.get(id) {
            Some(condition) => self.condition(condition, random),
            None => match id {
                "minecraft:tool/can_silk_touch" => {
                    self.enchantment_level("minecraft:silk_touch") > 0
                }
                // `match_tool` for the shears, which asks `shears_dig` (`stock_condition`)
                "minecraft:tool/can_shear" => self.condition(
                    &serde_json::json!({
                        "type": "minecraft:match_tool",
                        "predicate": {"items": "minecraft:shears"},
                    }),
                    random,
                ),
                _ => false,
            },
        }
    }

    /// A pool's or an entry's `condition` (`LootPool.CODEC`,
    /// `LootPoolEntryContainer.CODEC`) and the `conditions` list old pack data
    /// writes instead, all of which must hold.
    pub(super) fn conditions(&self, o: &Value, random: &mut R) -> bool {
        if let Some(c) = o.get("condition")
            && !self.condition(c, random)
        {
            return false;
        }
        o.get("conditions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .all(|c| self.condition(c, random))
    }
}
