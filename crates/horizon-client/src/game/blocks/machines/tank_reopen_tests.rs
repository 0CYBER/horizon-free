//! A committed mod tank survives a complete world save, close and reopen (arismoko/horizon#2412): the
//! create-mekanism capability audit's undetermined persistence row. The tanks are the game's
//! own (`native_block_entity_fluids`), filled through `LevelRef::capability(FLUIDS)` in a
//! committed transaction; an aborted transaction's insert must not reach the disk.
//!
//! Routes exercised: an integrated server in this process with one native mod; `load_world`
//! creating a flat world folder; the chunk streamed in under a `/forceload` ticket; the block
//! placed with `/setblock`; `Game::save_game` (the autosave's and the pause menu's save) writing
//! the region file; the game dropped; then a second game opening the same folder and streaming
//! the chunk back from that file. Not covered: LAN guests, a dedicated server process, wasm
//! mods, and `close_world`'s closing save, which finishes over event-loop stages a test can't
//! run.
use super::{BlockEntity, Game};
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use crate::game::world_loading::WorldRequest;
use mod_api::{
    BlocksEvent, FLUIDS, FluidResource, FluidStack, Hooks, HostTransaction, LevelRef, ModEntry,
    ModFluidTanks, Mods, Registrar, RegistrationContext,
};
use std::path::Path;
use std::time::{Duration, Instant};

const TANK: &str = "tank_reopen:tank";
const BRINE: &str = "tank_reopen:brine";

struct Tanks;
#[mod_api::__subscriptions]
impl Hooks for Tanks {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_fluid("brine");
        r.block_entity_type("tank").blocks(&[TANK]).build();
        let mut tanks = ModFluidTanks::default();
        tanks.tanks = vec![4000, 2000];
        tanks.inputs = vec![0, 1];
        tanks.outputs = vec![0, 1];
        tanks.faces = std::array::from_fn(|_| vec![0, 1]);
        let block = r.block_key(TANK);
        assert!(!r.is_block_capability_registered(&FLUIDS, &block));
        r.block_fluids(&block, tanks);
        assert!(r.is_block_capability_registered(&FLUIDS, &block));
    }
    fn blocks(&self, _: &RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .blocks
            .push(mod_api::Block::new(TANK, "minecraft:stone"));
    }
}

/// The world in folder `world`: created flat (quick to generate) when `create`, else opened as
/// saved.
fn request(world: &Path, create: bool) -> WorldRequest {
    let rules = std::sync::Arc::new(horizon_client::gamerules::RuleRegistry::vanilla());
    let options = horizon_client::title::WorldOptions {
        world_type: horizon_client::save::WorldPreset::Flat,
        rules: horizon_client::gamerules::GameRules::with_registry(rules.clone(), &[]),
        ..Default::default()
    };
    let mut request = WorldRequest::new(
        "tank",
        Some("tank"),
        Some(1),
        true,
        create.then_some(&options),
        rules,
    )
    .unwrap();
    request.save_root = world.to_owned();
    request
}

fn fluid(id: &str, amount: u32) -> FluidResource {
    FluidResource::of(&FluidStack::new(
        mod_api::__private::fluid_key(id).unwrap(),
        amount,
    ))
}

/// Loads chunk (0, 0) through the game's own streaming, under a `/forceload` ticket (asked for
/// unless the world already holds one), waiting for it as `Game::pump_chunks` does, then runs
/// a server tick: the first one builds the tables of the mods' tanks.
fn load_tank_chunk(game: &mut Game<'_>) {
    if !game.world.chunks.contains_key(&(0, 0)) {
        assert!(game.run_command("forceload add 8 8") || game.run_command("forceload query 8 8"));
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    while !game.world.chunks.contains_key(&(0, 0)) {
        assert!(Instant::now() < deadline, "chunk (0, 0) did not load");
        game.streaming.wait(Duration::from_millis(5));
        game.update_world();
    }
    game.tick_world(true).unwrap();
}

/// Each tank's fluid and amount, as the game's `FLUIDS` capability reports them.
fn contents(
    game: &mut Game<'_>,
    level: LevelRef,
    pos: [i32; 3],
) -> Vec<(Option<FluidResource>, u32)> {
    let tanks = level.capability(game, &FLUIDS, pos, None).unwrap();
    assert_eq!(tanks.owner(), "the game");
    (0..tanks.size(game))
        .map(|tank| (tanks.resource(game, tank), tanks.amount(game, tank)))
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_committed_mod_tank_survives_a_world_save_close_and_reopen() {
    if run_fresh_cases(
        "game::machines::tank_reopen_tests::a_committed_mod_tank_survives_a_world_save_close_and_reopen",
        "HORIZON_TANK_REOPEN_CHILD",
        &["game"],
    ) {
        return;
    }
    let mods: Mods = vec![ModEntry::new("tank_reopen", Box::new(Tanks))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default())
        .unwrap()
        .prepared()
        .to_path_buf();
    let saves = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("tank-reopen-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&saves);
    // A mod's own fluid and a vanilla one: the identity check covers both kinds of id.
    let expected = || {
        vec![
            (Some(fluid(BRINE, 1)), 1500),
            (Some(fluid("minecraft:lava", 1)), 1000),
        ]
    };

    let mut placed = None;
    with_game_at(root.clone(), |game| {
        game.load_world(request(&saves, true)).unwrap();
        game.set_allow_commands(true);
        load_tank_chunk(game);
        let pos = [8, 100, 8];
        let level = game.level("minecraft:overworld").unwrap();
        // `/setblock`: the game's own placement, which makes the block entity.
        assert!(game.run_command(&format!("setblock {} {} {} {TANK}", pos[0], pos[1], pos[2])));
        assert!(matches!(
            game.world.block_entities.get(&(pos[0], pos[1], pos[2])),
            Some(BlockEntity::Mod(entity)) if entity.id() == TANK
        ));
        let tanks = level.capability(game, &FLUIDS, pos, None).unwrap();
        assert_eq!(
            game.transaction(|host, tx| Ok::<_, ()>((
                tanks.insert(host, tx, 0, &fluid(BRINE, 1), 1500),
                tanks.insert(host, tx, 1, &fluid("minecraft:lava", 1), 1000),
            ))),
            Ok((1500, 1000))
        );
        // An aborted transaction's insert is rolled back and must not be saved.
        assert_eq!(
            game.transaction(|host, tx| {
                assert_eq!(tanks.insert(host, tx, 0, &fluid(BRINE, 1), 500), 500);
                Err::<(), _>("abort")
            }),
            Err("abort")
        );
        assert_eq!(contents(game, level, pos), expected());
        // The integrated server's save: what the autosave and the pause menu run (`save-all` is a
        // dedicated server's command). It waits for the region files to be written.
        game.save_game().unwrap();
        assert!(
            saves
                .join("dimensions/minecraft/overworld/region/r.0.0.mca")
                .exists()
        );
        placed = Some(pos);
    });
    let pos = placed.unwrap();

    // A new game on the same folder: nothing of the first one is in memory.
    with_game_at(root, |game| {
        game.load_world(request(&saves, false)).unwrap();
        game.set_allow_commands(true);
        load_tank_chunk(game);
        assert!(matches!(
            game.world.block_entities.get(&(pos[0], pos[1], pos[2])),
            Some(BlockEntity::Mod(entity)) if entity.id() == TANK
        ));
        let level = game.level("minecraft:overworld").unwrap();
        assert_eq!(contents(game, level, pos), expected());
    });
    let _ = std::fs::remove_dir_all(&saves);
}
