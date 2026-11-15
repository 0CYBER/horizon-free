//! Load-bound worker metadata. Each numeric registry retains its own immutable snapshot.
use super::api_values::block_state::{self, Index};
use horizon_mod_runtime::block_definitions::Blocks;
use horizon_world::{registry::BlockRegistry, worldgen::placement::PlacementQueryError};
use mod_api::{BlockKey, BlockMetadata, BlockState};
use std::sync::Arc;

/// No thread-local game, Host, registry rebuild or name resolution is needed by workers.
pub(in crate::game) struct WorkerResources {
    pub index: Arc<Index>,
    pub metadata: &'static Blocks,
    air: BlockState,
    air_blocks: [Option<BlockKey>; 3],
}
impl WorkerResources {
    pub fn get(registry: &BlockRegistry) -> Result<&Self, PlacementQueryError> {
        registry
            .mod_worker_resource
            .get()
            .and_then(|resource| resource.downcast_ref())
            .ok_or(PlacementQueryError::Unavailable)
    }
    pub fn state(&self, block: BlockKey) -> BlockState {
        BlockState::of(self.metadata, block, &[]).unwrap_or(self.air)
    }
    pub fn state_with(
        &self,
        registry: &BlockRegistry,
        block: BlockKey,
        properties: &[(&str, &str)],
    ) -> BlockState {
        // Keep the engine's partial-property/default-state rules, including duplicate properties.
        // This is a cold construction query, not the indexed placed-block conversion path.
        let name = self.metadata.name(block).unwrap_or("minecraft:air");
        let state =
            horizon_world::worldgen::structure::legacy::block_state(registry, name, properties);
        self.index.to_api(state).unwrap_or(self.air)
    }
    pub fn is_air(&self, state: BlockState) -> bool {
        self.index.to_engine(state).is_ok() && self.air_blocks.contains(&Some(state.block()))
    }
}

/// Freeze and return the very same index that owning threads install with PreparedParts.
/// A new or reordered registry has a separate once-initialized resource, never a global index.
pub(in crate::game) fn prepare(registry: &BlockRegistry) -> Result<Arc<Index>, String> {
    retain_or_prepare(registry, || block_state::prepare(registry))
}

/// Attach the already checked index from the owning-thread resolution boundary.
pub(in crate::game) fn retain(
    registry: &BlockRegistry,
    index: Arc<Index>,
) -> Result<Arc<Index>, String> {
    retain_or_prepare(registry, || Ok(index))
}

