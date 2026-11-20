//! What villagers do with each other and with the player: gossip and
//! reputation, sharing food and wares, picking up dropped items, making
//! babies, calling iron golems and the prices they ask. The logic runs in
//! `horizon_gameplay::villagers` on the game's `VillagerLevel`; what stays here
//! needs the game itself: the particles, the player UUIDs, the merchant menu, the
//! sleep hooks and the wake-up inside a mob pass.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_client::{
    gossip::{self, GossipType},
    items::Items,
    mobs::{Mob, MobType, MobWorld, Seen, spawning::BiomeFacts, villager, villager_brain},
    stations::StationData,
};
use horizon_core::geometry::Vec3;
use horizon_core::position::BlockPos;
use horizon_gameplay::villager_level::VillagerLevel;
use std::{collections::HashSet, sync::Arc};

impl Game<'_> {
    fn villager_index(&self, id: u32) -> Option<usize> {
        horizon_gameplay::villagers::villager_index(self, id)
    }
    fn item_max_stack(&self, item: &str) -> u32 {
        horizon_gameplay::villagers::item_max_stack(self.items(), item)
    }
    /// `UseBonemeal.tick`'s `BoneMealItem.growCrop`: villager `id`'s first
    /// bone meal on `pos`, with level event 1505 and the next crop to work
    /// on when it grew.
    pub(super) fn villager_bonemeal(&mut self, id: u32, pos: BlockPos) {
        horizon_gameplay::villagers::villager_bonemeal(self, id, pos);
    }
    /// `WorkAtComposter.useWorkstation`: at a composter the farmer makes
    /// bread, then composts seeds.
    pub(super) fn villager_work_at_composter(&mut self, id: u32, pos: [i32; 3]) {
        horizon_gameplay::villagers::villager_work_at_composter(self, id, pos);
    }
    /// `WorkAtComposter.makeBread`: up to three loaves from nine wheat while
    /// the villager holds at most 36 bread; what does not fit drops.
    fn villager_make_bread(&mut self, i: usize) {
        horizon_gameplay::villagers::villager_make_bread(self, i);
    }
    /// `BehaviorUtils.throwItem`: toss a stack from a villager's hand toward
    /// a point.
    fn villager_throw(&mut self, from: Vec3, eye: f64, to: Vec3, stack: ItemStack) {
        horizon_gameplay::villagers::villager_throw(self, from, eye, to, stack);
    }

    /// `TradeWithVillager.tick` for one pair: gossip (at most once in 1200
    /// ticks for both), then surplus food and wanted wares tossed over.
    pub(super) fn villager_meet(&mut self, a: u32, b: u32) {
        let new_gossip = horizon_gameplay::villagers::villager_meet(self, a, b);
        // Villager.gossip: a newly heard gossip refreshes the trading
        // player's prices (`updateSpecialPrices`) right away.
        if new_gossip > 0 {
            self.villager_prices_refresh(a);
        }
    }

    /// `Villager.updateSpecialPrices` for an open menu and tracked guests.
    fn villager_prices_refresh(&mut self, id: u32) {
        if self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .is_some_and(|m| m.mob == Some(id))
        {
            let reputation = self.villager_reputation(id);
            self.menu_session(|menu, env, _| {
                if let Some(session) = menu.merchant.as_mut() {
                    session.reputation = reputation;
                    session.update_special_prices();
                }
                menu.merchant_update(env);
                Some(true)
            });
        }
        self.lan_merchant_reputation(id);
    }

    /// `Villager.spawnGolemIfNeeded`: when enough villagers nearby that
    /// slept recently and saw no golem agree, an iron golem appears.
    pub(super) fn villager_golem_check(&mut self, id: u32, needed: usize) {
        horizon_gameplay::villagers::villager_golem_check(self, id, needed);
    }
    /// `SpawnUtil.trySpawnMob(IRON_GOLEM, ...)`: a golem near `start`, with
    /// the ambient sound `trySpawnMob` plays.
    fn try_spawn_iron_golem(&mut self, start: [i32; 3]) -> bool {
        horizon_gameplay::villagers::try_spawn_iron_golem(self, start)
    }

    /// `VillagerMakeLove.tryToGiveBirth`: both eat, a free bed within 48
    /// blocks is taken for the child, who is a baby villager of one of
    /// three villager types.
    pub(super) fn villager_birth(
        &mut self,
        parent: u32,
        partner: u32,
        child: Option<([i32; 3], Option<u8>)>,
    ) {
        horizon_gameplay::villagers::villager_birth(self, parent, partner, child);
    }
    /// `Level.broadcastEntityEvent` of an event whose `handleEntityEvent`
    /// draws particles about the mob (villager 12, 13 and 14, taming 6 and 7,
    /// love 18): the guests get the byte, and this game draws it as its own
    /// client would.
    pub(super) fn mob_event_particles(&mut self, id: u32, event: i8) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        self.lan_entity_event(id as i32, event, at);
        self.entity_event_particles(id, event);
    }
    /// `handleEntityEvent(event)`'s particles about mob `id`
    /// (`Villager`, `TamableAnimal`, `AbstractHorse`, `Animal`, `Allay`).
    pub(super) fn entity_event_particles(&mut self, id: u32, event: i8) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        if let Some((kind, burst)) =
            horizon_client::particles::entity_event_burst(&m.kind.id, event as u8)
        {
            self.particles_around_mob(id, kind, burst);
        }
    }
    /// `particles::around_self` about mob `id`.
    pub(super) fn particles_around_mob(
        &mut self,
        id: u32,
        kind: &'static str,
        burst: horizon_client::particles::Burst,
    ) {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let feet = [m.position.x, m.position.y, m.position.z];
        let size = [f64::from(m.width()), f64::from(m.height())];
        self.particles_around(kind, feet, size, burst);
    }
    /// `particles::around_self` on the client: the particle engine's random
    /// stands in for the client entity's, so no server draw moves.
    pub(super) fn particles_around(
        &mut self,
        kind: &'static str,
        feet: [f64; 3],
        size: [f64; 2],
        burst: horizon_client::particles::Burst,
    ) {
        let Some(ty) = horizon_client::particles::ParticleType::from_name(kind) else {
            return;
        };
        let options = horizon_client::particles::ParticleOptions::simple(ty);
        let mut spawned = Vec::new();
        horizon_client::particles::around_self(
            self.particles.random(),
            feet,
            size[0],
            size[1],
            burst,
            |p, v| spawned.push((p, v)),
        );
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }

    /// `NearestItemSensor.doTick` for the villagers whose sensor is due: the
    /// nearest dropped item within 32 blocks that `Villager.wantsToPickUp`
    /// accepts and the villager sees, or none.
    pub(super) fn villager_item_sense(&mut self) {
        horizon_gameplay::villagers::villager_item_sense(self);
    }
    /// `Mob.aiStep` item pickup for villagers (`InventoryCarrier`): food,
    /// seeds and what their profession wants, from within a block.
    pub(super) fn villager_pickups(&mut self) {
        horizon_gameplay::villagers::villager_pickups(self);
    }

    /// The UUID of a player by connection (None for the host).
    pub(super) fn player_uuid_of(&self, who: Option<u32>) -> Option<[i32; 4]> {
        let uuid = match who {
            None => return self.level.player_uuid,
            // While that guest acts its own UUID is swapped in.
            Some(conn) if self.lan_acting == Some(conn) => self.lan_acting_uuid?,
            Some(conn) => self.lan.as_ref()?.hub.guest(conn)?.uuid,
        };
        Some(horizon_client::lan::playerdata::uuid_ints(uuid))
    }
    /// `Villager.setLastHurtByMob` and `ServerLevel.onReputationEvent`: a hit
    /// from a player counts against that player with the villager that was hit.
    pub(super) fn villager_reputation_tick(&mut self) {
        horizon_gameplay::villagers::villager_reputation_tick(self);
    }
    /// `Villager.tellWitnessesThatIWasMurdered`: the villagers that saw a
    /// player kill one remember it as a major wrong.
    /// `killer` is the guest connection that last hurt it (None: the host).
    pub(super) fn villager_murdered(&mut self, victim: &Mob, by_player: bool, killer: Option<u32>) {
        horizon_gameplay::villagers::villager_murdered(self, victim, by_player, killer);
    }
    /// Retain the accepted trade's actual player until the eligible mob AI
    /// phase, including across menu close. XP and rewards stay in the session.
    pub(super) fn villager_trade_pending(&mut self, id: u32) {
        horizon_gameplay::villagers::villager_trade_pending(self, id);
    }
    /// The TRADE reputation event from the player `uuid` (a guest's too).
    pub(super) fn villager_traded_by(&mut self, id: u32, uuid: [i32; 4]) {
        let Some(i) = self.villager_index(id) else {
            return;
        };
        villager::ext_mut(&mut self.mobs[i]).gossips.add(
            uuid,
            GossipType::Trading,
            gossip::REPUTATION_CHANGE_PER_TRADE,
        );
        // Villager.customServerAiStep after a trade: entity event 14.
        self.mob_event_particles(id, 14);
        // onReputationEventFrom refreshes the current trading player's prices.
        if self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .is_some_and(|m| m.mob == Some(id))
        {
            let reputation = self.villager_reputation(id);
            self.menu_session(|menu, env, _| {
                if let Some(session) = menu.merchant.as_mut() {
                    session.reputation = reputation;
                    session.update_special_prices();
                }
                menu.merchant_update(env);
                Some(true)
            });
        }
        self.lan_merchant_reputation(id);
    }
    /// `Villager.getPlayerReputation` of the local player.
    pub(super) fn villager_reputation(&self, id: u32) -> i32 {
        horizon_gameplay::villagers::villager_reputation(self, id)
    }
    /// `Villager.getPlayerReputation` of the player `uuid`.
    pub(super) fn villager_reputation_of(&self, id: u32, uuid: [i32; 4]) -> i32 {
        horizon_gameplay::villagers::villager_reputation_of(self, id, uuid)
    }
    /// The Hero of the Village amplifier of the player, if any.
    pub(super) fn hero_amplifier(&self) -> Option<i32> {
        self.potions.effects.amplifier("hero_of_the_village")
    }
    /// `AbstractBedBlock.kickVillagerOutOfBed`: the first sleeping villager
    /// whose box touches the bed's head block wakes up
    /// (`Villager.stopSleeping`). Whether one did.
    pub(super) fn kick_villager_out_of_bed(&mut self, head: [i32; 3]) -> bool {
        horizon_gameplay::villagers::kick_villager_out_of_bed(self, head)
    }
    /// Ask after this mob's tick Pre; its living tick applies the answer.
    pub(super) fn sync_sleeping_mob(&mut self, index: usize) {
        if self.remote.is_some()
            || !mod_api::__private::listened(mod_api::HookId::CanContinueSleeping)
        {
            return;
        }
        let m = &self.mobs[index];
        if !m.alive() || m.removed {
            return;
        }
        let Some(pos) = villager::ext(m).and_then(|e| e.sleeping_pos) else {
            return;
        };
        let (id, kind) = (m.id, m.kind.id.clone());
        let uuid = horizon_client::lan::playerdata::ints_uuid(self.uuid_for(id));
        let bed = self.world.block(pos[0], pos[1], pos[2]);
        let bed_exists = self.res.facts().get(bed).name.ends_with("_bed");
        let problem = self.sleep_problem(bed_exists, false);
        let mut may_continue = None;
        self.native_can_continue_sleeping(uuid, "", &kind, pos, problem, &mut may_continue);
        villager::ext_mut(&mut self.mobs[index]).may_continue_sleeping =
            Some(may_continue.unwrap_or(problem.is_none()));
    }
    /// `LivingEntity.stopSleeping` for the villager `id`, outside its own
    /// tick: it stands up and its bed is no longer occupied.
    pub(super) fn wake_villager(&mut self, id: u32) {
        let Some(i) = self.mobs.iter().position(|m| m.id == id) else {
            return;
        };
        let mut mobs = self.mobs.lift();
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
        let mut events = Vec::new();
        villager::stop_sleeping(&mut mobs[i], &self.mob_world_base(&[]), &mut events);
        villager_brain::woken(&mut mobs[i], self.world.game_time);
        self.mobs
            .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
        if let Err(e) = self.handle_mob_events(events) {
            log::warn!("waking villager {id}: {e}");
        }
    }
}

