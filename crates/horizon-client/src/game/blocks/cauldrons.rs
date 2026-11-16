//! Using an item on a cauldron (`CauldronInteraction`): what the click does, and doing it.
use super::*;

/// `CauldronInteractions.isUnderWater`: the *fluid* above the cauldron is in
/// `FluidTags.WATER`. A waterlogged slab or stair, kelp, seagrass and a bubble
/// column are water there even though the block is not named `water`.
pub(super) fn is_under_water(facts: &BlockFacts, above: BlockStateId) -> bool {
    facts.get(above).fluid == Fluid::Water
}

impl Game<'_> {
    /// What the cauldron at `pos` answers to `held` on a client, which only
    /// asks (`CauldronInteractions` acts `if (!level.isClientSide())`). A
    /// sneak with something in hand skips `useItemOn`
    /// (`Player.isSecondaryUseActive`).
    pub(super) fn cauldron_outcome(
        &self,
        pos: [i32; 3],
        held: Option<&ItemStack>,
    ) -> horizon_client::cauldron::Outcome {
        self.client_cauldron_prediction(pos, held).0
    }
    /// `CauldronInteractions` for the held item aimed at a cauldron.
    pub(super) fn use_on_cauldron(&mut self, pos: [i32; 3]) -> Result<bool> {
        use horizon_client::cauldron::{self, Outcome};
        let registry = self.pack.assets.registry;
        let Some((state, under_water)) = self.cauldron_at(pos) else {
            return Ok(false);
        };
        let Some(held) = self.held() else {
            return Ok(false);
        };
        let removes_dye = cauldron::removes_dye(self.items(), &held.item);
        // `Stats.ITEM_USED.get(itemInHand.getItem())`: the item the hand held
        // before the interaction swaps it.
        let used_item = held.item.clone();
        let table = cauldron_interactions::snapshot();
        let items = self.items();
        let entry = table.select(&state.name, &held.item, removes_dye, |tag| {
            items.item_in_tag(tag, &held.item)
        });
        if let Some(cauldron::interactions::Entry::Mod(callback)) = &entry {
            return Ok(self.use_mod_cauldron(pos, under_water, callback, &table));
        }

        let slot = self.held_slot();
        let player = self.native_acting_id();
        if self.transactions.is_empty() {
            use transactions::DeferredEffect;
            let mut hand = powder_snow::HeldSlot {
                game: self,
                slot,
                dropper: None,
            };
            let result =
                cauldron::interact_selected(&state, under_water, entry.as_ref(), &mut hand);
            if result.outcome == Outcome::Pass {
                return Ok(false);
            }
            if let Some(stat) = result.stat {
                self.defer_effect(DeferredEffect::CustomStat {
                    player,
                    key: stat,
                    amount: 1,
                });
            }
            // The bottle and bucket interactions award `ITEM_USED` right after
            // their cauldron statistic.
            if result.item_used {
                self.defer_effect(DeferredEffect::ItemStat {
                    player,
                    stat: "used",
                    item: used_item,
                    amount: 1,
                });
            }
            if let Some(block) = result.block
                && let Ok(new) = registry.resolve(&block)
            {
                self.level_set(pos, new)?;
                if result.lower_fill_level {
                    self.post_block_game_event("minecraft:block_change", pos, None, Some(new));
                }
            }
            if let Some(sound) = result.sound {
                self.defer_effect(DeferredEffect::Sound {
                    sound,
                    at: pos.map(|v| f64::from(v) + 0.5),
                    volume: 1.0,
                    pitch: 1.0,
                });
            }
            if let Some(event) = result.fluid_event {
                self.post_block_game_event(event, pos, None, None);
            }
            return Ok(true);
        }
        let outcome = mod_api::run_transaction(self, |game, _| {
            use transactions::DeferredEffect;
            game.journal_player(player);
            let mut hand = powder_snow::HeldSlot {
                game,
                slot,
                dropper: Some(player),
            };
            let result =
                cauldron::interact_selected(&state, under_water, entry.as_ref(), &mut hand);
            if result.outcome == Outcome::Pass {
                return Err(());
            }
            if let Some(stat) = result.stat {
                game.defer_effect(DeferredEffect::CustomStat {
                    player,
                    key: stat,
                    amount: 1,
                });
            }
            // The bottle and bucket interactions award `ITEM_USED` right after
            // their cauldron statistic.
            if result.item_used {
                game.defer_effect(DeferredEffect::ItemStat {
                    player,
                    stat: "used",
                    item: used_item.clone(),
                    amount: 1,
                });
            }
            if let Some(block) = result.block {
                let new = registry.resolve(&block).map_err(|_| ())?;
                if !game.journal_set_block(pos, new) {
                    return Err(());
                }
                if result.lower_fill_level {
                    game.post_block_game_event("minecraft:block_change", pos, None, Some(new));
                }
            }
            if let Some(sound) = result.sound {
                game.defer_effect(DeferredEffect::Sound {
                    sound,
                    at: pos.map(|v| f64::from(v) + 0.5),
                    volume: 1.0,
                    pitch: 1.0,
                });
            }
            if let Some(event) = result.fluid_event {
                game.post_block_game_event(event, pos, None, None);
            }
            Ok(())
        });
        Ok(outcome.is_ok())
    }
}
