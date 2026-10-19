//! Processors for structures built under the sea, carried as data from
//! Better Ocean Monuments' Java processors (docs/modpacks.md):
//! - `horizon:flood_air`: the template's air below sea level becomes water
//!   (its AirProcessor; the monuments' templates are mostly air);
//! - `horizon:waterlog`: below sea level, blocks that can be waterlogged
//!   are, and the template's water and lava on the edge of the chunk being
//!   generated are marked for postprocessing, so they flow across it (its
//!   WaterlogProcessor);
//! - `horizon:keep_world_block`: blocks of `blocks` take the block the world
//!   holds there (its StructureVoidProcessor, which, unlike vanilla's
//!   structure void, lets the next processors see that block).
use super::{PlaceSettings, ProcessResult, pillars::outside_chunk};
use crate::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::{DecodeError, placement::PlacementContext, structure::BlockInfo},
};
use serde_json::Value;
use std::collections::BTreeSet;

fn known(registry: &BlockRegistry, name: &str) -> Result<BlockStateId, DecodeError> {
    registry
        .default_state(name)
        .ok_or_else(|| DecodeError::Registry(format!("unknown block {name}")))
}

/// `horizon:flood_air`: plain air (not cave air) under the sea level
/// becomes water, without a block entity.
#[derive(Clone, Debug)]
pub struct FloodAir {
    air: BlockStateId,
    water: BlockStateId,
}
impl FloodAir {
    pub(super) fn decode(registry: &BlockRegistry) -> Result<Self, DecodeError> {
        Ok(Self {
            air: known(registry, "minecraft:air")?,
            water: known(registry, "minecraft:water")?,
        })
    }
    pub(super) fn process<W: PlacementContext>(
        &self,
        world: &mut W,
        info: BlockInfo,
    ) -> ProcessResult {
        if info.state != self.air || info.pos[1] >= world.sea_level()? {
            return Ok(Some(info));
        }
        Ok(Some(BlockInfo {
            state: self.water,
            nbt: None,
            ..info
        }))
    }
}

/// `horizon:waterlog`: WaterlogProcessor.
#[derive(Clone, Debug, Default)]
pub struct Waterlog;
impl Waterlog {
    pub(super) fn process<W: PlacementContext>(
        world: &mut W,
        registry: &BlockRegistry,
        info: BlockInfo,
        settings: &PlaceSettings<'_>,
    ) -> ProcessResult {
        let Some(state) = registry.state(info.state) else {
            return Ok(Some(info));
        };
        if info.pos[1] >= world.sea_level()? {
            return Ok(Some(info));
        }
        // BlockState.liquid(): a liquid block, not a waterlogged one. Java's
        // `%` keeps the sign, as Rust's does: x = -1 is no edge for it.
        let liquid = matches!(state.name.as_str(), "minecraft:water" | "minecraft:lava");
        let [x, _, z] = info.pos;
        let edge = [x % 16, z % 16].iter().any(|v| *v == 0 || *v == 15);
        if liquid && edge && !outside_chunk(settings, info.pos) {
            world.mark_postprocessing(info.pos)?;
        }
        if state.properties.contains_key("waterlogged") {
            let mut wet = state.clone();
            wet.properties.insert("waterlogged".into(), "true".into());
            if let Some(id) = registry.id(&wet) {
                return Ok(Some(BlockInfo { state: id, ..info }));
            }
        }
        Ok(Some(info))
    }
}

