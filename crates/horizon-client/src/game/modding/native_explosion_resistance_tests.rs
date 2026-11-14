//! The explosion resistance reads' contract through both mod targets: a native call of the
//! handle methods and a wasm guest's call through the generated forwarder, each on a fresh game,
//! on the stack a wasm guest runs on.
#![cfg(test)]
use crate::game::Game;
use crate::game::execution_role::ExecutionRole;
use crate::game::native_entity_refs::tests::{run_on_guest_stack, with_game_role};
use horizon_client::{chunk::Chunk, projectiles::ProjectileKind};
use horizon_core::position::ChunkPos;
use mod_api::{
    Block, BlockPart, BlockState, BlocksEvent, EntityRef, ExplosionInteraction,
    ExplosionResistance::{self, NoResistance, Resists, Unavailable},
    ExplosionResistanceEvent, HostBlockStates, LevelHost, LevelRef, ModEntry, Registrar,
    RegistrationContext, ResistanceCells, ResistanceCellsError,
    wire::{Reader, Wire, Writer},
};
use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};

/// How the call reaches the game.
#[derive(Clone, Copy, Debug)]
pub(in crate::game) enum Via {
    Native,
    Wasm,
}
pub(in crate::game) const BOTH: [Via; 2] = [Via::Native, Via::Wasm];
impl Via {
    /// How many times one read reaches the game: a wasm read is checked against the native
    /// answer to the same call, so it reads twice.
    fn reads(self) -> i32 {
        match self {
            Via::Native => 1,
            Via::Wasm => 2,
        }
    }
}

/// The one position where [`Spot`] answers.
const SPOT: [i32; 3] = [6, 80, 1];

/// Dirt's part: 5000 at [`SPOT`] only, counting the asks.
#[derive(Clone, Default)]
struct Spot(Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Spot {
    fn explosion_resistance(
        &self,
        _: &dyn LevelHost,
        event: &mut ExplosionResistanceEvent<'_>,
    ) -> Option<f32> {
        self.0.fetch_add(1, Ordering::Relaxed);
        ([event.pos.x, event.pos.y, event.pos.z] == SPOT).then_some(5000.0)
    }
}
/// Oak planks' part: 7 everywhere, declared pure, counting the asks.
#[derive(Clone, Default)]
struct Pure(Arc<AtomicI32>);
#[mod_api::horizon_part]
impl BlockPart for Pure {
    fn explosion_resistance(
        &self,
        _: &dyn LevelHost,
        _: &mut ExplosionResistanceEvent<'_>,
    ) -> Option<f32> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Some(7.0)
    }
    fn explosion_resistance_is_pure(&self) -> bool {
        true
    }
}
#[derive(Clone, Default)]
struct Resisting {
    spot: Spot,
    pure: Pure,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Resisting {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let dirt = r.block_key("minecraft:dirt");
        r.block_part(dirt, self.spot.clone());
        let planks = r.block_key("minecraft:oak_planks");
        r.block_part(planks, self.pure.clone());
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        let mut block = Block::new("test:blastproof", "minecraft:stone");
        block.properties.explosion_resistance = Some(42.0);
        event.blocks.push(block);
    }
}

/// A server's overworld chunk (0, 0): from x 1 along y 80, z 1 stone, obsidian, bedrock,
/// water, air, dirt at [`SPOT`], dirt, the content block, then four oak planks on z 2.
fn row(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.options.dedicated = Some(Vec::new());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for (x, block) in [
        (1, "stone"),
        (2, "obsidian"),
        (3, "bedrock"),
        (4, "water"),
        (6, "dirt"),
        (7, "dirt"),
    ] {
        assert!(game.run_command(&format!("setblock {x} 80 1 minecraft:{block}")));
    }
    let content = game
        .pack
        .assets
        .registry
        .default_state("test:blastproof")
        .unwrap();
    game.world.set(8, 80, 1, content).unwrap();
    assert!(game.run_command("fill 1 80 2 4 80 2 minecraft:oak_planks"));
}

/// A sourceless block blast of radius 4 at the row, read through `via`.
fn read(
    game: &mut Game<'_>,
    via: Via,
    pos: [i32; 3],
    source: Option<EntityRef>,
) -> Option<ExplosionResistance> {
    let level = game.level("minecraft:overworld").unwrap();
    let (center, radius, fire, interaction) = blast();
    let native = level.explosion_resistance(game, pos, source, center, radius, fire, interaction);
    if let Via::Wasm = via {
        let mut input = Writer::new();
        input.put(&level);
        input.put(&pos);
        put_blast(&mut input, source);
        wasm_answer(game, level, "level_explosion_resistance", input, &native);
    }
    native
}

