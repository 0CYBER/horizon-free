#![cfg(test)]
//! `on_destroyed_by_push_reaction` heard by a block Part: a piston's move destroying a torch
//! tells the torch's part before its cell becomes air, and the move overwrites what the part
//! writes into the move's cells but keeps what it writes elsewhere.
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::{
    geometry::Direction,
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    level::LevelAccess,
    registry::{BlockRegistry, BlockStateId},
};
use mod_api::{
    BlockPart, BlockUpdateFlags, Flow, Hooks, LevelHost, LevelWriteHost,
    OnDestroyedByPushReactionEvent, PistonPreEvent, Registrar,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

/// A torch's part: records what it hears, then writes nothing (0), stone into its own cell
/// (1), dirt into the cell west of it (2) or stone into the cell above it (3).
#[derive(Clone, Default)]
struct Torch {
    write: Arc<AtomicU8>,
    heard: Arc<Mutex<Vec<(BlockPos, Direction)>>>,
}
#[mod_api::horizon_part]
impl BlockPart for Torch {
    fn on_destroyed_by_push_reaction(
        &self,
        level: &mut dyn LevelWriteHost,
        event: &mut OnDestroyedByPushReactionEvent,
    ) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        // The torch is still in its cell.
        assert_eq!(level.block_state(pos), Some(event.state));
        // The cell holds no fluid.
        assert!(event.fluid.is_empty());
        assert_eq!(level.state_block(event.state), Some("minecraft:torch"));
        self.heard
            .lock()
            .unwrap()
            .push((event.pos, event.direction));
        let (at, name) = match self.write.load(Ordering::Relaxed) {
            1 => (pos, "minecraft:stone"),
            2 => ([pos[0] - 1, pos[1], pos[2]], "minecraft:dirt"),
            3 => ([pos[0], pos[1] + 1, pos[2]], "minecraft:stone"),
            _ => return,
        };
        let state = level.block_state_id(name).unwrap();
        assert!(level.set_block(at, state, BlockUpdateFlags::CLIENTS));
    }
}

struct TorchMod {
    torch: Torch,
    cancel: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for TorchMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let torch = r.block_key("minecraft:torch");
        r.block_part(torch, self.torch.clone());
    }
    fn piston_pre(&self, _: &dyn LevelHost, _: &mut PistonPreEvent<'_>) -> Flow {
        if self.cancel.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn state(registry: &BlockRegistry, name: &str, props: &[(&str, &str)]) -> BlockStateId {
    registry
        .resolve(&horizon_world::chunk::BlockState {
            name: name.into(),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        })
        .unwrap()
}

#[test]
#[ignore = "daily"]
fn a_block_part_hears_a_piston_destroy_it_before_the_move_overwrites_its_cells() {
    let torch = Torch::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "torch",
        Box::new(TorchMod {
            torch: torch.clone(),
            cancel: cancel.clone(),
        }),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let registry = game.pack.assets.registry;
            let piston = state(registry, "minecraft:piston", &[("facing", "east")]);
            let [power, torch_state, stone, dirt] = [
                "minecraft:redstone_block",
                "minecraft:torch",
                "minecraft:stone",
                "minecraft:dirt",
            ]
            .map(|name| registry.default_state(name).unwrap());
            let is_moving = |game: &super::Game<'_>, x, z| {
                registry.state(game.world.block(x, 64, z)).unwrap().name
                    == "minecraft:moving_piston"
            };
            let moved = |game: &super::Game<'_>, x, z| {
                let at = BlockPos::new(x, 64, z);
                game.block_ticks
                    .moving_pistons
                    .iter()
                    .find(|(p, _)| *p == at)
                    .map(|(_, m)| m.moved_state)
            };
            // A powered piston facing east at (2, 64, z), `ahead` east of it, then a torch.
            let extend = |game: &mut super::Game<'_>, z, ahead: &[BlockStateId], client| {
                game.with_level(|b, level, ticks| {
                    level.set_raw(BlockPos::new(1, 64, z), power);
                    level.set_raw(BlockPos::new(2, 64, z), piston);
                    for (i, state) in ahead.iter().chain([&torch_state]).enumerate() {
                        level.set_raw(BlockPos::new(3 + i as i32, 64, z), *state);
                    }
                    if client {
                        // A client replays the server's move with no hooks.
                        level.client_side = true;
                        level.native_authoritative = false;
                    }
                    b.client_block_event(
                        level,
                        ticks,
                        BlockPos::new(2, 64, z),
                        0,
                        Direction::East as i32,
                    );
                });
            };
            let torches = |game: &super::Game<'_>| {
                game.dropped
                    .iter()
                    .filter(|d| &*d.stack.item == "minecraft:torch")
                    .count()
            };
            // Heard once, with the torch's cell and the push direction; the torch has dropped
            // and its cell then holds the arm's moving block.
            extend(game, 2, &[], false);
            assert_eq!(
                *torch.heard.lock().unwrap(),
                [(BlockPos::new(3, 64, 2), Direction::East)]
            );
            assert_eq!(torches(game), 1);
            assert!(is_moving(game, 3, 2));
            // Stone written into its own cell is overwritten by the move.
            torch.write.store(1, Ordering::Relaxed);
            extend(game, 5, &[], false);
            assert!(is_moving(game, 3, 5));
            // Dirt written into the cell of the stone pushed ahead of it: the stone moves.
            torch.write.store(2, Ordering::Relaxed);
            extend(game, 8, &[stone], false);
            assert_eq!(
                torch.heard.lock().unwrap().last(),
                Some(&(BlockPos::new(4, 64, 8), Direction::East))
            );
            assert_eq!(moved(game, 4, 8), Some(stone));
            assert_ne!(game.world.block(3, 64, 8), dirt);
            // Stone written above the move stays.
            torch.write.store(3, Ordering::Relaxed);
            extend(game, 11, &[], false);
            assert_eq!(game.world.block(3, 65, 11), stone);
            assert_eq!(torch.heard.lock().unwrap().len(), 4);
            // A move piston_pre cancels, and a client's replay, are not heard.
            torch.write.store(0, Ordering::Relaxed);
            cancel.store(true, Ordering::Relaxed);
            extend(game, 14, &[], false);
            assert_eq!(game.world.block(3, 64, 14), torch_state);
            cancel.store(false, Ordering::Relaxed);
            extend(game, 14, &[], true);
            assert!(is_moving(game, 3, 14));
            assert_eq!(torch.heard.lock().unwrap().len(), 4);
        })
    });
}