impl VillagerLevel for Game<'_> {
    fn items(&self) -> &Items {
        Game::items(self)
    }
    fn stations(&self) -> &StationData {
        Game::stations(self)
    }
    fn villager_picks_up(&self) -> Option<HashSet<String>> {
        self.res
            .items()
            .item_tags
            .get("minecraft:villager_picks_up")
            .cloned()
    }
    fn mob_type(&self, id: &str) -> Option<Arc<MobType>> {
        self.res.mob_types().get(id).cloned()
    }
    fn mob_world_seen<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        self.mob_world_base(others)
    }
    fn set_next_entity(&mut self, id: u32) {
        self.next_entity = id;
    }
    fn mob_joined(&mut self, from: usize, natural: bool) {
        self.native_mobs_joined(from, natural);
    }
    fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts {
        Game::biome_facts(self, x, y, z)
    }
    fn spawn_mob_raw(&mut self, kind: &str, at: [f64; 3], persistent: bool) -> Option<u32> {
        Game::spawn_mob_raw(self, kind, at, persistent)
    }
    fn mob_event_particles(&mut self, id: u32, event: i8) {
        Game::mob_event_particles(self, id, event);
    }
    fn bonemeal_grow(&mut self, pos: BlockPos, stack: &mut ItemStack) -> (bool, Option<bool>) {
        let ask = self.bonemeal(pos, false, stack);
        let grown = ask.crop.unwrap_or_else(|| {
            self.with_level(|b, level, ticks| b.grow_crop(level, ticks, pos, ask.check))
        });
        (grown, ask.crop)
    }
    fn level_event(&mut self, event: horizon_world::level::LevelEvent) {
        self.level_events.push(event);
        let _ = self.process_level_events();
    }
    fn composter_level(&self, pos: [i32; 3]) -> Option<i32> {
        Game::composter_level(self, pos)
    }
    fn compost_items(&mut self, mob: usize, pos: [i32; 3]) {
        self.villager_compost_items(mob, pos);
    }
    fn hurt_by_player_uuid(&self, id: u32) -> Option<[i32; 4]> {
        self.last_hurt_by_player.get(&id).and_then(|credit| {
            self.resolve_player_credit(*credit)
                .and_then(|who| self.player_uuid_of(who))
        })
    }
    fn player_uuid_of(&self, who: Option<u32>) -> Option<[i32; 4]> {
        Game::player_uuid_of(self, who)
    }
    fn lan_acting(&self) -> Option<u32> {
        self.lan_acting
    }
    fn wake_villager(&mut self, id: u32) {
        Game::wake_villager(self, id);
    }
    fn villager_index(&self, id: u32) -> Option<usize> {
        Game::villager_index(self, id)
    }
    fn item_max_stack(&self, item: &str) -> u32 {
        Game::item_max_stack(self, item)
    }
    fn villager_make_bread(&mut self, i: usize) {
        Game::villager_make_bread(self, i);
    }
    fn villager_throw(&mut self, from: Vec3, eye: f64, to: Vec3, stack: ItemStack) {
        Game::villager_throw(self, from, eye, to, stack);
    }
    fn villager_golem_check(&mut self, id: u32, needed: usize) {
        Game::villager_golem_check(self, id, needed);
    }
    fn try_spawn_iron_golem(&mut self, start: [i32; 3]) -> bool {
        Game::try_spawn_iron_golem(self, start)
    }
    fn villager_reputation_of(&self, id: u32, uuid: [i32; 4]) -> i32 {
        Game::villager_reputation_of(self, id, uuid)
    }
}

