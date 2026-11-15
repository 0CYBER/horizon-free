use super::*;
use horizon_client::items::BlockLootRoll;

impl GameLevel<'_> {
    /// A block's loot table reads an enchantment's level
    /// (`ApplyBonusCount.run`, `BonusLevelTableCondition.test`): NeoForge's
    /// `EnchantedBlockLootEvent`. Change `level` to change what that read
    /// uses. Server-side, authoritative levels only.
    #[inline(always)]
    pub(in crate::game) fn native_enchanted_block_loot(
        &self,
        call: super::fire::EnchantedBlockLootCall<'_>,
    ) {
        self.fire_enchanted_block_loot(call);
    }
    /// `Block.getDrops` of `state` at `pos` for `entity`, from its table's
    /// random sequence, else the level random, the mods having their say on
    /// the levels the table reads. The roll decides each read, the level
    /// fires the hook and the roll goes on with the answer; so the listeners
    /// can hold this level, the roll keeps the randoms until it ends.
    pub(in crate::game) fn roll_block_loot(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        entity: Option<EntityRef>,
        roll: BlockLootRoll<'_>,
    ) -> Vec<ItemStack> {
        let (items, registry, seed) = (self.items, self.registry, self.seed);
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::EnchantedBlockLoot)
        {
            let sequences = Some((&mut *self.rand_sequences, seed));
            return items.block_drops(registry, state, roll, &mut self.world.random, sequences);
        }
        let mut random = self.world.random.clone();
        let mut sequences = std::mem::take(&mut *self.rand_sequences);
        let tool = roll.held;
        let level = &*self;
        let levels = |enchantment: &str, mut read: u32| {
            if let Some(enchantment) = horizon_client::enchantment_levels::identifier(enchantment) {
                level.native_enchanted_block_loot(super::fire::EnchantedBlockLootCall {
                    dimension: level.dimension,
                    pos,
                    state,
                    tool,
                    enchantment: &enchantment,
                    level: &mut read,
                    entity,
                });
            }
            read
        };
        let roll = BlockLootRoll {
            levels: Some(&levels),
            ..roll
        };
        let drops = items.block_drops(
            registry,
            state,
            roll,
            &mut random,
            Some((&mut sequences, seed)),
        );
        self.world.random = random;
        *self.rand_sequences = sequences;
        drops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoApi;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_core::random::{LegacyRandom, RandomSource};
    use mod_api::{EnchantedBlockLootEvent, Hooks, LevelHost, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, PartialEq)]
    struct Read {
        pos: [i32; 3],
        state: mod_api::BlockState,
        tool: Option<String>,
        enchantment: String,
        level: u32,
        entity: Option<u32>,
    }

    /// Records every read and answers with `answer`, or leaves it.
    struct Fortune {
        answer: Option<u32>,
        reads: Arc<Mutex<Vec<Read>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Fortune {
        fn enchanted_block_loot(&self, _: &dyn LevelHost, event: &mut EnchantedBlockLootEvent<'_>) {
            self.reads.lock().unwrap().push(Read {
                pos: [event.pos.x, event.pos.y, event.pos.z],
                state: event.state,
                tool: event.tool.map(|tool| {
                    mod_api::__private::item_key_id(tool.item())
                        .unwrap()
                        .to_string()
                }),
                enchantment: event.enchantment.to_string(),
                level: event.level,
                entity: event.entity.map(|entity| entity.id()),
            });
            if let Some(answer) = self.answer {
                event.level = answer;
            }
        }
    }

    const POS: [i32; 3] = [1, 64, 1];
    const ROLLS: usize = 64;

    /// `ROLLS` player breaks of gravel: how many gave flint, and where the
    /// level random and the table's sequence stand after them.
    fn break_gravel(
        game: &mut Game<'_>,
        gravel: BlockStateId,
        tool: Option<&ItemStack>,
    ) -> (usize, i32, i32) {
        *game.world.random = LegacyRandom::new(7);
        game.rand_sequences = Default::default();
        let flint = (0..ROLLS)
            .flat_map(|_| game.block_loot_at(POS, gravel, tool, None))
            .filter(|stack| &*stack.item == "minecraft:flint")
            .count();
        let seed = game.level.seed;
        let sequence = game.rand_sequences.get("minecraft:blocks/gravel", seed);
        (flint, game.world.random.next_int(), sequence.next_int())
    }

    /// gravel.json's `table_bonus` is `[0.1, 0.14285715, 0.25, 1.0]`: at
    /// level 3 every break gives flint, at level 0 few do. The answer reaches
    /// the table on a player's break, a level-side break and a blast, each
    /// naming its breaker; a listener that answers nothing changes neither
    /// the drops nor either random.
    #[test]
    #[ignore = "daily"]
    fn a_listener_sets_the_level_a_block_table_reads() {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let gravel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:gravel")
                .unwrap();
            game.lan_acting_entity = Some(42);
            game.lan_acting = Some(1);
            let mut pick = ItemStack::new("minecraft:iron_pickaxe", 1);
            pick.enchant("minecraft:fortune", 3);
            let reads = Arc::new(Mutex::new(Vec::new()));
            let mods = |answer| {
                vec![ModEntry::new(
                    "fortune",
                    Box::new(Fortune {
                        answer,
                        reads: reads.clone(),
                    }),
                )]
                .into()
            };

            let unheard = break_gravel(game, gravel, None);
            assert!(unheard.0 < ROLLS / 2, "{unheard:?}");
            with_mods(mods(None), || {
                assert_eq!(break_gravel(game, gravel, None), unheard);
            });
            assert_eq!(reads.lock().unwrap().len(), ROLLS);
            assert_eq!(
                reads.lock().unwrap()[0],
                Read {
                    pos: POS,
                    state: gravel.try_into_api().unwrap(),
                    tool: None,
                    enchantment: "minecraft:fortune".into(),
                    level: 0,
                    entity: Some(42),
                }
            );
            reads.lock().unwrap().clear();

            game.lan_acting_entity = None;
            game.lan_acting = None;
            let origin = horizon_core::geometry::Vec3::ZERO;
            let mut host = horizon_client::motion::PlayerMotion::new(game.motion, origin).unwrap();
            host.entity_id = 7;
            game.player = Some(host);
            with_mods(mods(None), || {
                game.block_loot_at(POS, gravel, None, None);
            });
            game.player = None;
            assert_eq!(reads.lock().unwrap()[0].entity, Some(7));
            reads.lock().unwrap().clear();

            assert_eq!(break_gravel(game, gravel, Some(&pick)).0, ROLLS);
            with_mods(mods(Some(0)), || {
                assert!(break_gravel(game, gravel, Some(&pick)).0 < ROLLS / 2);
            });
            let first = reads.lock().unwrap().swap_remove(0);
            assert_eq!(first.level, 3);
            assert_eq!(first.tool.as_deref(), Some("minecraft:iron_pickaxe"));
            reads.lock().unwrap().clear();

            with_mods(mods(Some(3)), || {
                assert_eq!(break_gravel(game, gravel, None).0, ROLLS);
                reads.lock().unwrap().clear();

                game.dropped.clear();
                let at = BlockPos::new(POS[0], POS[1], POS[2]);
                game.with_level(|_, level, _| level.drop_resources_now(at, gravel));
                assert_eq!(&*game.dropped[0].stack.item, "minecraft:flint");
                let level_side = reads.lock().unwrap().swap_remove(0);
                assert_eq!(level_side.entity, None);
                reads.lock().unwrap().clear();

                game.combat.explosion_direct = Some(9);
                game.world.set(POS[0], POS[1], POS[2], gravel).unwrap();
                let center = POS.map(|c| f64::from(c) + 0.5);
                let blast = crate::game::explosion::BlockInteraction::Destroy;
                game.server_explode(center, 4.0, blast, false).unwrap();
                game.combat.explosion_direct = None;
                let blast = reads.lock().unwrap().swap_remove(0);
                assert_eq!((blast.tool, blast.entity), (None, Some(9)));
            });
        });
    }
}
