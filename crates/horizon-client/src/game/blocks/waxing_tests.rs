//! Honeycomb (the player's and the dispenser's, both through `honeycomb_waxed`) and the axe's
//! scrape and wax-off (`axe_mapped`), with and without the world's NeoForge block maps.
use crate::game::native_entity_refs::tests::with_game;
use horizon_world::level::BlockMaps;
use horizon_world::registry::BlockStateId;
use mod_api::ToolAbility::{AxeScrape, AxeWaxOff};
use serde_json::json;
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn honeycomb_and_the_axe_read_the_block_maps_before_vanillas_tables() {
    with_game(|game| {
        let registry = game.pack.assets.registry;
        let state = |name: &str, props: &[(&str, &str)]| -> BlockStateId {
            let mut state = registry
                .state(registry.default_state(name).unwrap())
                .unwrap()
                .clone();
            for (key, value) in props {
                state
                    .properties
                    .insert((*key).to_owned(), (*value).to_owned());
            }
            registry.resolve(&state).unwrap()
        };
        let stair = [("facing", "east"), ("half", "top")];
        let copper_stairs = state("minecraft:cut_copper_stairs", &stair);
        let waxed_copper_stairs = state("minecraft:waxed_cut_copper_stairs", &stair);
        let stairs = state("minecraft:stone_brick_stairs", &stair);
        let mossy = state("minecraft:mossy_stone_brick_stairs", &stair);
        let slab = [("type", "top")];
        let exposed = state("minecraft:exposed_cut_copper_slab", &slab);
        let oxidized = state("minecraft:oxidized_cut_copper_slab", &slab);

        // No maps: vanilla's `WAXABLES`, read from the axe's wax-off transforms (the dispenser's
        // fallback since it stopped building `waxed_` names), properties kept.
        assert_eq!(
            game.honeycomb_waxed(copper_stairs),
            Some(waxed_copper_stairs)
        );
        assert_eq!(
            game.honeycomb_waxed(state("minecraft:lightning_rod", &[])),
            Some(state("minecraft:waxed_lightning_rod", &[]))
        );
        assert_eq!(game.honeycomb_waxed(waxed_copper_stairs), None);
        assert_eq!(game.honeycomb_waxed(stairs), None);
        assert!(game.axe_mapped("minecraft:axe", oxidized).is_none());

        let to_oxidized = json!({"next_oxidation_stage": "minecraft:oxidized_cut_copper_slab"});
        let to_mossy = json!({"waxed": "minecraft:mossy_stone_brick_stairs"});
        let table = &game.res.behaviours().table;
        let rank = |name: &str| {
            table
                .block_by_name(name)
                .map_or(u32::MAX, |id| u32::from(id.0))
        };
        game.res
            .behaviours()
            .set_block_maps(Some(Arc::new(BlockMaps::from_values(
                [],
                [("minecraft:exposed_cut_copper_slab", &to_oxidized)],
                [("minecraft:stone_brick_stairs", &to_mossy)],
                rank,
            ))));
        assert_eq!(game.honeycomb_waxed(stairs), Some(mossy));
        // A block the maps leave out keeps vanilla's twin.
        assert_eq!(
            game.honeycomb_waxed(copper_stairs),
            Some(waxed_copper_stairs)
        );
        assert!(matches!(
            game.axe_mapped("minecraft:axe", mossy),
            Some((state, AxeWaxOff)) if state == stairs
        ));
        assert!(matches!(
            game.axe_mapped("minecraft:axe", oxidized),
            Some((state, AxeScrape)) if state == exposed
        ));
        assert!(game.axe_mapped("minecraft:hoe", mossy).is_none());

        // A reload with no maps installs none: vanilla's answers again.
        game.res.behaviours().set_block_maps(None);
        assert_eq!(game.honeycomb_waxed(stairs), None);
        assert!(game.axe_mapped("minecraft:axe", mossy).is_none());
    });
}
