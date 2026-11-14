//! A state's `PathType` follows the `isPathfindable(LAND)` answer a mod part gave (#3179). The
//! expected types are vanilla 26.3's `WalkNodeEvaluator.getPathTypeFromState`: air and the
//! block-specific types come first, then `Blocked` when the state is not land-pathfindable, else
//! `Water` for a water state, else `Open`.
use super::{MobData, PartAnswers, PathComputationType, PathType};
use crate::blocks::BlockFacts;
use horizon_world::registry::{BlockRegistry, BlockStateId};

/// The prepared 26.3 resources the game tests read: a missing folder fails the test.
fn load() -> (&'static BlockRegistry, BlockFacts, MobData) {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("HORIZON_TEST_RESOURCES names the prepared 26.3 resources"),
    );
    let registry = crate::assets::Assets::load(&root).unwrap().registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let data = MobData::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root),
        &facts,
    );
    (registry, facts, data)
}

#[test]
#[ignore = "daily"]
fn a_states_path_type_follows_the_land_answer_its_parts_gave() {
    let (registry, facts, data) = load();
    let state = |name: &str| registry.default_state(name).unwrap();
    let names = [
        "minecraft:glass",
        "minecraft:short_grass",
        "minecraft:seagrass",
        "minecraft:oak_fence",
        "minecraft:oak_leaves",
        "minecraft:air",
    ];
    let exported = names.map(|name| data.path_type(state(name)));
    assert_eq!(
        exported,
        [
            PathType::Blocked,
            PathType::Open,
            PathType::Water,
            PathType::Fence,
            PathType::Leaves,
            PathType::Open,
        ]
    );
    // Glass becomes land; grass, seagrass, the fence, the leaves and air stop being land.
    let glass = state("minecraft:glass");
    let land = |id: BlockStateId, kind: PathComputationType, own: bool| {
        if kind == PathComputationType::Land {
            id == glass
        } else {
            own
        }
    };
    data.resolve_parts(
        &facts,
        &PartAnswers {
            pathfindable: Some(&land),
        },
    );
    assert_eq!(
        names.map(|name| data.path_type(state(name))),
        [
            PathType::Open,
            PathType::Blocked,
            PathType::Blocked,
            // Fences, leaves and air have types of their own, asked before `isPathfindable`.
            PathType::Fence,
            PathType::Leaves,
            PathType::Open,
        ]
    );
    // Adjacency is positional now; its fluid fallback is covered by native_path_types_tests.
    // Without the answers, every state is back to its exported type.
    data.resolve_parts(&facts, &PartAnswers::default());
    assert_eq!(names.map(|name| data.path_type(state(name))), exported);
    for (i, row) in data.states.iter().enumerate() {
        assert_eq!(data.path_type(BlockStateId(i as u32)), row.path);
    }
}
