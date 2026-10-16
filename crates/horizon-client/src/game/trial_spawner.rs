//! Trial spawners in the game: `TrialSpawner.tickServer` for every trial
//! spawner (`mobs::trial_spawner`), the block's state as it changes, the mobs
//! it adds and the rewards it ejects.
use super::{Game, natural_spawning::GameLevel, spawner::Planned};
use horizon_client::{
    Result,
    containers::BlockEntity,
    items::ItemStack,
    mobs::{
        Who,
        spawner::SpawnerSpawn,
        trial_spawner::{
            TrialEvent, TrialPlayer, TrialSpawner, TrialSpawnerLevel, TrialState, Uuid,
        },
    },
};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::nbt::Tag;
use std::collections::HashMap;

type Pos = (i32, i32, i32);

impl TrialSpawnerLevel for GameLevel<'_, '_> {
    fn game_time(&self) -> i64 {
        self.game.world.game_time
    }
    fn mob_spawning(&self) -> bool {
        self.game.ui.rules.bool("spawn_mobs")
    }
    fn players(&self) -> Vec<TrialPlayer> {
        let (Some(p), Some(view)) = (self.game.player.as_ref(), self.game.player_view()) else {
            return Vec::new();
        };
        vec![TrialPlayer {
            uuid: self.game.level.player_uuid.unwrap_or_default(),
            block: [p.position.x, p.position.y, p.position.z].map(|v| v.floor() as i32),
            eye: view.eye,
            creative: self.game.creative,
            spectator: self.game.spectator(),
        }]
    }
    fn visual_hit(&self, from: [f64; 3], to: [f64; 3]) -> Option<[i32; 3]> {
        let v = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
        self.w.visual_hit(v(from), v(to))
    }
    fn entity_block(&self, uuid: Uuid) -> Option<[i32; 3]> {
        let (id, _) = self.game.persist.uuids.iter().find(|(_, u)| **u == uuid)?;
        let m = self.game.mobs.iter().find(|m| m.id == *id)?;
        (!m.removed && m.alive())
            .then(|| [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i32))
    }
}

