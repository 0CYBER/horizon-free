//! Loot rolls with a mod listening to `enchanted_entity_loot`.

use super::Game;
use super::commands::Target;
use super::creatures::death_hurt_source;
use horizon_client::items::{ItemStack, Items};
use horizon_client::mobs::drops::{DeathRolls, roll_asked};
use horizon_client::random_sequences::RandomSequences;
use horizon_core::random::LegacyRandom;
use mod_api::entity::health::HurtSource;
use mod_api::{EntityRef, LivingRef};

/// A table's rolls with a mod listening to `enchanted_entity_loot`: the
/// randoms stay in the game, where the listener's host reaches them.
pub(in crate::game) struct ListenedRolls<'g, 'a, 'h> {
    game: &'g mut Game<'a>,
    entity: LivingRef,
    source: &'h HurtSource<'h>,
    /// `LootContextParams.ATTACKING_ENTITY`.
    attacker: Option<EntityRef>,
}

impl<'g, 'a, 'h> ListenedRolls<'g, 'a, 'h> {
    /// A death's rolls: its attacker is the blow's causing entity.
    pub(in crate::game) fn new(
        game: &'g mut Game<'a>,
        entity: LivingRef,
        source: &'h HurtSource<'h>,
    ) -> Self {
        let attacker = u32::try_from(source.causing)
            .ok()
            .filter(|id| *id != 0)
            .map(|id| EntityRef::new(entity.entity().epoch(), id));
        Self {
            game,
            entity,
            source,
            attacker,
        }
    }
}

impl DeathRolls for ListenedRolls<'_, '_, '_> {
    fn sequences(&mut self) -> &mut RandomSequences {
        &mut self.game.rand_sequences
    }
    fn level_random(&mut self) -> &mut LegacyRandom {
        &mut self.game.world.random
    }
    fn asks_enchanted_level(&self) -> bool {
        true
    }
    fn enchanted_level(&mut self, enchantment: &str, level: i32) -> i32 {
        let Ok(id) = mod_api::Identifier::parse(enchantment) else {
            return level;
        };
        let mut asked = level.max(0) as u32;
        self.game.native_enchanted_entity_loot(
            self.entity,
            self.source,
            self.attacker,
            &id,
            &mut asked,
        );
        i32::try_from(asked).unwrap_or(i32::MAX)
    }
}

/// `/loot ... kill`'s roll (`LootCommand.dropKillLoot`).
pub(in crate::game) struct KillRoll<'a> {
    /// The target's type and its table.
    pub(in crate::game) kind: &'a str,
    pub(in crate::game) table: &'a str,
    pub(in crate::game) target: Target,
    /// `LootContextParams.ATTACKING_ENTITY`: the command's source.
    pub(in crate::game) killer: Option<Target>,
    /// `LootContextParams.LAST_DAMAGE_PLAYER` is set.
    pub(in crate::game) by_player: bool,
    pub(in crate::game) held: Option<&'a ItemStack>,
    pub(in crate::game) keep_empty: bool,
}

impl Game<'_> {
    /// The entity `/loot ... kill` names, with a mod to ask: `EnchantedEntityLootEvent`
    /// needs a `LivingEntity`, and a table of any other entity is not asked about.
    fn listened_kill_target(&self, target: Target) -> Option<LivingRef> {
        if !mod_api::__private::listened(mod_api::HookId::EnchantedEntityLoot) {
            return None;
        }
        let (Target::Mob(_) | Target::Stand(_) | Target::Dragon(_) | Target::Guest(_)) = target
        else {
            return None;
        };
        self.effect_living_ref(Some(self.network_entity_id(target)?))
    }
    /// The drops of `/loot ... kill`, from the table's random sequence, else
    /// the level random. It reads levels as a death does, and a mod listening
    /// to `enchanted_entity_loot` is asked: the damage is `magic()`, as
    /// vanilla's, whose source names nobody, so the attacker is the command's.
    pub(in crate::game) fn roll_kill_loot(&mut self, kill: KillRoll<'_>) -> Vec<ItemStack> {
        let KillRoll {
            kind,
            table,
            target,
            killer,
            by_player,
            held,
            keep_empty,
        } = kill;
        // LootCommand.dropKillLoot supplies the selected live mob as THIS_ENTITY.
        let live_target = match target {
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|mob| mob.id == id)
                .map(|mob| horizon_client::mobs::drops::loot_entity(mob, None)),
            _ => None,
        };
        let loot = live_target.as_ref().and_then(|_| {
            horizon_client::structure_blocks::loot_data(
                &self.options.resources,
                &self.pack_files(),
                self.res.item_defaults(),
            )
        });
        let matcher = |predicate: &serde_json::Value, entity: &horizon_world::loot::LootEntity| {
            loot.as_ref()
                .is_some_and(|data| data.entity_matches(predicate, Some(entity)))
        };
        let live_target = live_target.as_ref().filter(|_| loot.is_some());
        // The listeners get the game, so the roll holds its own share of the items.
        let listened = self
            .server_items
            .clone()
            .zip(self.listened_kill_target(target));
        let sequence = world_items!(self).loot_sequence(table);
        let Some((items, entity)) = listened else {
            let items = world_items!(self);
            return match sequence {
                Some(id) => {
                    let random = self.rand_sequences.get(&id, self.level.seed);
                    if let Some(target) = live_target {
                        items.entity_drops_with_target(
                            kind, by_player, held, target, &matcher, random, keep_empty,
                        )
                    } else {
                        items.entity_drops_for_command(kind, by_player, held, random, keep_empty)
                    }
                }
                None => {
                    if let Some(target) = live_target {
                        items.entity_drops_with_target(
                            kind,
                            by_player,
                            held,
                            target,
                            &matcher,
                            &mut self.world.random,
                            keep_empty,
                        )
                    } else {
                        items.entity_drops_for_command(
                            kind,
                            by_player,
                            held,
                            &mut self.world.random,
                            keep_empty,
                        )
                    }
                }
            };
        };
        let epoch = entity.entity().epoch();
        let attacker = killer
            .and_then(|t| self.network_entity_id(t))
            .map(|id| EntityRef::new(epoch, id as u32));
        let seed = self.level.seed;
        let source = death_hurt_source("minecraft:magic", 0);
        let mut rolls = ListenedRolls {
            game: self,
            entity,
            source: &source,
            attacker,
        };
        let items: &Items = &items;
        match sequence {
            Some(id) => roll_asked(
                &mut rolls,
                |r| r.sequences().get(&id, seed),
                |random, ask| {
                    if let Some(target) = live_target {
                        items.entity_drops_asked_with_target(
                            kind, by_player, held, target, &matcher, random, ask, keep_empty,
                        )
                    } else {
                        items.entity_drops_asked(kind, by_player, held, random, ask, keep_empty)
                    }
                },
            ),
            None => roll_asked(
                &mut rolls,
                |r| r.level_random(),
                |random, ask| {
                    if let Some(target) = live_target {
                        items.entity_drops_asked_with_target(
                            kind, by_player, held, target, &matcher, random, ask, keep_empty,
                        )
                    } else {
                        items.entity_drops_asked(kind, by_player, held, random, ask, keep_empty)
                    }
                },
            ),
        }
    }
}