#[cfg(test)]
mod entity_event_particle_tests {
    /// The raid sweat (42) and the dolphin's treasure hearts (38) are in the
    /// table that now decides which entity events draw particles.
    #[test]
    fn the_table_draws_the_raid_sweat_and_the_dolphin_event() {
        let kind = |entity, event| {
            horizon_client::particles::entity_event_burst(entity, event).map(|(k, _)| k)
        };
        assert_eq!(kind("villager", 42), Some("splash"));
        assert_eq!(kind("dolphin", 38), Some("happy_villager"));
        assert_eq!(kind("villager", 2), None);
    }
}

#[cfg(test)]
mod nested_spawn_tests {
    #[test]
    #[ignore = "daily"]
    fn lifted_mob_callback_keeps_nested_spawns_until_the_next_pass() {
        crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                99,
                kind,
                horizon_core::geometry::Vec3::new(0.5, 80.0, 0.5),
                0.0,
            ));
            crate::game::entity::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    game.wake_villager(99);
                },
            );
        });
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn retained_ancestor_does_not_consume_a_due_villager_sensor() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            let mut m = Mob::new(73, kind.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            villager_brain::init(&mut m, 0, 0, 0);
            let mut health = horizon_client::health::TestLevel::new(0);
            for _ in 0..40 {
                villager_brain::tick(
                    &mut m,
                    &game.mob_world_base(&[]),
                    &mut health.level(),
                    &mut Vec::new(),
                );
            }
            m.extra.mount = Some(72);
            game.mobs.push(m);
            let ghost = Mob::new(72, kind, Vec3::ZERO, 0.0);
            game.persist
                .off_level
                .insert(72, crate::game::persist::OffLevel::Mob(ghost));
            game.tick_creature_entities().unwrap();
            assert!(
                villager_brain::take_item_scan(game.mob_object_mut(73).unwrap()),
                "a frozen resident's due sensor was consumed by independent item sensing"
            );
        });
    }
}

