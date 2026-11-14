//! Positional block Parts run against the owning readonly level, not during state resolution.
use super::Game;
use super::native_entity_refs::tests::with_game;
use horizon_client::mobs::{
    Mob,
    path::{Evaluator, Finder},
};
use horizon_core::geometry::Vec3;
use mod_api::{
    AdjacentBlockPathTypeEvent, BlockPart, BlockPathTypeEvent, Hooks, Host, IsPathfindableEvent,
    PathComputationType, PathType, Registrar,
};
use std::sync::{Arc, Mutex};

struct Thorny;
#[mod_api::horizon_part]
impl BlockPart for Thorny {
    fn block_path_type(&self, host: &dyn Host, event: &mut BlockPathTypeEvent) -> Option<PathType> {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let answer = match pos {
            [4, 64, _] if host.block_state_id("minecraft:smooth_stone") == Some(event.state) => {
                Some(PathType::Damaging)
            }
            [15, 65, 1] if host.block_state_id("minecraft:air") == Some(event.state) => {
                Some(PathType::Blocked)
            }
            _ => None,
        };
        if answer.is_some() {
            // These override positions are in the loaded fixture chunk. Finder
            // also asks void-air positions outside it, where the Host read is None.
            let level = host.level(host.dimension()).unwrap();
            assert_eq!(level.block_state(host, pos), Some(event.state));
        }
        answer
    }
}
struct ThornyMod;
#[mod_api::__subscriptions]
impl Hooks for ThornyMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in [
            "minecraft:smooth_stone",
            "minecraft:air",
            "minecraft:oak_fence",
            "minecraft:oak_door",
        ] {
            let key = r.block_key(name);
            r.block_part(key, Thorny);
        }
    }
}

type AdjacentCalls = Arc<Mutex<Vec<([i32; 3], PathType)>>>;

struct Beside(AdjacentCalls);
#[mod_api::horizon_part]
impl BlockPart for Beside {
    fn is_pathfindable(&self, host: &dyn Host, event: &mut IsPathfindableEvent) {
        if event.kind == PathComputationType::Land
            && host.block_state_id("minecraft:glass") == Some(event.state)
        {
            event.pathfindable = true;
        }
    }
    fn block_path_type(&self, host: &dyn Host, event: &mut BlockPathTypeEvent) -> Option<PathType> {
        (host.block_state_id("minecraft:seagrass") == Some(event.state))
            .then_some(PathType::Blocked)
    }
    fn adjacent_block_path_type(
        &self,
        host: &dyn Host,
        event: &mut AdjacentBlockPathTypeEvent,
    ) -> Option<PathType> {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let level = host.level(host.dimension()).unwrap();
        assert_eq!(level.block_state(host, pos), Some(event.state));
        self.0.lock().unwrap().push((pos, event.path_type));
        if pos == [0, 64, 0] {
            Some(PathType::Walkable)
        } else if host.block_state_id("minecraft:glass") == Some(event.state) {
            Some(PathType::FireInNeighbor)
        } else {
            None
        }
    }
}
struct BesideMod(AdjacentCalls);
#[mod_api::__subscriptions]
impl Hooks for BesideMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for name in [
            "minecraft:stone",
            "minecraft:magma_block",
            "minecraft:glass",
            "minecraft:seagrass",
        ] {
            let key = r.block_key(name);
            r.block_part(key, Beside(Arc::clone(&self.0)));
        }
    }
}

#[test]
#[ignore = "daily"]
fn cache_follows_update_flags_and_direct_block_entity_notifications() {
    with_game(|game| {
        use horizon_core::position::BlockPos;
        use std::cell::Cell;
        let pos = [5, 64, 5];
        let packed_pos = BlockPos::new(pos[0], pos[1], pos[2]);
        let air = game.res.facts().air;
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        game.world.block_log = None;
        game.world.enable_path_type_cache();
        let calls = Cell::new(0);
        let read = |game: &Game<'_>| {
            game.world.cached_path_type(packed_pos, || {
                calls.set(calls.get() + 1);
                if game.world.block(pos[0], pos[1], pos[2]) == air {
                    PathType::Open
                } else {
                    PathType::Blocked
                }
            })
        };
        assert_eq!(read(game), PathType::Open);
        game.level_set_with_flags(pos, stone, 0).unwrap();
        assert_eq!(read(game), PathType::Open);
        assert_eq!(
            calls.get(),
            1,
            "a non-notifying write must leave the cached type stale"
        );
        game.level_set_with_flags(pos, stone, 2).unwrap();
        read(game);
        assert_eq!(
            calls.get(),
            1,
            "an unchanged state must not reach sendBlockUpdated"
        );
        game.level_set_with_flags(pos, air, 2).unwrap();
        read(game);
        assert_eq!(
            calls.get(),
            2,
            "UPDATE_CLIENTS clears despite the absent network log"
        );
        game.mark_block_entity_updated((pos[0], pos[1], pos[2]));
        read(game);
        assert_eq!(
            calls.get(),
            3,
            "direct block-entity notification clears unchanged state"
        );
        game.notify_block_clients((pos[0], pos[1], pos[2]));
        read(game);
        assert_eq!(calls.get(), 4);
        game.world.block_log = Some(vec![(pos[0], pos[1], pos[2])]);
        game.notify_block_clients((pos[0], pos[1], pos[2]));
        read(game);
        assert_eq!(
            calls.get(),
            5,
            "log deduplication must not suppress invalidation"
        );
    });
    super::native_entity_refs::tests::with_client_game(|game| {
        let _world = game.mob_world_base(&[]);
        let calls = std::cell::Cell::new(0);
        for _ in 0..2 {
            game.world
                .cached_path_type(horizon_core::position::BlockPos::new(0, 64, 0), || {
                    calls.set(calls.get() + 1);
                    PathType::Open
                });
        }
        assert_eq!(
            calls.get(),
            2,
            "a client mob view must not initialize a server cache"
        );
    });
}

