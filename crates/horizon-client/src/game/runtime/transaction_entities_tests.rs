//! Entities added and removed inside a transaction (#796), on a real `Game`: an abort takes
//! back what the action spawned and undoes what it removed, telling no one; a commit announces
//! joins and runs removals at the root commit; and a spawn refuses near the end of the ids.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

use horizon_client::{chunk::Chunk, items::ItemStack, mobs::StoredEffect};
use horizon_core::nbt::{Tag, parse_snbt};
use horizon_mod_runtime::attachments::Holder;
use mod_api::{
    AttachmentType, EntityConstructingEvent, EntityJoinLevelEvent, EntityLeaveLevelEvent,
    EntityRef, Flow, Hooks, HostSide, RemovalReason, ServerHost,
};

use super::{Dim, Game, commands::Target, lan_entities::ENTITY_ID_HEADROOM};

/// Kept for the session only.
static MARK: AttachmentType<i32> = AttachmentType {
    namespace: "journal",
    name: "mark",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

/// What the observer heard, in order.
#[derive(Debug, Default, PartialEq)]
struct Heard {
    constructed: Vec<u32>,
    joined: Vec<u32>,
    /// The joined entities told `loaded_from_disk`.
    from_disk: Vec<u32>,
    left: Vec<u32>,
    /// Every join (`true`) and leave (`false`) in the order heard.
    order: Vec<(bool, u32)>,
}

static HEARD: Mutex<Heard> = Mutex::new(Heard {
    constructed: Vec::new(),
    joined: Vec::new(),
    from_disk: Vec::new(),
    left: Vec::new(),
    order: Vec::new(),
});

/// The entity whose join the observer answers with a spawn of its own, what it spawned, and an
/// entity it removes in a transaction of its own that it commits around the spawn.
static ON_JOIN: Mutex<(Option<u32>, Vec<u32>, Option<EntityRef>)> =
    Mutex::new((None, Vec::new(), None));

/// The entity a nested commit removed, still waiting for its removal to run, and then what
/// `entity_kind` answered the listener of the next join for it.
static PENDING: Mutex<Option<EntityRef>> = Mutex::new(None);
static KIND_OF_PENDING: Mutex<Option<Option<String>>> = Mutex::new(None);

/// Where the observer is in the nested script of
/// `a_level_commit_told_inside_another_levels_commit_keeps_its_own_batch`: 0 is off.
static NEST: AtomicU8 = AtomicU8::new(0);

/// Hears construction, joins and leaves; marks each entity it constructs; keeps sheep out.
struct Observer;
#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
        if let (HostSide::Server(host), Holder::Entity(id)) = (host, event.holder) {
            HEARD.lock().unwrap().constructed.push(id);
            MARK.set(host, event.holder, 1).unwrap();
        }
    }
    fn entity_join_level(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityJoinLevelEvent<'_>,
    ) -> Flow {
        let mut heard = HEARD.lock().unwrap();
        heard.joined.push(event.entity);
        heard.order.push((true, event.entity));
        if event.loaded_from_disk {
            heard.from_disk.push(event.entity);
        }
        drop(heard);
        let pending = PENDING.lock().unwrap().take();
        if let Some(entity) = pending {
            *KIND_OF_PENDING.lock().unwrap() = Some(entity.kind(host).map(|key| key.id(host)));
        }
        let answers = {
            let mut on_join = ON_JOIN.lock().unwrap();
            (on_join.0 == Some(event.entity)).then(|| on_join.0.take())
        };
        if answers.is_some() {
            let removes = ON_JOIN.lock().unwrap().2.take();
            let seal = removes.map(|_| host.transaction_open());
            let spawned = host.summon(
                "minecraft:overworld",
                "minecraft:pig",
                [1.5, 70.0, 1.5],
                Default::default(),
            );
            ON_JOIN.lock().unwrap().1.extend(spawned);
            if let (Some(entity), Some(seal)) = (removes, seal) {
                entity.remove(host, RemovalReason::Discarded);
                *PENDING.lock().unwrap() = Some(entity);
                host.transaction_close(&seal, true);
            }
        }
        let stage = NEST.load(Ordering::SeqCst);
        match (stage, event.kind) {
            // A's commit tells a pig: a command in the nether spawns a zombie, told at once.
            (1, "minecraft:pig") => {
                NEST.store(2, Ordering::SeqCst);
                host.commands().run_command(
                    "execute in minecraft:the_nether run summon minecraft:zombie 1.5 70 1.5",
                );
            }
            // Its listener commits a nether transaction of its own that spawned a cow.
            (2, "minecraft:zombie") => {
                NEST.store(3, Ordering::SeqCst);
                let seal = host.transaction_open();
                host.commands().run_command(
                    "execute in minecraft:the_nether run summon minecraft:cow 1.5 70 1.5",
                );
                host.transaction_close(&seal, true);
            }
            // The cow's join is told while that commit runs: a command in the overworld
            // spawns a chicken, which A's commit still holds.
            (3, "minecraft:cow") => {
                NEST.store(0, Ordering::SeqCst);
                host.commands().run_command(
                    "execute in minecraft:overworld run summon minecraft:chicken 1.5 70 1.5",
                );
            }
            _ => {}
        }
        if event.kind == "minecraft:sheep" {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn entity_leave_level(&self, _: &mut dyn ServerHost, event: &mut EntityLeaveLevelEvent<'_>) {
        let mut heard = HEARD.lock().unwrap();
        heard.left.push(event.entity);
        heard.order.push((false, event.entity));
    }
}

/// Runs the test `name` with the observer installed, in a fresh process (installed mods are
/// process-wide), on a dedicated server's overworld with chunk 0 0 loaded.
fn with_observer(name: &str, test: impl FnOnce(&mut Game<'_>)) {
    use super::native_entity_refs::tests::{run_fresh_cases, with_game};
    if run_fresh_cases(name, "HORIZON_TRANSACTION_ENTITIES_CHILD", &["valid"]) {
        return;
    }
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("journal", Box::new(Observer))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.native_joins_skip();
            test(game);
        });
    });
}

