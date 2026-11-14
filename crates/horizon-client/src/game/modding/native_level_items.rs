//! Item entities and experience orbs a native mod adds to a level through a
//! `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"): a block's
//! drop (`Block.popResource`), a plain item entity (`new ItemEntity` and
//! `addFreshEntity`) and experience (`ExperienceOrb.award`). They act where
//! the world is simulated, in the level in place: on the server thread, for a
//! server's players, and in-frame on the window's own level. Clients get the
//! new entities through the tracker, as any other.
use super::{Game, spill};
use horizon_items::items::ItemStack;
use mod_api::{EntityRef, LevelRef};

impl Game<'_> {
    /// Whether vanilla could make an item entity of `stack`: not empty
    /// (`ItemStack.isEmpty`) and of an item the registry has.
    pub(super) fn stack_spawns(&self, stack: &ItemStack) -> bool {
        stack.count > 0
            && &*stack.item != "minecraft:air"
            && self.res.items().get(&stack.item).is_some()
    }
    /// `spawn_item` of `stack` and the new item entity as a handle, unless a
    /// mod kept it out (`native_entity_join_level`). It is found by its id,
    /// since the hooks the spawn runs may add entities after it.
    fn spawn_level_item(
        &mut self,
        level: LevelRef,
        stack: ItemStack,
        at: [f64; 3],
        motion: Option<[f64; 3]>,
        pickup_delay: u32,
    ) -> Option<EntityRef> {
        // `spawn_item` takes the next id.
        let id = self.next_entity + 1;
        self.add_item_entity(stack, at, motion, pickup_delay);
        self.dropped
            .iter()
            .rev()
            .any(|d| d.id == id)
            .then(|| EntityRef::new(level.epoch(), id))
    }
    /// Drops `stack` as a broken block drops it (`Block.popResource`): an
    /// item entity at a random point within a quarter block of the middle of
    /// `pos` (three draws from the level random, made for an empty stack and
    /// with `block_drops` off too), with its own random motion and the
    /// default pickup delay of 10 ticks. None, with nothing spawned, for an
    /// empty stack or an unknown item, with the `block_drops` rule off, on a
    /// client, for a stale handle, a level not in place, or an item a mod
    /// kept out (`native_entity_join_level`).
    pub(super) fn level_pop_resource(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        stack: ItemStack,
    ) -> Option<EntityRef> {
        if !self.level_in_place(level) {
            return None;
        }
        let at = spill::pop_position(&mut self.world.random, pos);
        // Block.popResource's private overload checks these after the draws.
        if !self.stack_spawns(&stack)
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::BLOCK_DROPS)
        {
            return None;
        }
        self.spawn_level_item(level, stack, at, None, 10)
    }
    /// Spawns `stack` as an item entity at `x y z` (`new ItemEntity(level, x,
    /// y, z, stack)` and `addFreshEntity`): its own random motion (up 0.2, up
    /// to 0.1 either way across), with `pickup_delay` ticks before it can be
    /// picked up, as `ItemEntity.setPickupDelay` at construction (vanilla
    /// passes 10 for a block's drop, 40 for a player's throw and 32767 for
    /// never). None, with nothing
    /// spawned, for an empty stack or an unknown item, a value that isn't
    /// finite, on a client, for a stale handle, a level not in place, or an
    /// item a mod kept out (`native_entity_join_level`).
    pub(super) fn level_spawn_item(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        pickup_delay: u32,
    ) -> Option<EntityRef> {
        self.spawn_item_at(level, x, y, z, stack, None, pickup_delay)
    }
    /// `spawn_item` with the item entity's motion set to
    /// `motion` (blocks a tick), as `new ItemEntity(level, x, y, z, stack,
    /// deltaX, deltaY, deltaZ)`, and the same `pickup_delay` ticks before it
    /// can be picked up (`ItemEntity.setPickupDelay`). None in the same
    /// cases, and for a motion that isn't finite.
    pub(super) fn level_spawn_item_with_motion(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: [f64; 3],
        pickup_delay: u32,
    ) -> Option<EntityRef> {
        self.spawn_item_at(level, x, y, z, stack, Some(motion), pickup_delay)
    }
    fn spawn_item_at(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        stack: ItemStack,
        motion: Option<[f64; 3]>,
        pickup_delay: u32,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        let finite = [x, y, z]
            .into_iter()
            .chain(motion.into_iter().flatten())
            .all(f64::is_finite);
        if !finite || !self.level_in_place(level) || !self.stack_spawns(&stack) {
            return None;
        }
        self.spawn_level_item(level, stack, [x, y, z], motion, pickup_delay)
    }
    /// Awards `amount` experience at `x y z` as vanilla's `ExperienceOrb.award`
    /// does: orbs of the largest sizes first (2477, 1237, ... 3, 1), each
    /// drawing the id `tryMergeToExisting` compares from the level random and
    /// merging into an orb of the same value whose box meets the 1 block box
    /// around `x y z`, or else a new orb with its own random motion. The
    /// `block_drops` rule doesn't apply (vanilla's blocks check it before
    /// they award). True once the orbs are awarded, even if a mod keeps a new
    /// one out (`native_entity_join_level`). False, with no orbs, for 0, a
    /// value that isn't finite, on a client, for a stale handle or a level
    /// not in place.
    pub(super) fn level_award_experience(
        &mut self,
        level: LevelRef,
        x: f64,
        y: f64,
        z: f64,
        amount: u32,
    ) -> bool {
        if amount == 0 || ![x, y, z].iter().all(|v| v.is_finite()) || !self.level_in_place(level) {
            return false;
        }
        self.spawn_orbs([x, y, z], amount);
        true
    }
}