fn with_parts(id: &'static str, hooks: impl Hooks + 'static, test: impl FnOnce(&mut Game<'_>)) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(id, Box::new(hooks))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.resolve_block_paths();
            test(game);
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_block_part_gives_its_block_a_path_type() {
    with_parts("thorny", ThornyMod, |game| {
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let smooth = registry.default_state("minecraft:smooth_stone").unwrap();
        for x in 0..9 {
            for z in 0..3 {
                game.world
                    .set(x, 64, z, if x == 4 { smooth } else { stone })
                    .unwrap();
            }
        }
        game.world.set(2, 64, 1, smooth).unwrap();
        game.world
            .set(
                14,
                65,
                1,
                registry.default_state("minecraft:oak_fence").unwrap(),
            )
            .unwrap();
        game.world
            .set(
                13,
                65,
                1,
                registry.default_state("minecraft:oak_door").unwrap(),
            )
            .unwrap();
        let world = game.mob_world_base(&[]);
        // The same state answers differently by position; None keeps vanilla's special cases.
        assert_eq!(world.path_type_from_state(4, 64, 1), PathType::Damaging);
        assert_eq!(world.path_type_from_state(2, 64, 1), PathType::Blocked);
        assert_eq!(world.path_type_from_state(14, 65, 1), PathType::Fence);
        assert_eq!(
            world.path_type_from_state(13, 65, 1),
            PathType::DoorWoodClosed
        );
        // The override runs even before the air early return.
        assert_eq!(world.path_type_from_state(15, 65, 1), PathType::Blocked);
        assert_eq!(world.path_type_static(4, 65, 1), PathType::Damaging);
        assert_eq!(world.path_type_static(1, 65, 1), PathType::Walkable);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut zombie = Mob::new(50, kind, Vec3::new(0.5, 65.0, 1.5), 0.0).path_mob();
        zombie.on_ground = true;
        let path = Finder::new(&world, &zombie, Evaluator::Walk).find(&[[8, 65, 1]], 16.0, 0, 256);
        assert!(path.is_none_or(
            |path| !path.reached && path.nodes.iter().map(|node| node.x).max() < Some(4)
        ));
    });
}

#[test]
#[ignore = "daily"]
fn a_block_part_gives_the_nodes_beside_its_block_a_path_type() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    with_parts("beside", BesideMod(Arc::clone(&calls)), |game| {
        let registry = game.pack.assets.registry;
        let state = |name: &str| registry.default_state(name).unwrap();
        for x in 0..6 {
            for z in 0..6 {
                game.world.set(x, 64, z, state("minecraft:stone")).unwrap();
            }
        }
        game.world
            .set(1, 64, 0, state("minecraft:magma_block"))
            .unwrap();
        game.world.set(1, 65, 4, state("minecraft:glass")).unwrap();
        game.world.set(8, 64, 8, state("minecraft:stone")).unwrap();
        game.world
            .set(7, 64, 7, state("minecraft:seagrass"))
            .unwrap();
        let world = game.mob_world_base(&[]);
        // First neighbour is (0,64,0), whose own type is Blocked, not the node's Walkable.
        // Some(Walkable) ends the scan even though it equals the node's incoming type.
        assert_eq!(
            world.check_neighbours(1, 65, 1, PathType::Walkable),
            PathType::Walkable
        );
        assert_eq!(*calls.lock().unwrap(), [([0, 64, 0], PathType::Blocked)]);
        // None from magma does not suppress its vanilla fire contribution.
        assert_eq!(world.path_type_static(0, 65, 0), PathType::FireInNeighbor);
        assert_eq!(world.path_type_static(0, 65, 4), PathType::FireInNeighbor);
        assert_eq!(world.path_type_static(4, 65, 4), PathType::Walkable);
        // Seagrass is overridden to Blocked, but a declined adjacent answer still
        // asks its water fluid before trying the vanilla type mappings.
        assert_eq!(world.path_type_from_state(7, 64, 7), PathType::Blocked);
        assert_eq!(world.path_type_static(8, 65, 8), PathType::WaterBorder);
        let observed = calls.lock().unwrap();
        // Land made glass Open, the positional path query declined, and adjacency
        // saw that final baseline. Seagrass instead reached adjacency as the
        // positional override's Blocked, not its exported Water or the node type.
        assert!(observed.contains(&([1, 65, 4], PathType::Open)));
        assert!(observed.contains(&([7, 64, 7], PathType::Blocked)));
    });
}
