//! Actual departure/recreation and TNT blasts, not calls to the arrival-effects helper.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{chunk::Chunk, items::ItemStack};

fn prepare(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.native_lifecycle.world_open_ready().unwrap();
    game.world
        .chunks
        .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
}

/// Capture a real portal traveller, open its destination, and light its arrival
/// area. A restarted waiting arrival has only its saved payload, not a live token.
fn depart(game: &mut Game<'_>, id: u32, restart_waiting: bool) -> Vec3 {
    let entry = BlockPos::new(8, 70, 8);
    let portal = game
        .pack
        .assets
        .registry
        .default_state("minecraft:nether_portal")
        .unwrap();
    game.world.set(entry.x, entry.y, entry.z, portal).unwrap();
    game.send_through_portal(id, entry);
    assert_eq!(game.portals.arrivals.len(), 1);
    if restart_waiting {
        let saved = arrival_tag(&game.portals.arrivals[0]);
        game.portals.arrivals[0] = arrival_from_tag(&saved).unwrap();
    }
    game.enter_level(Dim::Nether).unwrap();
    let plan = game.portals.arrivals[0].plan;
    assert_eq!(plan.dim, Dim::Nether);
    let (cx, cz) = (plan.approximate.x >> 4, plan.approximate.z >> 4);
    for z in cz - 1..=cz + 1 {
        for x in cx - 1..=cx + 1 {
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(x, z));
        }
    }
    let Exit::Found(exit) = game.resolve_exit_portal(&plan) else {
        panic!("exit portal built");
    };
    game.arrival(&plan, exit).0
}

#[test]
#[ignore = "daily"]
fn portal_arrival_effects_merge_items_before_their_first_tick() {
    for (incoming, waiting, incoming_age, waiting_age, restart) in [
        (3, 5, 5999, 41, false),
        (5, 3, 7, 41, true),
        (3, 5, -32768, 41, false),
        (3, 5, 6000, 41, true),
    ] {
        with_game(|game| {
            prepare(game);
            game.spawn_item_own(
                ItemStack::new("minecraft:stick", incoming),
                [8.5, 70.0, 8.5],
                0,
            );
            game.dropped[0].age = incoming_age;
            let id = game.dropped[0].id;
            let at = depart(game, id, restart);
            assert!(game.dropped.is_empty());
            game.spawn_item_own(
                ItemStack::new("minecraft:stick", waiting),
                [at.x, at.y, at.z],
                0,
            );
            game.dropped[0].age = waiting_age;
            game.resolve_arrivals();
            assert!(game.portals.arrivals.is_empty());
            let mergeable = incoming_age != -32768 && incoming_age < 6000;
            assert_eq!(game.dropped.len(), if mergeable { 1 } else { 2 });
            if mergeable {
                let item = &game.dropped[0];
                assert_eq!(item.stack.count, incoming + waiting);
                assert_eq!(item.age, incoming_age.min(waiting_age));
                assert_eq!(item.ticks, 0, "merge must not wait for ItemEntity.tick");
            }
        });
    }
}

/// Expire an actual primed TNT beside a portal and an ordinary block. A valid
/// frame keeps neighbour updates from removing a protected portal indirectly.
pub(in crate::game) fn assert_tnt_blast(game: &mut Game<'_>, protected: bool) {
    assert_eq!(game.tnts.len(), 1);
    let at = game.tnts[0].position;
    let (x, y, z) = (
        at.x.floor() as i32,
        at.y.floor() as i32,
        at.z.floor() as i32,
    );
    let registry = game.pack.assets.registry;
    let portal = registry.default_state("minecraft:nether_portal").unwrap();
    let obsidian = registry.default_state("minecraft:obsidian").unwrap();
    let dirt = registry.default_state("minecraft:dirt").unwrap();
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (cx, cz) = ((x >> 4) + dx, (z >> 4) + dz);
            if !game.world.chunks.contains_key(&(cx, cz)) {
                game.world
                    .chunks
                    .insert((cx, cz), std::sync::Arc::new(Chunk::empty(cx, cz)));
            }
        }
    }
    for yy in y - 1..=y + 3 {
        for xx in x - 1..=x + 2 {
            let state = if yy == y - 1 || yy == y + 3 || xx == x - 1 || xx == x + 2 {
                obsidian
            } else {
                portal
            };
            game.world.set(xx, yy, z, state).unwrap();
        }
    }
    game.world.set(x, y, z + 1, dirt).unwrap();
    let id = game.tnts[0].id;
    game.portals.entities.insert(
        id,
        PortalState {
            cooldown: if protected { 0 } else { 300 },
            process: None,
        },
    );
    game.tnts[0].velocity = Vec3::ZERO;
    game.tnts[0].no_gravity = true;
    game.tnts[0].fuse = 1;
    game.tick_one_tnt(0, &[]).unwrap();
    assert!(game.tnts.is_empty());
    assert_eq!(game.world.block(x, y, z) == portal, protected);
    assert_ne!(
        game.world.block(x, y, z + 1),
        dirt,
        "ordinary blocks still explode"
    );
    assert_eq!(game.world.block(x - 1, y, z), obsidian);
}