/// `read` for every cell of `cells`.
fn reads(
    game: &mut Game<'_>,
    via: Via,
    cells: &ResistanceCells,
    source: Option<EntityRef>,
) -> Result<Vec<ExplosionResistance>, ResistanceCellsError> {
    let level = game.level("minecraft:overworld").unwrap();
    let (center, radius, fire, interaction) = blast();
    let native =
        level.explosion_resistances(game, cells, source, center, radius, fire, interaction);
    if let Via::Wasm = via {
        let mut input = Writer::new();
        input.put(&level);
        input.put(cells);
        put_blast(&mut input, source);
        wasm_answer(game, level, "level_explosion_resistances", input, &native);
    }
    native
}

fn blast() -> ([f64; 3], f32, bool, ExplosionInteraction) {
    ([4.5, 80.5, 1.5], 4.0, false, ExplosionInteraction::Block)
}

fn put_blast(input: &mut Writer, source: Option<EntityRef>) {
    let (center, radius, fire, interaction) = blast();
    input.put(&source);
    input.put(&center);
    input.put(&radius);
    input.put(&fire);
    input.put(&interaction);
}

/// A guest whose server tick calls host row `name` with `input` through its import and answers
/// the tick only when the host's answer is `expected`, byte for byte (it traps otherwise). The
/// wasm answer is thus the native one.
pub(in crate::game) fn wasm_answer<T: Wire>(
    game: &mut Game<'_>,
    level: LevelRef,
    name: &str,
    input: Writer,
    expected: &T,
) {
    let owner = if name.starts_with("level_") {
        "BackendMut"
    } else {
        "Backend"
    };
    let mut answer = Writer::new();
    answer.put(expected);
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
    let id = horizon_mod_wasm::fixture::import_id(owner, name).unwrap();
    let bytes = wat::parse_str(format!(
        r#"(module
          (@custom "dylink.0" "\01\04\00\00\00\00")
          (import "env" "memory" (memory 1))
          (import "env" "__indirect_function_table" (table 1 funcref))
          (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
          (import "horizon" "host_reply" (func (param i32)))
          (func (export "horizon_layout") (result i64) i64.const {layout})
          (data $input "{input}")
          (data (i32.const 16384) "{reply}")
          (func (export "horizon_in") (param i32) (result i32) i32.const 512)
          (func (export "horizon_hook") (param i32) (result i64)
            i32.const {pages} memory.grow drop
            i32.const {input_at} i32.const 0 i32.const {input_len} memory.init $input
            i32.const {id} i32.const {input_at} i32.const {input_len} i32.const 32768 i32.const 8192 call $host drop
            {check}
            i64.const {reply_at}))"#,
        layout = *mod_api::wire::LAYOUT,
        // The input goes after the memory the group already has, grown to hold it: the first
        // page holds the reply and the answer.
        pages = 1 + input.bytes().len() / 65536,
        input_at = 4 * 65536,
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
    mod_api::Hooks::server_tick(&module, game, &mut tick);
    assert!(
        matches!(
            tick.phase,
            mod_api::ServerTickPhase::ServerTickEnd {
                world_running: true
            }
        ),
        "the guest's {name} answer differed from the native one"
    );
}

/// Runs `test` on a fresh game with the resisting mod, for each target.
fn each_target(test: impl Fn(&mut Game<'_>, Via, &Resisting) + Send + Sync + 'static) {
    run_on_guest_stack(move || {
        for via in BOTH {
            let parts = Resisting::default();
            let mods: mod_api::Mods = vec![ModEntry::new("test", Box::new(parts.clone()))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                // The content block needs a content root built with the mod's blocks.
                let base =
                    std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
                // One build at a time: the tests share the root's folder.
                static BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());
                let root = {
                    let _build = BUILD
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    crate::game::mods::content_root(&base, &Default::default()).unwrap()
                };
                with_game_role(
                    root.prepared().to_path_buf(),
                    ExecutionRole::IntegratedServer,
                    |game| {
                        row(game);
                        test(game, via, &parts);
                    },
                );
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_block_state_reads_the_resistance_its_block_declares() {
    each_target(|game, via, _| {
        let level = game.level("minecraft:overworld").unwrap();
        let states: Vec<Option<BlockState>> = [
            "minecraft:stone",
            "minecraft:obsidian",
            "minecraft:bedrock",
            "minecraft:water",
            "minecraft:dirt",
            "test:blastproof",
        ]
        .into_iter()
        .map(|name| mod_api::prelude::BlockStatesHost::block_state_id(&*game, name))
        .collect();
        let read = |game: &mut Game<'_>, state: BlockState| {
            let native = game.block_states().explosion_resistance(state);
            if let Via::Wasm = via {
                let mut input = Writer::new();
                input.put(&state);
                wasm_answer(
                    game,
                    level,
                    "block_state_explosion_resistance",
                    input,
                    &native,
                );
            }
            native
        };
        let got: Vec<Option<f32>> = states.into_iter().map(|s| read(game, s.unwrap())).collect();
        // Dirt's part answers 5000 at one position; the state's own value stays 0.5.
        assert_eq!(
            got,
            [
                Some(6.0),
                Some(1200.0),
                Some(3_600_000.0),
                Some(100.0),
                Some(0.5),
                Some(42.0)
            ],
            "{via:?}"
        );
        // Stone has no state past its one: an ordinal no state has.
        let stone = mod_api::prelude::BlockStatesHost::block_state_id(&*game, "minecraft:stone");
        let mut bytes = Writer::new();
        bytes.put(&stone.unwrap().block());
        bytes.put(&u16::MAX);
        let unknown: BlockState = Reader::new(bytes.bytes()).take().unwrap();
        assert_eq!(read(game, unknown), None, "{via:?}");
    });
}

#[test]
#[ignore = "daily"]
fn a_cell_reads_what_a_blast_meets_there() {
    each_target(|game, via, parts| {
        let at = |x: i32| [x, 80, 1];
        let got: Vec<_> = (1..=8)
            .map(|x| read(game, via, at(x), None).unwrap())
            .collect();
        assert_eq!(
            got,
            [
                Resists(6.0),
                Resists(1200.0),
                Resists(3_600_000.0),
                Resists(100.0),
                NoResistance,
                Resists(5000.0),
                Resists(0.5),
                Resists(42.0),
            ],
            "{via:?}"
        );
        assert_eq!(
            parts.spot.0.load(Ordering::Relaxed),
            2 * via.reads(),
            "{via:?}: each dirt asked once"
        );
        // A chunk that isn't loaded is never loaded for a read.
        assert_eq!(
            read(game, via, [100, 80, 100], None),
            Some(Unavailable),
            "{via:?}"
        );
        assert!(!game.world.chunks.contains_key(&(6, 6)));
        // A state the game doesn't know.
        game.world
            .set(9, 80, 1, horizon_world::registry::BlockStateId(u32::MAX))
            .unwrap();
        assert_eq!(read(game, via, at(9), None), Some(Unavailable), "{via:?}");
        // Above the level: void air.
        assert_eq!(
            read(game, via, [1, 400, 1], None),
            Some(NoResistance),
            "{via:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_batched_read_answers_each_cell_in_order() {
    each_target(|game, via, parts| {
        let cells = ResistanceCells::Box {
            min: [1, 80, 1],
            max: [8, 80, 2],
        };
        let got = reads(game, via, &cells, None).unwrap();
        let mut expected = vec![
            Resists(6.0),
            Resists(1200.0),
            Resists(3_600_000.0),
            Resists(100.0),
            NoResistance,
            Resists(5000.0),
            Resists(0.5),
            Resists(42.0),
        ];
        expected.extend([Resists(7.0); 4]);
        expected.extend([NoResistance; 4]);
        assert_eq!(got, expected, "{via:?}");
        // The pure planks part is asked once for its four cells; dirt's once a cell.
        assert_eq!(parts.pure.0.load(Ordering::Relaxed), via.reads(), "{via:?}");
        assert_eq!(
            parts.spot.0.load(Ordering::Relaxed),
            2 * via.reads(),
            "{via:?}"
        );
        let columns = ResistanceCells::Columns {
            columns: vec![(3, 1), (100, 100), (3, 1)],
            min_y: 79,
            max_y: 80,
        };
        assert_eq!(
            reads(game, via, &columns, None).unwrap(),
            [
                NoResistance,
                Resists(3_600_000.0),
                Unavailable,
                Unavailable,
                NoResistance,
                Resists(3_600_000.0)
            ],
            "{via:?}"
        );
        assert!(!game.world.chunks.contains_key(&(6, 6)));
        let too_many = ResistanceCells::Box {
            min: [0, 0, 0],
            max: [63, 63, 64],
        };
        assert_eq!(
            reads(game, via, &too_many, None),
            Err(ResistanceCellsError::TooMany(266_240)),
            "{via:?}"
        );
        assert_eq!(
            parts.pure.0.load(Ordering::Relaxed),
            via.reads(),
            "{via:?}: refused, nothing read"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_two_layer_box_reads_y_after_z_after_x() {
    each_target(|game, via, _| {
        // [1, 81, 1] and [2, 81, 2] over the row.
        assert!(game.run_command("setblock 1 81 1 minecraft:obsidian"));
        assert!(game.run_command("setblock 2 81 2 minecraft:bedrock"));
        let cells = ResistanceCells::Box {
            min: [1, 80, 1],
            max: [2, 81, 2],
        };
        assert_eq!(
            reads(game, via, &cells, None).unwrap(),
            [
                // y 80: z 1 (stone, obsidian), z 2 (planks, planks)
                Resists(6.0),
                Resists(1200.0),
                Resists(7.0),
                Resists(7.0),
                // y 81: z 1 (obsidian, air), z 2 (air, bedrock)
                Resists(1200.0),
                NoResistance,
                NoResistance,
                Resists(3_600_000.0),
            ],
            "{via:?}"
        );
    });
}

/// The source's calculator is the one `level.explode` uses: what a read says, the rays do.
/// `name` is the block at `at`, read with and without `source` and then blasted from beside it
/// with and without `source`; `kept` is whether it survives each blast.
fn read_and_blast(
    game: &mut Game<'_>,
    via: Via,
    source: Option<EntityRef>,
    at: [i32; 3],
    expected: [ExplosionResistance; 2],
    kept: [bool; 2],
) {
    let level = game.level("minecraft:overworld").unwrap();
    let block = game.world.block(at[0], at[1], at[2]);
    let center = [
        f64::from(at[0]) + 0.5,
        f64::from(at[1]) + 0.5,
        f64::from(at[2]) + 1.5,
    ];
    for (i, source) in [source, None].into_iter().enumerate() {
        let read = |game: &mut Game<'_>| {
            let native = level.explosion_resistance(
                game,
                at,
                source,
                center,
                2.0,
                false,
                ExplosionInteraction::Block,
            );
            if let Via::Wasm = via {
                let mut input = Writer::new();
                input.put(&level);
                input.put(&at);
                input.put(&source);
                input.put(&center);
                input.put(&2.0f32);
                input.put(&false);
                input.put(&ExplosionInteraction::Block);
                wasm_answer(game, level, "level_explosion_resistance", input, &native);
            }
            native
        };
        game.world.set(at[0], at[1], at[2], block).unwrap();
        assert_eq!(read(game), Some(expected[i]), "{via:?} {source:?}");
        let settings = mod_api::ExplosionSettings::default();
        match via {
            Via::Native => assert_eq!(
                level.explode(
                    game,
                    source,
                    center,
                    2.0,
                    false,
                    ExplosionInteraction::Block,
                    settings
                ),
                Ok(())
            ),
            Via::Wasm => {
                let mut input = Writer::new();
                input.put(&level);
                input.put(&source);
                input.put(&center);
                input.put(&2.0f32);
                input.put(&false);
                input.put(&ExplosionInteraction::Block);
                input.put(&settings);
                wasm_answer(
                    game,
                    level,
                    "level_explode",
                    input,
                    &Ok::<(), mod_api::ExplodeError>(()),
                );
            }
        }
        let survived = game.world.block(at[0], at[1], at[2]) == block;
        assert_eq!(
            survived, kept[i],
            "{via:?} {source:?}: the rays did as read"
        );
    }
}

#[test]
#[ignore = "daily"]
fn a_reads_calculator_is_the_one_the_blasts_rays_use() {
    each_target(|game, via, _| {
        // A dangerous wither skull caps obsidian at 0.8, and its blast breaks it; without it
        // obsidian resists 1200 and stays.
        assert!(game.run_command("summon minecraft:wither_skull 1.5 90 14.5 {dangerous:1b}"));
        let skull = game
            .creatures
            .projectiles
            .iter()
            .find(|p| matches!(p.kind, ProjectileKind::WitherSkull { dangerous: true }))
            .unwrap()
            .id;
        let skull = game.entity(skull);
        assert!(game.run_command("setblock 12 80 12 minecraft:obsidian"));
        read_and_blast(
            game,
            via,
            skull,
            [12, 80, 12],
            [Resists(0.8), Resists(1200.0)],
            [false, true],
        );
        // A primed TNT minecart's blast neither resists on nor breaks a rail; without it the
        // rail resists 0.7 and breaks.
        assert!(game.run_command("summon minecraft:tnt_minecart 2.5 80 12.5 {fuse:80s}"));
        let cart = game.carts.iter().next().unwrap().id;
        let cart = game.entity(cart);
        assert!(game.run_command("setblock 12 80 4 minecraft:stone"));
        assert!(game.run_command("setblock 12 81 4 minecraft:rail"));
        read_and_blast(
            game,
            via,
            cart,
            [12, 81, 4],
            [Resists(0.0), Resists(0.7)],
            [true, false],
        );
    });
}
