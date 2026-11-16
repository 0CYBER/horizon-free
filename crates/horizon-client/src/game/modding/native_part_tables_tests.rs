//! The client `BlockPart` tables the fluid mesher reads (arismoko/horizon#3191), resolved on the client thread:
//! with no part there are none, and a part's answers land per state, fluid and direction.
use super::native_entity_refs::tests::with_client_game;
use horizon_client::mesher::part_tables::PartTables;
use horizon_core::geometry::Direction;
use horizon_world::fluid::FluidKind;
use mod_api::{
    BlockPart, ClientHost, Hooks, Registrar, ShouldDisplayFluidOverlayEvent,
    ShouldHideAdjacentFluidFaceEvent,
};

/// Glass hides the top of still water under it, and shows no overlay beside lava's flowing form.
struct Sealed;
#[mod_api::horizon_part]
impl BlockPart for Sealed {
    fn should_hide_adjacent_fluid_face(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldHideAdjacentFluidFaceEvent,
    ) {
        event.hidden = event.direction == mod_api::Direction::Up
            && mod_api::__private::fluid_id(event.fluid.fluid) == Some("minecraft:water");
    }
    fn should_display_fluid_overlay(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldDisplayFluidOverlayEvent,
    ) {
        if mod_api::__private::fluid_id(event.fluid.fluid) == Some("minecraft:flowing_lava") {
            event.overlay = false;
        }
    }
}
struct SealedMod;
#[mod_api::__subscriptions]
impl Hooks for SealedMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let key = r.block_key("minecraft:glass");
        r.block_part(key, Sealed);
    }
}

#[test]
#[ignore = "daily"]
fn without_a_part_there_are_no_tables() {
    with_client_game(|game| {
        assert_eq!(game.resolve_part_tables(), PartTables::NONE);
    });
}

#[test]
#[ignore = "daily"]
fn a_fluid_face_part_answers_per_state_fluid_and_direction() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("sealed", Box::new(SealedMod))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let registry = game.pack.assets.registry;
            let glass = registry.default_state("minecraft:glass").unwrap().0;
            let stone = registry.default_state("minecraft:stone").unwrap().0;
            let tables = game.resolve_part_tables();
            assert!(tables.hides_fluid_face(glass, FluidKind::Water, Direction::Up));
            for direction in Direction::ALL {
                assert!(!tables.hides_fluid_face(stone, FluidKind::Water, direction));
                if direction != Direction::Up {
                    assert!(!tables.hides_fluid_face(glass, FluidKind::Water, direction));
                }
            }
            // Flowing water is a fluid of its own, and lava is not water.
            assert!(!tables.hides_fluid_face(glass, FluidKind::FlowingWater, Direction::Up));
            assert!(!tables.hides_fluid_face(glass, FluidKind::Lava, Direction::Up));
            // The overlay starts with vanilla's test: glass is half transparent, stone is not.
            assert!(tables.fluid_overlay(glass, FluidKind::Water, false));
            assert!(!tables.fluid_overlay(glass, FluidKind::FlowingLava, true));
            assert!(!tables.fluid_overlay(stone, FluidKind::FlowingLava, true));
        });
    });
}
