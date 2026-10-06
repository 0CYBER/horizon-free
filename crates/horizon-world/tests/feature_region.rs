use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    heightmap::{HeightmapState, HeightmapType},
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        feature::FeatureWorld,
        feature_region::{RegionError, TerrainFeatureAccess, TerrainRegion},
        placement::PlacementContext,
        terrain::{PreLightChunkStage, TerrainBlockStates, TerrainChunk},
    },
};
#[derive(Clone, Copy)]
struct State(u32);
impl HeightmapState for State {
    fn is_air(self) -> bool {
        self.0 < 2
    }
    fn is_air_block(self) -> bool {
        self.0 == 0
    }
    fn has_fluid(self) -> bool {
        self.0 == 3
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.0 >= 2 && self.0 != 3
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.blocks_motion_in_heightmap()
    }
}
struct Properties;
impl TerrainBlockStates for Properties {
    type State = State;
    fn heightmap_state(&self, id: BlockStateId) -> State {
        State(id.0)
    }
}
fn registry() -> BlockRegistry {
    let names = [
        "minecraft:air",
        "minecraft:void_air",
        "minecraft:stone",
        "minecraft:water",
        "minecraft:grass_block",
        "minecraft:mycelium",
        "minecraft:dirt",
    ];
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                names
                    .iter()
                    .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::Compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(id, name)| ((*name).into(), Tag::Int(id as i32)))
                    .collect(),
            ),
        ),
    ]))
    .unwrap()
}
#[test]
fn rejects_incompatible_regions_and_routes_negative_chunk_coordinates() {
    let other_registry = registry();
    let registry = registry();
    let context = WorldGenerationContext::new(0, 16, 8);
    let chunk =
        |x, min, reg| TerrainChunk::new(ChunkPos::new(x, 0), min, 16, reg, Properties).unwrap();
    assert!(TerrainRegion::<Properties>::new(vec![], context).is_err());
    assert!(
        TerrainRegion::new(
            vec![chunk(0, 0, &registry), chunk(0, 0, &registry)],
            context
        )
        .is_err()
    );
    assert!(
        TerrainRegion::new(
            vec![chunk(0, 0, &registry), chunk(1, 16, &registry)],
            context
        )
        .is_err()
    );
    assert!(
        TerrainRegion::new(
            vec![chunk(0, 0, &registry), chunk(1, 0, &other_registry)],
            context
        )
        .is_err()
    );
    let mut region = TerrainRegion::new(
        vec![chunk(-1, 0, &registry), chunk(0, 0, &registry)],
        context,
    )
    .unwrap();
    region
        .set_block_worldgen([-1, 7, 0], BlockStateId(2))
        .unwrap();
    assert_eq!(region.block([-1, 7, 0]).unwrap(), BlockStateId(2));
    assert_eq!(region.block([15, 7, 0]).unwrap(), BlockStateId(0));
    assert!(matches!(
        region.set_block_worldgen([16, 7, 0], BlockStateId(2)),
        Err(RegionError::MissingChunk([1, 0]))
    ));
    assert_eq!(region.block([0, 7, 0]).unwrap(), BlockStateId(0));
}
#[test]
fn write_callback_controls_mutation_and_receives_all_flags() {
    let registry = registry();
    let mut chunk = TerrainChunk::new(ChunkPos::new(0, 0), 0, 16, &registry, Properties).unwrap();
    chunk.set_generation_stage(PreLightChunkStage::Terrain);
    let mut region =
        TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-16, 32, 8)).unwrap();
    assert_eq!(region.generation(), WorldGenerationContext::new(0, 16, 8));
    assert_eq!(region.height(HeightmapType::WorldSurface, 0, 0).unwrap(), 0);
    let mut flags = Vec::new();
    {
        let mut world = TerrainFeatureAccess::new(
            &mut region,
            |region: &mut TerrainRegion<'_, Properties>, p, state, flag| {
                flags.push(flag);
                if flag == 2 {
                    return Ok(false);
                }
                region.set_block_worldgen(p, state)?;
                Ok(true)
            },
        );
        assert!(!world.set_block([0, 7, 0], BlockStateId(2), 2).unwrap());
        assert_eq!(world.height(HeightmapType::WorldSurface, 0, 0).unwrap(), 0);
        assert!(world.set_block([0, 7, 0], BlockStateId(2), 3).unwrap());
        assert_eq!(world.height(HeightmapType::WorldSurface, 0, 0).unwrap(), 8);
    }
    assert_eq!(flags, [2, 3]);
}

#[test]
fn postprocessing_routes_to_resident_chunks_and_preserves_partial_marks() {
    let registry = registry();
    let chunk = TerrainChunk::new(ChunkPos::new(-1, 1), -16, 32, &registry, Properties).unwrap();
    let mut region =
        TerrainRegion::new(vec![chunk], WorldGenerationContext::new(-16, 32, 8)).unwrap();
    {
        let mut world = TerrainFeatureAccess::new(
            &mut region,
            |_: &mut TerrainRegion<'_, Properties>, _: [i32; 3], _: BlockStateId, _: i32| {
                Ok::<_, RegionError>(false)
            },
        );
        world.mark_postprocessing([-1, -1, 16]).unwrap();
        world.mark_postprocessing([-1, -1, 16]).unwrap();
        world.mark_postprocessing([-16, 0, 31]).unwrap();
        world.mark_postprocessing([-1, -17, 16]).unwrap();
        world.mark_postprocessing([-1, 16, 16]).unwrap();
        assert!(world.mark_postprocessing([0, 0, 16]).is_err());
    }
    assert_eq!(
        region.chunk(-1, 1).unwrap().postprocessing(),
        &[vec![255, 255], vec![3840]]
    );
}