/// Summons `kind` with `nbt` at 1.5 70 1.5: its id.
fn summon(game: &mut Game<'_>, kind: &str, nbt: &str) -> Option<u32> {
    let (Tag::Compound(nbt), _) = parse_snbt(nbt).unwrap() else {
        panic!("not a compound");
    };
    let dimension = game.dim.id();
    game.summon(dimension, kind, [1.5, 70.0, 1.5], nbt)
}

/// Takes what the observer heard so far.
fn heard() -> Heard {
    std::mem::take(&mut *HEARD.lock().unwrap())
}

/// A boat with an announced cow riding it.
fn boat_with_rider(game: &mut Game<'_>) -> (EntityRef, EntityRef) {
    let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
    let cow = summon(game, "minecraft:cow", "{}").unwrap();
    let (boat, cow) = (game.entity(boat).unwrap(), game.entity(cow).unwrap());
    assert!(game.entity_start_riding(cow, boat, true));
    heard();
    (boat, cow)
}

#[test]
#[ignore = "daily"]
fn a_spawn_then_abort_leaves_no_entity_map_or_hook() {
    const NAME: &str =
        "game::transaction_entities_tests::a_spawn_then_abort_leaves_no_entity_map_or_hook";
    with_observer(NAME, |game| {
        let (mobs, next) = (game.mobs.len(), game.next_entity);
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity(pig).is_some(), "in the level inside the action");
        assert_eq!(MARK.get(game, Holder::Entity(pig)), Ok(1));
        assert_eq!(heard().constructed, [pig], "constructed at the spawn call");
        game.transaction_close(&seal, false);

        assert_eq!(game.entity(pig), None);
        assert_eq!(game.mobs.len(), mobs);
        assert!(!game.entity_attachments.contains_key(&pig));
        assert!(!game.persist.uuids.contains_key(&pig));
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default(), "no join and no leave");
        assert_eq!(game.next_entity, next + 1, "its id is burnt");
    });
}

#[test]
#[ignore = "daily"]
fn a_spawn_joins_at_the_root_commit() {
    const NAME: &str = "game::transaction_entities_tests::a_spawn_joins_at_the_root_commit";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        game.native_joins_flush(false, 0);
        assert!(heard().joined.is_empty(), "the join waits");
        game.transaction_close(&seal, true);
        assert_eq!(heard().joined, [pig]);
        assert!(game.entity(pig).is_some());
        assert_eq!(MARK.get(game, Holder::Entity(pig)), Ok(1));
    });
}

#[test]
#[ignore = "daily"]
fn a_join_cancelled_at_commit_leaves_a_stale_handle() {
    const NAME: &str =
        "game::transaction_entities_tests::a_join_cancelled_at_commit_leaves_a_stale_handle";
    with_observer(NAME, |game| {
        assert_eq!(
            summon(game, "minecraft:sheep", "{}"),
            None,
            "kept out at once"
        );
        heard();
        let sheep_before = game.mobs.len();
        let seal = game.transaction_open();
        let sheep = summon(game, "minecraft:sheep", "{}").expect("the handle comes back at once");
        assert!(game.entity(sheep).is_some());
        game.transaction_close(&seal, true);
        assert_eq!(game.entity(sheep), None);
        assert_eq!(game.mobs.len(), sheep_before);
        game.native_leaves_flush();
        let heard = heard();
        assert_eq!(heard.joined, [sheep]);
        assert!(heard.left.is_empty(), "never announced, so it never leaves");
    });
}

#[test]
#[ignore = "daily"]
fn outside_a_transaction_joins_and_leaves_are_told_at_the_call() {
    const NAME: &str = "game::transaction_entities_tests::outside_a_transaction_joins_and_leaves_are_told_at_the_call";
    with_observer(NAME, |game| {
        let next = game.next_entity;
        let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
        assert_eq!(boat, next + 1);
        assert_eq!(
            heard(),
            Heard {
                constructed: vec![boat],
                joined: vec![boat],
                from_disk: vec![],
                left: vec![],
                order: vec![(true, boat)],
            }
        );
        assert!(game.entity_discard(game.entity(boat).unwrap()));
        assert_eq!(heard().left, [boat]);
    });
}