#[cfg(test)]
mod tests {
    use mod_api::ServerHost;
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_pops_spawns_and_awards_as_vanilla() {
        use super::super::{
            Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole,
            load_resources, spill,
        };
        use horizon_client::gamerules::Value as RuleValue;
        use horizon_client::items::ItemStack;
        use horizon_core::random::RandomSource;
        use mod_api::{Authority, LevelRef, WorldEpoch};

        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.native_lifecycle
            .begin_open(Authority::IntegratedServer);
        let level = game.level(game.dim.id()).unwrap();
        let stone = || ItemStack::new("minecraft:stone", 3);

        // Block.popResource: three level-random offsets, then the entity.
        let mut random = game.world.random.clone();
        let at = spill::pop_position(&mut random, [4, 64, -2]);
        let popped = game
            .level_pop_resource(level, [4, 64, -2], stone())
            .unwrap();
        assert_eq!(
            game.entity_kind(popped)
                .map(|kind| kind.id(&game))
                .as_deref(),
            Some("minecraft:item")
        );
        assert_eq!(game.entity_position(popped), Some(at));
        let item = game.dropped.last().unwrap();
        assert_eq!((item.pickup_delay, item.stack.count), (10, 3));
        assert_eq!(item.velocity.y, 0.2);
        // The offsets are drawn even when nothing drops.
        assert!(
            game.level_pop_resource(level, [0; 3], ItemStack::new("minecraft:air", 1))
                .is_none()
        );
        game.ui.rules.set("block_drops", RuleValue::Bool(false));
        assert!(
            game.level_pop_resource(level, [4, 64, -2], stone())
                .is_none()
        );
        let _ = spill::pop_position(&mut random, [0; 3]);
        let _ = spill::pop_position(&mut random, [0; 3]);
        assert_eq!(random.next_long(), game.world.random.next_long());

        // new ItemEntity(level, x, y, z, stack, delta...): the pickup delay
        // is the caller's, as ItemEntity.setPickupDelay at construction.
        let spawned = game
            .level_spawn_item_with_motion(level, 1.5, 70.0, 2.5, stone(), [0.0, 0.3, 0.0], 40)
            .unwrap();
        assert_eq!(game.entity_position(spawned), Some([1.5, 70.0, 2.5]));
        let item = game.dropped.last().unwrap();
        assert_eq!((item.pickup_delay, item.velocity.y), (40, 0.3));
        assert!(
            game.level_spawn_item(level, 1.5, 70.0, 2.5, stone(), 32767)
                .is_some()
        );
        assert_eq!(game.dropped.last().unwrap().pickup_delay, 32767);
        assert!(
            game.level_spawn_item(level, f64::NAN, 70.0, 2.5, stone(), 0)
                .is_none()
        );
        for empty in [
            ItemStack::new("minecraft:stone", 0),
            ItemStack::new("minecraft:nothing", 1),
        ] {
            assert!(
                game.level_spawn_item(level, 0.0, 0.0, 0.0, empty, 0)
                    .is_none()
            );
        }

        // An item a hook spawns while the tick has the list out joins after
        // the others, which keep their numbers.
        use horizon_client::join_seq::Joined;
        let joins = |game: &Game| -> Vec<(u32, u64)> {
            game.dropped
                .iter()
                .map(|d| (d.id, d.join_seq().get()))
                .collect()
        };
        let before = joins(&game);
        let out = game.dropped.lift();
        let hooked = game
            .level_spawn_item(level, 1.5, 70.0, 2.5, stone(), 0)
            .unwrap();
        assert_eq!(joins(&game).len(), 1);
        game.put_back_dropped(out);
        let after = joins(&game);
        assert_eq!(after[..before.len()], before[..]);
        assert_eq!(after[before.len()].0, hooked.id());
        assert!(after[before.len()].1 > before.iter().map(|j| j.1).max().unwrap());
        assert!(game.entity_kind(hooked).is_some());

        // ExperienceOrb.award: 100 is 73, 17, 7 and 3, far apart from other orbs.
        assert!(game.level_award_experience(level, 100.5, 70.0, 0.5, 100));
        let values: Vec<u32> = game.orbs.iter().map(|o| o.points()).collect();
        assert_eq!(values, [73, 17, 7, 3]);
        assert!(!game.level_award_experience(level, 100.5, 70.0, 0.5, 0));

        let stale = LevelRef::new(WorldEpoch::new(0), game.dim.key());
        assert!(
            game.level_spawn_item(stale, 0.0, 0.0, 0.0, stone(), 0)
                .is_none()
        );
        assert!(game.level_pop_resource(stale, [0; 3], stone()).is_none());
        assert!(!game.level_award_experience(stale, 0.0, 0.0, 0.0, 5));
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn an_orb_awarded_in_a_merge_hook_is_merged_by_a_later_scan() {
        use horizon_client::entities::ExperienceOrb;
        use horizon_core::geometry::Vec3;
        use mod_api::{self, ExperienceOrbMergeEvent, Flow, Hooks};

        // Each scanForMerges queries the level afresh (ExperienceOrb.java:183),
        // so B's scan finds the orb a hook awarded during A's.
        struct Award;
        #[mod_api::__subscriptions]
        impl Hooks for Award {
            fn experience_orb_merge(
                &self,
                host: &mut dyn ServerHost,
                event: &mut ExperienceOrbMergeEvent<'_>,
            ) -> Flow {
                if event.from == 80 {
                    let level = host.level(event.dimension).unwrap();
                    assert!(level.award_experience(host, 101.3, 70.0, 0.5, 3));
                }
                Flow::Continue
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("level-item-awards", Box::new(Award))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                let orb = |id, x, value, ticks| {
                    let p = Vec3::new(x, 70.0, 0.5);
                    ExperienceOrb {
                        attachment_lifetime: Default::default(),
                        fall_distance: 0.0,
                        rotation: [0.0, 0.0],
                        join: Default::default(),
                        id,
                        position: p,
                        previous: p,
                        velocity: Vec3::ZERO,
                        value,
                        age: 0,
                        on_ground: true,
                        count: 1,
                        ticks,
                        health: 5,
                        air: 300,
                        no_gravity: true,
                        following: None,
                    }
                };
                // A takes X; B, far off, ticks after A. The award is id 160.
                // The per-orb scan runs before tick increments ticks, so 0 scans now.
                game.orbs.push(orb(40, 0.5, 3, 0));
                game.orbs.push(orb(80, 0.5, 3, 1));
                game.orbs.push(orb(120, 100.5, 3, 0));
                game.next_entity = 159;
                // Only initial residents take turns; each scan sees hook-awarded residents.
                let ticking: Vec<_> = game.orbs.iter().map(|orb| orb.join).collect();
                for join in ticking {
                    if let Some(index) = game.orbs.locate(join) {
                        game.merge_orb(index);
                    }
                }
                let left: Vec<(u32, u32)> = game.orbs.iter().map(|o| (o.id, o.count)).collect();
                assert_eq!(left, [(40, 2), (120, 2)]);
            });
        });
    }
}