fn retain_or_prepare(
    registry: &BlockRegistry,
    make_index: impl FnOnce() -> Result<Arc<Index>, String>,
) -> Result<Arc<Index>, String> {
    let metadata = mod_api::__private::block_definitions()
        .ok_or_else(|| "worker block metadata is unavailable".to_owned())?;
    if registry.mod_worker_resource.get().is_none() {
        let index = make_index()?;
        let air = registry
            .default_state("minecraft:air")
            .ok_or_else(|| "worker block registry has no minecraft:air".to_owned())?;
        let air = index
            .to_api(air)
            .map_err(|error| format!("worker air state: {error:?}"))?;
        let air_blocks =
            ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"].map(|name| {
                registry
                    .default_state(name)
                    .and_then(|state| index.to_api(state).ok())
                    .map(BlockState::block)
            });
        let _ = registry.mod_worker_resource.set(Arc::new(WorkerResources {
            index,
            metadata,
            air,
            air_blocks,
        }));
    }
    let resource = WorkerResources::get(registry)
        .map_err(|_| "worker registry contains an incompatible resource".to_owned())?;
    // Check the winning resource, not only a pre-set snapshot: another owner may win initialization.
    if !std::ptr::eq(resource.metadata, metadata) {
        return Err("worker block registry belongs to a different metadata snapshot".to_owned());
    }
    super::native_block_entity_transform::prepare(registry);
    Ok(resource.index.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::registry::BlockStateId;

    #[test]
    fn retained_worker_snapshots_keep_distinct_reordered_indices_without_tls() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"north"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"facing":"east"}}]]}"#;
        let first = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        crate::game::api_values::resolve_block_definitions(&first).unwrap();
        let first_index = WorkerResources::get(&first).unwrap().index.clone();
        let north = first_index.to_api(BlockStateId(1)).unwrap();
        let reordered = facts
            .replace("north", "TEMP")
            .replace("east", "north")
            .replace("TEMP", "east");
        let second = BlockRegistry::from_facts_json(reordered.as_bytes()).unwrap();
        crate::game::api_values::resolve_block_definitions(&second).unwrap();
        let second_index = WorkerResources::get(&second).unwrap().index.clone();
        assert!(!Arc::ptr_eq(&first_index, &second_index));
        assert!(Arc::ptr_eq(&prepare(&first).unwrap(), &first_index));
        assert!(Arc::ptr_eq(&block_state::current().unwrap(), &second_index));
        std::thread::spawn(move || {
            assert!(block_state::current().is_err());
            let a = WorkerResources::get(&first).unwrap();
            let b = WorkerResources::get(&second).unwrap();
            assert_eq!(a.index.to_engine(north), Ok(BlockStateId(1)));
            assert_eq!(b.index.to_engine(north), Ok(BlockStateId(2)));
            assert_eq!(a.index.to_api(BlockStateId(1)), Ok(north));
            assert_eq!(b.index.to_api(BlockStateId(2)), Ok(north));
            assert_eq!(north.property(a.metadata, "facing"), Ok(Some("north")));
            assert_eq!(a.metadata.name(north.block()), Ok("minecraft:stairs"));
        })
        .join()
        .unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn fresh_game_lifetimes_keep_worker_and_generator_snapshots_separate() {
        use crate::game::native_entity_refs::tests::with_game;
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap())
            .join("worldgen");
        let mut first = None;
        with_game(|game| {
            let registry = game.pack.assets.registry;
            let index = prepare(registry).unwrap();
            let metadata = WorkerResources::get(registry).unwrap().metadata;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let state = index.to_api(stone).unwrap();
            let data = horizon_client::vanillagen::shared_data(&root, registry).unwrap();
            assert!(std::ptr::eq(data.registry(), registry));
            first = Some((registry, index, metadata, stone, state, data));
        });
        let (old_registry, old_index, old_metadata, stone, state, old_data) = first.unwrap();
        with_game(|game| {
            let registry = game.pack.assets.registry;
            assert!(!std::ptr::eq(old_registry, registry));
            assert_eq!(
                prepare(old_registry).err().unwrap(),
                "worker block registry belongs to a different metadata snapshot"
            );
            let index = prepare(registry).unwrap();
            assert!(!Arc::ptr_eq(&old_index, &index));
            let metadata = WorkerResources::get(registry).unwrap().metadata;
            assert!(!std::ptr::eq(old_metadata, metadata));
            // Repeated queries within a lifetime share the retained index and generator data.
            assert!(Arc::ptr_eq(&index, &prepare(registry).unwrap()));
            let data = horizon_client::vanillagen::shared_data(&root, registry).unwrap();
            assert!(!Arc::ptr_eq(&old_data, &data));
            assert!(std::ptr::eq(data.registry(), registry));
            assert!(Arc::ptr_eq(
                &data,
                &horizon_client::vanillagen::shared_data(&root, registry).unwrap()
            ));
            assert!(std::ptr::eq(old_data.registry(), old_registry));
            // A worker with no owning-thread TLS still reads the original immutable snapshot.
            std::thread::spawn(move || {
                let worker = WorkerResources::get(old_registry).unwrap();
                assert!(Arc::ptr_eq(&worker.index, &old_index));
                assert!(std::ptr::eq(worker.metadata, old_metadata));
                assert_eq!(worker.index.to_api(stone), Ok(state));
                assert_eq!(worker.index.to_engine(state), Ok(stone));
                assert_eq!(worker.metadata.name(state.block()), Ok("minecraft:stone"));
            })
            .join()
            .unwrap();
        });
    }

    #[test]
    fn racing_owners_cannot_install_an_index_for_different_metadata() {
        let registry = Arc::new(BlockRegistry::from_facts_json(br#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:stone"]]}"#.as_slice()).unwrap());
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let owners: Vec<_> = (0..2)
            .map(|_| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
                    let properties = std::collections::BTreeMap::new();
                    mod_api::__private::resolve_block_keys(
                        ["minecraft:air", "minecraft:stone"],
                        |_| {
                            horizon_mod_runtime::block_definitions::BlockDefinition::new(
                                &[&properties],
                                0,
                            )
                            .map_err(|error| format!("{error:?}"))
                        },
                    )
                    .unwrap();
                    let answer = retain_or_prepare(&registry, || {
                        let index = block_state::prepare(&registry)?;
                        barrier.wait();
                        Ok(index)
                    });
                    assert!(block_state::current().is_err());
                    answer
                })
            })
            .collect();
        let mut accepted = 0;
        let mut refused = 0;
        for owner in owners {
            match owner.join().unwrap() {
                Ok(index) => {
                    assert!(Arc::ptr_eq(
                        &index,
                        &WorkerResources::get(&registry).unwrap().index
                    ));
                    accepted += 1;
                }
                Err(error) => {
                    assert_eq!(
                        error,
                        "worker block registry belongs to a different metadata snapshot"
                    );
                    refused += 1;
                }
            }
        }
        assert_eq!((accepted, refused), (1, 1));
    }
}