#[test]
#[ignore = "daily"]
fn a_removal_then_abort_keeps_the_row_map_and_rider() {
    const NAME: &str =
        "game::transaction_entities_tests::a_removal_then_abort_keeps_the_row_map_and_rider";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        let pig = game.entity(pig).unwrap();
        MARK.set(game, Holder::Entity(boat.id()), 7).unwrap();
        heard();
        let seal = game.transaction_open();
        assert!(game.entity_discard(boat));
        assert!(game.entity_remove(pig, RemovalReason::Killed));
        assert_eq!(game.entity(boat.id()), None, "stale at once");
        assert_eq!(game.entity(pig.id()), None);
        assert!(!game.entity_discard(boat), "not removed twice");
        assert!(
            game.boats.iter().any(|b| b.id == boat.id()),
            "the row stays"
        );
        let mount = |game: &Game<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == cow.id())
                .unwrap()
                .extra
                .mount
        };
        assert_eq!(mount(game), Some(boat.id()), "the rider stays on");
        game.transaction_close(&seal, false);

        assert_eq!(game.entity(boat.id()), Some(boat));
        assert_eq!(game.entity(pig.id()), Some(pig));
        assert_eq!(MARK.get(game, Holder::Entity(boat.id())), Ok(7));
        assert_eq!(game.entity_vehicle(cow), Some(boat));
        assert!(game.persist.removals.is_empty());
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default());
    });
}

#[test]
#[ignore = "daily"]
fn a_removal_then_commit_runs_each_effect_once() {
    const NAME: &str =
        "game::transaction_entities_tests::a_removal_then_commit_runs_each_effect_once";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let seal = game.transaction_open();
        assert!(game.entity_discard(boat));
        game.transaction_close(&seal, true);

        assert_eq!(game.entity(boat.id()), None);
        assert_eq!(game.entity_vehicle(cow), None, "the rider got off");
        let boat_target = Target::Boat(boat.id());
        let queued = game.persist.removals.iter();
        assert_eq!(queued.filter(|t| **t == boat_target).count(), 1);
        game.native_leaves_flush();
        assert!(!game.boats.iter().any(|b| b.id == boat.id()));
        assert_eq!(heard().left, [boat.id()], "told once");
    });
}

#[test]
#[ignore = "daily"]
fn a_killed_mobs_oozing_spawns_only_at_commit() {
    const NAME: &str =
        "game::transaction_entities_tests::a_killed_mobs_oozing_spawns_only_at_commit";
    with_observer(NAME, |game| {
        let zombie = summon(game, "minecraft:zombie", "{}").unwrap();
        let at = game.mobs.iter().position(|m| m.id == zombie).unwrap();
        (game.mobs[at].extra.effects).push(StoredEffect::new("minecraft:oozing", 600, 0));
        let slimes = |game: &Game<'_>| {
            let slime = horizon_client::mobs::Species::Slime;
            game.mobs.iter().filter(|m| m.species() == slime).count()
        };
        let zombie = game.entity(zombie).unwrap();
        let seal = game.transaction_open();
        assert!(game.entity_remove(zombie, RemovalReason::Killed));
        assert_eq!(slimes(game), 0);
        game.transaction_close(&seal, true);
        assert_eq!(slimes(game), 2);
        assert!(game.mobs.iter().any(|m| m.id == zombie.id() && m.removed));
    });
}

#[test]
#[ignore = "daily"]
fn a_nested_commit_goes_with_its_parents_abort() {
    const NAME: &str =
        "game::transaction_entities_tests::a_nested_commit_goes_with_its_parents_abort";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let outer = game.transaction_open();
        let inner = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(boat));
        game.transaction_close(&inner, true);
        assert!(game.entity(pig).is_some());
        game.transaction_close(&outer, false);

        assert_eq!(game.entity(pig), None);
        assert_eq!(game.entity(boat.id()), Some(boat));
        assert_eq!(game.entity_vehicle(cow), Some(boat));
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard().joined, Vec::<u32>::new());
    });
}

