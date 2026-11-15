//! arismoko/horizon#795, the mob slice: a handle fired inside a mob's own tick resolves the
//! mob the tick holds out of the game, without a copy or a placeholder.
use super::*;
use crate::game::Dim;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::chunk::Chunk;
use mod_api::{Despawn, EntityTickListenedEvent, EntityTickPreEvent, Flow, ModEntry, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

/// The `mob_despawn` facts a listener reads through its handle: its own
/// position, and another mob's through a nested lookup.
#[derive(Debug, Default)]
struct DespawnRecords(Vec<(u32, Option<[f64; 3]>, Option<[f64; 3]>)>);

struct DespawnProbe {
    records: Arc<Mutex<DespawnRecords>>,
    allow: Arc<AtomicU32>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DespawnProbe {
    fn mob_despawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let id = event.mob.entity().id();
        let own = event.mob.entity().position(host);
        let other_id = if id == 71 { 72 } else { 71 };
        let other = host
            .entity(other_id)
            .and_then(|entity| entity.position(host));
        self.records.lock().unwrap().0.push((id, own, other));
        if id == self.allow.load(Ordering::Relaxed) {
            *event.result = Despawn::Allow;
        }
    }
}

/// Reads and moves the mob in its `entity_tick_pre`, through its handle.
struct MoveOnPre {
    id: u32,
    seen: Arc<Mutex<Vec<Option<[f64; 3]>>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for MoveOnPre {
    fn entity_tick_listened(&self, _: &mut dyn ServerHost, event: &mut EntityTickListenedEvent) {
        event.listened = true;
    }
    fn entity_tick_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityTickPreEvent<'_>,
    ) -> Flow {
        if event.entity == self.id {
            let entity = host.entity(self.id).expect("a live handle in its own Pre");
            let at = entity
                .position(host)
                .expect("the tick view reads its position");
            self.seen.lock().unwrap().push(Some(at));
            assert!(
                entity.teleport_to(host, [at[0], at[1] + 10.0, at[2]], None),
                "the tick view writes its position"
            );
        }
        Flow::Continue
    }
}

/// Two idle pigs in an entity-ticking chunk, at x = 1 and x = 2, ready for
/// the real creature pass.
fn with_two_pigs(test: impl FnOnce(&mut Game<'_>)) {
    let root = std::env::temp_dir().join(format!(
        "horizon-mob-tick-access-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    with_game(|game| {
        game.world_root = root.clone();
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        assert!(
            game.entity_ticking_area()(1, 0),
            "fixture chunk is not entity-ticking"
        );
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        for (id, x) in [(71u32, 1.0f64), (72u32, 2.0f64)] {
            let mut mob = Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
            mob.no_ai = true;
            mob.no_gravity = true;
            mob.on_ground = true;
            game.mobs.push(mob);
        }
        test(game);
    });
    let _ = std::fs::remove_dir_all(&root);
}

/// Removes the mob it names, and records what every despawn callback reads of
/// that mob's handle: the victim's own callback runs before the removal, a
/// later callback after it.
struct RemovedPeerProbe {
    victim: u32,
    records: Arc<Mutex<Vec<(u32, Option<[f64; 3]>)>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for RemovedPeerProbe {
    fn mob_despawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let id = event.mob.entity().id();
        let victim = host
            .entity(self.victim)
            .and_then(|entity| entity.position(host));
        self.records.lock().unwrap().push((id, victim));
        if id == self.victim {
            *event.result = Despawn::Allow;
        }
    }
}

/// The view's removed state is authoritative: once the first mob is removed by
/// its own callback, a later callback asking for its handle gets nothing, even
/// though the removed row stays indexed with its old position.
#[test]
#[ignore = "daily"]
fn a_removed_mob_handle_resolves_nothing_in_a_later_callback() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-removed-peer",
        Box::new(RemovedPeerProbe {
            victim: 71,
            records: records.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            let records = records.lock().unwrap();
            assert_eq!(records.len(), 2, "both mobs heard their despawn check");
            // 71 runs first and is removed only after its own answer.
            assert_eq!(records[0].0, 71);
            assert_eq!(
                records[0].1,
                Some([1.0, 80.0, 0.5]),
                "the victim still resolves inside its own callback"
            );
            // 72 runs after 71's removal: its row is indexed but marked removed.
            assert_eq!(records[1].0, 72);
            assert_eq!(
                records[1].1, None,
                "a removed mob's handle resolves nothing"
            );
            assert!(
                !game.mobs.iter().any(|mob| mob.id == 71),
                "the allowed mob was removed"
            );
            assert!(
                game.mobs.iter().any(|mob| mob.id == 72),
                "the denied mob stayed"
            );
        });
    });
}

