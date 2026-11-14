#![cfg(test)]
//! `on_block_state_change` heard by a block Part at the end of `Level.setBlock`: after the
//! neighbours' shapes settled, where `block_changed` hears the write before them, and on a
//! client for each block the server sends and each write of a piston's replay.
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::level::LevelAccess;
use mod_api::{
    BlockChangedEvent, BlockPart, BlockState, Hooks, LevelHost, LevelSide, OnBlockStateChangeEvent,
    Registrar,
};
use std::sync::{Arc, Mutex};

/// One notice: the side, the position, the state asked, old, new, and whether the fence west
/// of the position showed an east side then.
type Heard = (bool, [i32; 3], BlockState, BlockState, BlockState, bool);

/// Whether the block west of `pos` is a fence connected east.
fn fence_east(level: &dyn LevelHost, pos: [i32; 3]) -> bool {
    level
        .block_state([pos[0] - 1, pos[1], pos[2]])
        .is_some_and(|fence| level.state_with_property(fence, "east", "true") == Some(fence))
}

#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<Heard>>>);
#[mod_api::horizon_part]
impl BlockPart for Listener {
    fn on_block_state_change(&self, level: LevelSide<'_>, event: &mut OnBlockStateChangeEvent) {
        assert!(
            !event.predicted,
            "server writes and piston replays are not predictions"
        );
        let (client, level) = match level {
            LevelSide::Server(level) => (false, level),
            LevelSide::Client(level) => (true, level),
        };
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        // The write holds `new`.
        assert_eq!(level.block_state(pos), Some(event.new));
        self.0.lock().unwrap().push((
            client,
            pos,
            event.state,
            event.old,
            event.new,
            fence_east(level, pos),
        ));
    }
}

struct Mod {
    listener: Listener,
    /// What `block_changed` saw of the fence beside each change.
    changed: Arc<Mutex<Vec<bool>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in ["minecraft:stone", "minecraft:gold_block"] {
            let key = r.block_key(name);
            r.block_part(key, self.listener.clone());
        }
    }
    fn block_changed(&self, level: &dyn LevelHost, event: &mut BlockChangedEvent) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        self.changed.lock().unwrap().push(fence_east(level, pos));
    }
}

fn mods(listener: &Listener, changed: &Arc<Mutex<Vec<bool>>>) -> mod_api::Mods {
    vec![mod_api::ModEntry::new(
        "state_change",
        Box::new(Mod {
            listener: listener.clone(),
            changed: changed.clone(),
        }),
    )]
    .into()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_part_hears_a_state_change_after_the_neighbours_shapes_settled() {
    let listener = Listener::default();
    let changed = Arc::new(Mutex::new(Vec::new()));
    let mods = mods(&listener, &changed);
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let [fence, stone, dirt] = ["minecraft:oak_fence", "minecraft:stone", "minecraft:dirt"]
                .map(|name| registry.default_state(name).unwrap());
            game.world.set(1, 64, 0, fence).unwrap();
            game.with_level(|b, level, ticks| {
                b.set_block(level, ticks, BlockPos::new(2, 64, 0), stone, 3);
            });
            // `block_changed` saw the stone beside an unconnected fence (then the fence's own
            // change); the part hears the stone beside the connected fence.
            assert_eq!(changed.lock().unwrap().first(), Some(&false));
            let heard = listener.0.lock().unwrap().clone();
            assert_eq!(heard.len(), 1);
            let (client, pos, asked, _, new, east) = heard[0];
            assert_eq!((client, pos, east), (false, [2, 64, 0], true));
            assert_eq!((asked, new), (api(stone), api(stone)));
            // Stone to dirt: stone's part hears it, dirt has none.
            game.with_level(|b, level, ticks| {
                b.set_block(level, ticks, BlockPos::new(2, 64, 0), dirt, 3);
            });
            let heard = listener.0.lock().unwrap().clone();
            assert_eq!(heard.len(), 2);
            assert_eq!((heard[1].2, heard[1].4), (api(stone), api(dirt)));
        })
    });
}

/// The API state of the engine state `state`.
fn api(state: horizon_world::registry::BlockStateId) -> BlockState {
    use crate::game::api_values::TryIntoApi;
    state.try_into_api().unwrap()
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_client_hears_each_block_the_server_sends_and_each_write_of_a_piston_replay() {
    use super::super::execution_role::ExecutionRole;
    use horizon_client::level_loading::Reason;
    use horizon_protocol::{
        connection::Pipe,
        packets::play::{self, Clientbound as C, SpawnInfo},
    };
    let listener = Listener::default();
    let changed = Arc::new(Mutex::new(Vec::new()));
    let mods = mods(&listener, &changed);
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "StateChangeTest").unwrap();
            let spawn = SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 0,
                game_type: 1,
                previous_game_type: None,
                debug: false,
                flat: true,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.enter_remote_dimension(&spawn, false, Reason::Other);
            game.remote.as_mut().unwrap().logged_in = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            // One BlockUpdate, one notice, on the client side.
            game.handle_packet(C::BlockUpdate(play::BlockUpdate {
                pos: [3, 64, 5],
                state: stone.0 as i32,
            }))
            .unwrap();
            let heard = listener.0.lock().unwrap().clone();
            assert_eq!(heard.len(), 1);
            assert_eq!((heard[0].0, heard[0].1), (true, [3, 64, 5]));
            // The same state again does not change the block.
            game.handle_packet(C::BlockUpdate(play::BlockUpdate {
                pos: [3, 64, 5],
                state: stone.0 as i32,
            }))
            .unwrap();
            assert_eq!(listener.0.lock().unwrap().len(), 1);
            // A piston replay moving a gold block: each of the replay's writes the gold block's
            // part takes part in is heard on the client.
            listener.0.lock().unwrap().clear();
            let piston = registry
                .resolve(&horizon_world::chunk::BlockState {
                    name: "minecraft:piston".into(),
                    properties: [("facing".to_owned(), "east".to_owned())].into(),
                })
                .unwrap();
            let gold = registry.default_state("minecraft:gold_block").unwrap();
            game.with_level(|b, level, ticks| {
                level.set_raw(BlockPos::new(2, 64, 9), piston);
                level.set_raw(BlockPos::new(3, 64, 9), gold);
                b.client_block_event(
                    level,
                    ticks,
                    BlockPos::new(2, 64, 9),
                    0,
                    Direction::East as i32,
                );
            });
            let heard = listener.0.lock().unwrap().clone();
            // The gold block's cell becomes the arm's moving block: the gold block's part hears
            // that one write, as a client.
            assert_eq!(heard.len(), 1, "{heard:?}");
            assert_eq!((heard[0].0, heard[0].1), (true, [3, 64, 9]));
            assert_eq!(heard[0].3, api(gold));
        })
    });
}
