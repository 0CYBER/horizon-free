//! The `entity_move` sweep (#2414, Ari's 1b): one entity of every walk
//! family, one full family round each, and a counting listener that must
//! hear exactly each family's case - the backstop the clippy ban cannot
//! give, because a forgotten family moves without any resolve call. The
//! cases are generated from `entity_lists!` itself and the coverage is
//! asserted against the real `entity_handles()` walk (review round 1).
use super::native_entity_refs::tests::with_game;
use mod_api::{EntityMoveEvent, Flow, Hooks, LoopContext, MoveCause};
use std::sync::{Arc, Mutex};

/// `kind -> (fires, causes)` heard during the round.
type Heard = Vec<(String, MoveCause)>;

struct Counter(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Counter {
    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push((event.kind.to_owned(), event.cause));
        Flow::Continue
    }
}

/// The walk's families, straight from `entity_lists!` through its own
/// `walk_lists` expansion (review round 1): the macro walks one field per
/// list, so a family added to the lists fails to compile here until
/// [`Families`] — and a case in [`CASES`] below — names it. No hand-written
/// census to drift from the lists.
struct Families {
    mobs: [&'static str; 1],
    dropped: [&'static str; 1],
    boats: [&'static str; 1],
    carts: [&'static str; 1],
    armor_stands: [&'static str; 1],
    tnts: [&'static str; 1],
    falling: [&'static str; 1],
    orbs: [&'static str; 1],
    projectiles: [&'static str; 1],
    rockets: [&'static str; 1],
    tridents: [&'static str; 1],
    general: [&'static str; 1],
}

/// One census case: the family (as `EntityHandle`'s Debug names it), the
/// representative's entity kind, and the fires its own round must produce —
/// one self move for the walk movers, none for vanilla's `setPos` flyers
/// (#2414 point 3) and none for the push-only general rows.
struct Case {
    family: &'static str,
    kind: &'static str,
    fires: &'static [MoveCause],
}

/// The walk families' cases: one per entry of `entity_lists!`, checked
/// against that list by [`Families`] and against the live walk below.
const CASES: &[Case] = &[
    Case {
        family: "Mob",
        kind: "minecraft:zombie",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Item",
        kind: "minecraft:item",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Boat",
        kind: "minecraft:oak_boat",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Cart",
        kind: "minecraft:minecart",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "ArmorStand",
        kind: "minecraft:armor_stand",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Tnt",
        kind: "minecraft:tnt",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Falling",
        kind: "minecraft:falling_block",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Orb",
        kind: "minecraft:experience_orb",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Projectile",
        kind: "minecraft:arrow",
        fires: &[],
    },
    Case {
        family: "Rocket",
        kind: "minecraft:firework_rocket",
        fires: &[MoveCause::SelfMovement],
    },
    Case {
        family: "Trident",
        kind: "minecraft:trident",
        fires: &[],
    },
    Case {
        family: "General",
        kind: "mymod:robot",
        fires: &[],
    },
];

/// Declares the census's general entity type: the `General` family's
/// representative, part-free and unsaved, loaded as a record below.
struct DeclareGeneral;

#[mod_api::__subscriptions]
impl Hooks for DeclareGeneral {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.declare_general_entity(
            "robot",
            mod_api::GeneralEntityCategory::Misc,
            0.8,
            1.9,
            mod_api::GeneralSavePolicy::NoSave,
            false,
            8,
            3,
            1,
            &[],
        )
        .unwrap();
    }
}

/// The handle's family, as `EntityHandle`'s Debug names it (`Mob(3)`): every
/// variant is a tuple of its index, so the name is the part before `(`.
fn family_of(handle: super::entity::EntityHandle) -> String {
    let printed = format!("{handle:?}");
    printed[..printed.find('(').expect("a tuple variant")].to_owned()
}

/// One entity of every walk family, one full family round each, and a
/// counting listener: the cases come from `entity_lists!` itself
/// ([`Families`]), the coverage is asserted against the real
/// `entity_handles()` walk — every family the walk visits has a case, every
/// case is live in the walk — and each family fires exactly its case's
/// causes. The backstop the clippy ban cannot give, because a forgotten
/// family moves without any resolve call (review round 1).
#[test]
#[ignore = "daily"]
fn every_walk_family_of_the_entity_lists_fires_its_case() {
    mod_platform::__private::initialize().unwrap();
    // The registration phase owns the general type the General case loads;
    // the ticks run in World, as the server's own do.
    let _registration =
        mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new("counter", Box::new(Counter(log.clone()))),
        mod_api::ModEntry::new("mymod", Box::new(DeclareGeneral)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(crate::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            let _world = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            // One representative per family, each in its own column high
            // over the floor, airborne or in flight so its family's own tick
            // moves it.
            let at = |i: usize| [4.5 + f64::from(i as u16), 40.5, 5.5];
            for (i, (name, kind)) in [
                (0, ("zombie", "minecraft:zombie")),
                (1, ("item", "minecraft:item")),
                (2, ("boat", "minecraft:oak_boat")),
                (3, ("minecart", "minecraft:minecart")),
                (4, ("armor_stand", "minecraft:armor_stand")),
                (5, ("tnt", "minecraft:tnt")),
                (8, ("rocket", "minecraft:firework_rocket")),
                (9, ("arrow", "minecraft:arrow")),
            ] {
                let nbt = match name {
                    "item" => "{Item:{id:\"minecraft:stone\",Count:1b},Motion:[0.5d,0.0d,0.0d]}",
                    "arrow" => "{Motion:[1.0d,0.0d,0.0d]}",
                    _ => "{Motion:[0.5d,0.0d,0.0d]}",
                };
                let at = at(i);
                game.summon_with_nbt(kind, at, nbt)
                    .unwrap_or_else(|e| panic!("summon {name}: {e:?}"));
            }
            // The orb: the summon command's own spawn (value 0), moved by
            // its tick's gravity.
            assert!(game.run_command(&format!("summon experience_orb {} 40.5 5.5", at(7)[0])));
            // The falling block: thrown by the game's own spawn, eastbound.
            assert!(game.spawn_falling_block(
                "minecraft:overworld",
                stone,
                at(6),
                [0.5, 0.0, 0.0],
                mod_api::Falling::default()
            ));
            // The trident: a thrown one in flight, a `setPos` flyer.
            let id = game.take_entity_id();
            let mut trident = super::trident::ThrownTrident::remote(
                id,
                horizon_core::geometry::Vec3::new(at(10)[0], 40.5, at(10)[2]),
                10.0,
                0.0,
            );
            trident.velocity = horizon_core::geometry::Vec3::new(1.0, 0.0, 0.0);
            game.tridents.push(trident);
            // The general row: the declared type's own record, loaded as the
            // game loads one from a save.
            let record = crate::game::general::GeneralEntityRecord {
                position: at(11),
                velocity: [0.5, 0.0, 0.0],
                rotation: [0.0, 90.0],
                uuid: [11, 12, 13, 14],
                envelope: crate::game::general::GeneralTypeEnvelope {
                    type_key: "mymod:robot".to_owned(),
                    payload_version: 3,
                    payload_bytes: Vec::new(),
                },
                passengers: Vec::new(),
            };
            game.load_listed("mymod:robot", None, None, record.to_tag())
                .unwrap();
            // The cases must be exactly the families of `entity_lists!`:
            // its own walk of [`Families`] names them, one field per list.
            let families = Families {
                mobs: ["Mob"],
                dropped: ["Item"],
                boats: ["Boat"],
                carts: ["Cart"],
                armor_stands: ["ArmorStand"],
                tnts: ["Tnt"],
                falling: ["Falling"],
                orbs: ["Orb"],
                projectiles: ["Projectile"],
                rockets: ["Rocket"],
                tridents: ["Trident"],
                general: ["General"],
            };
            let mut listed = Vec::new();
            super::entity::entity_lists!(walk_lists, families, family, {
                listed.push(family);
            });
            let case_families: std::collections::BTreeSet<&str> =
                CASES.iter().map(|case| case.family).collect();
            let listed_families: std::collections::BTreeSet<&str> =
                listed.iter().copied().collect();
            assert_eq!(
                listed_families, case_families,
                "every family of entity_lists! has a census case"
            );
            // The coverage runs against the real walk, before the round: a
            // family's representative may spend itself in the round (a
            // rocket flies out), so the census is taken of the world the
            // round is about to run on — every family the walk visits is a
            // case, and every case is live in the walk.
            let present: std::collections::BTreeSet<String> =
                game.entity_handles().map(family_of).collect();
            assert_eq!(
                present
                    .iter()
                    .map(String::as_str)
                    .collect::<std::collections::BTreeSet<_>>(),
                case_families,
                "the walk visits exactly the census cases' families"
            );
            log.lock().unwrap().clear();
            // One round of every family's tick, the ordered pass covering
            // the orbs, the TNT and the falling blocks in join order.
            let mut round = game.native_tick_round();
            game.tick_dropped_items(&mut round);
            let _ = game.tick_creatures();
            game.tick_boats();
            game.tick_carts();
            game.tick_rockets();
            let _ = game.tick_armor_stands();
            let _ = game.tick_projectiles();
            game.tick_tridents();
            game.tickets.update(None);
            // Existing residents whose spawn packets were already announced
            // (as the ordered tick's own test sets it): the round's TNT and
            // falling blocks go in the first pass.
            game.ordered.announced = horizon_client::join_seq::JoinSeq::MAX;
            game.begin_ordered_ticks();
            game.run_ordered_ticks(horizon_client::join_seq::JoinSeq::MAX, false)
                .unwrap();
            game.tick_general().unwrap();
            let heard = std::mem::take(&mut *log.lock().unwrap());
            let mut by_kind: std::collections::BTreeMap<&str, Vec<MoveCause>> = Default::default();
            for (kind, cause) in &heard {
                by_kind.entry(kind).or_default().push(*cause);
            }
            for case in CASES {
                let causes = by_kind.get(case.kind).map(Vec::as_slice).unwrap_or(&[]);
                assert_eq!(
                    causes, case.fires,
                    "{} ({}) fired its case's causes, got {causes:?}",
                    case.family, case.kind
                );
            }
        });
    });
}

/// Arrows in flight are vanilla `setPos` movers: their own tick fires
/// nothing; a piston push fires once with the piston cause (#2414 point 3).
#[test]
#[ignore = "daily"]
fn an_arrow_in_flight_fires_nothing_and_a_piston_push_fires_once() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "counter",
        Box::new(Counter(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                    chunk.set(x, 20, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            assert!(game.run_command("summon arrow 5.5 15.5 5.5 {Motion:[1.0d,0.0d,0.0d]}"));
            log.lock().unwrap().clear();
            let _ = game.tick_projectiles();
            let heard = std::mem::take(&mut *log.lock().unwrap());
            assert!(
                heard.is_empty(),
                "an arrow's own flight is setPos, not Entity.move: {heard:?}"
            );
        });
    });
}

/// The movers `entity_handles()` never sees — the dragon and the player's
/// own server simulation — through `entity_ids_off_walk`, as the #2414
/// ruling asks: the dragon's physics-less step and the player window's
/// `move_self` both fire with the self cause. (The bobber and a LAN guest
/// need their own fixtures - a rod cast and a hosted connection - and stay
/// for the live run.)
#[test]
#[ignore = "daily"]
fn the_off_walk_movers_fire_too() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "counter",
        Box::new(Counter(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                    chunk.set(x, 40, z, stone.0);
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            // The dragon: off the walk, moved by its own physics-less step.
            game.spawn_remote_dragon(605, [5.0, 30.0, 5.0], 0.0);
            let off_walk: std::collections::HashSet<u32> = game.entity_ids_off_walk().collect();
            let dragon_id = game.dragon.as_ref().unwrap().id;
            assert!(
                off_walk.contains(&dragon_id),
                "the dragon is an off-walk mover"
            );
            log.lock().unwrap().clear();
            // Overworld: no fight, so the fight tick runs `aiStep` directly.
            game.tick_fight();
            let heard = std::mem::take(&mut *log.lock().unwrap());
            assert!(
                heard.iter().any(|(kind, cause)| {
                    kind == "minecraft:ender_dragon" && *cause == MoveCause::SelfMovement
                }),
                "the dragon's step fires with the self cause: {heard:?}"
            );
            // The player window: also off the walk, moved by its own
            // `move_self` through the server's own simulation of it. The
            // fixture's game carries no player until one moves in.
            use horizon_client::motion::PlayerMotion;
            if game
                .player
                .as_ref()
                .is_none_or(|p| p.position.y < 20.0 || p.position.y > 45.0)
            {
                game.player = Some(
                    PlayerMotion::new(
                        game.motion,
                        horizon_core::geometry::Vec3::new(5.5, 30.0, 5.5),
                    )
                    .unwrap(),
                );
            }
            game.player.as_mut().unwrap().velocity =
                horizon_core::geometry::Vec3::new(0.5, 0.0, 0.0);
            log.lock().unwrap().clear();
            {
                use horizon_client::motion::MotionWorld;
                let mut player = game.player.take().unwrap();
                let mut world =
                    std::mem::replace(&mut game.world, horizon_client::world::World::new(stone, 0));
                MotionWorld {
                    entities: &[],
                    player: &mut player,
                    scene: &mut world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                }
                .server_travel()
                .unwrap();
                game.world = world;
                game.player.replace(player);
            }
            let heard = std::mem::take(&mut *log.lock().unwrap());
            assert!(
                heard.iter().any(|(kind, cause)| {
                    kind == "minecraft:player" && *cause == MoveCause::SelfMovement
                }),
                "the player window's own move fires: {heard:?}"
            );
            // Position assignments the design excludes fire nothing: a /tp
            // teleport moves its target without an Entity.move, so the
            // counting listener stays silent across one.
            log.lock().unwrap().clear();
            assert!(game.run_command("tp @s 12.5 30.0 12.5"));
            let heard = std::mem::take(&mut *log.lock().unwrap());
            assert!(
                heard.is_empty(),
                "a teleport is a position assignment, not a move: {heard:?}"
            );
        });
    });
}

/// A mob goal's own `Entity.move` — `BreathAirGoal`'s drowning swim-up (#2414
/// «goals 1:1») — fires through the same gate `do_move` arms, with the self
/// cause: the goal's tick moves the dolphin by itself, beside travel's own
/// move, exactly the two `Entity.move` invocations vanilla makes.
#[test]
#[ignore = "daily"]
fn a_drowning_dolphins_goal_move_fires_too() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "counter",
        Box::new(Counter(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let registry = &game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 8, z, stone.0);
                    // Water deep enough to drown in, open to the air above.
                    for y in 9..16 {
                        chunk.set(x, y, z, water.0);
                    }
                }
            }
            game.world.chunks.insert((0, 0), std::sync::Arc::new(chunk));
            game.summon_with_nbt("minecraft:dolphin", [5.5, 10.5, 5.5], "{NoAI:0b}")
                .unwrap_or_else(|e| panic!("summon dolphin: {e:?}"));
            // The air supply low enough that BreathAirGoal takes over.
            game.mobs[0].air = 0;
            log.lock().unwrap().clear();
            let _ = game.tick_creatures();
            let heard: Vec<MoveCause> = std::mem::take(&mut *log.lock().unwrap())
                .into_iter()
                .filter(|(kind, _)| kind == "minecraft:dolphin")
                .map(|(_, cause)| cause)
                .collect();
            assert_eq!(
                heard,
                vec![MoveCause::SelfMovement, MoveCause::SelfMovement],
                "the drowning dolphin's round: the goal's own move beside travel's, \
                 vanilla's two Entity.move invocations"
            );
        });
    });
}
