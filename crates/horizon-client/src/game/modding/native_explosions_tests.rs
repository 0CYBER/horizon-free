//! `LevelRef::explode`'s contract through both mod targets: a native call of the handle method
//! and a wasm guest's call through the generated forwarder, each on a fresh game, on the stack
//! a wasm guest runs on. The source and interactions are #3929's; the damage type, the
//! calculator, the effects and the unbounded radius #4009's.
#![cfg(test)]
use crate::game::Game;
use crate::game::native_entity_refs::tests::{bind_local_player, run_on_guest_stack, with_game};
use horizon_client::{chunk::Chunk, mobs::Mob, mobs::Who, motion::PlayerMotion};
use horizon_core::{
    geometry::Vec3,
    position::{BlockPos, ChunkPos},
};
use mod_api::{
    EntityRef, ExplodeError, ExplosionCalculator, ExplosionDetonateEvent, ExplosionInteraction,
    ExplosionSettings, ExplosionStartEvent, Flow, Hooks, LevelRandomHost, LivingDamagePostEvent,
    LivingDeathEvent, ModEntry, ServerHost,
    wire::{Reader, Writer},
};
use std::sync::{Arc, Mutex};

/// How the call reaches the game.
#[derive(Clone, Copy, Debug)]
enum Via {
    Native,
    Wasm,
}
const BOTH: [Via; 2] = [Via::Native, Via::Wasm];

/// One `explode` call.
struct Blast {
    source: Option<EntityRef>,
    center: [f64; 3],
    radius: f32,
    fire: bool,
    interaction: ExplosionInteraction,
    settings: ExplosionSettings,
}
impl Blast {
    /// A sourceless `Block` blast without fire and with vanilla's settings.
    fn at(center: [f64; 3], radius: f32) -> Self {
        Self {
            source: None,
            center,
            radius,
            fire: false,
            interaction: ExplosionInteraction::Block,
            settings: ExplosionSettings::default(),
        }
    }
}

/// What `explode` answers.
type Answer = Result<(), ExplodeError>;

/// Runs `blast` in the overworld through `via` and checks its answer.
fn explode(game: &mut Game<'_>, via: Via, blast: Blast, expected: Answer) {
    let level = game.level("minecraft:overworld").unwrap();
    match via {
        Via::Native => assert_eq!(
            level.explode(
                game,
                blast.source,
                blast.center,
                blast.radius,
                blast.fire,
                blast.interaction,
                blast.settings
            ),
            expected
        ),
        Via::Wasm => wasm_explode(game, level, blast, expected),
    }
}

/// A guest whose server tick calls `level_explode` through its import and
/// answers the tick only when the host's answer is `expected` (it traps otherwise).
fn wasm_explode(game: &mut Game<'_>, level: mod_api::LevelRef, blast: Blast, expected: Answer) {
    let mut input = Writer::new();
    input.put(&level);
    input.put(&blast.source);
    input.put(&blast.center);
    input.put(&blast.radius);
    input.put(&blast.fire);
    input.put(&blast.interaction);
    input.put(&blast.settings);
    let mut answer = Writer::new();
    answer.put(&expected);
    let mut reply = Writer::new();
    reply.put(&mod_api::ServerTickPhase::ServerTickEnd {
        world_running: true,
    });
    let escape = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("\\{b:02x}"))
            .collect::<String>()
    };
    let check = answer
        .bytes()
        .iter()
        .enumerate()
        .map(|(i, byte)| {
            format!(
                "i32.const {} i32.load8_u i32.const {byte} i32.ne if unreachable end ",
                32768 + i
            )
        })
        .collect::<String>();
    let id = horizon_mod_wasm::fixture::import_id("BackendMut", "level_explode").unwrap();
    let bytes = wat::parse_str(format!(
        r#"(module
          (@custom "dylink.0" "\01\04\00\00\00\00")
          (import "env" "memory" (memory 1))
          (import "env" "__indirect_function_table" (table 1 funcref))
          (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
          (import "horizon" "host_reply" (func (param i32)))
          (func (export "horizon_layout") (result i64) i64.const {layout})
          (data (i32.const 4096) "{input}")
          (data (i32.const 16384) "{reply}")
          (func (export "horizon_in") (param i32) (result i32) i32.const 512)
          (func (export "horizon_hook") (param i32) (result i64)
            i32.const {id} i32.const 4096 i32.const {input_len} i32.const 32768 i32.const 4096 call $host drop
            {check}
            i64.const {reply_at}))"#,
        layout = *mod_api::wire::LAYOUT,
        input = escape(input.bytes()),
        reply = escape(reply.bytes()),
        input_len = input.bytes().len(),
        reply_at = (16384u64 << 32) | reply.bytes().len() as u64,
    ))
    .unwrap();
    let module = horizon_mod_wasm::fixture::module(&bytes).unwrap();
    let mut context = Writer::new();
    context.put(&mod_api::Authority::DedicatedServer);
    context.put(&level.epoch());
    context.put(&mod_api::TickId::new(1));
    let context: mod_api::LifecycleContext = Reader::new(context.bytes()).take().unwrap();
    let mut tick = mod_api::ServerTickEvent {
        context: &context,
        phase: mod_api::ServerTickPhase::ServerTickStart {
            world_running: true,
        },
    };
    Hooks::server_tick(&module, game, &mut tick);
    assert!(
        matches!(
            tick.phase,
            mod_api::ServerTickPhase::ServerTickEnd {
                world_running: true
            }
        ),
        "the guest saw the answer {expected:?}"
    );
}

