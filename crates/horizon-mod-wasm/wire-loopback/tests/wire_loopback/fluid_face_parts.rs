//! The fluid-face `BlockPart` methods (arismoko/horizon#3191) cross the wire with their arguments and answers
//! intact: a wasm part sees the state, the direction and the fluid the game asked with, and its
//! answer comes back to the game.
use super::*;
use mod_api::{
    BlockMetadata, BlockPart, BlockState, ClientHost, ShouldDisplayFluidOverlayEvent,
    ShouldHideAdjacentFluidFaceEvent,
};

/// A client host the generated proxies can carry; the parts read nothing from it.
struct Client;
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl ClientHost for Client {}
mod_api::impl_client_host_areas!(Client);

/// Hides a still fluid's top under the block, and turns the overlay on beside a flowing fluid.
struct Part;
#[mod_api::horizon_part]
impl BlockPart for Part {
    fn should_hide_adjacent_fluid_face(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldHideAdjacentFluidFaceEvent,
    ) {
        event.hidden = event.direction == mod_api::Direction::Up && event.fluid.is_source();
    }
    fn should_display_fluid_overlay(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldDisplayFluidOverlayEvent,
    ) {
        event.overlay ^= !event.fluid.is_source();
    }
}
struct Register;
#[mod_api::__subscriptions]
impl Hooks for Register {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.block_key("test:fluid_face_block");
    }
}

/// The installed test block's state and a proxy to the part, inside an installed mod set.
fn with_proxy(test: impl FnOnce(BlockState, &WasmHandler)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("test", Box::new(Register))].into();
    mod_api::testing::with_mods(mods.clone(), || {
        mod_api::__private::part_tests::install_parts_fixture(&mods).unwrap();
        let properties = std::collections::BTreeMap::new();
        mod_api::__private::resolve_block_keys(["test:fluid_face_block"], |_| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                .map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        mod_api::__private::resolve_fluids(
            &[
                "minecraft:empty",
                "minecraft:flowing_water",
                "minecraft:water",
            ],
            &[],
        )
        .unwrap();
        let metadata = mod_api::__private::block_definitions().unwrap();
        let state = BlockState::from(
            metadata
                .of(
                    mod_api::__private::part_tests::block_key("test:fluid_face_block"),
                    &[],
                )
                .unwrap(),
        );
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let index = guest::add_handler(vec![guest::view::<dyn BlockPart>(Arc::new(Part))]);
        let proxy = WasmHandler {
            index,
            module: WasmMod {
                group: 0,
                subscribed: &[],
                listeners: &[],
                corrupt: false,
            },
        };
        test(state, &proxy);
    });
}

/// Water as the tables ask it: a source at amount 8, or flowing at amount 7.
fn water(source: bool) -> mod_api::FluidState {
    let id = if source {
        "minecraft:water"
    } else {
        "minecraft:flowing_water"
    };
    let key = mod_api::__private::fluid_key(id).unwrap();
    mod_api::__private::fluid_state(key, if source { 8 } else { 7 }, false, source)
}

#[test]
fn wire_loopback_should_hide_adjacent_fluid_face_carries_direction_and_fluid() {
    with_proxy(|state, proxy| {
        let ask = |direction, fluid| {
            let mut event = ShouldHideAdjacentFluidFaceEvent {
                state,
                direction,
                fluid,
                hidden: false,
            };
            proxy.should_hide_adjacent_fluid_face(&Client, &mut event);
            assert_eq!(
                (event.state, event.direction, event.fluid),
                (state, direction, fluid)
            );
            event.hidden
        };
        assert!(ask(mod_api::Direction::Up, water(true)));
        assert!(!ask(mod_api::Direction::North, water(true)));
        assert!(!ask(mod_api::Direction::Up, water(false)));
    });
}

#[test]
fn wire_loopback_should_display_fluid_overlay_starts_from_the_games_answer() {
    with_proxy(|state, proxy| {
        let ask = |fluid, overlay| {
            let mut event = ShouldDisplayFluidOverlayEvent {
                state,
                fluid,
                overlay,
            };
            proxy.should_display_fluid_overlay(&Client, &mut event);
            assert_eq!((event.state, event.fluid), (state, fluid));
            event.overlay
        };
        assert!(ask(water(true), true));
        assert!(!ask(water(true), false));
        assert!(ask(water(false), false));
        assert!(!ask(water(false), true));
    });
}