#[test]
#[ignore = "daily"]
fn portal_arrival_effects_tnt_protection_is_live_but_not_saved() {
    for restart_waiting in [false, true] {
        with_game(|game| {
            prepare(game);
            game.summon_with_nbt(
                "minecraft:tnt",
                [8.5, 70.0, 8.5],
                "{fuse:200s,NoGravity:1b}",
            )
            .unwrap();
            let old = game.tnts[0].id;
            assert!(!game.tnts[0].used_portal);
            depart(game, old, restart_waiting);
            game.resolve_arrivals();
            assert!(game.portals.arrivals.is_empty());
            assert_eq!(game.tnts.len(), 1);
            assert_ne!(game.tnts[0].id, old);
            assert!(game.tnts[0].used_portal);
            let saved = game.tnt_nbt(0);
            for key in ["usedPortal", "used_portal", "UsedPortal"] {
                assert!(saved.get(key).is_none());
            }
            let id = game.tnts[0].id;
            game.apply_entity_nbt(super::super::commands::Target::Tnt(id), saved.clone())
                .unwrap();
            assert!(
                game.tnts[0].used_portal,
                "loading data into the live instance does not reset it"
            );
            assert_tnt_blast(game, true);
            game.forget_entity(id);
            let uuid = horizon_client::entity_nbt::uuid(&saved);
            game.load_tnt(saved, uuid);
            assert!(
                !game.tnts[0].used_portal,
                "a disk recreation starts with the constructor default"
            );
            assert_tnt_blast(game, false);
            game.summon_with_nbt(
                "minecraft:tnt",
                [8.5, 70.0, 8.5],
                "{fuse:200s,NoGravity:1b}",
            )
            .unwrap();
            assert!(!game.tnts[0].used_portal);
            assert_tnt_blast(game, false);
        });
    }
}

