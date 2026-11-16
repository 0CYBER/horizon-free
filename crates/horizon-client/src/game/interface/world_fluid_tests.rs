//! A mod's world fluid in a real game: its bucket places it, it flows by its own rules and its
//! part's, a bucket collects it and a save keeps it, while water beside it stays vanilla's.
use super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
use horizon_client::chunk::Chunk;
use horizon_core::position::BlockPos;
use horizon_world::{chunk::BlockState, fluid::FluidKind, registry::BlockStateId};
use mod_api::WorldFluid;
use mod_api::{FluidPart, FluidRulesEvent, Hooks, LevelHost, ModEntry, Registrar};
use std::sync::Arc;

const THICK_ROW: i32 = 8;

/// Loses twice as much in one row of the world, whatever the fluid declares.
struct Thick;
#[mod_api::horizon_part]
impl FluidPart for Thick {
    fn fluid_rules(&self, _: &dyn LevelHost, event: &mut FluidRulesEvent<'_>) {
        assert_eq!(event.fluid, "oily:oil");
        if event.pos.z == THICK_ROW {
            event.drop_off *= 2;
        }
    }
}

struct Oily;
#[mod_api::__subscriptions]
impl Hooks for Oily {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let mut fluid = WorldFluid::default();
        fluid.drop_off = 2;
        r.declare_world_fluid("oil", fluid);
        r.fluid_part("oily:oil", Thick);
    }
}

#[test]
#[ignore = "daily"]
fn a_mods_fluid_is_placed_flows_is_collected_and_saved() {
    if run_fresh_cases(
        "game::world_fluid_tests::a_mods_fluid_is_placed_flows_is_collected_and_saved",
        "HORIZON_WORLD_FLUID_CHILD",
        &["valid"],
    ) {
        return;
    }
    let mods: mod_api::Mods = vec![ModEntry::new("oily", Box::new(Oily))].into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    {
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::mods::content_root(&base, &Default::default())
            .unwrap()
            .prepared()
            .to_path_buf();
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            // A floor wider than any spread looks: an edge would be a way down to flow to.
            for cx in -1..=1 {
                for cz in -1..=1 {
                    game.world
                        .chunks
                        .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                }
            }
            let registry = game.pack.assets.registry;
            let oil = registry.default_state("oily:oil").unwrap();
            let water = registry.default_state("minecraft:water").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let level = |name: &str, level: u8| {
                registry
                    .resolve(&BlockState {
                        name: name.into(),
                        properties: [("level".to_string(), level.to_string())].into(),
                    })
                    .unwrap()
            };
            for x in -16..32 {
                for z in -16..32 {
                    game.world.set(x, 63, z, stone).unwrap();
                }
            }
            let fluid_at = |game: &super::Game<'_>, x, z| {
                game.res.behaviours().fluid(game.world.block(x, 64, z))
            };

            // The block is a fluid of its own, in both forms, and water is still water.
            let behaviours = game.res.behaviours();
            assert_eq!(behaviours.fluid(oil).kind().name(), "oily:oil");
            assert!(behaviours.fluid(oil).is_source());
            let flowing: BlockStateId = level("oily:oil", 1);
            assert_eq!(behaviours.fluid(flowing).kind().name(), "oily:flowing_oil");
            assert_eq!(behaviours.fluid(flowing).amount(), 7);
            assert_eq!(behaviours.fluid(water).kind(), FluidKind::Water);
            assert!(!behaviours.fluid(oil).kind().same_family(FluidKind::Water));

            // A bucket's ray and the client's block facts see a source, not water or lava.
            let facts = game.res.facts();
            assert_eq!(facts.get(oil).fluid, horizon_client::blocks::Fluid::Modded);
            assert_eq!(facts.get(oil).fluid_level, 8);
            assert_eq!(facts.get(flowing).fluid_level, 7);
            assert_eq!(facts.get(water).fluid, horizon_client::blocks::Fluid::Water);

            // Its bucket places it.
            let (content, sound, _) =
                horizon_client::items::bucket_contents("oily:oil_bucket").unwrap();
            let at = BlockPos::new(4, 64, 4);
            let placed = game.with_level(|b, level, ticks| {
                b.bucket_empty(level, ticks, at, None, content, sound, false, false)
            });
            assert_eq!(placed, Some(at));
            assert_eq!(game.world.block(4, 64, 4), oil);

            // It flows by what it declares: 2 lost per block, where water loses 1.
            game.with_level(|b, level, ticks| b.scheduled_fluid_tick(level, ticks, at));
            let beside = fluid_at(game, 5, 4);
            assert_eq!(beside.kind().name(), "oily:flowing_oil");
            assert_eq!(beside.amount(), 6);
            game.world.set(4, 64, 12, water).unwrap();
            let pool = BlockPos::new(4, 64, 12);
            game.with_level(|b, level, ticks| b.scheduled_fluid_tick(level, ticks, pool));
            assert_eq!(fluid_at(game, 5, 12).kind(), FluidKind::FlowingWater);
            assert_eq!(fluid_at(game, 5, 12).amount(), 7);

            // Its part answers where it asks to: twice that in one row.
            game.world.set(4, 64, THICK_ROW, oil).unwrap();
            let thick = BlockPos::new(4, 64, THICK_ROW);
            game.with_level(|b, level, ticks| b.scheduled_fluid_tick(level, ticks, thick));
            assert_eq!(fluid_at(game, 5, THICK_ROW).amount(), 4);
            assert_eq!(fluid_at(game, 4, THICK_ROW + 1).amount(), 6);

            // Two sources make no third unless it declares so.
            game.world.set(9, 64, 2, oil).unwrap();
            game.world.set(11, 64, 2, oil).unwrap();
            game.world.set(10, 64, 2, flowing).unwrap();
            let between = BlockPos::new(10, 64, 2);
            game.with_level(|b, level, ticks| b.scheduled_fluid_tick(level, ticks, between));
            assert!(!fluid_at(game, 10, 2).is_source());

            // A bucket collects a source, not what flows from it.
            let beside_pos = BlockPos::new(5, 64, 4);
            let none =
                game.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, beside_pos, false));
            assert_eq!(none, None);
            let filled =
                game.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, at, false));
            assert_eq!(filled, Some("oily:oil_bucket"));
            assert_eq!(game.world.block(4, 64, 4), air);

            // A save keeps both forms, by name.
            game.world.set(1, 64, 1, oil).unwrap();
            game.world.set(2, 64, 1, flowing).unwrap();
            let biomes = game.res.biomes().clone();
            let chunk = game.world.chunks.get(&(0, 0)).unwrap();
            let saved = horizon_client::save::encode(chunk, registry, &biomes);
            let loaded = horizon_client::save::decode(&saved, registry, &biomes).unwrap();
            assert_eq!(loaded.block(1, 64, 1), oil);
            assert_eq!(loaded.block(2, 64, 1), flowing);
        });
    }
}