#[cfg(test)]
#[path = "tests/villager_witness_tests.rs"]
mod witness_tests;

#[cfg(test)]
mod birth_tests {
    use super::*;
    use horizon_client::mobs::MobEvent;
    use mod_api::{
        BabyEntitySpawnEvent, EntityJoinLevelEvent, FinalizeSpawnEvent, Flow, Hooks, Host,
        ServerHost,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    const BED: [i32; 3] = [3, 64, 1];

    fn parents(game: &mut Game<'_>) -> (u32, u32) {
        game.mobs.clear();
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        let bed = game
            .pack
            .assets
            .registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "minecraft:red_bed".into(),
                properties: [
                    ("facing".into(), "east".into()),
                    ("occupied".into(), "false".into()),
                    ("part".into(), "head".into()),
                ]
                .into_iter()
                .collect(),
            })
            .unwrap();
        game.world.set(BED[0], BED[1], BED[2], bed).unwrap();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let parent = game.next_entity + 1;
        let partner = parent + 1;
        game.next_entity = partner;
        for id in [parent, partner] {
            game.mobs.push(Mob::with_random(
                id,
                kind.clone(),
                Vec3::new(1.5, 64.0, 1.5),
                0.0,
                horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(
                    i64::from(id),
                )),
            ));
        }
        // The brain has already eaten, selected BED and aged the initiating
        // parent before emitting VillagerBirth; the Game owns the other half.
        game.mobs[0].extra.age = 6000;
        game.native_joins_skip();
        game.audio.capture = Some(Vec::new());
        game.lan_entity_events.clear();
        (parent, partner)
    }

    fn birth(game: &mut Game<'_>, parent: u32, partner: u32) {
        game.handle_mob_events(vec![MobEvent::VillagerBirth {
            parent,
            partner,
            child: Some((BED, Some(2))),
        }])
        .unwrap();
        for id in [parent, partner] {
            assert_eq!(
                game.mobs.iter().find(|m| m.id == id).unwrap().extra.age,
                6000
            );
        }
    }

    fn bed_owners(game: &Game<'_>) -> Vec<u32> {
        // The same Seen.bed projection supplies the POI ticket searches and
        // take_bed_for_child; no independent reservation exists in the Game.
        game.mobs
            .iter()
            .filter(|m| m.seen(None, game.items()).bed == Some(BED))
            .map(|m| m.id)
            .collect()
    }

    struct RejectJoin(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for RejectJoin {
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            assert_eq!(event.kind, "minecraft:villager");
            self.0.fetch_add(1, Ordering::SeqCst);
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn rejected_birth_leaves_bed_free_and_parents_resting() {
        let joins = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reject-villager-birth",
            Box::new(RejectJoin(joins.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let (parent, partner) = parents(game);
                assert!(bed_owners(game).is_empty());
                birth(game, parent, partner);
                assert_eq!(joins.load(Ordering::SeqCst), 1);
                assert_eq!(game.mobs.len(), 2);
                assert!(bed_owners(game).is_empty());
                assert!(game.lan_entity_events.is_empty());
            });
        });
    }

    fn assert_baby(game: &Game<'_>, parent: u32, partner: u32) {
        assert_eq!(game.mobs.len(), 3);
        let baby = game
            .mobs
            .iter()
            .find(|m| m.id != parent && m.id != partner)
            .unwrap();
        assert_eq!(baby.extra.age, -24000);
        assert_eq!(baby.extra.variant, 2);
        assert_eq!(baby.extra.bed, Some(BED));
        assert_eq!(bed_owners(game), vec![baby.id]);
        assert_eq!(game.lan_entity_events, vec![(baby.id as i32, 12)]);
    }

    #[test]
    #[ignore = "daily"]
    fn untouched_birth_claims_one_bed_and_announces_one_baby() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let (parent, partner) = parents(game);
                assert!(bed_owners(game).is_empty());
                birth(game, parent, partner);
                assert_baby(game, parent, partner);
            });
        });
    }

    struct RejectOtherBirthPaths(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for RejectOtherBirthPaths {
        fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            self.0.fetch_add(1, Ordering::SeqCst);
            event.spawn_cancelled = true;
            Flow::Continue
        }

        fn baby_entity_spawn(
            &self,
            _: &mut dyn ServerHost,
            _: &mut BabyEntitySpawnEvent<'_>,
        ) -> Flow {
            self.0.fetch_add(1, Ordering::SeqCst);
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn villager_birth_is_raw_and_does_not_post_the_animal_hook() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reject-non-villager-birth-paths",
            Box::new(RejectOtherBirthPaths(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let (parent, partner) = parents(game);
                birth(game, parent, partner);
                assert_baby(game, parent, partner);
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            });
        });
    }
}