use mod_api::{EntityConstructingEvent, EntityJoinLevelEvent, Flow, Hooks, HostSide, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

struct RemoveOnArrival {
    armed: Arc<AtomicBool>,
    victim: Arc<AtomicU32>,
    calls: Arc<AtomicU32>,
}

#[mod_api::__subscriptions]
impl Hooks for RemoveOnArrival {
    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityJoinLevelEvent<'_>,
    ) -> Flow {
        if event.kind == "minecraft:item" && self.armed.swap(false, Ordering::SeqCst) {
            let victim = self.victim.load(Ordering::SeqCst);
            let victim = if victim == 0 { event.entity } else { victim };
            assert!(host.entity(victim).unwrap().discard(host));
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        Flow::Continue
    }
}

fn callback_destination(game: &mut Game<'_>, id: u32, portal: bool, restart: bool) -> Vec3 {
    if portal {
        depart(game, id, restart)
    } else {
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        Vec3::new(8.5, 70.0, 8.5)
    }
}

fn complete_callback_arrival(game: &mut Game<'_>, id: u32, portal: bool, at: Vec3) {
    if portal {
        game.resolve_arrivals();
        assert!(game.portals.arrivals.is_empty());
    } else {
        game.enter_level(Dim::Overworld).unwrap();
        let entity = game.entity(id).unwrap();
        assert!(game.entity_teleport_to_level(
            entity,
            "minecraft:the_nether",
            [at.x, at.y, at.z],
            None,
        ));
        game.enter_level(Dim::Nether).unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn portal_arrival_effects_join_removals_cannot_consume_live_stacks() {
    for portal in [true, false] {
        for remove_incoming in [false, true] {
            let armed = Arc::new(AtomicBool::new(false));
            let victim = Arc::new(AtomicU32::new(0));
            let calls = Arc::new(AtomicU32::new(0));
            let mods = vec![mod_api::ModEntry::new(
                "arrival-removal",
                Box::new(RemoveOnArrival {
                    armed: armed.clone(),
                    victim: victim.clone(),
                    calls: calls.clone(),
                }),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    prepare(game);
                    game.spawn_item_own(ItemStack::new("minecraft:stick", 3), [8.5, 70.0, 8.5], 0);
                    let id = game.dropped[0].id;
                    let incoming_uuid = game.uuid_for(id);
                    let at = callback_destination(game, id, portal, false);
                    game.spawn_item_own(
                        ItemStack::new("minecraft:stick", 5),
                        [at.x, at.y, at.z],
                        0,
                    );
                    let waiting = game.dropped[0].id;
                    let waiting_uuid = game.uuid_for(waiting);
                    game.native_joins_flush(false, 0);
                    victim.store(if remove_incoming { 0 } else { waiting }, Ordering::SeqCst);
                    armed.store(true, Ordering::SeqCst);
                    complete_callback_arrival(game, id, portal, at);
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
                    game.native_removals_flush();
                    assert_eq!(game.dropped.len(), 1);
                    let survivor = &game.dropped[0];
                    assert_eq!(survivor.stack.count, if remove_incoming { 5 } else { 3 });
                    assert_eq!(
                        game.persist.uuids.get(&survivor.id),
                        Some(&if remove_incoming {
                            waiting_uuid
                        } else {
                            incoming_uuid
                        }),
                    );
                    assert_eq!(survivor.ticks, 0);
                });
            });
        }
    }
}

struct SpawnDuringArrival {
    armed: Arc<AtomicBool>,
    calls: Arc<AtomicU32>,
    at: Arc<std::sync::Mutex<[f64; 3]>>,
    constructing: bool,
}

impl SpawnDuringArrival {
    fn spawn(&self, host: &mut dyn ServerHost) {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        let nbt = Tag::compound([(
            "Item",
            Tag::compound([
                ("id", Tag::String("minecraft:dirt".into())),
                ("count", Tag::Int(1)),
            ]),
        )]);
        let Tag::Compound(nbt) = nbt else {
            unreachable!()
        };
        assert!(
            host.summon(
                "minecraft:the_nether",
                "minecraft:item",
                *self.at.lock().unwrap(),
                nbt,
            )
            .is_some()
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[mod_api::__subscriptions]
impl Hooks for SpawnDuringArrival {
    fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
        if self.constructing
            && event.kind == "minecraft:item"
            && let HostSide::Server(host) = host
        {
            self.spawn(host);
        }
    }

    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityJoinLevelEvent<'_>,
    ) -> Flow {
        if !self.constructing && event.kind == "minecraft:item" {
            self.spawn(host);
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn portal_arrival_effects_nested_spawns_do_not_hide_the_real_arrival() {
    for portal in [true, false] {
        let armed = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicU32::new(0));
        let nested_at = Arc::new(std::sync::Mutex::new([0.0; 3]));
        let mods = vec![mod_api::ModEntry::new(
            "arrival-spawn",
            Box::new(SpawnDuringArrival {
                armed: armed.clone(),
                calls: calls.clone(),
                at: nested_at.clone(),
                constructing: portal,
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                prepare(game);
                game.spawn_item_own(ItemStack::new("minecraft:stick", 3), [8.5, 70.0, 8.5], 0);
                let id = game.dropped[0].id;
                let at = callback_destination(game, id, portal, true);
                game.spawn_item_own(ItemStack::new("minecraft:stick", 5), [at.x, at.y, at.z], 0);
                let waiting = game.dropped[0].id;
                game.native_joins_flush(false, 0);
                *nested_at.lock().unwrap() = [at.x, at.y, at.z];
                armed.store(true, Ordering::SeqCst);
                complete_callback_arrival(game, id, portal, at);
                assert_eq!(calls.load(Ordering::SeqCst), 1);
                assert_eq!(game.dropped.len(), 2);
                let survivor = game.dropped.iter().find(|item| item.id == waiting).unwrap();
                assert_eq!(survivor.stack.count, 8);
                assert_eq!(survivor.ticks, 0);
                let nested = game
                    .dropped
                    .iter()
                    .find(|item| &*item.stack.item == "minecraft:dirt")
                    .unwrap();
                assert_eq!(nested.stack.count, 1);
            });
        });
    }
}
