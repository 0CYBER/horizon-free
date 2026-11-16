//! A mod's own advancement criteria triggers (docs/modding/src/how-to/native-mods.md,
//! "Advancement triggers"). An advancement of a content mod or a world's data
//! pack names a trigger of the mod's (`mymod:pipe_connected`); the tree keeps
//! it as it keeps vanilla's, and the mod fires it where its event happens.
use super::{Game, advancements::item_view};
use horizon_client::advancements::{self as adv, EntityView};
use mod_api::{
    CriterionContext, CriterionTest, CriterionTestError, Identifier, PlayerRef,
    TriggerCriterionError,
};
use serde_json::Value;
use std::{cell::Cell, sync::Arc};

/// One open criterion of a mod's trigger, as the mod's test sees it.
struct NativeCriterion<'a> {
    conditions: &'a Value,
    world: &'a dyn adv::World,
}

impl CriterionContext for NativeCriterion<'_> {
    fn conditions_json(&self) -> String {
        self.conditions.to_string()
    }

    /// Whether `stack` passes the item predicate under `key` in the
    /// conditions, as the game's own triggers read an `ItemPredicate`:
    /// `items` (an id, a list or a `#tag`), `count`, the `potion_contents`
    /// and `damage` components, and the `enchantments` and `damage`
    /// predicates; other components and predicates aren't checked. A missing
    /// key passes, as an absent `Optional<ItemPredicate>` does.
    fn item(&self, key: &str, stack: &mod_api::ItemStack) -> bool {
        let Ok(stack) = super::api_values::stack_value(stack) else {
            return false;
        };
        adv::item(&self.conditions[key], &item_view(&stack), self.world)
    }
}

impl Game<'_> {
    /// Fires the mod's criteria trigger `trigger` (`mymod:pipe_connected`)
    /// for `player`: vanilla's
    /// `SimpleCriterionTrigger.trigger`. Each criterion of theirs with that
    /// trigger, not yet granted, of an advancement not yet done, is tested
    /// by `test` and then by its `player` predicate; those that pass are
    /// granted, and an advancement they complete is awarded with its toast,
    /// chat line and rewards, as the game's own triggers award.
    ///
    /// The acting player is reached inside their hook; another live player
    /// is reached in their own level, including during a guest's turn. An
    /// outer player displaced by a nested turn is unavailable. `Ok(())`
    /// means the player was reached, whether or not anything was granted.
    /// A stale/offline/displaced handle, a remote client, or a `minecraft:`
    /// trigger or empty id is refused with a typed reason.
    ///
    /// `test` is lent to this call only. A failed test stops further testing
    /// and reports `Test`; earlier matching criteria may already be granted.
    /// Wasm's generated resource route exists, but a callback into the group
    /// currently firing the trigger is refused by the runtime's reentry guard.
    pub(super) fn player_trigger_criterion(
        &mut self,
        player: PlayerRef,
        trigger: &Identifier,
        test: Arc<dyn CriterionTest>,
    ) -> Result<(), TriggerCriterionError> {
        if trigger.namespace().is_empty()
            || trigger.path().is_empty()
            || trigger.namespace() == "minecraft"
        {
            return Err(TriggerCriterionError::InvalidTrigger);
        }
        if self.remote.is_some() {
            return Err(TriggerCriterionError::NotServer);
        }
        let target = self
            .player_handle_read(player)
            .and_then(|_| self.native_entity_target(player.entity()))
            .ok_or(TriggerCriterionError::PlayerUnavailable)?;
        // In their level: the predicates read where they are, and a reward
        // that doesn't fit drops there. Reuse the existing player swap and
        // advancement trigger, not a second predicate or awarding path.
        self.for_player(target, |g| {
            g.trigger_mod_criteria(&trigger.to_string(), test.as_ref())
        })
        .ok_or(TriggerCriterionError::PlayerUnavailable)?
    }

    /// Fire only `default_block_use` for a use a mod handled.
    pub(super) fn player_trigger_block_use(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
    ) -> Result<(), TriggerCriterionError> {
        self.with_block_use_player(player, pos, |g| {
            g.trigger_default_block_use(pos);
        })
    }

    /// Fire only `item_used_on_block` against the pre-use snapshot.
    pub(super) fn player_trigger_item_used_on_block(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        original: &horizon_client::items::ItemStack,
    ) -> Result<(), TriggerCriterionError> {
        self.with_block_use_player(player, pos, |g| {
            g.trigger_block("minecraft:item_used_on_block", pos, original);
        })
    }

    /// Resolve the player as `player_trigger_criterion` does, refuse an unloaded chunk, then
    /// fire in the player's own level, so predicates read where they are.
    fn with_block_use_player(
        &mut self,
        player: PlayerRef,
        pos: [i32; 3],
        fire: impl FnOnce(&mut Self),
    ) -> Result<(), TriggerCriterionError> {
        if self.remote.is_some() {
            return Err(TriggerCriterionError::NotServer);
        }
        let target = self
            .player_handle_read(player)
            .and_then(|_| self.native_entity_target(player.entity()))
            .ok_or(TriggerCriterionError::PlayerUnavailable)?;
        self.for_player(target, |g| {
            if !g.world.chunks.contains_key(&(pos[0] >> 4, pos[2] >> 4)) {
                return Err(TriggerCriterionError::UnloadedChunk);
            }
            fire(g);
            Ok(())
        })
        .ok_or(TriggerCriterionError::PlayerUnavailable)?
    }

    /// `trigger` for a mod's trigger, for the player in place.
    fn trigger_mod_criteria(
        &mut self,
        trigger: &str,
        test: &dyn CriterionTest,
    ) -> Result<(), TriggerCriterionError> {
        let me = self.player_entity_for(trigger);
        let refused = Cell::new(None);
        self.trigger(trigger, |conditions, world| {
            if refused.get().is_some() {
                return false;
            }
            match passes(conditions, &me, world, test) {
                Ok(pass) => pass,
                Err(error) => {
                    refused.set(Some(error));
                    false
                }
            }
        });
        refused
            .get()
            .map_or(Ok(()), |error| Err(TriggerCriterionError::Test(error)))
    }
}

