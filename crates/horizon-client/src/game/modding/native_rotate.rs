// game-module-visibility: pub(crate)
//! The engine side of `rotate`: the callback every template placement turns its states through,
//! asking the parts of each state's block on the worldgen workers and on the server thread. It
//! reads what the server published (`native_worldgen_parts`), never a game thread's tables.
use super::api_values::block_state::Index;
use super::native_worldgen_parts;
use horizon_world::registry::BlockStateId as EngineState;
use horizon_world::worldgen::structure::Rotation;
use mod_api::{BlockState, HookId, StructureRotation};

/// The engine's `rotate` callback: the state the parts of `state`'s block place at `pos` for
/// `state` turned by `rotation`, starting from the game's `table`. Nothing is read or converted
/// while no part answers the hook; a refused answer or a panicking part keeps `table`.
pub(crate) fn rotate(
    state: EngineState,
    pos: [i32; 3],
    rotation: Rotation,
    table: EngineState,
) -> EngineState {
    if !mod_api::__private::has_part_hook(HookId::Rotate) {
        return table;
    }
    let Some(index) = native_worldgen_parts::index() else {
        return table;
    };
    if !index.worldgen_listens(HookId::Rotate, state) {
        return table;
    }
    let (Ok(asked), Ok(mut rotated)) = (index.to_api(state), index.to_api(table)) else {
        return table;
    };
    let rotation = match rotation {
        Rotation::None => StructureRotation::None,
        Rotation::Clockwise90 => StructureRotation::Clockwise90,
        Rotation::Clockwise180 => StructureRotation::Clockwise180,
        Rotation::Counterclockwise90 => StructureRotation::Counterclockwise90,
    };
    if let Err(error) = horizon_client::crash::contain(|| {
        native_worldgen_parts::with_host(|host| {
            mod_api::world::rotation::native_rotate(host, asked, pos, rotation, &mut rotated);
        });
    }) {
        log::warn!("rotate callback failed: {error}; the template places the game's state");
        return table;
    }
    turned(&index, asked, rotated).unwrap_or(table)
}

/// The parts' answer for `state`, as the engine places it: None for a state of another block or
/// one the game's registry does not have, naming the mods whose parts answer for `state`'s
/// block.
fn turned(index: &Index, state: BlockState, answer: BlockState) -> Option<EngineState> {
    let refused = |why: &str| {
        log::warn!(
            "rotate: a part of {} answered {why}; the template places the game's state",
            mod_api::__private::worldgen_part_mods(HookId::Rotate, state).join(", ")
        );
        None
    };
    if answer.block() != state.block() {
        return refused("a state of another block");
    }
    match index.to_engine(answer) {
        Ok(placed) => Some(placed),
        Err(error) => refused(&format!(
            "a block state this game does not have ({error:?})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    use horizon_world::registry::BlockRegistry;
    use horizon_world::worldgen::structure::Mirror;
    use mod_api::{BlockPart, Hooks, ModEntry, Mods, RotateEvent, WorldgenHost};
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, PoisonError};

    /// The states the furnace's part was asked with.
    static ASKED: Mutex<Vec<BlockState>> = Mutex::new(Vec::new());

    /// A furnace that always faces north once placed.
    struct North;
    #[mod_api::horizon_part]
    impl BlockPart for North {
        fn rotate(&self, context: &dyn WorldgenHost, event: &mut RotateEvent<'_>) {
            ASKED
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(event.state);
            *event.rotated = context
                .state_with_property(event.state, "facing", "north")
                .unwrap();
        }
    }
    /// The state of another block the smoker's part answers.
    static OTHER: Mutex<Option<BlockState>> = Mutex::new(None);
    /// A smoker's part answering a state of another block.
    struct Stone;
    #[mod_api::horizon_part]
    impl BlockPart for Stone {
        fn rotate(&self, _: &dyn WorldgenHost, event: &mut RotateEvent<'_>) {
            *event.rotated = OTHER
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap();
        }
    }
    /// A lectern's part that panics.
    struct Panics;
    #[mod_api::horizon_part]
    impl BlockPart for Panics {
        fn rotate(&self, _: &dyn WorldgenHost, _: &mut RotateEvent<'_>) {
            panic!("a part that panics");
        }
    }
    struct Turns;
    #[mod_api::__subscriptions]
    impl Hooks for Turns {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let furnace = r.block_key("minecraft:furnace");
            r.block_part(furnace, North);
            let smoker = r.block_key("minecraft:smoker");
            r.block_part(smoker, Stone);
            let lectern = r.block_key("minecraft:lectern");
            r.block_part(lectern, Panics);
        }
    }

    /// `name` facing `facing`.
    fn facing(registry: &BlockRegistry, name: &str, facing: &str) -> EngineState {
        let default = registry.default_state(name).unwrap();
        crate::game::native_hooks::with_property(registry, default, "facing", facing).unwrap()
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_part_turns_its_block_as_a_template_places_it() {
        let _publishing = native_worldgen_parts::PUBLISHING
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mods: Mods = vec![ModEntry::new("turns", Box::new(Turns))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let registry = game.pack.assets.registry;
                let transforms = game.state_transforms().unwrap();
                // A server resolves its parts against its tags and publishes them.
                game.options.dedicated = Some(Vec::new());
                game.resolve_parts().unwrap();
                let index = native_worldgen_parts::index().unwrap();
                *OTHER.lock().unwrap_or_else(PoisonError::into_inner) = index
                    .to_api(registry.default_state("minecraft:stone").unwrap())
                    .ok();
                let root = std::env::temp_dir().join("horizon-rotate-no-settings");
                let load = LoadContext::new(
                    DataLoadOrigin::ServerDataLoad,
                    Arc::new(LoadInputs::new(BTreeMap::new(), root)),
                    [],
                );
                horizon_world::native_worldgen_hooks::publish_context(Arc::new(
                    load.worldgen_context([]),
                ));
                // `/place template … clockwise_90 left_right`: LEFT_RIGHT turns north to south,
                // then a clockwise quarter turn makes it west.
                let place = |name: &str| {
                    let mirrored =
                        transforms.mirror(facing(registry, name, "north"), Mirror::LeftRight);
                    let table = transforms.rotate(mirrored, Rotation::Clockwise90);
                    (
                        rotate(mirrored, [4, 64, 2], Rotation::Clockwise90, table),
                        table,
                    )
                };
                let (blast, table) = place("minecraft:blast_furnace");
                assert_eq!(blast, facing(registry, "minecraft:blast_furnace", "west"));
                assert_eq!(blast, table);
                let (furnace, _) = place("minecraft:furnace");
                assert_eq!(furnace, facing(registry, "minecraft:furnace", "north"));
                let asked = ASKED.lock().unwrap_or_else(PoisonError::into_inner).clone();
                let south = index
                    .to_api(facing(registry, "minecraft:furnace", "south"))
                    .unwrap();
                assert_eq!(asked, [south]);
                // A state of another block, and a panicking part, keep the game's state.
                let (smoker, table) = place("minecraft:smoker");
                assert_eq!(
                    (smoker, table),
                    (table, facing(registry, "minecraft:smoker", "west"))
                );
                let (lectern, table) = place("minecraft:lectern");
                assert_eq!(
                    (lectern, table),
                    (table, facing(registry, "minecraft:lectern", "west"))
                );
            });
        });
    }
}

#[cfg(test)]
#[path = "native_rotate/jigsaw_tests.rs"]
mod jigsaw_tests;