#[test]
#[ignore = "daily"]
fn a_nested_abort_takes_back_only_its_own() {
    const NAME: &str = "game::transaction_entities_tests::a_nested_abort_takes_back_only_its_own";
    with_observer(NAME, |game| {
        let (boat, cow) = boat_with_rider(game);
        let outer = game.transaction_open();
        let kept = summon(game, "minecraft:pig", "{}").unwrap();
        let inner = game.transaction_open();
        let dropped = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(boat));
        game.transaction_close(&inner, false);
        assert_eq!(game.entity(dropped), None);
        assert_eq!(game.entity(boat.id()), Some(boat));
        game.transaction_close(&outer, true);

        assert!(game.entity(kept).is_some());
        assert_eq!(game.entity_vehicle(cow), Some(boat));
        game.native_leaves_flush();
        assert_eq!(
            heard(),
            Heard {
                constructed: vec![kept, dropped],
                joined: vec![kept],
                from_disk: vec![],
                left: vec![],
                order: vec![(true, kept)],
            }
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_spawned_then_removed_entity_never_joins_or_leaves() {
    const NAME: &str =
        "game::transaction_entities_tests::a_spawned_then_removed_entity_never_joins_or_leaves";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        let kept = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(game.entity(boat).unwrap()));
        assert!(game.entity_discard(game.entity(pig).unwrap()));
        game.transaction_close(&seal, true);
        game.native_leaves_flush();
        game.native_joins_flush(false, 0);
        let heard = heard();
        assert_eq!(heard.joined, [kept]);
        assert!(heard.left.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn another_writer_sees_the_spawn_and_its_writes_roll_back() {
    const NAME: &str =
        "game::transaction_entities_tests::another_writer_sees_the_spawn_and_its_writes_roll_back";
    with_observer(NAME, |game| {
        let witness = summon(game, "minecraft:cow", "{}").unwrap();
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        // A second mod, lent the host inside the same action.
        let host: &mut dyn ServerHost = game;
        assert!(host.entity(pig).is_some(), "it sees the spawn");
        MARK.set(host, Holder::Entity(pig), 5).unwrap();
        MARK.set(host, Holder::Entity(witness), 5).unwrap();
        game.transaction_close(&seal, false);

        assert_eq!(game.entity(pig), None);
        assert!(!game.entity_attachments.contains_key(&pig));
        assert_eq!(MARK.get(game, Holder::Entity(witness)), Ok(1));
    });
}

#[test]
#[ignore = "daily"]
fn ids_never_go_back_and_a_stale_handle_names_no_later_entity() {
    const NAME: &str = "game::transaction_entities_tests::ids_never_go_back_and_a_stale_handle_names_no_later_entity";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let aborted = summon(game, "minecraft:pig", "{}").unwrap();
        let handle = game.entity(aborted).unwrap();
        game.transaction_close(&seal, false);
        let next = summon(game, "minecraft:pig", "{}").unwrap();
        assert_eq!(next, aborted + 1);
        assert!(game.next_entity >= next);
        assert_eq!(game.native_entity_target(handle), None);
        assert_eq!(game.entity(aborted), None);
    });
}

#[test]
#[ignore = "daily"]
fn late_id_kinds_spawned_then_aborted_are_removed() {
    const NAME: &str =
        "game::transaction_entities_tests::late_id_kinds_spawned_then_aborted_are_removed";
    with_observer(NAME, |game| {
        let counts = |game: &Game<'_>| {
            [
                game.tnts.len(),
                game.falling.len(),
                game.rockets.len(),
                game.tridents.len(),
                game.creatures.projectiles.len(),
            ]
        };
        let before = counts(game);
        let seal = game.transaction_open();
        for (kind, nbt) in [
            ("minecraft:tnt", "{fuse:80s}"),
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:firework_rocket", "{LifeTime:20}"),
            ("minecraft:trident", "{}"),
            ("minecraft:arrow", "{}"),
        ] {
            summon(game, kind, nbt).unwrap();
        }
        assert_eq!(counts(game), before.map(|n| n + 1));
        game.transaction_close(&seal, false);
        assert_eq!(counts(game), before);
        game.native_joins_flush(false, 0);
        assert!(heard().joined.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_traveller_arriving_inside_an_aborted_action_stays() {
    const NAME: &str =
        "game::transaction_entities_tests::a_traveller_arriving_inside_an_aborted_action_stays";
    with_observer(NAME, |game| {
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let item = summon(
            game,
            "minecraft:item",
            "{Item:{id:\"minecraft:diamond\",count:3}}",
        );
        game.enter_level(Dim::Overworld).unwrap();
        let diamonds = |game: &Game<'_>| {
            (game.dropped.iter())
                .filter(|d| &*d.stack.item == "minecraft:diamond")
                .count()
        };
        let seal = game.transaction_open();
        // As `/execute in the_nether run tp …` does from inside the action.
        game.enter_level(Dim::Nether).unwrap();
        assert!(game.teleport_to_level(
            Target::Item(item.unwrap()),
            Dim::Overworld,
            [1.5, 70.0, 1.5],
            None
        ));
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(diamonds(game), 1, "arrived in the action's level");
        game.transaction_close(&seal, false);
        assert_eq!(diamonds(game), 1, "not taken out with the spawns");
    });
}

#[test]
#[ignore = "daily"]
fn an_aborted_spawn_is_in_no_tracking_pass() {
    const NAME: &str = "game::transaction_entities_tests::an_aborted_spawn_is_in_no_tracking_pass";
    with_observer(NAME, |game| {
        // Above the guest's fixed entity id.
        game.next_entity = 100;
        let _rx = super::lan::world_spawn_sync_tests::host_for(game);
        let tracked = |game: &mut Game<'_>, id: u32| {
            let mut states = horizon_client::lan::entity::States::default();
            game.lan_entities(&mut states, false, 0, None);
            states.as_slice().iter().any(|e| e.id == id as i32)
        };
        let seal = game.transaction_open();
        let aborted = summon(game, "minecraft:pig", "{}").unwrap();
        game.transaction_close(&seal, false);
        let seal = game.transaction_open();
        let committed = summon(game, "minecraft:pig", "{}").unwrap();
        game.transaction_close(&seal, true);
        assert!(!tracked(game, aborted));
        assert!(tracked(game, committed));
    });
}

/// One mod spawn function, called with everything it needs: whether it spawned.
type Spawn = fn(&mut Game<'_>, Fixture) -> bool;

#[derive(Clone, Copy)]
struct Fixture {
    level: mod_api::LevelRef,
    mob: EntityRef,
}

#[test]
#[ignore = "daily"]
fn exhausted_ids_refuse_every_mod_spawn() {
    const NAME: &str = "game::transaction_entities_tests::exhausted_ids_refuse_every_mod_spawn";
    let rows: [(&str, Spawn); 13] = [
        ("summon", |game, _| {
            summon(game, "minecraft:pig", "{}").is_some()
        }),
        ("spawn_falling_block", |game, _| {
            let sand = (game.pack.assets.registry)
                .default_state("minecraft:sand")
                .unwrap();
            let dimension = game.dim.id();
            game.spawn_falling_block(
                dimension,
                sand,
                [1.5, 72.0, 1.5],
                [0.0; 3],
                Default::default(),
            )
        }),
        ("entity_spawn_at_location", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            game.entity_spawn_at_location(f.mob, stack, 0.0).is_some()
        }),
        ("drop_at", |game, _| {
            let before = game.dropped.len();
            game.drop_at([1, 70, 1], ItemStack::new("minecraft:stone", 1));
            game.dropped.len() > before
        }),
        ("level_spawn_mob", |game, f| {
            let reason = mod_api::SpawnReason::Command;
            (game.level_spawn_mob(f.level, "minecraft:cow", [1.5, 70.0, 1.5], reason)).is_some()
        }),
        ("level_block_drops", |game, f| {
            game.level_block_drops(f.level, [1, 70, 1], None, None)
                .is_some()
        }),
        ("level_explode", |game, f| {
            let none = mod_api::ExplosionInteraction::None;
            let settings = mod_api::ExplosionSettings::default();
            (game.level_explode(f.level, None, [1.5, 70.0, 14.5], 1.0, false, none, settings))
                .is_ok()
        }),
        ("level_pop_resource", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            game.level_pop_resource(f.level, [1, 70, 1], stack)
                .is_some()
        }),
        ("level_spawn_item", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            (game.level_spawn_item(f.level, 1.5, 70.0, 1.5, stack, 10)).is_some()
        }),
        ("level_spawn_item_with_motion", |game, f| {
            let stack = ItemStack::new("minecraft:stone", 1);
            game.level_spawn_item_with_motion(f.level, 1.5, 70.0, 1.5, stack, [0.0; 3], 10)
                .is_some()
        }),
        ("living_drop", |game, f| {
            let living = game.living(f.mob).unwrap();
            let stack = ItemStack::new("minecraft:stone", 1);
            game.living_drop(living, stack, true, false).is_some()
        }),
        ("player_drop_stack", |game, _| {
            let player = game.player(123).unwrap();
            let stack = ItemStack::new("minecraft:stone", 1);
            game.player_drop_stack(player, stack, false).is_none()
        }),
        ("spawn_projectile_from_rotation", |game, f| {
            let owner = game.living(f.mob).unwrap();
            let snowball = mod_api::Identifier::parse("minecraft:snowball").unwrap();
            let stack = ItemStack::new("minecraft:snowball", 1);
            (game.spawn_projectile_from_rotation(owner, &snowball, &stack, 0.0, 1.5, 1.0)).is_some()
        }),
    ];
    with_observer(NAME, |game| {
        let player = horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(1.5, 70.0, 1.5),
        );
        game.player = Some(player.unwrap());
        super::native_entity_refs::tests::bind_local_player(game, 20, 123);
        let mob = summon(game, "minecraft:zombie", "{}").unwrap();
        let fixture = Fixture {
            level: game.level(game.dim.id()).unwrap(),
            mob: game.entity(mob).unwrap(),
        };
        // Each spawns below the headroom; ids only ever go up.
        let last = u32::MAX - ENTITY_ID_HEADROOM;
        game.next_entity = last - 1024;
        for (name, spawn) in rows {
            assert!(spawn(game, fixture), "{name} spawns below the headroom");
        }
        assert!(!game.native_lifecycle.ids_exhausted_told);
        game.next_entity = last;
        for (name, spawn) in rows {
            let entities = game.live_entity_ids().len();
            assert!(!spawn(game, fixture), "{name} refuses");
            assert_eq!(game.next_entity, last, "{name} takes no id");
            assert_eq!(
                game.live_entity_ids().len(),
                entities,
                "{name} adds nothing"
            );
        }
        assert!(game.native_lifecycle.ids_exhausted_told);
    });
}