/// A server's overworld chunk (0, 0) with a stone floor at y 79.
fn floor(game: &mut Game<'_>) {
    game.build_registries();
    game.load_server_loot();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 79 0 15 79 15 stone"));
}

fn name(game: &Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// How many floor blocks are still stone.
fn stone(game: &Game<'_>) -> usize {
    (0..16)
        .flat_map(|x| (0..16).map(move |z| [x, 79, z]))
        .filter(|&p| name(game, p) == "minecraft:stone")
        .count()
}

fn mob(game: &mut Game<'_>, id: u32, kind: &str, at: Vec3, health: f32) {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut mob = Mob::new(id, kind, at, 0.0);
    mob.persistent = true;
    mob.no_ai = true;
    mob.on_ground = true;
    mob.health = health;
    game.mobs.push(mob);
}

/// What `living_death` heard: the dying entity, the damage type and the causing entity.
type Deaths = Arc<Mutex<Vec<(u32, String, i32)>>>;
struct Deathwatch(Deaths);
#[mod_api::__subscriptions]
impl Hooks for Deathwatch {
    fn living_death(&self, _: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        let source = event.source;
        self.0
            .lock()
            .unwrap()
            .push((event.entity, source.damage_type.into(), source.causing));
        Flow::Continue
    }
}

/// A blast at (4.5, 80, 4.5) whose source `source` makes (it answers the
/// source's entity id) kills a zombie on one health beside it: what the
/// zombie's death names, and whether a player is credited with the kill
/// (`last_hurt_by_player`). The player in place is entity 11, out of reach.
fn kill(via: Via, source: impl Fn(&mut Game<'_>) -> u32) -> (Vec<(u32, String, i32)>, bool) {
    crate::entry::install_client_hooks();
    let deaths = Deaths::default();
    let mut credited = false;
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "explode-deaths",
            Box::new(Deathwatch(deaths.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                floor(game);
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(14.5, 80.0, 14.5)).unwrap());
                bind_local_player(game, 11, 123);
                game.set_mode_id(0);
                game.next_entity = 100;
                mob(game, 41, "minecraft:zombie", Vec3::new(6.0, 80.0, 4.5), 1.0);
                let id = source(game);
                let source = game.entity(id);
                assert!(source.is_some(), "the source {id} resolves");
                let blast = Blast {
                    source,
                    center: [4.5, 80.0, 4.5],
                    radius: 4.0,
                    fire: false,
                    interaction: ExplosionInteraction::None,
                    settings: ExplosionSettings::default(),
                };
                explode(game, via, blast, Ok(()));
                credited = game.last_hurt_by_player.contains_key(&41);
                game.tick_creature_entities().unwrap();
            })
        },
    );
    let deaths = deaths.lock().unwrap().clone();
    (deaths, credited)
}

/// [`kill`] through both targets, each answering `expected`.
fn kills(name: &str, source: fn(&mut Game<'_>) -> u32, expected: (i32, &'static str, bool)) {
    crate::game::in_own_process(&format!("game::native_explosions_tests::{name}"), || {
        run_on_guest_stack(move || {
            for via in BOTH {
                let (causing, damage_type, credited) = expected;
                assert_eq!(
                    kill(via, source),
                    (vec![(41, damage_type.to_owned(), causing)], credited),
                    "{via:?}"
                );
            }
        });
    });
}