/// `SimpleCriterionTrigger.trigger`'s test of one criterion for the player
/// `me`: the mod's matcher, then the `player` predicate, in the player's own
/// context (`EntityPredicate.createContext(player, player)`).
fn passes(
    conditions: &Value,
    me: &EntityView,
    world: &dyn adv::World,
    test: &dyn CriterionTest,
) -> Result<bool, CriterionTestError> {
    let player = adv::Entities {
        this: Some(me),
        origin: me.position,
        tool: None,
        block_pos: None,
    };
    Ok(test.test(&NativeCriterion { conditions, world })?
        && adv::conditions(&conditions["player"], &player, world))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::ItemStack;
    use serde_json::json;
    use std::collections::BTreeMap;

    struct Plains;
    impl adv::World for Plains {
        fn dimension(&self) -> &str {
            "minecraft:overworld"
        }
        fn weather(&self) -> (bool, bool) {
            (false, false)
        }
        fn biome(&self, _: [i32; 3]) -> String {
            "minecraft:plains".into()
        }
        fn structures(&self, _: [i32; 3]) -> Vec<String> {
            Vec::new()
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            ("minecraft:air".into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            true
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            15
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, registry: &str, tag: &str, id: &str) -> bool {
            registry == "item" && tag == "c:ingots/copper" && id == "minecraft:copper_ingot"
        }
    }

    /// A mod's `pipe_connected` criterion: the pipe's item by a convention
    /// tag, and only for a player on fire (a loot condition list, as
    /// vanilla's advancements write `player`).
    fn criterion() -> Value {
        json!({
            "pipe": {"items": "#c:ingots/copper", "count": {"min": 2}},
            "player": [{
                "condition": "minecraft:entity_properties",
                "entity": "this",
                "predicate": {"flags": {"is_on_fire": true}}
            }]
        })
    }

    struct ItemTest {
        key: &'static str,
        stack: ItemStack,
    }

    impl CriterionTest for ItemTest {
        fn test(&self, criterion: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
            Ok(criterion.item(self.key, &self.stack))
        }
    }

    #[test]
    fn a_mods_criterion_takes_its_own_test_and_the_player_predicate() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
        mod_platform::__private::install(Default::default());
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:copper_ingot", "minecraft:iron_ingot"])
            .unwrap();
        let burning = EntityView {
            kind: "minecraft:player".into(),
            on_fire: true,
            ..EntityView::default()
        };
        let calm = EntityView {
            on_fire: false,
            ..burning.clone()
        };
        let copper = ItemStack::new(
            mod_api::__private::item_key("minecraft:copper_ingot").unwrap(),
            2,
        )
        .unwrap();
        let pipe = |stack: &ItemStack| ItemTest {
            key: "pipe",
            stack: stack.clone(),
        };
        let c = criterion();
        assert_eq!(passes(&c, &burning, &Plains, &pipe(&copper)), Ok(true));
        assert_eq!(
            passes(&c, &calm, &Plains, &pipe(&copper)),
            Ok(false),
            "the player predicate"
        );
        let one = ItemStack::new(
            mod_api::__private::item_key("minecraft:copper_ingot").unwrap(),
            1,
        )
        .unwrap();
        assert_eq!(
            passes(&c, &burning, &Plains, &pipe(&one)),
            Ok(false),
            "the item's count"
        );
        let iron = ItemStack::new(
            mod_api::__private::item_key("minecraft:iron_ingot").unwrap(),
            2,
        )
        .unwrap();
        assert_eq!(
            passes(&c, &burning, &Plains, &pipe(&iron)),
            Ok(false),
            "the tag"
        );
        // A criterion without the key or any conditions passes the item test.
        let any = ItemTest {
            key: "filter",
            stack: iron,
        };
        assert_eq!(passes(&c, &burning, &Plains, &any), Ok(true));
        assert_eq!(passes(&Value::Null, &calm, &Plains, &any), Ok(true));
        let context = NativeCriterion {
            conditions: &c,
            world: &Plains,
        };
        assert_eq!(
            serde_json::from_str::<Value>(&context.conditions_json()).unwrap(),
            c
        );
        assert_eq!(
            NativeCriterion {
                conditions: &Value::Null,
                world: &Plains
            }
            .conditions_json(),
            "null"
        );
    }

    #[test]
    fn an_unavailable_test_is_not_a_nonmatching_criterion() {
        struct Unavailable;
        impl CriterionTest for Unavailable {
            fn test(&self, _: &dyn CriterionContext) -> Result<bool, CriterionTestError> {
                Err(CriterionTestError::Unavailable)
            }
        }
        assert_eq!(
            passes(&criterion(), &EntityView::default(), &Plains, &Unavailable),
            Err(CriterionTestError::Unavailable)
        );
    }
}
