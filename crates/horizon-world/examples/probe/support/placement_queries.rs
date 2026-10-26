use horizon_world::{
    heightmap::HeightmapType,
    worldgen::{
        anchor::WorldGenerationContext,
        placement::{LayerBlock, PlacementContext, PlacementQueryError},
    },
};
use serde_json::Value;
use std::{cell::RefCell, io::Write};
pub struct Queries<'a, W> {
    generation: WorldGenerationContext,
    world_min: i32,
    height_base: i32,
    mode: i32,
    layer_world: bool,
    fail: i32,
    calls: i32,
    out: &'a RefCell<W>,
}
impl<'a, W: Write> Queries<'a, W> {
    pub fn new(f: &Value, generation: WorldGenerationContext, out: &'a RefCell<W>) -> Self {
        let height_base = generation.min_y;
        let world_min = f["world_min"]
            .as_i64()
            .map(|v| v as i32)
            .unwrap_or(generation.min_y);
        let generation = WorldGenerationContext::new(
            generation.min_y.max(world_min),
            generation.height,
            generation.sea_level,
        );
        Self {
            generation,
            world_min,
            height_base,
            layer_world: f["layer_world"].as_bool().unwrap_or(false),
            mode: f["height_mode"].as_i64().unwrap_or(0) as i32,
            fail: f["query_fail"].as_i64().unwrap_or(-1) as i32,
            calls: 0,
            out,
        }
    }
}
impl<W: Write> PlacementContext for Queries<'_, W> {
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self
            .world_min
            .wrapping_add(self.generation.height)
            .wrapping_sub(1))
    }
    fn inside_build_height(&mut self, [x, y, z]: [i32; 3]) -> Result<bool, PlacementQueryError> {
        writeln!(self.out.borrow_mut(), "inside {x} {y} {z}")
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            return Err(PlacementQueryError::Failed("bounds query failure".into()));
        }
        let result = y >= self.world_min && y <= self.max_y()?;
        writeln!(self.out.borrow_mut(), "bounds_result {result}")
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        Ok(result)
    }
    fn outside_build_height(&mut self, y: i32) -> Result<bool, PlacementQueryError> {
        writeln!(self.out.borrow_mut(), "outside {y}")
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            return Err(PlacementQueryError::Failed("bounds query failure".into()));
        }
        let result = y < self.world_min || y > self.max_y()?;
        writeln!(self.out.borrow_mut(), "bounds_result {result}")
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        Ok(result)
    }
    fn layer_block(&mut self, [x, y, z]: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        let mut out = self.out.borrow_mut();
        writeln!(out, "block {x} {y} {z}")
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            return Err(PlacementQueryError::Failed("block query failure".into()));
        }
        let names = [
            "bedrock",
            "stone",
            "air",
            "water",
            "dirt",
            "cave_air",
            "lava",
            "bubble_column",
            "void_air",
            "grass_block",
            "air",
            "water",
        ];
        let index = if y <= self.height_base {
            0
        } else {
            y.wrapping_sub(self.height_base)
                .wrapping_add(x.wrapping_mul(31).wrapping_add(z.wrapping_mul(17)) & 3)
                .rem_euclid(12) as usize
        };
        writeln!(out, "block_result minecraft:{}", names[index])
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        Ok(match index {
            0 => LayerBlock::Bedrock,
            1 | 4 | 7 | 9 => LayerBlock::Ground,
            _ => LayerBlock::Empty,
        })
    }
    fn generation(&self) -> WorldGenerationContext {
        self.generation
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.world_min)
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        let mut out = self.out.borrow_mut();
        writeln!(out, "height {} {x} {z}", kind.name())
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            return Err(PlacementQueryError::Failed("height query failure".into()));
        }
        let min = self.height_base;
        let result = if self.layer_world {
            min.wrapping_add(24).wrapping_add((x ^ z) & 3)
        } else if self.mode == 0 {
            min.wrapping_add(
                x.wrapping_mul(31)
                    .wrapping_add(z.wrapping_mul(17))
                    .wrapping_add((kind as i32) * 7)
                    .rem_euclid(129),
            )
            .wrapping_sub(64)
        } else {
            [i32::MIN, i32::MAX, 0, -1, min, min.wrapping_add(1)]
                [(kind as usize + ((x ^ z) & 3) as usize) % 6]
        };
        writeln!(out, "height_result {result}")
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        Ok(result)
    }
}