/// Primed TNT lit by `owner` beside the zombie.
fn tnt(game: &mut Game<'_>, owner: Option<Who>) -> u32 {
    game.add_primed_tnt(BlockPos::new(4, 80, 4), 0.0, 80, owner, None);
    game.tnts.last().unwrap().id
}

/// A summoned entity's id.
fn summoned(game: &mut Game<'_>, kind: &str, nbt: &str) -> u32 {
    let target = game
        .summon_with_nbt(kind, [4.5, 80.5, 4.5], nbt)
        .unwrap()
        .unwrap();
    use crate::game::commands::Target;
    match target {
        Target::Stand(id) | Target::Trident(id) | Target::Rocket(id) => id,
        other => panic!("{kind} summoned as {other:?}"),
    }
}

#[test]
#[ignore = "daily"]
fn player_lit_tnt_as_the_source_credits_the_player_with_the_kill() {
    kills(
        "player_lit_tnt_as_the_source_credits_the_player_with_the_kill",
        |game| tnt(game, Some(Who::Player)),
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn unowned_tnt_as_the_source_credits_no_one() {
    kills(
        "unowned_tnt_as_the_source_credits_no_one",
        |game| tnt(game, None),
        (0, "minecraft:explosion", false),
    );
}

#[test]
#[ignore = "daily"]
fn a_player_as_the_source_is_credited_as_a_player() {
    kills(
        "a_player_as_the_source_is_credited_as_a_player",
        |_| 11,
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_living_entity_outside_the_mob_list_causes_its_own_blast() {
    kills(
        "a_living_entity_outside_the_mob_list_causes_its_own_blast",
        |game| summoned(game, "minecraft:armor_stand", "{}"),
        (101, "minecraft:player_explosion", false),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_thrown_trident_credits_the_player() {
    kills(
        "a_players_thrown_trident_credits_the_player",
        |game| {
            let id = summoned(game, "minecraft:trident", "{NoGravity:1b}");
            game.tridents.for_each_mut(|t| t.owner = 11);
            id
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_firework_rocket_credits_the_player() {
    kills(
        "a_players_firework_rocket_credits_the_player",
        |game| {
            let id = summoned(game, "minecraft:firework_rocket", "{LifeTime:100}");
            game.rockets.for_each_mut(|r| r.owner = Some(Who::Player));
            id
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_players_fishing_bobber_credits_the_player() {
    kills(
        "a_players_fishing_bobber_credits_the_player",
        |game| {
            let hook =
                crate::game::fishing::Hook::remote(900, Vec3::new(4.5, 81.0, 4.5), [1, 2, 3, 4]);
            game.install_fishing_hook(hook);
            900
        },
        (11, "minecraft:player_explosion", true),
    );
}

#[test]
#[ignore = "daily"]
fn a_mob_blast_breaks_blocks_only_when_its_source_may_grief() {
    run_on_guest_stack(|| {
        for via in BOTH {
            for griefing in [false, true] {
                with_game(|game| {
                    floor(game);
                    assert!(game.ui.rules.set(
                        "mob_griefing",
                        horizon_client::gamerules::Value::Bool(griefing),
                    ));
                    mob(
                        game,
                        42,
                        "minecraft:creeper",
                        Vec3::new(8.5, 80.0, 8.5),
                        20.0,
                    );
                    let blast = Blast {
                        source: game.entity(42),
                        center: [8.5, 80.0, 8.5],
                        radius: 3.0,
                        fire: false,
                        interaction: ExplosionInteraction::Mob,
                        settings: ExplosionSettings::default(),
                    };
                    explode(game, via, blast, Ok(()));
                    let (under, floor) = (name(game, [8, 79, 8]), stone(game));
                    if griefing {
                        assert_eq!(under, "minecraft:air", "{via:?}");
                        assert!(floor < 256, "{via:?}: {floor}");
                    } else {
                        assert_eq!((under.as_str(), floor), ("minecraft:stone", 256), "{via:?}");
                    }
                    // The source is spared by its own blast.
                    assert_eq!(game.mobs[0].health, 20.0, "{via:?}");
                });
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_trigger_blast_presses_a_button_and_breaks_nothing() {
    run_on_guest_stack(|| {
        for via in BOTH {
            with_game(|game| {
                floor(game);
                assert!(game.run_command("setblock 6 80 4 minecraft:stone_button[face=floor]"));
                let blast = Blast {
                    source: None,
                    center: [4.5, 80.5, 4.5],
                    radius: 3.0,
                    fire: false,
                    interaction: ExplosionInteraction::Trigger,
                    settings: ExplosionSettings::default(),
                };
                explode(game, via, blast, Ok(()));
                let state = game.world.block(6, 80, 4);
                let powered = game
                    .pack
                    .assets
                    .registry
                    .state(state)
                    .and_then(|s| s.properties.get("powered").cloned());
                assert_eq!(name(game, [6, 80, 4]), "minecraft:stone_button", "{via:?}");
                assert_eq!(powered.as_deref(), Some("true"), "{via:?}");
                assert_eq!(stone(game), 256, "{via:?}");
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_blast_that_keeps_blocks_still_lights_fire() {
    run_on_guest_stack(|| {
        for via in BOTH {
            with_game(|game| {
                floor(game);
                let blast = Blast {
                    source: None,
                    center: [8.5, 80.5, 8.5],
                    radius: 4.0,
                    fire: true,
                    interaction: ExplosionInteraction::None,
                    settings: ExplosionSettings::default(),
                };
                explode(game, via, blast, Ok(()));
                let fires = (0..16)
                    .flat_map(|x| (0..16).map(move |z| [x, 80, z]))
                    .filter(|&p| name(game, p) == "minecraft:fire")
                    .count();
                assert!(fires > 0, "{via:?}");
                assert_eq!(stone(game), 256, "{via:?}");
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_source_explodes_nothing() {
    run_on_guest_stack(|| {
        for via in BOTH {
            with_game(|game| {
                floor(game);
                mob(
                    game,
                    43,
                    "minecraft:zombie",
                    Vec3::new(8.5, 80.0, 9.5),
                    20.0,
                );
                let live = game.entity(43).unwrap();
                let stale =
                    EntityRef::new(mod_api::WorldEpoch::new(live.epoch().get() + 1), live.id());
                let nothing = EntityRef::new(live.epoch(), 9999);
                for source in [stale, nothing] {
                    let blast = Blast {
                        source: Some(source),
                        center: [8.5, 80.5, 8.5],
                        radius: 4.0,
                        fire: true,
                        interaction: ExplosionInteraction::Block,
                        settings: ExplosionSettings::default(),
                    };
                    explode(game, via, blast, Err(ExplodeError::Unavailable));
                }
                assert_eq!(stone(game), 256, "{via:?}");
                assert_eq!(game.mobs[0].health, 20.0, "{via:?}");
            });
        }
    });
}

/// What a blast's hooks heard: the damage type `explosion_start` names, how
/// often `explosion_detonate` ran, and each `living_damage` (post) with its
/// entity and damage type.
#[derive(Default)]
struct Heard {
    starts: Vec<String>,
    detonates: usize,
    damage: Vec<(i64, String)>,
}
struct Listener(Arc<Mutex<Heard>>);
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn explosion_start(&self, _: &mut dyn ServerHost, event: &mut ExplosionStartEvent<'_>) -> Flow {
        let damage_type = event.explosion.damage_type.to_owned();
        self.0.lock().unwrap().starts.push(damage_type);
        Flow::Continue
    }
    fn explosion_detonate(&self, _: &mut dyn ServerHost, _: &mut ExplosionDetonateEvent<'_>) {
        self.0.lock().unwrap().detonates += 1;
    }
    fn living_damage_post(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePostEvent<'_>,
    ) {
        let heard = (i64::from(event.entity), event.source.damage_type.to_owned());
        self.0.lock().unwrap().damage.push(heard);
    }
}

/// Runs `test` on a fresh game with a stone floor through both targets, the
/// [`Listener`] hearing it, and hands it what was heard.
fn heard(test: impl Fn(&mut Game<'_>, Via, &Arc<Mutex<Heard>>) + Send + Sync + 'static) {
    crate::entry::install_client_hooks();
    run_on_guest_stack(move || {
        for via in BOTH {
            let heard = Arc::new(Mutex::new(Heard::default()));
            let mods = vec![ModEntry::new(
                "explode-heard",
                Box::new(Listener(heard.clone())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_game(|game| {
                    floor(game);
                    test(game, via, &heard);
                })
            });
        }
    });
}

/// The game's damage types plus `test:fuel_tank`, whose death message is
/// `death.attack.fuelTank` and which is in `#minecraft:is_explosion`.
fn register_fuel_tank(game: &mut Game<'_>) {
    use serde_json::{Value, json};
    let old = &game.health_resources.damage;
    let mut types: Vec<Value> = old
        .entries()
        .map(|(_, d)| {
            json!({
                "id": d.name, "message_id": d.message_id,
                "exhaustion": format!("{:08x}", d.exhaustion.to_bits()),
                "scaling": d.scaling.name(), "effects": d.effects,
                "death_message_type": d.death_message_type,
            })
        })
        .collect();
    types.push(json!({
        "id": "test:fuel_tank", "message_id": "fuelTank", "exhaustion": "3dcccccd",
        "scaling": "never", "effects": "hurt", "death_message_type": "default",
    }));
    let mut tags = serde_json::Map::new();
    for tag in old.tags() {
        let mut values: Vec<&str> = old
            .entries()
            .filter(|&(id, _)| old.in_tag(id, tag))
            .map(|(_, d)| d.name.as_str())
            .collect();
        if tag == "minecraft:is_explosion" {
            values.push("test:fuel_tank");
        }
        tags.insert(tag.into(), json!({ "values": values }).to_string().into());
    }
    let types = json!({ "data_version": horizon_world::DATA_VERSION, "types": types });
    game.health_resources.damage =
        horizon_world::damage_source::Catalog::from_json(&types, &Value::Object(tags)).unwrap();
}

#[test]
#[ignore = "daily"]
fn a_mod_damage_type_names_the_death_message_and_is_what_living_damage_hears() {
    heard(move |game, via, heard| {
        register_fuel_tank(game);
        mob(
            game,
            44,
            "minecraft:zombie",
            Vec3::new(10.5, 80.0, 8.5),
            20.0,
        );
        let blast = Blast {
            interaction: ExplosionInteraction::None,
            settings: ExplosionSettings {
                damage_type: Some("test:fuel_tank".into()),
                ..Default::default()
            },
            ..Blast::at([8.5, 80.5, 8.5], 4.0)
        };
        explode(game, via, blast, Ok(()));
        let heard = heard.lock().unwrap();
        assert_eq!(heard.starts, ["test:fuel_tank"], "{via:?}");
        assert_eq!(heard.damage, [(44, "test:fuel_tank".to_owned())], "{via:?}");
        let zombie = game.mobs.remove(0);
        assert!(zombie.health < 20.0, "{via:?}: {}", zombie.health);
        let death = game.mob_death_message(&zombie, None).component();
        assert_eq!(death["translate"], "death.attack.fuelTank", "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn immune_blocks_survive_and_every_other_cell_resists_nothing() {
    for immune in [Some("minecraft:logs"), None] {
        heard(move |game, via, _| {
            assert!(game.run_command("setblock 8 80 10 minecraft:oak_log"));
            assert!(game.run_command("setblock 8 80 6 minecraft:obsidian"));
            let blast = Blast {
                settings: ExplosionSettings {
                    calculator: Some(ExplosionCalculator {
                        immune_blocks: immune.map(str::to_owned),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 6.0)
            };
            explode(game, via, blast, Ok(()));
            let (log, obsidian) = (name(game, [8, 80, 10]), name(game, [8, 80, 6]));
            assert_eq!(
                name(game, [8, 79, 8]),
                "minecraft:air",
                "{via:?} {immune:?}"
            );
            if immune.is_some() {
                // The tag replaces the rule: obsidian's 1200 counts for nothing.
                assert_eq!(
                    (log.as_str(), obsidian.as_str()),
                    ("minecraft:oak_log", "minecraft:air"),
                    "{via:?}"
                );
            } else {
                assert_eq!(
                    (log.as_str(), obsidian.as_str()),
                    ("minecraft:air", "minecraft:obsidian"),
                    "{via:?}"
                );
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_calculator_that_damages_no_entities_leaves_health_while_blocks_break() {
    for damages_entities in [false, true] {
        heard(move |game, via, heard| {
            mob(
                game,
                45,
                "minecraft:zombie",
                Vec3::new(10.5, 80.0, 8.5),
                20.0,
            );
            let blast = Blast {
                settings: ExplosionSettings {
                    calculator: Some(ExplosionCalculator {
                        damages_entities,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, via, blast, Ok(()));
            assert!(stone(game) < 256, "{via:?}: blocks break");
            let health = game.mobs[0].health;
            if damages_entities {
                assert!(health < 20.0, "{via:?}: {health}");
            } else {
                assert_eq!(health, 20.0, "{via:?}");
                assert!(heard.lock().unwrap().damage.is_empty(), "{via:?}");
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_knockback_multiplier_of_zero_leaves_velocity_unchanged() {
    for knockback_multiplier in [Some(0.0), None] {
        heard(move |game, via, _| {
            mob(
                game,
                46,
                "minecraft:zombie",
                Vec3::new(11.5, 80.0, 8.5),
                20.0,
            );
            let blast = Blast {
                interaction: ExplosionInteraction::None,
                settings: ExplosionSettings {
                    calculator: Some(ExplosionCalculator {
                        knockback_multiplier,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, via, blast, Ok(()));
            let zombie = &game.mobs[0];
            assert!(zombie.health < 20.0, "{via:?}: hurt either way");
            if knockback_multiplier.is_some() {
                assert_eq!(zombie.velocity, Vec3::new(0.0, 0.0, 0.0), "{via:?}");
            } else {
                assert!(zombie.velocity.x > 0.0, "{via:?}: {:?}", zombie.velocity);
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_radius_of_48_breaks_a_block_40_blocks_from_the_centre() {
    heard(move |game, via, _| {
        for x in 1..4 {
            game.world
                .chunks
                .insert((x, 0), Arc::new(Chunk::empty(x, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, 0))
                .unwrap();
        }
        // The ray along (1, 1/15, 1/15) from the centre crosses this block
        // 39.7 blocks out.
        assert!(game.run_command("setblock 48 103 11 minecraft:dirt"));
        explode(game, via, Blast::at([8.5, 100.5, 8.5], 48.0), Ok(()));
        assert_eq!(name(game, [48, 103, 11]), "minecraft:air", "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn an_unknown_damage_type_particle_or_tag_explodes_nothing() {
    let refusals = [
        (
            ExplosionSettings {
                damage_type: Some("test:no_such_type".into()),
                ..Default::default()
            },
            ExplodeError::UnknownDamageType,
        ),
        (
            ExplosionSettings {
                large_particle: Some("test:no_such_particle".into()),
                ..Default::default()
            },
            ExplodeError::InvalidParticle,
        ),
        (
            ExplosionSettings {
                block_particles: Some(vec![mod_api::ExplosionBlockParticle {
                    weight: 0,
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ExplodeError::InvalidParticle,
        ),
        (
            ExplosionSettings {
                sound: Some("Not A Sound".into()),
                ..Default::default()
            },
            ExplodeError::UnknownSound,
        ),
        (
            ExplosionSettings {
                calculator: Some(ExplosionCalculator {
                    immune_blocks: Some("test:no_such_tag".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ExplodeError::UnknownBlockTag,
        ),
    ];
    for (settings, refusal) in refusals {
        refused(settings, refusal);
    }
}

/// A blast with `settings` beside a zombie answers `refusal` through both
/// targets: no block breaks, the zombie keeps its health and no hook hears it.
fn refused(settings: ExplosionSettings, refusal: ExplodeError) {
    heard(move |game, via, heard| {
        mob(
            game,
            47,
            "minecraft:zombie",
            Vec3::new(10.5, 80.0, 8.5),
            20.0,
        );
        let blast = Blast {
            settings: settings.clone(),
            ..Blast::at([8.5, 80.5, 8.5], 4.0)
        };
        explode(game, via, blast, Err(refusal));
        assert_eq!(stone(game), 256, "{via:?} {refusal:?}");
        assert_eq!(game.mobs[0].health, 20.0, "{via:?} {refusal:?}");
        assert!(
            heard.lock().unwrap().starts.is_empty(),
            "{via:?} {refusal:?}"
        );
    });
}

/// Smoke block particles with these weights.
fn weighted(weights: &[u32]) -> ExplosionSettings {
    let list = weights
        .iter()
        .map(|&weight| mod_api::ExplosionBlockParticle {
            weight,
            ..Default::default()
        });
    ExplosionSettings {
        block_particles: Some(list.collect()),
        ..Default::default()
    }
}

#[test]
#[ignore = "daily"]
fn a_block_particle_weight_past_the_packets_int_explodes_nothing() {
    let past = i32::MAX as u32 + 1;
    refused(weighted(&[past]), ExplodeError::InvalidParticle);
}

#[test]
#[ignore = "daily"]
fn block_particle_weights_whose_total_is_past_an_int_explode_nothing() {
    let max = i32::MAX as u32;
    refused(weighted(&[max, 1]), ExplodeError::InvalidParticle);
}

/// A nether portal block at (8, 80, 10) between obsidian on its axis (x) and
/// above and below, which nothing around it changes.
fn portal(game: &mut Game<'_>) {
    for pos in ["7 80 10", "9 80 10", "8 81 10", "8 79 10"] {
        assert!(game.run_command(&format!("setblock {pos} minecraft:obsidian")));
    }
    assert!(game.run_command("setblock 8 80 10 minecraft:nether_portal[axis=x]"));
}

#[test]
#[ignore = "daily"]
fn primed_tnt_that_went_through_a_portal_spares_a_portal_unless_a_calculator_is_passed() {
    // (source, calculator, the portal survives)
    let cases = [
        (true, false, true),
        (false, false, false),
        (true, true, false),
    ];
    for (source, calculator, survives) in cases {
        heard(move |game, via, _| {
            portal(game);
            let source = source.then(|| {
                let id = tnt(game, None);
                game.tnts.last_mut().unwrap().used_portal = true;
                game.entity(id).unwrap()
            });
            let blast = Blast {
                source,
                settings: ExplosionSettings {
                    calculator: calculator.then(ExplosionCalculator::default),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, via, blast, Ok(()));
            let case = (source.is_some(), calculator);
            assert_eq!(name(game, [8, 79, 8]), "minecraft:air", "{via:?} {case:?}");
            let expected = if survives {
                "minecraft:nether_portal"
            } else {
                "minecraft:air"
            };
            assert_eq!(name(game, [8, 80, 10]), expected, "{via:?} {case:?}");
            assert_eq!(
                name(game, [7, 80, 10]),
                "minecraft:obsidian",
                "{via:?} {case:?}"
            );
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_wind_charge_source_brings_its_tag_calculator_unless_a_calculator_is_passed() {
    use horizon_client::projectiles::{Projectile, ProjectileKind};
    // (source, calculator, obsidian survives)
    let cases = [
        (true, false, false),
        (false, false, true),
        (true, true, true),
    ];
    for (source, calculator, survives) in cases {
        heard(move |game, via, _| {
            assert!(game.run_command("setblock 8 80 10 minecraft:obsidian"));
            assert!(game.run_command("setblock 8 80 6 minecraft:bedrock"));
            let source = source.then(|| {
                let charge =
                    Projectile::new(ProjectileKind::WindCharge, Vec3::new(4.5, 80.5, 4.5), None);
                let id = game.add_projectile(charge);
                game.entity(id).unwrap()
            });
            let blast = Blast {
                source,
                settings: ExplosionSettings {
                    calculator: calculator.then(ExplosionCalculator::default),
                    ..Default::default()
                },
                ..Blast::at([8.5, 80.5, 8.5], 4.0)
            };
            explode(game, via, blast, Ok(()));
            let case = (source.is_some(), calculator);
            assert_eq!(name(game, [8, 79, 8]), "minecraft:air", "{via:?} {case:?}");
            // `#blocks_wind_charge_explosions` (barrier, bedrock) stands either way.
            assert_eq!(
                name(game, [8, 80, 6]),
                "minecraft:bedrock",
                "{via:?} {case:?}"
            );
            let expected = if survives {
                "minecraft:obsidian"
            } else {
                "minecraft:air"
            };
            assert_eq!(name(game, [8, 80, 10]), expected, "{via:?} {case:?}");
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_non_finite_radius_is_refused_and_one_of_zero_finds_no_blocks() {
    heard(move |game, via, heard| {
        for radius in [f32::NAN, f32::INFINITY] {
            let blast = Blast::at([8.5, 80.5, 8.5], radius);
            explode(game, via, blast, Err(ExplodeError::InvalidBlast));
        }
        assert!(heard.lock().unwrap().starts.is_empty(), "{via:?}");
        // As in vanilla: the start hook, no blocks, no detonate.
        explode(game, via, Blast::at([8.5, 79.5, 8.5], 0.0), Ok(()));
        assert_eq!(stone(game), 256, "{via:?}");
        let heard = heard.lock().unwrap();
        assert_eq!((heard.starts.len(), heard.detonates), (1, 0), "{via:?}");
    });
}