#[test]
#[ignore = "daily"]
fn exhausted_ids_stop_the_tick_spawners() {
    const NAME: &str = "game::transaction_entities_tests::exhausted_ids_stop_the_tick_spawners";
    with_observer(NAME, |game| {
        game.next_entity = u32::MAX - ENTITY_ID_HEADROOM - 1;
        game.tick_spawners();
        assert!(!game.native_lifecycle.ids_exhausted_told);
        game.next_entity = u32::MAX - ENTITY_ID_HEADROOM;
        game.tick_spawners();
        assert!(game.native_lifecycle.ids_exhausted_told, "named in the log");
        assert_eq!(game.next_entity, u32::MAX - ENTITY_ID_HEADROOM);
    });
}

#[test]
#[ignore = "daily"]
fn an_aborted_spawn_puts_back_the_raw_record_it_wrote_over() {
    const NAME: &str =
        "game::transaction_entities_tests::an_aborted_spawn_puts_back_the_raw_record_it_wrote_over";
    with_observer(NAME, |game| {
        // Crystals are kept as raw records by kind and position: a second one at the same place
        // writes over the first's.
        let own = summon(game, "minecraft:end_crystal", r#"{Tags:["own"]}"#).unwrap();
        let seal = game.transaction_open();
        summon(game, "minecraft:end_crystal", r#"{Tags:["aborted"]}"#).unwrap();
        game.transaction_close(&seal, false);

        assert!(game.entity(own).is_some());
        let saved = game.entity_chunks(&std::collections::HashSet::from([(0, 0)]), false);
        let tags: Vec<_> = saved
            .iter()
            .flat_map(|(_, records)| records)
            .filter_map(|record| record.get("Tags").cloned())
            .collect();
        let own_tags = Tag::List(vec![Tag::String("own".into())]);
        assert_eq!(tags, [own_tags], "the crystal is saved with its own record");
    });
}

#[test]
#[ignore = "daily"]
fn a_held_join_keeps_loaded_from_disk() {
    const NAME: &str = "game::transaction_entities_tests::a_held_join_keeps_loaded_from_disk";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let fresh = summon(game, "minecraft:pig", "{}").unwrap();
        let (saved, _) = parse_snbt(r#"{id:"minecraft:cow",Pos:[1.5d,70.0d,1.5d]}"#).unwrap();
        game.spawn_loaded_entity((0, 0), saved, None);
        assert!(heard().joined.is_empty(), "both joins wait");
        game.transaction_close(&seal, true);

        let heard = heard();
        assert_eq!(heard.joined.len(), 2);
        assert_eq!(heard.joined[0], fresh, "in the order they were added");
        assert_eq!(
            heard.from_disk,
            [heard.joined[1]],
            "only the saved cow came from disk"
        );
    });
}

#[test]
#[ignore = "daily"]
fn commit_tells_the_joins_before_the_effects_of_the_removals() {
    const NAME: &str = "game::transaction_entities_tests::commit_tells_the_joins_before_the_effects_of_the_removals";
    with_observer(NAME, |game| {
        let zombie = summon(game, "minecraft:zombie", "{}").unwrap();
        let at = game.mobs.iter().position(|m| m.id == zombie).unwrap();
        (game.mobs[at].extra.effects).push(StoredEffect::new("minecraft:oozing", 600, 0));
        let zombie = game.entity(zombie).unwrap();
        heard();
        let seal = game.transaction_open();
        assert!(game.entity_remove(zombie, RemovalReason::Killed));
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        game.transaction_close(&seal, true);

        let heard = heard();
        assert_eq!(
            heard.joined.len(),
            3,
            "the pig, then the two slimes the death spawns"
        );
        assert_eq!(heard.joined[0], pig, "the tentative entity joins first");
        assert!(heard.joined[1..].iter().all(|id| *id > pig));
    });
}

#[test]
#[ignore = "daily"]
fn a_pending_boat_removal_is_hidden_from_selectors_until_it_commits() {
    const NAME: &str = "game::transaction_entities_tests::a_pending_boat_removal_is_hidden_from_selectors_until_it_commits";
    with_observer(NAME, |game| {
        let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
        let handle = game.entity(boat).unwrap();
        let target = Target::Boat(boat);
        assert!(game.level_candidates().contains(&target));

        let seal = game.transaction_open();
        assert!(game.entity_discard(handle));
        assert_eq!(game.entity(boat), None, "the handle is stale");
        assert!(
            !game.level_candidates().contains(&target),
            "selectors do not find it"
        );
        assert!(!game.global_selector_candidates().contains(&target));
        game.transaction_close(&seal, false);
        assert_eq!(game.entity(boat), Some(handle));
        assert!(
            game.level_candidates().contains(&target),
            "an abort brings it back"
        );

        let seal = game.transaction_open();
        assert!(game.entity_discard(handle));
        game.transaction_close(&seal, true);
        assert_eq!(game.entity(boat), None);
        assert!(
            !game.level_candidates().contains(&target),
            "a commit removed it"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_spawn_a_join_listener_makes_during_the_commit_joins_once_after_it() {
    const NAME: &str = "game::transaction_entities_tests::a_spawn_a_join_listener_makes_during_the_commit_joins_once_after_it";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let a = summon(game, "minecraft:pig", "{}").unwrap();
        let b = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(game.entity(b).unwrap()));
        ON_JOIN.lock().unwrap().0 = Some(a);
        game.transaction_close(&seal, true);

        let spawned = ON_JOIN.lock().unwrap().1.clone();
        let [c] = spawned[..] else {
            panic!("the listener spawned one entity, not {spawned:?}")
        };
        let told = heard();
        assert_eq!(
            told.joined,
            [a, c],
            "a, then c once, and never the removed b"
        );
        assert!(told.left.is_empty(), "b never leaves: {:?}", told.left);
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default(), "nothing is told twice");
        assert!(game.entity(c).is_some());
        assert_eq!(game.entity(b), None);
    });
}

#[test]
#[ignore = "daily"]
fn a_transaction_covers_its_own_level_and_another_levels_spawn_is_told_at_once() {
    const NAME: &str = "game::transaction_entities_tests::a_transaction_covers_its_own_level_and_another_levels_spawn_is_told_at_once";
    with_observer(NAME, |game| {
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.enter_level(Dim::Overworld).unwrap();
        let seal = game.transaction_open();
        // As `/execute in the_nether run summon …` does from inside the action.
        game.enter_level(Dim::Nether).unwrap();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert_eq!(heard().joined, [pig], "the nether's spawn is told at once");
        assert!(game.entity(pig).is_some());
        assert!(game.entity_discard(game.entity(pig).unwrap()));
        assert_eq!(game.entity(pig), None, "and its removal happens at once");
        assert_eq!(heard().left, [pig]);
        let kept = summon(game, "minecraft:cow", "{}").unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        game.transaction_close(&seal, false);

        game.enter_level(Dim::Nether).unwrap();
        assert!(
            game.entity(kept).is_some(),
            "an abort takes back the action's level only"
        );
        game.enter_level(Dim::Overworld).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn a_level_holds_its_spawns_for_its_own_transaction_under_another_levels() {
    const NAME: &str = "game::transaction_entities_tests::a_level_holds_its_spawns_for_its_own_transaction_under_another_levels";
    with_observer(NAME, |game| {
        let overworld = game.transaction_open();
        // As a command run `in` the nether that opens a transaction of its own does.
        game.enter_level(Dim::Nether).unwrap();
        let nether = game.transaction_open();
        game.enter_level(Dim::Overworld).unwrap();
        // The nether's transaction is the innermost; the overworld's still holds this spawn.
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity(pig).is_some());
        assert!(
            heard().joined.is_empty(),
            "held for the overworld's transaction"
        );
        game.enter_level(Dim::Nether).unwrap();
        game.transaction_close(&nether, true);
        game.enter_level(Dim::Overworld).unwrap();
        assert!(heard().joined.is_empty(), "still held");
        game.transaction_close(&overworld, false);

        assert_eq!(
            game.entity(pig),
            None,
            "taken back with the overworld's abort"
        );
        game.native_joins_flush(false, 0);
        assert_eq!(heard(), Heard::default(), "and never told");
    });
}

#[test]
#[ignore = "daily"]
fn a_nested_commit_from_a_join_listener_tells_its_join_before_its_removal() {
    const NAME: &str = "game::transaction_entities_tests::a_nested_commit_from_a_join_listener_tells_its_join_before_its_removal";
    with_observer(NAME, |game| {
        // A boat, not a mob: a mob reads as removed by its flag, not by the pending list.
        let d = summon(game, "minecraft:oak_boat", "{}").unwrap();
        game.native_joins_flush(false, 0);
        heard();
        let seal = game.transaction_open();
        let a = summon(game, "minecraft:pig", "{}").unwrap();
        // a's join listener commits a transaction of its own that spawns c and removes d.
        {
            let mut on_join = ON_JOIN.lock().unwrap();
            on_join.0 = Some(a);
            on_join.2 = game.entity(d);
        }
        game.transaction_close(&seal, true);

        let c = ON_JOIN.lock().unwrap().1[0];
        assert_eq!(
            heard().order,
            [(true, a), (true, c), (false, d)],
            "c's join is told before d's removal takes effect, and nothing twice"
        );
        assert_eq!(
            KIND_OF_PENDING.lock().unwrap().take(),
            Some(None),
            "d reads as gone to c's join listener while its removal waits"
        );
        assert_eq!(game.entity(d), None);
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default());
    });
}

#[test]
#[ignore = "daily"]
fn a_level_commit_told_inside_another_levels_commit_keeps_its_own_batch() {
    const NAME: &str = "game::transaction_entities_tests::a_level_commit_told_inside_another_levels_commit_keeps_its_own_batch";
    with_observer(NAME, |game| {
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.enter_level(Dim::Overworld).unwrap();
        let seal = game.transaction_open();
        let a = summon(game, "minecraft:pig", "{}").unwrap();
        let b = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(game.entity(b).unwrap()));
        NEST.store(1, Ordering::SeqCst);
        game.transaction_close(&seal, true);

        let told = heard();
        assert_eq!(NEST.load(Ordering::SeqCst), 0, "the script ran to its end");
        assert_eq!(
            told.joined.len(),
            4,
            "a, the zombie, the cow, the chicken: {told:?}"
        );
        assert_eq!(told.joined[0], a);
        assert!(!told.joined.contains(&b), "the removed b is never told");
        assert!(told.left.is_empty(), "and never leaves: {:?}", told.left);
        game.native_joins_flush(false, 0);
        game.native_leaves_flush();
        assert_eq!(heard(), Heard::default(), "nothing is told twice");
    });
}

#[test]
#[ignore = "daily"]
fn a_block_break_at_the_commit_never_announces_a_mob_the_transaction_removed() {
    const NAME: &str = "game::transaction_entities_tests::a_block_break_at_the_commit_never_announces_a_mob_the_transaction_removed";
    with_observer(NAME, |game| {
        // A poppy the pour of water breaks: `level_place_fluid` makes its drops at the call and
        // `After::FluidWrite` spawns them at the root commit (`commit_fluid_write`, `flush_spill`),
        // whose item join flushes joins as the commit's own effect.
        let at = [2, 65, 2];
        let poppy = (game.pack.assets.registry)
            .default_state("minecraft:poppy")
            .unwrap();
        game.world.set(at[0], at[1], at[2], poppy).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let water = mod_api::__private::fluid_key("minecraft:water").unwrap();
        let items = game.dropped.len();
        let seal = game.transaction_open();
        let pig = summon(game, "minecraft:pig", "{}").unwrap();
        assert!(game.entity_discard(game.entity(pig).unwrap()));
        assert!(game.level_place_fluid(level, at, water, None, false, seal.transaction()));
        assert_eq!(game.dropped.len(), items, "the drop waits for the commit");
        game.transaction_close(&seal, true);
        assert_eq!(game.dropped.len(), items + 1, "the broken poppy dropped");
        let told = heard();
        assert!(
            !told.joined.contains(&pig),
            "the removed pig is never told: {told:?}"
        );
        assert_eq!(
            told.joined.len(),
            1,
            "only the dropped item joins: {told:?}"
        );
        game.native_leaves_flush();
        assert!(heard().left.is_empty(), "and the pig never leaves");
    });
}

#[test]
#[ignore = "daily"]
fn an_exhausted_level_pop_draws_nothing_from_the_level_random() {
    use horizon_core::random::RandomSource;
    const NAME: &str = "game::transaction_entities_tests::an_exhausted_level_pop_draws_nothing_from_the_level_random";
    with_observer(NAME, |game| {
        let level = game.level(game.dim.id()).unwrap();
        game.next_entity = u32::MAX - ENTITY_ID_HEADROOM;
        let expected = game.world.random.clone().next_long();
        let stack = ItemStack::new("minecraft:stone", 1);
        assert!(game.level_pop_resource(level, [1, 70, 1], stack).is_none());
        assert_eq!(game.world.random.clone().next_long(), expected);
        assert_eq!(game.next_entity, u32::MAX - ENTITY_ID_HEADROOM);
    });
}

#[test]
#[ignore = "daily"]
fn a_parked_levels_pending_removal_is_hidden_from_another_level_until_it_aborts() {
    use horizon_client::join_seq::residency::table::EntityLookup;
    const NAME: &str = "game::transaction_entities_tests::a_parked_levels_pending_removal_is_hidden_from_another_level_until_it_aborts";
    with_observer(NAME, |game| {
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        let boat = summon(game, "minecraft:oak_boat", "{}").unwrap();
        let target = super::commands::Target::Boat(boat);
        MARK.set(game, Holder::Entity(boat), 7).unwrap();
        // Parked, nothing pending: the boat is found and visible (else the checks below prove
        // nothing).
        game.enter_level(Dim::Nether).unwrap();
        assert!(
            matches!(
                game.server_resident_entity(boat),
                EntityLookup::Found { .. }
            ),
            "the parked boat is in the resident table"
        );
        assert!(game.resident_source_target_visible(mod_api::DimensionKey::OVERWORLD, target));
        // An attachment is reached through the level in place only (`attachment_slot` asks the
        // resident table for `self.dim`), so from the nether the overworld boat's is out of reach,
        // pending removal or not.
        assert!(MARK.get(game, Holder::Entity(boat)).is_err());
        game.enter_level(Dim::Overworld).unwrap();
        let seal = game.transaction_open();
        assert!(game.entity_discard(game.entity(boat).unwrap()));
        game.enter_level(Dim::Nether).unwrap();
        assert!(
            matches!(game.server_resident_entity(boat), EntityLookup::Missing),
            "the parked level's boat reads as gone while its removal is pending"
        );
        assert!(!game.resident_source_target_visible(mod_api::DimensionKey::OVERWORLD, target));
        assert!(MARK.get(game, Holder::Entity(boat)).is_err());
        game.enter_level(Dim::Overworld).unwrap();
        game.transaction_close(&seal, false);
        game.enter_level(Dim::Nether).unwrap();
        assert!(
            matches!(
                game.server_resident_entity(boat),
                EntityLookup::Found { .. }
            ),
            "the abort put it back"
        );
        assert!(game.resident_source_target_visible(mod_api::DimensionKey::OVERWORLD, target));
        assert!(MARK.get(game, Holder::Entity(boat)).is_err());
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(
            MARK.get(game, Holder::Entity(boat)),
            Ok(7),
            "kept by the abort"
        );
    });
}

#[test]
#[ignore = "daily"]
fn each_spawn_in_a_transaction_is_constructed_at_its_own_call_and_joins_at_the_commit() {
    const NAME: &str = "game::transaction_entities_tests::each_spawn_in_a_transaction_is_constructed_at_its_own_call_and_joins_at_the_commit";
    with_observer(NAME, |game| {
        let seal = game.transaction_open();
        let a = summon(game, "minecraft:pig", "{}").unwrap();
        let told = heard();
        assert_eq!(told.constructed, [a], "a is constructed at its call");
        assert!(told.joined.is_empty(), "and its join is held");
        let b = summon(game, "minecraft:pig", "{}").unwrap();
        let told = heard();
        assert_eq!(
            told.constructed,
            [b],
            "b is constructed at its call, once and alone"
        );
        assert!(told.joined.is_empty(), "and its join is held");
        game.transaction_close(&seal, true);
        let told = heard();
        assert_eq!(told.joined, [a, b], "both join at the commit, in order");
        assert!(told.constructed.is_empty(), "nothing is constructed twice");
    });
}
