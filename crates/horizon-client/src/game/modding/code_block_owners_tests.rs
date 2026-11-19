//! A turned-off code mod's blocks keep still: their templates' own behaviour waits until the mod
//! is on again, while an enabled mod's part on them still hears their ticks (#18519).
use crate::game::{
    Game,
    native_entity_refs::tests::{run_fresh_cases, with_game_at},
};
use horizon_core::position::BlockPos;
use mod_api::{BlockPart, Flow, Hooks, RandomTickEvent, Registrar};
use std::sync::atomic::{AtomicUsize, Ordering};

const CHILD: &str = "HORIZON_OFF_MOD_BLOCKS_CHILD";

/// The mod that goes off: blocks on templates that tick, react to a neighbour and burn fuel.
struct Frost;
#[mod_api::__subscriptions]
impl Hooks for Frost {
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        for (id, template) in [
            ("frost:cane", "minecraft:sugar_cane"),
            ("frost:tnt", "minecraft:tnt"),
            ("frost:furnace", "minecraft:furnace"),
        ] {
            event.blocks.push(mod_api::Block::new(id, template));
        }
    }
}

/// The random ticks the enabled mod's part on `frost:cane` hears.
static HEARD: AtomicUsize = AtomicUsize::new(0);
struct Listen;
#[mod_api::horizon_part]
impl BlockPart for Listen {
    fn random_tick(&self, _: &mut dyn mod_api::LevelWriteHost, _: &mut RandomTickEvent) -> Flow {
        HEARD.fetch_add(1, Ordering::Relaxed);
        Flow::Continue
    }
}
struct Watcher;
#[mod_api::__subscriptions]
impl Hooks for Watcher {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let cane = r.block_key("frost:cane");
        r.block_part(cane, Listen);
    }
}

fn frost_on(on: bool) {
    assert!(mod_api::__private::set_mod_enabled("frost", on));
}

#[test]
#[ignore = "daily"]
fn an_off_code_mods_blocks_keep_still_until_it_is_on_again() {
    // The mods are installed for the whole process: each case runs in its own.
    if run_fresh_cases(
        "game::code_block_owners_tests::an_off_code_mods_blocks_keep_still_until_it_is_on_again",
        CHILD,
        &["ticks", "neighbour", "furnace"],
    ) {
        return;
    }
    let mods: mod_api::Mods = std::sync::Arc::from([
        mod_api::ModEntry::new("frost", Box::new(Frost)),
        mod_api::ModEntry::new("watcher", Box::new(Watcher)),
    ]);
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    with_game_at(root, |game| {
        game.options.dedicated = Some(Vec::new());
        // A block-ticking chunk: lit, its neighbours loaded, with its tick list.
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
            .unwrap();
        match std::env::var(CHILD).unwrap().as_str() {
            "ticks" => ticks(game),
            "neighbour" => neighbour(game),
            _ => furnace(game),
        }
    });
}

/// A ripe cane's random tick and a loose cane's scheduled tick.
fn ticks(game: &mut Game<'_>) {
    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap();
    let sand = registry.default_state("minecraft:sand").unwrap();
    let cane = |age: &str| {
        registry
            .resolve(&horizon_world::chunk::BlockState {
                name: "frost:cane".into(),
                properties: [("age".to_string(), age.to_string())].into(),
            })
            .unwrap()
    };
    let (ripe, young) = (cane("15"), cane("0"));
    let random_tick = |game: &mut Game<'_>| {
        game.with_level(|b, level, ticks| b.random_tick(level, ticks, BlockPos::new(0, 64, 0)));
    };
    game.world.set(0, 63, 0, sand).unwrap();
    game.world.set(0, 64, 0, ripe).unwrap();
    // Nothing under it: its own scheduled tick would break it.
    let loose = BlockPos::new(4, 64, 4);
    game.world.set(4, 64, 4, young).unwrap();
    let start = game.world.game_time;
    game.with_level(|b, level, ticks| b.schedule_tick(level, ticks, loose, 1));

    frost_on(false);
    random_tick(game);
    assert_eq!(game.world.block(0, 64, 0), ripe, "the cane keeps its age");
    assert_eq!(game.world.block(0, 65, 0), air, "and grows nothing");
    assert_eq!(
        HEARD.load(Ordering::Relaxed),
        1,
        "an enabled mod's part still hears the tick"
    );
    game.world.game_time = start + 1;
    game.tick_scheduled().unwrap();
    assert_eq!(game.world.block(4, 64, 4), young, "the due tick leaves it");

    frost_on(true);
    random_tick(game);
    assert_eq!(
        game.world.block(0, 65, 0),
        young,
        "on again, the cane grows"
    );
}

/// A TNT template beside a new power source.
fn neighbour(game: &mut Game<'_>) {
    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap();
    let tnt = registry.default_state("frost:tnt").unwrap();
    let power = registry.default_state("minecraft:redstone_block").unwrap();
    let at = BlockPos::new(8, 64, 0);
    game.world.set(8, 64, 0, tnt).unwrap();

    frost_on(false);
    game.with_level(|b, level, ticks| b.set_block(level, ticks, BlockPos::new(9, 64, 0), power, 3));
    assert_eq!(game.world.block(8, 64, 0), tnt, "it doesn't prime");

    frost_on(true);
    game.with_level(|b, level, ticks| b.handle_neighbor_changed(level, ticks, at, b.block(power)));
    assert_eq!(game.world.block(8, 64, 0), air, "on again, it primes");
}

/// A furnace template's block entity with fuel burning.
fn furnace(game: &mut Game<'_>) {
    use horizon_client::{
        containers::{BlockEntity, Furnace, FurnaceKind},
        items::ItemStack,
    };
    let state = game
        .pack
        .assets
        .registry
        .default_state("frost:furnace")
        .unwrap();
    let pos = (8, 64, 8);
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
    game.world.block_entities.insert(
        pos,
        BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::Furnace,
            items: [
                Some(ItemStack::new("minecraft:raw_iron", 1)),
                Some(ItemStack::new("minecraft:coal", 1)),
                None,
            ],
            lit_remaining: 5,
            lit_total: 200,
            cooking_timer: 0,
            cooking_total: 200,
            speed: 1.0,
            recipes_used: std::collections::HashMap::new(),
        }),
    );
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(
            game.motion,
            horizon_core::geometry::Vec3::new(8.5, 65.0, 8.5),
        )
        .unwrap(),
    );
    game.update_chunk_tickets();
    assert!(game.block_ticking_area()(8, 8), "the furnace's chunk ticks");
    let lit = |game: &Game<'_>| match game.world.block_entities.get(&pos) {
        Some(BlockEntity::Furnace(f)) => f.lit_remaining,
        _ => panic!("no furnace"),
    };

    frost_on(false);
    game.tick_block_entities().unwrap();
    assert_eq!(lit(game), 5, "its fuel doesn't burn");

    frost_on(true);
    game.tick_block_entities().unwrap();
    assert!(lit(game) < 5, "on again, it burns");
}
