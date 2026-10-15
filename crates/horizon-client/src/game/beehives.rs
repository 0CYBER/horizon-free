//! Beehives and bee nests in the game (`crate::beehive` holds their logic):
//! their server tick, bees going in, and the bees let out, angry, when a
//! player breaks the hive or fire reaches it.
use super::Game;
use horizon_client::{
    beehive::{self, Beehive, HiveHost, Release},
    containers::BlockEntity,
    mobs::{Mob, Who, bee},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{nbt::Tag, registry::BlockStateId};

struct GameHive<'g, 'a> {
    game: &'g mut Game<'a>,
}
impl HiveHost for GameHive<'_, '_> {
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.game.world.random
    }
    fn bees_stay_in_hive(&self, _at: [i32; 3]) -> bool {
        bee::bees_stay_in_hive(&self.game.mob_world_base(&[]))
    }
    fn blocked(&self, at: [i32; 3]) -> bool {
        !self
            .game
            .mob_world_base(&[])
            .block_collision_boxes(at[0], at[1], at[2])
            .is_empty()
    }
    fn set_honey_level(&mut self, at: [i32; 3], level: i32) {
        let state = self.game.world.block(at[0], at[1], at[2]);
        if let Some(state) = self.game.hive_with_honey(state, level) {
            let _ = self.game.level_set(at, state);
        }
    }
    fn load(&mut self, tag: &Tag) -> Option<Mob> {
        let kind = horizon_client::entity_nbt::kind(tag)?;
        if !self
            .game
            .res
            .mob_data
            .entity_has("minecraft:beehive_inhabitors", &kind)
        {
            return None;
        }
        let id = self.game.load_mob(tag.clone(), None)?;
        let i = self.game.mobs.iter().position(|m| m.id == id)?;
        Some(self.game.mobs.remove(i))
    }
    fn add(&mut self, m: Mob) -> bool {
        self.game.mobs.push(m);
        true
    }
    fn sound(&mut self, name: &'static str, at: [f64; 3]) {
        self.game
            .audio
            .play(&format!("minecraft:{name}"), Some(at), 1.0, 1.0);
    }
}