/// The mob the pass holds out of the game answers its own handle, and the
/// handle of the other mob (the tick snapshot) too; `Allow` removes the
/// mob and its handle then resolves nothing.
#[test]
#[ignore = "daily"]
fn mob_despawn_handles_resolve_the_ticking_mob_and_its_peers() {
    let records = Arc::new(Mutex::new(DespawnRecords::default()));
    let allow = Arc::new(AtomicU32::new(72));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-access",
        Box::new(DespawnProbe {
            records: records.clone(),
            allow: allow.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            let records = records.lock().unwrap();
            assert_eq!(records.0.len(), 2, "both mobs heard their despawn check");
            for (id, own, peer) in &records.0 {
                let (expected_own, expected_peer) = match id {
                    71 => ([1.0, 80.0, 0.5], [2.0, 80.0, 0.5]),
                    72 => ([2.0, 80.0, 0.5], [1.0, 80.0, 0.5]),
                    _ => panic!("unexpected mob {id}"),
                };
                assert_eq!(*own, Some(expected_own), "own position, mob {id}");
                assert_eq!(*peer, Some(expected_peer), "different peer, mob {id}");
            }
            // `Allow` removed 72 during the pass; 71 is still there.
            assert!(
                game.mobs.iter().any(|mob| mob.id == 71),
                "the denied mob stayed"
            );
            assert!(
                !game.mobs.iter().any(|mob| mob.id == 72),
                "the allowed mob was removed"
            );
        });
    });
}

/// A mob's own `entity_tick_pre` handle reads where it stands and moves it;
/// the real tick then starts from the position the handle wrote.
#[test]
#[ignore = "daily"]
fn entity_tick_pre_reads_and_moves_the_ticking_mob() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-pre-access",
        Box::new(MoveOnPre {
            id: 71,
            seen: seen.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                &[Some([1.0, 80.0, 0.5])],
                "the Pre handle read the mob's position once"
            );
            let mob = game.mobs.iter().find(|mob| mob.id == 71).unwrap();
            assert_eq!(
                mob.position.y, 90.0,
                "the tick started from the position the handle wrote"
            );
        });
    });
}

/// Observations made by the actual scoped despawn hook, not by the spawn
/// fixture or a replacement host.
#[derive(Debug, PartialEq)]
struct PassObservation {
    actor: u32,
    peer_velocity: Option<[f64; 3]>,
    spawned_position: Option<[f64; 3]>,
    spawned_handles: [bool; 3],
}

struct PassProbe {
    records: Arc<Mutex<Vec<PassObservation>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for PassProbe {
    fn mob_despawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
        let spawned = mod_api::EntityRef::new(event.mob.entity().epoch(), 74);
        self.records.lock().unwrap().push(PassObservation {
            actor: event.mob.entity().id(),
            peer_velocity: host.entity(72).and_then(|peer| peer.velocity(host)),
            spawned_position: spawned.position(host),
            spawned_handles: [
                host.entity(74).is_some(),
                host.living(spawned).is_some(),
                host.mob(spawned).is_some(),
            ],
        });
        *event.result = Despawn::Deny;
    }
}