impl Game<'_> {
    /// Every trial spawner's server tick.
    pub(super) fn tick_trial_spawners(&mut self) -> Result<()> {
        let keys: Vec<Pos> = self
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::TrialSpawner(_)))
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            let state = self.world.block(key.0, key.1, key.2);
            let Some(block) = self.res.assets.registry.state(state).cloned() else {
                continue;
            };
            if block.name != "minecraft:trial_spawner" {
                continue;
            }
            let Some(BlockEntity::TrialSpawner(mut spawner)) =
                self.world.block_entities.remove(&key)
            else {
                continue;
            };
            let current = block
                .properties
                .get("trial_spawner_state")
                .and_then(|s| TrialState::from_name(s))
                .unwrap_or_default();
            let ominous = block.properties.get("ominous").is_some_and(|v| v == "true");
            let result = self.server_tick_trial_spawner(key, &mut spawner, current, ominous);
            self.world
                .block_entities
                .insert(key, BlockEntity::TrialSpawner(spawner));
            result?;
        }
        Ok(())
    }
    fn server_tick_trial_spawner(
        &mut self,
        key: Pos,
        spawner: &mut TrialSpawner,
        current: TrialState,
        ominous: bool,
    ) -> Result<()> {
        let pos = [key.0, key.1, key.2];
        let mut random = std::mem::replace(&mut self.world.random, LegacyRandom::new(0));
        let mut sequences = std::mem::take(&mut self.rand_sequences);
        let mut events = Vec::new();
        let mut planned: Vec<Planned> = Vec::new();
        let mut uuids: Vec<(u32, Uuid)> = Vec::new();
        let mut ejected: Vec<(ItemStack, [f64; 3])> = Vec::new();
        let next = {
            let level = self.natural_level();
            let loot = horizon_client::structure_blocks::loot_data(&self.options.resources);
            let seed = self.level.seed;
            let mut spawn = |l: &GameLevel, r: &SpawnerSpawn| {
                let added = l.game.plan_spawner_mob(l, r, &mut planned)?;
                let uuid = match planned.last_mut() {
                    Some(Planned::Loaded(Tag::Compound(tag))) => {
                        let tag_uuid =
                            horizon_client::entity_nbt::uuid(&Tag::Compound(tag.clone()));
                        tag_uuid.unwrap_or_else(|| {
                            let uuid = horizon_client::save::random_uuid();
                            tag.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
                            uuid
                        })
                    }
                    _ => horizon_client::save::random_uuid(),
                };
                uuids.push((added.id, uuid));
                Some(uuid)
            };
            // TrialSpawner.ejectReward: the table's items, each thrown up out
            // of the spawner (DefaultDispenseItemBehavior.spawnItem, accuracy 2).
            let mut eject = |table: &str, random: &mut LegacyRandom| {
                let Some(loot) = &loot else {
                    return false;
                };
                let drops = match loot.random_sequence(table) {
                    Some(s) => loot.roll_split(table, sequences.get(s, seed)),
                    None => loot.roll_split(table, random),
                };
                for stack in &drops {
                    // The power, which only scales the direction's x and z (0 going up).
                    random.next_double();
                    let spread = 0.017_227_5 * 2.0;
                    let v = [
                        random.triangle(0.0, spread),
                        random.triangle(0.2, spread),
                        random.triangle(0.0, spread),
                    ];
                    ejected.push((
                        horizon_client::structure_blocks::loot_item_with_components(stack),
                        v,
                    ));
                }
                !drops.is_empty()
            };
            spawner.server_tick(
                &level,
                &mut random,
                pos,
                current,
                ominous,
                &mut spawn,
                &mut eject,
                &mut events,
            )
        };
        self.world.random = random;
        self.rand_sequences = sequences;
        let ids = self.add_planned(planned);
        let mut by_uuid = HashMap::new();
        for (provisional, uuid) in uuids {
            if let Some(id) = ids.get(&provisional) {
                self.persist.uuids.insert(*id, uuid);
                by_uuid.insert(uuid, *id);
            }
        }
        let center = pos.map(|v| f64::from(v) + 0.5);
        // Vec3.atBottomCenterOf(pos).relative(UP, 1.2), lowered by spawnItem.
        let above = [center[0], center[1] + 0.7 - 0.125, center[2]];
        for (stack, v) in ejected {
            self.spawn_item(stack, above, v, 0);
        }
        if next != current || ominous != spawner.is_ominous {
            self.set_trial_spawner_state(key, next, spawner.is_ominous)?;
        }
        for event in events {
            match event {
                TrialEvent::BlockUpdated => self.mark_block_entity_updated(key),
                TrialEvent::LevelEvent(id, at, data) => self.level_event_at(id, at, data),
                TrialEvent::Sound(name) => {
                    self.audio
                        .play(&format!("minecraft:{name}"), Some(center), 1.0, 1.0)
                }
                // Written with the state above.
                TrialEvent::OminousSet(_) => {}
                TrialEvent::EntityPlace(uuid, at) => {
                    let who = by_uuid.get(&uuid).map(|id| Who::Mob(*id));
                    self.post_block_game_event("minecraft:entity_place", at, who, None);
                }
            }
        }
        Ok(())
    }
    /// `setState` (and `removeOminous`): the block's state and ominous flag.
    fn set_trial_spawner_state(
        &mut self,
        key: Pos,
        state: TrialState,
        ominous: bool,
    ) -> Result<()> {
        let current = self.world.block(key.0, key.1, key.2);
        let Some(mut block) = self.res.assets.registry.state(current).cloned() else {
            return Ok(());
        };
        block
            .properties
            .insert("trial_spawner_state".into(), state.name().into());
        block
            .properties
            .insert("ominous".into(), ominous.to_string());
        let id = self.res.assets.registry.resolve(&block)?;
        self.level_set([key.0, key.1, key.2], id)
    }
    /// `sendBlockUpdated` of a block entity: the clients load its new update tag.
    pub(super) fn mark_block_entity_updated(&mut self, key: Pos) {
        if let Some(log) = self.world.block_log.as_mut() {
            log.push(key);
        }
        self.world.modified.insert((key.0 >> 4, key.2 >> 4));
    }
}
