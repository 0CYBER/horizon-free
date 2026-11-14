//! `CauldronInteractions.isUnderWater` asks the *fluid state* above the cauldron
//! (`FluidTags.WATER`), not the block's name: a waterlogged slab, kelp, seagrass
//! or a bubble column is water above it, so a lava or powder snow bucket on the
//! cauldron there is `InteractionResult.CONSUME` (nothing happens) instead of
//! filling it.
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;

fn game() -> Game<'static> {
    let root = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(8.5, 101.0, 8.5)).unwrap());
    game.selected = 0;
    game.set_mode_id(0);
    game
}

fn name(game: &Game<'_>, at: [i32; 3]) -> String {
    let state = game.world.block(at[0], at[1], at[2]);
    game.pack
        .assets
        .registry
        .state(state)
        .unwrap()
        .name
        .to_string()
}

fn held(game: &Game<'_>) -> Option<String> {
    game.inventory[0].as_ref().map(|s| s.item.to_string())
}

/// Run `command`, reporting what it said when it failed.
fn command(game: &mut Game<'_>, command: &str) {
    game.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.chat_capture.take().unwrap_or_default();
    assert!(
        ok,
        "{command}: {}",
        said.iter()
            .map(horizon_client::text::Text::plain)
            .collect::<String>()
    );
}

/// `setblock`, skipped when the cell already holds a block of that name
/// (`setblock` reports failure when the state would not change).
fn set(game: &mut Game<'_>, at: [i32; 3], block: &str) {
    let want = block.split('[').next().unwrap();
    if name(game, at) != want {
        command(
            game,
            &format!("setblock {} {} {} {block}", at[0], at[1], at[2]),
        );
    }
}

/// Empty `bucket` onto the empty cauldron at 8 100 8, with `above` in the cell
/// over it; the cauldron's state afterwards.
fn click_under(game: &mut Game<'_>, above: &str, bucket: &str) -> String {
    set(game, [8, 100, 8], "minecraft:cauldron");
    set(game, [8, 101, 8], above);
    game.inventory[0] = Some(ItemStack::new(bucket, 1));
    assert!(game.use_on_cauldron([8, 100, 8]).unwrap(), "{above}");
    name(game, [8, 100, 8])
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cauldron_under_water_swallows_a_lava_or_powder_snow_bucket() {
    let mut game = game();
    for (bucket, filled) in [
        ("minecraft:lava_bucket", "minecraft:lava_cauldron"),
        (
            "minecraft:powder_snow_bucket",
            "minecraft:powder_snow_cauldron",
        ),
    ] {
        // `getFluidState(pos.above()).is(FluidTags.WATER)`: the fluid, so these
        // are all water above the cauldron even though the block is not water.
        for above in [
            "minecraft:water",
            "minecraft:oak_slab[type=bottom,waterlogged=true]",
            "minecraft:oak_stairs[waterlogged=true]",
            "minecraft:kelp",
            "minecraft:seagrass",
            "minecraft:bubble_column",
        ] {
            let got = click_under(&mut game, above, bucket);
            assert_eq!(got, "minecraft:cauldron", "{bucket} under {above}");
            assert_eq!(held(&game).as_deref(), Some(bucket), "under {above}");
        }
        // Air and lava are not water, so the bucket empties into the cauldron.
        for above in ["minecraft:air", "minecraft:lava"] {
            assert_eq!(
                click_under(&mut game, above, bucket),
                filled,
                "{bucket} under {above}"
            );
            assert_eq!(held(&game).as_deref(), Some("minecraft:bucket"));
        }
    }
}
