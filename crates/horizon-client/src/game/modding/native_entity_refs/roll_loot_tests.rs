//! `LevelRef::roll_loot`: one roll of a world pack's loot table, handed back. On the
//! same random state it gives what the engine's own roll in the same context gives,
//! `/loot … fish <table> <pos> <tool>` (whose `loot` path the loot harnesses hold to
//! vanilla 26.3).
use crate::game::{
    Audio, Game, Options, Progress, Settings, execution_role, functions, load_resources,
};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use horizon_core::{
    geometry::Vec3,
    position::ChunkPos,
    random::{LegacyRandom, RandomSource as _},
};
use horizon_items::items::ItemStack;
use mod_api::{Authority, LevelRef};
use std::sync::Arc;

/// A weighted pool rolled 1 to 3 times (flint 6, iron nuggets 3 with a binomial
/// count that may be 0, a gold nugget 1); a pool behind a 30% chance whose
/// alternatives give a diamond to an iron shovel and coal otherwise; gravel
/// with Fortune's uniform bonus; 40 ender pearls to an iron shovel.
const SIFT: &str = r#"{"type":"minecraft:fishing","pools":[
 {"rolls":{"type":"minecraft:uniform","min":1,"max":3},"entries":[
  {"type":"minecraft:item","name":"minecraft:flint","weight":6,
   "modifier":[{"type":"minecraft:set_count","count":{"type":"minecraft:uniform","min":1,"max":4}}]},
  {"type":"minecraft:item","name":"minecraft:iron_nugget","weight":3,
   "modifier":[{"type":"minecraft:set_count","count":{"type":"minecraft:binomial","n":5,"p":0.4}}]},
  {"type":"minecraft:item","name":"minecraft:gold_nugget","weight":1}]},
 {"rolls":1,"condition":{"type":"minecraft:random_chance","chance":0.3},"entries":[
  {"type":"minecraft:alternatives","children":[
   {"type":"minecraft:item","name":"minecraft:diamond",
    "condition":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:iron_shovel"}}},
   {"type":"minecraft:item","name":"minecraft:coal"}]}]},
 {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gravel",
  "modifier":[{"type":"minecraft:apply_bonus","enchantment":"minecraft:fortune",
   "formula":"minecraft:uniform_bonus_count","parameters":{"bonusMultiplier":1}}]}]},
 {"rolls":1,"condition":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:iron_shovel"}},
  "entries":[{"type":"minecraft:item","name":"minecraft:ender_pearl",
   "modifier":[{"type":"minecraft:set_count","count":40}]}]}]}"#;

/// One flint from 1 to 3 rolls, drawing from its own random sequence.
const SEQUENCE: &str = r#"{"type":"minecraft:fishing","random_sequence":"test:sift_sequence","pools":[
 {"rolls":{"type":"minecraft:uniform","min":1,"max":3},"entries":[{"type":"minecraft:item","name":"minecraft:flint"}]}]}"#;

/// The centre of block 8 100 8, the origin `fish … 8 100 8` gives.
const ORIGIN: [f64; 3] = [8.5, 100.5, 8.5];
const COMMAND_SHOVEL: &str = "minecraft:iron_shovel";
const COMMAND_FORTUNE_SHOVEL: &str =
    r#"minecraft:iron_shovel[minecraft:enchantments={"minecraft:fortune":3}]"#;

fn shovel(fortune: bool) -> ItemStack {
    let mut stack = ItemStack::new("minecraft:iron_shovel", 1);
    if fortune {
        stack.data_mut().enchantments = vec![("minecraft:fortune".into(), 3)];
    }
    stack
}

/// A server game in an open world with the `sift` data pack, and its overworld handle.
fn game<'a>(
    resources: &'a mut Option<Arc<crate::game::Resources>>,
    name: &str,
) -> (Game<'a>, LevelRef) {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.native_lifecycle
        .begin_open(Authority::IntegratedServer);
    // The player the command's `@s` is.
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 100.0, 8.5)).unwrap());
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("gamemode survival"));

    let dir = std::env::temp_dir().join(format!("horizon-roll-loot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let tables = dir.join("datapacks/sift/data/test/loot_table");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        dir.join("datapacks/sift/pack.mcmeta"),
        r#"{"pack":{"description":"fixture"}}"#,
    )
    .unwrap();
    std::fs::write(tables.join("sift.json"), SIFT).unwrap();
    std::fs::write(tables.join("sequence.json"), SEQUENCE).unwrap();
    std::fs::write(tables.join("small.json"), r#"{"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:ender_pearl","modifier":[{"type":"minecraft:set_count","count":40},{"type":"minecraft:set_components","components":{"minecraft:max_stack_size":8}}]}]}]}"#).unwrap();
    std::fs::write(
        dir.join("datapacks.json"),
        r#"{"enabled":["file/sift"],"disabled":[]}"#,
    )
    .unwrap();
    game.world_root = dir;
    let stack = functions::pack_stack(
        &game.world_root,
        &root,
        &horizon_assets::resource_source::ResourceSource::directory(root.clone()),
        &Default::default(),
        false,
        &[],
        &[],
    );
    game.world_tables = game.stage_world_tables(&stack, false, &root).unwrap().0;
    // The command's table argument knows the pack's tables, as after a world opens.
    game.sync_loot_table_ids();
    let level = game.level("minecraft:overworld").unwrap();
    (game, level)
}

/// The stacks as (item, count, enchantments) lines, empties left out.
fn lines<'a>(stacks: impl IntoIterator<Item = &'a ItemStack>) -> Vec<String> {
    stacks
        .into_iter()
        .filter(|s| s.count > 0)
        .map(|s| {
            let enchantments = s
                .data()
                .map_or(String::new(), |d| format!("{:?}", d.enchantments));
            format!("{} x{} {enchantments}", s.item, s.count)
        })
        .collect()
}