impl Game<'_> {
    fn hive_with_honey(&self, state: BlockStateId, level: i32) -> Option<BlockStateId> {
        let registry = self.res.assets.registry;
        let mut s = registry.state(state)?.clone();
        s.properties
            .insert("honey_level".to_owned(), level.to_string());
        registry.resolve(&s).ok()
    }
    /// The hive's honey level and front, from its block state.
    pub(super) fn hive_state(&self, at: [i32; 3]) -> Option<(Option<i32>, [i32; 2])> {
        let state = self.world.block(at[0], at[1], at[2]);
        let name = &self.res.facts.get(state).name;
        if !self
            .res
            .items
            .block_tags
            .get("minecraft:beehives")
            .is_some_and(|t| t.contains(&**name))
        {
            return None;
        }
        let props = &self.res.assets.registry.state(state)?.properties;
        let honey = props.get("honey_level").and_then(|v| v.parse().ok());
        let facing = beehive::facing_step(props.get("facing").map(String::as_str));
        Some((honey, facing))
    }
    /// `BeehiveBlockEntity.serverTick` for every hive, after an emergency
    /// exit for those with fire beside them (`BeehiveBlock.updateShape`).
    pub(super) fn tick_beehives(&mut self) {
        let mut keys: Vec<(i32, i32, i32)> = self
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::Beehive(_)))
            .map(|(k, _)| *k)
            .collect();
        keys.sort_unstable();
        for key in keys {
            let at = [key.0, key.1, key.2];
            let Some((honey, facing)) = self.hive_state(at) else {
                continue;
            };
            let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.remove(&key)
            else {
                continue;
            };
            let fire = [
                [1, 0, 0],
                [-1, 0, 0],
                [0, 1, 0],
                [0, -1, 0],
                [0, 0, 1],
                [0, 0, -1],
            ]
            .iter()
            .any(|d| {
                let state = self.world.block(at[0] + d[0], at[1] + d[1], at[2] + d[2]);
                &*self.res.facts.get(state).name == "minecraft:fire"
            });
            let mut host = GameHive { game: self };
            if fire && !hive.bees.is_empty() {
                hive.release_all(at, facing, honey, Release::Emergency, &mut host);
            }
            hive.server_tick(at, facing, honey, &mut host);
            self.world
                .block_entities
                .insert(key, BlockEntity::Beehive(hive));
        }
    }
    /// `BeehiveBlockEntity.addOccupant` for a bee its `BeeEnterHiveGoal`
    /// sent in: off its lead, its tag stored, the bee gone.
    pub(super) fn bee_enter_hive(&mut self, mob: u32, at: [i32; 3]) {
        let key = (at[0], at[1], at[2]);
        let Some(i) = self
            .mobs
            .iter()
            .position(|m| m.id == mob && m.alive() && !m.removed)
        else {
            return;
        };
        if !matches!(self.world.block_entities.get(&key), Some(BlockEntity::Beehive(h)) if !h.full())
        {
            return;
        }
        if self.mobs[i].extra.leash.is_some() {
            self.break_leash(mob, true, false);
            self.mobs[i].extra.leash = None;
        }
        self.mobs[i].extra.mount = None;
        let tag = self.mob_tree_nbt(i);
        let flower = bee::get(&self.mobs[i]).and_then(|b| b.flower);
        let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.remove(&key) else {
            return;
        };
        let stored = hive.add_occupant(at, tag, flower, &mut GameHive { game: self });
        self.world
            .block_entities
            .insert(key, BlockEntity::Beehive(hive));
        if stored && let Some(m) = self.mobs.iter_mut().find(|m| m.id == mob) {
            m.removed = true;
        }
    }
    /// `BeehiveBlockEntity.emptyAllLivingFromHive` with no player about.
    pub(super) fn empty_hive(&mut self, at: [i32; 3], facing: [i32; 2], status: Release) {
        let key = (at[0], at[1], at[2]);
        let Some(BlockEntity::Beehive(mut hive)) = self.world.block_entities.remove(&key) else {
            return;
        };
        hive.release_all(at, facing, None, status, &mut GameHive { game: self });
        self.world
            .block_entities
            .insert(key, BlockEntity::Beehive(hive));
    }
    /// `BeehiveBlock.playerDestroy` without silk touch: the bees come out
    /// angry at the player unless smoke calms them, then
    /// `angerNearbyBees`.
    pub(super) fn hive_broken_by_player(
        &mut self,
        at: [i32; 3],
        mut hive: Beehive,
        honey: Option<i32>,
        facing: [i32; 2],
    ) {
        let released = hive.release_all(
            at,
            facing,
            honey,
            Release::Emergency,
            &mut GameHive { game: self },
        );
        let sedated = beehive::smokey(&self.mob_world_base(&[]), at);
        if let Some(p) = self.player_view().map(|p| p.position) {
            for m in self.mobs.iter_mut().filter(|m| released.contains(&m.id)) {
                if m.distance_sqr(p) <= 16.0 {
                    if sedated {
                        bee::set_stay_out_of_hive_countdown(m);
                    } else {
                        m.ai.target = Some(Who::Player);
                    }
                }
            }
        }
        self.anger_nearby_bees(at);
    }
    /// `BeehiveBlock.angerNearbyBees`: bees within 8 blocks (6 up and down)
    /// without a target go for a player in the same box.
    fn anger_nearby_bees(&mut self, at: [i32; 3]) {
        let min = Vec3::new(
            f64::from(at[0]) - 8.0,
            f64::from(at[1]) - 6.0,
            f64::from(at[2]) - 8.0,
        );
        let area = Aabb::new(min, Vec3::new(min.x + 17.0, min.y + 13.0, min.z + 17.0));
        if !self
            .player_view()
            .is_some_and(|p| p.bounds.intersects(area))
        {
            return;
        }
        let mut bees: Vec<usize> = (0..self.mobs.len())
            .filter(|&i| {
                let m = &self.mobs[i];
                m.species() == horizon_client::mobs::Species::Bee
                    && !m.removed
                    && m.bounds().intersects(area)
            })
            .collect();
        bees.retain(|&i| self.mobs[i].ai.target.is_none());
        for i in bees {
            // Util.getRandom over the one player.
            let _ = self.world.random.next_int_bound(1);
            self.mobs[i].ai.target = Some(Who::Player);
        }
    }
}
