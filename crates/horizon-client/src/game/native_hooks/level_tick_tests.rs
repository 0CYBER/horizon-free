#[cfg(test)]
mod tests {
    use super::super::super::{
        native_entity_refs::tests::with_client_game, native_lifecycle::NativeLifecycle,
    };
    use horizon_client::level_loading::Reason;
    use horizon_protocol::{connection::Pipe, packets::play::SpawnInfo};
    use mod_api::{Hooks, HostSide, LevelRef, LevelTickPostEvent, LevelTickPreEvent, ModEntry};
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore = "daily"]
    fn server_hook_resolves_uuid_only_while_the_entity_is_in_its_active_level() {
        use super::super::super::{Dim, native_entity_refs::tests::with_game};
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use mod_api::EntityUuid;

        const UUID: EntityUuid = EntityUuid(0x0123_4567_89ab_cdef_1020_3040_ffff_ffff);

        type Hit = Option<(u32, [f64; 3])>;

        struct Lookup(Arc<Mutex<Vec<Hit>>>);

        #[mod_api::__subscriptions]
        impl Hooks for Lookup {
            fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
                let HostSide::Server(host) = host else {
                    return;
                };
                assert_eq!(host.entity_by_uuid(EntityUuid(u128::MAX)), None);
                let hit = host
                    .entity_by_uuid(UUID)
                    .map(|entity| (entity.id(), entity.position(host).unwrap()));
                self.0.lock().unwrap().push(hit);
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "uuid_lookup",
            Box::new(Lookup(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let _phase =
                    mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs
                    .push(Mob::new(91, kind.clone(), Vec3::new(3.5, 70.0, 5.5), 0.0));
                game.mobs.push(Mob::new(92, kind, Vec3::ZERO, 0.0));
                game.persist
                    .uuids
                    .insert(91, [0x0123_4567, 0x89ab_cdefu32 as i32, 0x1020_3040, -1]);
                let home = game.level("minecraft:overworld").unwrap();

                let uuids_before = game.persist.uuids.clone();
                game.native_level_tick_pre(home);
                assert_eq!(*game.persist.uuids, uuids_before);

                game.enter_level(Dim::Nether).unwrap();
                let nether = game.level("minecraft:the_nether").unwrap();
                let uuids_before = game.persist.uuids.clone();
                game.native_level_tick_pre(nether);
                assert_eq!(game.dim, Dim::Nether);
                assert_eq!(*game.persist.uuids, uuids_before);

                game.enter_level(Dim::Overworld).unwrap();
                use horizon_client::join_seq::Joined;
                let join = game.mobs[0].join_seq();
                let taken = game.mobs.extract(join).unwrap();
                game.native_level_tick_pre(home);
                game.mobs.put_back(0, taken);
                game.native_level_tick_pre(home);
                game.mobs.retain(|mob| mob.id != 91);
                let uuids_before = game.persist.uuids.clone();
                game.native_level_tick_pre(home);
                assert_eq!(*game.persist.uuids, uuids_before);

                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        Some((91, [3.5, 70.0, 5.5])),
                        None,
                        None,
                        Some((91, [3.5, 70.0, 5.5])),
                        None,
                    ]
                );
            });
        });
    }

    struct Moments(Arc<Mutex<Vec<(&'static str, LevelRef, Option<i64>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Moments {
        fn level_tick_pre(&self, host: HostSide, event: &mut LevelTickPreEvent) {
            let time = match host {
                HostSide::Client(h) => event.level.game_time(h),
                HostSide::Server(h) => event.level.game_time(h),
            };
            self.0.lock().unwrap().push(("pre", event.level, time));
        }
        fn level_tick_post(&self, host: HostSide, event: &mut LevelTickPostEvent) {
            let time = match host {
                HostSide::Client(h) => event.level.game_time(h),
                HostSide::Server(h) => event.level.game_time(h),
            };
            self.0.lock().unwrap().push(("post", event.level, time));
        }
    }

    /// One pre then one post for each client level tick, naming the level in
    /// place, and nothing before a tick or for a client with no level.
    #[test]
    #[ignore = "daily"]
    fn client_level_tick_fires_pre_then_post_around_each_tick() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("moments", Box::new(Moments(heard.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                game.native_lifecycle = NativeLifecycle::default();
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "LevelTickTest").unwrap();
                game.tick_remote_level().unwrap();
                assert!(heard.lock().unwrap().is_empty(), "no level means no event");
                let spawn = SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 1,
                    previous_game_type: None,
                    debug: false,
                    flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                };
                game.enter_remote_dimension(&spawn, false, Reason::Other);
                game.remote.as_mut().unwrap().logged_in = true;
                let level = game.level("minecraft:overworld").unwrap();

                // Login alone is no tick, and neither is a level with no tick.
                assert!(heard.lock().unwrap().is_empty());

                let before = game.world.game_time;
                game.tick_remote_level().unwrap();
                let after = game.world.game_time;
                assert_eq!(after, before + 1, "the tick advanced the world's time");
                assert_eq!(
                    *heard.lock().unwrap(),
                    [("pre", level, Some(before)), ("post", level, Some(after))],
                    "the level's own work ran between them"
                );

                heard.lock().unwrap().clear();
                game.tick_remote_level().unwrap();
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        ("pre", level, Some(after)),
                        ("post", level, Some(after + 1))
                    ]
                );
                heard.lock().unwrap().clear();
                let failed = game.tick_remote_level_with(|_| Err("failed tick body".into()));
                assert!(failed.is_err());
                assert_eq!(
                    *heard.lock().unwrap(),
                    [("pre", level, Some(after + 1))],
                    "failed ticks have no post event"
                );
                heard.lock().unwrap().clear();
                let mut second = spawn.clone();
                second.dimension = "minecraft:the_nether".into();
                game.enter_remote_dimension(&second, false, Reason::Other);
                let other = game.level("minecraft:the_nether").unwrap();
                assert_ne!(level, other);
                game.tick_remote_level().unwrap();
                let moments = heard.lock().unwrap();
                assert_eq!(moments.len(), 2);
                assert_eq!((moments[0].1, moments[1].1), (other, other));
            });
        });
    }
}