/// What `/loot replace entity @s container.0 fish …` puts in the inventory, in
/// the order the roll gave it.
fn command_roll(game: &mut Game<'_>, table: &str, tool: &str) -> Vec<String> {
    let _ = game.run_command("clear @s");
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(&format!(
        "loot replace entity @s container.0 fish {table} 8 100 8 {tool}"
    ));
    let said: Vec<String> = game
        .chat_capture
        .take()
        .unwrap_or_default()
        .iter()
        .map(horizon_client::text::Text::plain)
        .collect();
    assert!(ok, "{said:?}");
    lines(game.inventory.iter().take(36).flatten())
}

fn count_of(stacks: &[ItemStack], item: &str) -> Vec<u32> {
    stacks
        .iter()
        .filter(|s| &*s.item == item)
        .map(|s| s.count)
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_roll_is_the_engines_own_roll_for_the_same_seed_and_tool() {
    let mut resources = None;
    let (mut game, level) = game(&mut resources, "same");
    for (tool, command_tool) in [
        (shovel(false), COMMAND_SHOVEL),
        (shovel(true), COMMAND_FORTUNE_SHOVEL),
        (ItemStack::new("minecraft:stick", 1), "minecraft:stick"),
    ] {
        for seed in 0..64 {
            *game.world.random = LegacyRandom::new(seed);
            let rolled = game
                .level_roll_loot(level, "test:sift", ORIGIN, Some(&tool), None)
                .unwrap();
            let after = game.world.random.next_long();
            *game.world.random = LegacyRandom::new(seed);
            let by_command = command_roll(&mut game, "test:sift", command_tool);
            assert_eq!(
                game.world.random.next_long(),
                after,
                "seed {seed}: the same draws"
            );
            // The command fills a slot per stack and merges nothing; the pearls
            // are one stack of 40 there, split to stack size by the roll.
            let mut rolled = lines(&rolled);
            if let Some(i) = rolled
                .iter()
                .position(|l| l.starts_with("minecraft:ender_pearl"))
            {
                rolled.splice(i..i + 3, ["minecraft:ender_pearl x40 ".to_owned()]);
            }
            assert_eq!(rolled, by_command, "seed {seed}, tool {command_tool}");
        }
    }
}

#[test]
#[ignore = "daily"]
fn weighted_pools_conditions_and_functions_roll_as_the_table_says() {
    let mut resources = None;
    let (mut game, level) = game(&mut resources, "table");
    let (shovel, fortune) = (shovel(false), shovel(true));
    let (mut extra_pool, mut picks, mut flint_picks) = (0, 0, 0);
    let mut gravel_counts = std::collections::BTreeSet::new();
    for seed in 0..400 {
        *game.world.random = LegacyRandom::new(seed);
        let with_shovel = game
            .level_roll_loot(level, "test:sift", ORIGIN, Some(&shovel), None)
            .unwrap();
        // The weighted pool: 1 to 3 picks, each one stack.
        let flint = count_of(&with_shovel, "minecraft:flint");
        let nuggets = count_of(&with_shovel, "minecraft:iron_nugget");
        let gold = count_of(&with_shovel, "minecraft:gold_nugget");
        assert!(flint.iter().all(|c| (1..=4).contains(c)), "{flint:?}");
        assert!(nuggets.iter().all(|c| (1..=5).contains(c)), "{nuggets:?}");
        assert!(gold.iter().all(|&c| c == 1));
        assert!(flint.len() + nuggets.len() + gold.len() <= 3);
        // A binomial count of 0 gives nothing, so a roll may hold fewer.
        flint_picks += flint.len();
        picks += flint.len() + gold.len();
        // The 30% pool's alternatives: the diamond, as the shovel matches.
        assert!(count_of(&with_shovel, "minecraft:coal").is_empty());
        extra_pool += count_of(&with_shovel, "minecraft:diamond").len();
        // Pearls only to the shovel: 40, split to their stack size.
        assert_eq!(count_of(&with_shovel, "minecraft:ender_pearl"), [16, 16, 8]);
        assert_eq!(
            count_of(&with_shovel, "minecraft:gravel"),
            [1],
            "no Fortune"
        );

        // No tool: no diamond and no pearls; the alternatives give coal.
        let bare = game
            .level_roll_loot(level, "test:sift", ORIGIN, None, None)
            .unwrap();
        assert!(count_of(&bare, "minecraft:diamond").is_empty());
        assert!(count_of(&bare, "minecraft:ender_pearl").is_empty());
        assert!(count_of(&bare, "minecraft:coal").iter().all(|&c| c == 1));

        // Fortune III: 1 + nextInt(4) gravel.
        let lucky = game
            .level_roll_loot(level, "test:sift", ORIGIN, Some(&fortune), None)
            .unwrap();
        gravel_counts.extend(count_of(&lucky, "minecraft:gravel"));
    }
    assert_eq!(gravel_counts.into_iter().collect::<Vec<_>>(), [1, 2, 3, 4]);
    // 400 rolls: about 120 pass the 30% pool, and flint is 6 of the 7 weights
    // of the picks that aren't iron nuggets (whose count can be 0).
    assert!((90..=150).contains(&extra_pool), "{extra_pool}");
    let share = flint_picks as f64 / picks as f64;
    assert!((0.8..0.92).contains(&share), "{share}");
}

#[test]
#[ignore = "daily"]
fn a_table_with_a_random_sequence_draws_from_it_not_the_level() {
    let mut resources = None;
    let (mut game, level) = game(&mut resources, "sequence");
    let sequences = game.rand_sequences.clone();
    *game.world.random = LegacyRandom::new(7);
    let rolled = game
        .level_roll_loot(level, "test:sequence", ORIGIN, None, None)
        .unwrap();
    assert_eq!(
        game.world.random.next_long(),
        LegacyRandom::new(7).next_long(),
        "the level's random is untouched"
    );
    let n: u32 = rolled.iter().map(|s| s.count).sum();
    assert!((1..=3).contains(&n));
    // The command draws from the same sequence, from where it was.
    game.rand_sequences = sequences;
    assert_eq!(
        lines(&rolled),
        command_roll(&mut game, "test:sequence", "minecraft:stick")
    );
}

#[test]
#[ignore = "daily"]
fn a_missing_table_and_a_stale_handle_are_none() {
    let mut resources = None;
    let (mut game, level) = game(&mut resources, "none");
    assert_eq!(
        game.level_roll_loot(level, "test:nothing_here", ORIGIN, None, None),
        None
    );
    // A handle of an earlier open.
    game.native_lifecycle
        .begin_open(Authority::IntegratedServer);
    assert_eq!(
        game.level_roll_loot(level, "test:sift", ORIGIN, None, None),
        None
    );
    let level = game.level("minecraft:overworld").unwrap();
    assert!(
        game.level_roll_loot(level, "test:sift", ORIGIN, None, None)
            .is_some()
    );
}

#[test]
#[ignore = "daily"]
fn component_stack_capacity_is_preserved_on_every_split() {
    let mut resources = None;
    let (mut game, level) = game(&mut resources, "component-split");
    let stacks = game
        .level_roll_loot(level, "test:small", ORIGIN, None, None)
        .unwrap();
    assert_eq!(count_of(&stacks, "minecraft:ender_pearl"), [8, 8, 8, 8, 8]);
}

/// `Game::loot_place_at` reads a loaded origin's block and light; an unloaded
/// origin leaves those facts unset so a predicate fails instead of reading the
/// air and full light an absent chunk falls back to. A state the caller passes
/// (the block being broken) is kept.
#[test]
#[ignore = "daily"]
fn a_loot_place_reads_only_a_loaded_origin() {
    let mut resources = None;
    let (game, _level) = game(&mut resources, "place-residency");
    let loaded = game.loot_place_at(ORIGIN, None);
    assert!(loaded.block.is_some(), "a loaded origin has a block");
    assert!(loaded.light.is_some(), "a loaded origin has a light");
    assert!(loaded.can_see_sky.is_some());
    assert!(loaded.structures.is_some());

    let unloaded = game.loot_place_at([40.5, 100.5, 40.5], None);
    assert_eq!(unloaded.origin, Some([40.5, 100.5, 40.5]));
    assert!(unloaded.block.is_none(), "an unloaded origin has no block");
    assert!(unloaded.light.is_none(), "an unloaded origin has no light");
    assert_eq!(unloaded.can_see_sky, None);
    assert_eq!(unloaded.fluid, None);
    assert_eq!(unloaded.structures, None);

    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let supplied = game.loot_place_with_block([40.5, 100.5, 40.5], None, Some(stone));
    assert_eq!(
        supplied.block.as_ref().map(|block| block.name.as_str()),
        Some("minecraft:stone"),
        "an explicitly supplied state is kept"
    );
    assert!(
        supplied.light.is_none(),
        "the unloaded origin still has no light"
    );
}