/// NoAI still drops motion below 0.003 during its real turn. The observers
/// bracket that turn, so a frozen initial snapshot or an all-post-turn view
/// cannot satisfy both assertions. The existing lifted-pass spawn fixture
/// inserts a real mob after the pass has taken its snapshot and storage;
/// observations still go through the actual scoped hook on both passes.
#[test]
#[ignore = "daily"]
fn scoped_despawn_view_tracks_peer_turns_and_defers_mid_pass_spawns() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-pass-visibility",
        Box::new(PassProbe {
            records: records.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.mobs[1].position.x = 4.0;
            game.mobs[1].velocity = Vec3::new(0.001, 0.0, 0.0);
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut observer = Mob::new(73, kind, Vec3::new(7.0, 80.0, 0.5), 0.0);
            observer.no_ai = true;
            observer.no_gravity = true;
            observer.on_ground = true;
            game.mobs.push(observer);
            super::super::super::entity::taken_pass_tests::with_spawn(
                |game| {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    let mut spawned = Mob::new(74, kind, Vec3::new(10.0, 80.0, 0.5), 0.0);
                    spawned.no_ai = true;
                    spawned.no_gravity = true;
                    spawned.on_ground = true;
                    game.mobs.push(spawned);
                    // The ordinary host can already read the spawn buffer.
                    // Only the scoped view must keep it outside this pass.
                    let entity = game.entity(74).expect("spawn has a resident handle");
                    assert!(game.mob(entity).is_some());
                    assert_eq!(entity.position(game), Some([10.0, 80.0, 0.5]));
                },
                || game.tick_creature_entities().unwrap(),
            );
            assert_eq!(
                records.lock().unwrap().as_slice(),
                &[
                    PassObservation {
                        actor: 71,
                        peer_velocity: Some([0.001, 0.0, 0.0]),
                        spawned_position: None,
                        spawned_handles: [false; 3],
                    },
                    PassObservation {
                        actor: 72,
                        peer_velocity: Some([0.001, 0.0, 0.0]),
                        spawned_position: None,
                        spawned_handles: [false; 3],
                    },
                    PassObservation {
                        actor: 73,
                        peer_velocity: Some([0.0, 0.0, 0.0]),
                        spawned_position: None,
                        spawned_handles: [false; 3],
                    },
                ]
            );
            let spawned = game.mobs.iter().find(|mob| mob.id == 74).unwrap();
            assert_eq!(spawned.age, 0, "the mid-pass spawn has not taken a turn");
            records.lock().unwrap().clear();
            game.tick_creature_entities().unwrap();
            let records = records.lock().unwrap();
            assert_eq!(records.len(), 4, "the spawn joins the next pass");
            for actor in [71, 72, 73, 74] {
                let observation = records.iter().find(|record| record.actor == actor).unwrap();
                assert_eq!(observation.peer_velocity, Some([0.0, 0.0, 0.0]));
                assert_eq!(observation.spawned_position, Some([10.0, 80.0, 0.5]));
                assert_eq!(observation.spawned_handles, [true; 3]);
            }
        });
    });
}

/// Extends the lifetime of the dropped item it names, through `extra_life`.
struct ExtendLife {
    id: Arc<AtomicU32>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for ExtendLife {
    fn item_expire(&self, _: &mut dyn ServerHost, event: &mut mod_api::ItemExpireEvent<'_>) {
        if event.entity == self.id.load(Ordering::Relaxed) {
            event.extra_life += 100;
        }
    }
}

/// The item family keeps its own contract while the mob slice changes: the
/// item tick still lifts its list, and an `item_expire` listener still edits
/// the item's lifetime, which the engine applies when the tick ends.
#[test]
#[ignore = "daily"]
fn item_lifetime_edit_in_the_item_tick_still_lands() {
    let named = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "item-lifetime-access",
        Box::new(ExtendLife { id: named.clone() }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let root = std::env::temp_dir().join(format!(
            "horizon-item-lifetime-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        with_game(|game| {
            game.world_root = root.clone();
            game.enter_level(Dim::Overworld).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.push_item_entity(
                ItemStack::new("minecraft:apple", 1),
                [1.5, 64.0, 1.5],
                Some([0.0, 0.0, 0.0]),
                0,
                None,
            );
            let id = game.dropped.last().unwrap().id;
            named.store(id, Ordering::Relaxed);
            game.ticked_entities_through = id;
            game.dropped.last_mut().unwrap().age = 6000;
            let mut round = game.native_tick_round();
            game.tick_dropped_items(&mut round);
            let item = game
                .dropped
                .iter()
                .find(|item| item.id == id)
                .expect("the extended item stayed");
            assert_eq!(
                item.age, 5900,
                "the listener's 100 ticks landed on the item's age"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    });
}

/// Discards another mob from the first mob's `entity_tick_pre`: a hook that
/// re-enters the entity handles while a tick is running.
struct DiscardOnPre {
    id: u32,
    victim: u32,
    seen: Arc<Mutex<Vec<(u32, bool, bool)>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DiscardOnPre {
    fn entity_tick_listened(&self, _: &mut dyn ServerHost, event: &mut EntityTickListenedEvent) {
        event.listened = true;
    }
    fn entity_tick_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityTickPreEvent<'_>,
    ) -> Flow {
        if event.entity == self.id {
            let victim = host
                .entity(self.victim)
                .expect("the other mob is live at the Pre");
            let discarded = victim.discard(host);
            let self_live = host.entity(self.id).is_some();
            self.seen
                .lock()
                .unwrap()
                .push((self.victim, discarded, self_live));
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_tick_hook_discards_another_mob_nested() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-tick-nested-discard",
        Box::new(DiscardOnPre {
            id: 71,
            victim: 72,
            seen: seen.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_two_pigs(|game| {
            game.tick_creature_entities().unwrap();
            assert_eq!(
                seen.lock().unwrap().as_slice(),
                &[(72, true, true)],
                "the nested discard ran, and the actor's handle still resolved"
            );
            assert!(game.mobs.iter().any(|mob| mob.id == 71));
            assert!(!game.mobs.iter().any(|mob| mob.id == 72));
        });
    });
}
