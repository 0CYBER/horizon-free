//! `minecraft:huge_fungus` (crimson and warped fungi, natural and planted).
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::BlockPredicate,
    feature::FeatureWorld,
    feature_graph::FeatureExecutionError,
    optional,
    placement::{PlacementContext, PlacementQueryError},
    required,
    tree::{TreeResult, kind, read, write},
    tree_blocks::{self as tb, TreeBlocks},
};
use crate::registry::BlockStateId;
use horizon_core::random::RandomSource;
use std::sync::Arc;

fn query<E>(error: PlacementQueryError) -> FeatureExecutionError<E> {
    FeatureExecutionError::Query(error)
}
/// Mth.nextInt(random, min, max).
fn next_int<E>(random: &mut impl RandomSource, min: i32, max: i32) -> TreeResult<i32, E> {
    if min >= max {
        Ok(min)
    } else {
        Ok(random.next_int_bound(max - min + 1)? + min)
    }
}

#[derive(Clone, Debug)]
pub struct HugeFungusFeature {
    valid_base: BlockStateId,
    stem: BlockStateId,
    hat: BlockStateId,
    decor: BlockStateId,
    replaceable: BlockPredicate,
    planted: bool,
    air: BlockStateId,
    weeping_vines: BlockStateId,
    weeping_vines_plant: BlockStateId,
    nether_wart_block: BlockStateId,
    blocks: Arc<TreeBlocks>,
}
impl HugeFungusFeature {
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        if kind(&object)? != "minecraft:huge_fungus" {
            return Err(DecodeError::Field("huge fungus type"));
        }
        let state = |name: &'static str| {
            bindings
                .codec
                .decode(required(&object, name)?.get(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))
        };
        let registry = bindings.registry();
        let default = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("missing fungus block {name}")))
        };
        Ok(Self {
            valid_base: state("valid_base_block")?,
            stem: state("stem_state")?,
            hat: state("hat_state")?,
            decor: state("decor_state")?,
            replaceable: BlockPredicate::decode_with_blocks(
                required(&object, "replaceable_blocks")?.get(),
                depth + 1,
                remaining,
                Some(bindings),
            )?,
            planted: optional(&object, "planted")
                .map(|raw| serde_json::from_str::<bool>(raw.get()))
                .transpose()?
                .unwrap_or(false),
            air: default("minecraft:air")?,
            weeping_vines: default("minecraft:weeping_vines")?,
            weeping_vines_plant: default("minecraft:weeping_vines_plant")?,
            nether_wart_block: default("minecraft:nether_wart_block")?,
            blocks: bindings.tree_blocks()?,
        })
    }
    pub(crate) fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        let below = read(world, tb::offset(origin, 0, -1, 0))?;
        if !self.blocks.same_block(below, self.valid_base) {
            return Ok(false);
        }
        let mut height = next_int(random, 4, 13)?;
        if random.next_int_bound(12)? == 0 {
            height *= 2;
        }
        if !self.planted {
            // ChunkGenerator.getGenDepth, compared against the absolute Y.
            let depth = world.generation().height;
            if origin[1].wrapping_add(height).wrapping_add(1) >= depth {
                return Ok(false);
            }
        }
        let huge = !self.planted && random.next_float() < 0.06;
        write(world, origin, self.air, 260)?;
        self.stem(world, random, origin, height, huge)?;
        self.hat(world, random, origin, height, huge)?;
        Ok(true)
    }
    fn replaceable<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        p: [i32; 3],
        plants: bool,
    ) -> TreeResult<bool, W::Error> {
        if self.blocks.has(read(world, p)?, tb::REPLACEABLE) {
            return Ok(true);
        }
        if plants {
            return self.replaceable.test(world, p).map_err(query);
        }
        Ok(false)
    }
    fn stem<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        height: i32,
        huge: bool,
    ) -> TreeResult<(), W::Error> {
        let r = i32::from(huge);
        for dx in -r..=r {
            for dz in -r..=r {
                let corner = huge && dx.abs() == r && dz.abs() == r;
                for dy in 0..height {
                    let p = tb::offset(origin, dx, dy, dz);
                    if !self.replaceable(world, p, true)? {
                        continue;
                    }
                    if self.planted {
                        if !self.blocks.is_air(read(world, tb::offset(p, 0, -1, 0))?) {
                            world.destroy_block_without_entity(p, true).map_err(query)?;
                        }
                        write(world, p, self.stem, 3)?;
                    } else if corner {
                        if random.next_float() < 0.1 {
                            write(world, p, self.stem, 3)?;
                        }
                    } else {
                        write(world, p, self.stem, 3)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn hat<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        origin: [i32; 3],
        height: i32,
        huge: bool,
    ) -> TreeResult<(), W::Error> {
        let vines = self.blocks.same_block(self.hat, self.nether_wart_block);
        let hat_height = (random.next_int_bound(1 + height / 3)? + 5).min(height);
        let start = height - hat_height;
        for dy in start..=height {
            let mut radius = if dy < height - random.next_int_bound(3)? {
                2
            } else {
                1
            };
            if hat_height > 8 && dy < start + 4 {
                radius = 3;
            }
            if huge {
                radius += 1;
            }
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let edge_x = dx == -radius || dx == radius;
                    let edge_z = dz == -radius || dz == radius;
                    let inside = !edge_x && !edge_z && dy != height;
                    let corner = edge_x && edge_z;
                    let bottom = dy < start + 3;
                    let p = tb::offset(origin, dx, dy, dz);
                    if !self.replaceable(world, p, false)? {
                        continue;
                    }
                    if self.planted && !self.blocks.is_air(read(world, tb::offset(p, 0, -1, 0))?) {
                        world.destroy_block_without_entity(p, true).map_err(query)?;
                    }
                    if bottom {
                        if !inside {
                            self.hat_drop(world, random, p, vines)?;
                        }
                    } else if inside {
                        self.hat_block(world, random, p, 0.1, 0.2, if vines { 0.1 } else { 0.0 })?;
                    } else if corner {
                        self.hat_block(
                            world,
                            random,
                            p,
                            0.01,
                            0.7,
                            if vines { 0.083 } else { 0.0 },
                        )?;
                    } else {
                        self.hat_block(
                            world,
                            random,
                            p,
                            5.0e-4,
                            0.98,
                            if vines { 0.07 } else { 0.0 },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn hat_block<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        p: [i32; 3],
        decor: f32,
        hat: f32,
        vines: f32,
    ) -> TreeResult<(), W::Error> {
        if random.next_float() < decor {
            write(world, p, self.decor, 3)?;
        } else if random.next_float() < hat {
            write(world, p, self.hat, 3)?;
            if random.next_float() < vines {
                self.weeping_vines(world, random, p)?;
            }
        }
        Ok(())
    }
    fn hat_drop<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        p: [i32; 3],
        vines: bool,
    ) -> TreeResult<(), W::Error> {
        let below = read(world, tb::offset(p, 0, -1, 0))?;
        if self.blocks.same_block(below, self.hat) {
            write(world, p, self.hat, 3)?;
        } else if f64::from(random.next_float()) < 0.15 {
            write(world, p, self.hat, 3)?;
            if vines && random.next_int_bound(11)? == 0 {
                self.weeping_vines(world, random, p)?;
            }
        }
        Ok(())
    }
    fn weeping_vines<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        random: &mut R,
        hat: [i32; 3],
    ) -> TreeResult<(), W::Error> {
        let mut p = tb::offset(hat, 0, -1, 0);
        if !self.blocks.is_air(read(world, p)?) {
            return Ok(());
        }
        let mut goal = next_int(random, 1, 5)?;
        if random.next_int_bound(7)? == 0 {
            goal *= 2;
        }
        for height in 0..=goal {
            if self.blocks.is_air(read(world, p)?) {
                if height == goal || !self.blocks.is_air(read(world, tb::offset(p, 0, -1, 0))?) {
                    let age = next_int(random, 23, 25)?;
                    let state = self
                        .blocks
                        .with(self.weeping_vines, "age", &age.to_string())
                        .ok_or(FeatureExecutionError::InvalidState)?;
                    write(world, p, state, 2)?;
                    break;
                }
                write(world, p, self.weeping_vines_plant, 2)?;
            }
            p = tb::offset(p, 0, -1, 0);
        }
        Ok(())
    }
}
