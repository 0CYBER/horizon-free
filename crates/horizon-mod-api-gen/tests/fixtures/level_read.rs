// Inserted into hook_index.rs's boundary module. Production LevelHost read methods
// are extracted by syn and compiled unchanged, except their trait import's module path.
use crate::BlockPos;
struct World;
impl World {
    fn block(&self, x: i32, y: i32, _: i32) -> crate::registry::BlockStateId {
        // The engine contract: absent height/chunk reads are vanilla air (ID 0).
        crate::registry::BlockStateId(if y != 64 || x > 15 {
            0
        } else if x == 2 {
            999
        } else {
            1
        })
    }
}
struct Game {
    world: World,
}
struct GameLevel {
    world: World,
}
struct RawLevel(World);
impl RawLevel {
    fn block(&self, [x, y, z]: [i32; 3]) -> crate::registry::BlockStateId {
        self.0.block(x, y, z)
    }
}
struct HitLevel(RawLevel);
struct ReadLevel {
    game: Game,
}
struct LevelTicks<'a> {
    level: &'a GameLevel,
}
impl crate::level::LevelAccess for GameLevel {
    fn block_state(&self, pos: crate::BlockPos) -> crate::registry::BlockStateId {
        self.world.block(pos.0[0], pos.0[1], pos.0[2])
    }
}
/* READ_IMPLS */
pub fn run() {
    use crate::{registry::*, *};
    let game = GameLevel { world: World };
    let hit = HitLevel(RawLevel(World));
    let read = ReadLevel {
        game: Game { world: World },
    };
    let ticks = LevelTicks { level: &game };
    let levels: [&dyn crate::LevelHost; 4] = [&game, &hit, &read, &ticks];
    let positions = [[0, 64, 0], [16, 64, 0], [0, -999, 0], [0, 999, 0]];
    block_state::clear();
    for level in levels {
        for pos in positions {
            assert_eq!(level.block_state(pos), None);
        }
    }
    // Neither API identity is slot zero, and air is not ordinal zero.
    let air = BlockState(BlockKey(19), 3);
    let stone = BlockState(BlockKey(7), 1);
    block_state::bind(&BlockRegistry(vec![Ok(air), Ok(stone)])).unwrap();
    for level in levels {
        assert_eq!(level.block_state([0, 64, 0]), Some(stone));
        for pos in positions.into_iter().skip(1) {
            assert_eq!(level.block_state(pos), Some(air));
        }
        assert_eq!(
            level.block_state([2, 64, 0]),
            None,
            "unknown engine state is refused, not guessed"
        );
    }
    block_state::clear();
    for level in levels {
        assert_eq!(level.block_state([0, 64, 0]), None);
    }
    println!(
        "4 actual LevelHost method bodies: unavailable/cleared index -> None; engine air -> Some(nonzero air); unknown engine state -> None"
    );
}