/// `horizon:keep_world_block`: StructureVoidProcessor. The block entity
/// stays the template's, as the mod's does.
#[derive(Clone, Debug)]
pub struct KeepWorldBlock {
    blocks: BTreeSet<String>,
}
impl KeepWorldBlock {
    pub(super) fn decode(value: &Value) -> Result<Self, DecodeError> {
        let blocks = value["blocks"]
            .as_array()
            .ok_or(DecodeError::Field("keep_world_block blocks"))?
            .iter()
            .map(|b| {
                let b = b
                    .as_str()
                    .ok_or(DecodeError::Field("keep_world_block blocks"))?;
                Ok(crate::worldgen::tags::identifier(b)?.to_string())
            })
            .collect::<Result<_, DecodeError>>()?;
        Ok(Self { blocks })
    }
    pub(super) fn process<W: PlacementContext>(
        &self,
        world: &mut W,
        registry: &BlockRegistry,
        info: BlockInfo,
        settings: &PlaceSettings<'_>,
    ) -> ProcessResult {
        let name = registry.state(info.state).map(|s| s.name.as_str());
        // A block outside the chunk is not placed, and the world there may
        // not be readable.
        if !name.is_some_and(|n| self.blocks.contains(n)) || outside_chunk(settings, info.pos) {
            return Ok(Some(info));
        }
        let state = world.block_state(info.pos)?;
        Ok(Some(BlockInfo { state, ..info }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        heightmap::HeightmapType,
        worldgen::{
            anchor::WorldGenerationContext,
            placement::PlacementQueryError,
            structure::{BoundingBox, Rotation},
        },
    };
    use serde_json::json;

    /// Sea level 63; every block of the world is `world`; marks recorded.
    struct Sea {
        world: BlockStateId,
        marked: Vec<[i32; 3]>,
    }
    impl PlacementContext for Sea {
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, 384, 63)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64)
        }
        fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
            Err(PlacementQueryError::Unavailable)
        }
        fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(63)
        }
        fn mark_postprocessing(&mut self, pos: [i32; 3]) -> Result<(), PlacementQueryError> {
            self.marked.push(pos);
            Ok(())
        }
        fn block_state(&mut self, _: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
            Ok(self.world)
        }
    }

    // 0 air, 1 cave air, 2 water, 3/4 slab dry/wet, 5 purpur slab, 6 stone.
    fn registry() -> BlockRegistry {
        let row = |name: &str, props: Value| json!([name, false, false, "", {"id": name, "properties": props}]);
        let facts = json!({"defaults": {"minecraft:air": 0, "minecraft:cave_air": 1,
                "minecraft:water": 2, "minecraft:prismarine_slab": 3, "minecraft:purpur_slab": 5,
                "minecraft:stone": 6},
            "states": [
                ["minecraft:air", false, false, "", "minecraft:air"],
                ["minecraft:cave_air", false, false, "", "minecraft:cave_air"],
                row("minecraft:water", json!({"level": "0"})),
                row("minecraft:prismarine_slab", json!({"waterlogged": "false"})),
                row("minecraft:prismarine_slab", json!({"waterlogged": "true"})),
                ["minecraft:purpur_slab", false, false, "", "minecraft:purpur_slab"],
                ["minecraft:stone", false, false, "", "minecraft:stone"]]});
        BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap()
    }

    fn info(state: u32, pos: [i32; 3]) -> BlockInfo {
        BlockInfo {
            pos,
            state: BlockStateId(state),
            nbt: None,
        }
    }

    fn chunk(x: i32, z: i32) -> PlaceSettings<'static> {
        let mut settings = PlaceSettings::new(Rotation::None);
        settings.bounding_box = Some(BoundingBox::new([x, -64, z], [x + 15, 319, z + 15]));
        settings
    }

    #[test]
    fn air_floods_below_the_sea_only() {
        let registry = registry();
        let flood = FloodAir::decode(&registry).unwrap();
        let mut sea = Sea {
            world: BlockStateId(6),
            marked: vec![],
        };
        let state = |i: BlockInfo, sea: &mut Sea| flood.process(sea, i).unwrap().unwrap().state;
        assert_eq!(state(info(0, [0, 62, 0]), &mut sea), BlockStateId(2));
        assert_eq!(state(info(0, [0, 63, 0]), &mut sea), BlockStateId(0));
        // Cave air is not Blocks.AIR.
        assert_eq!(state(info(1, [0, 10, 0]), &mut sea), BlockStateId(1));
    }

    #[test]
    fn waterlog_wets_and_marks_the_chunk_edges() {
        let registry = registry();
        let mut sea = Sea {
            world: BlockStateId(6),
            marked: vec![],
        };
        let settings = chunk(-16, 0);
        let mut run = |i: BlockInfo| {
            Waterlog::process(&mut sea, &registry, i, &settings)
                .unwrap()
                .unwrap()
                .state
        };
        assert_eq!(run(info(3, [-10, 40, 5])), BlockStateId(4));
        assert_eq!(run(info(3, [-10, 63, 5])), BlockStateId(3));
        assert_eq!(run(info(6, [-10, 40, 5])), BlockStateId(6));
        // Water on x = -16 (x % 16 == 0) and z = 15 is marked; x = -1
        // (x % 16 == -1), the middle and other chunks are not.
        for pos in [
            [-16, 40, 5],
            [-10, 40, 15],
            [-1, 40, 5],
            [-8, 40, 8],
            [0, 40, 0],
        ] {
            run(info(2, pos));
        }
        assert_eq!(sea.marked, vec![[-16, 40, 5], [-10, 40, 15]]);
    }

    #[test]
    fn kept_blocks_take_the_worlds() {
        let registry = registry();
        let keep = KeepWorldBlock::decode(&json!({"blocks": ["purpur_slab"]})).unwrap();
        let mut sea = Sea {
            world: BlockStateId(1),
            marked: vec![],
        };
        let settings = chunk(0, 0);
        let mut run = |i: BlockInfo| {
            keep.process(&mut sea, &registry, i, &settings)
                .unwrap()
                .unwrap()
                .state
        };
        assert_eq!(run(info(5, [3, 40, 3])), BlockStateId(1));
        assert_eq!(run(info(6, [3, 40, 3])), BlockStateId(6));
        assert_eq!(run(info(5, [16, 40, 3])), BlockStateId(5));
        assert!(KeepWorldBlock::decode(&json!({})).is_err());
    }
}
