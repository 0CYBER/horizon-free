//! Formations' SimpleStructure (`horizon:formations`, which
//! `horizon-port-pack` writes for its `formations:simple`; docs/modpacks.md):
//! one pool element at a random spot of the chunk, at a height its
//! placement finds in the noise columns under the element's corners and
//! centre ([`StartContext::noise_column`]).
//!
//! Its PiecedStructure (the fountain) starts the same way, then lays out
//! the element's jigsaw children with vanilla's jigsaw
//! ([`super::jigsaw::add_children`]): `horizon:pieced` on the structure
//! holds its `max_depth` and `max_distance_from_center`.
use super::{
    BoundingBox, Rotation, StructureResources,
    jigsaw::{JigsawConfig, PoolElement, PoolPiece, Pools, add_children},
    start::{NoiseBlock, NoiseColumn, StartContext},
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{DecodeError, block_feature::FeatureBlockBindings},
};
use horizon_core::position::ChunkPos;
use horizon_core::random::RandomSource;
use serde_json::Value;

/// StructurePlacement: how the height is found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Surface,
    Ceiling,
    OnWater,
    OnLava,
    Underground,
    UndergroundSurface,
    UndergroundCeiling,
    UndergroundBuried,
    UndergroundOnLava,
}
impl Placement {
    pub fn decode(name: &str) -> Result<Self, DecodeError> {
        Ok(match name {
            "surface" => Self::Surface,
            "ceiling" => Self::Ceiling,
            "on_water" => Self::OnWater,
            "on_lava" => Self::OnLava,
            "underground" => Self::Underground,
            "underground_surface" => Self::UndergroundSurface,
            "underground_ceiling" => Self::UndergroundCeiling,
            "underground_buried" => Self::UndergroundBuried,
            "underground_on_lava" => Self::UndergroundOnLava,
            _ => return Err(DecodeError::Field("formations placement")),
        })
    }
}

/// The structure: a pool to draw the element from, and its placement.
#[derive(Clone, Debug)]
pub struct Def {
    pub pool: String,
    pub placement: Placement,
    /// PiecedStructure's jigsaw: (max depth, max distance from the centre).
    pub pieced: Option<(i32, i32)>,
}
impl Def {
    pub fn decode(v: &Value) -> Result<Self, DecodeError> {
        Ok(Self {
            // An inline element is never written by Formations' generators.
            pool: v["template"]
                .as_str()
                .ok_or(DecodeError::Field("formations template"))?
                .to_owned(),
            placement: Placement::decode(v["placement"].as_str().unwrap_or_default())?,
            pieced: match v.get("horizon:pieced") {
                None => None,
                Some(p) => {
                    let int = |k: &'static str| {
                        p[k].as_i64().map(|n| n as i32).ok_or(DecodeError::Field(k))
                    };
                    Some((int("max_depth")?, int("max_distance_from_center")?))
                }
            },
        })
    }
}

/// SimpleStructure.findGenerationPoint, or PiecedStructure's: the start's
/// position and pieces.
pub fn generate(
    def: &Def,
    context: &mut impl StartContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    chunk: ChunkPos,
) -> Option<([i32; 3], Vec<PoolPiece>)> {
    let element = pools
        .get(&def.pool, resources, bindings)?
        .random_template(random);
    if matches!(element.as_ref(), PoolElement::Empty) {
        return None;
    }
    let x = chunk.x * 16 + random.next_int_bound(16).ok()?;
    let z = chunk.z * 16 + random.next_int_bound(16).ok()?;
    let rotation = Rotation::random(random);
    let b = element.bounding_box([x, 0, z], rotation);
    let height = find_height(def.placement, context, b, random)?;
    let ground = element.ground_level_delta();
    let y = height - ground + 1;
    if def.pieced.is_none() && y < context.min_y() {
        return None;
    }
    let piece = PoolPiece {
        bounding_box: b.moved([0, y, 0]),
        ground_level_delta: ground,
        element,
        position: [x, y, z],
        rotation,
        junctions: Vec::new(),
        // LiquidSettings.APPLY_WATERLOGGING.
        waterlogging: true,
    };
    let Some((max_depth, distance)) = def.pieced else {
        return Some(([x, y, z], vec![piece]));
    };
    // PiecedStructure.addPieces: the start sits at the element's centre, one
    // above the height found, and its children may fill the box of
    // `distance` around it (not clamped to the world's height) less its own.
    let (cx, cy, cz) = (
        (b.max[0] + b.min[0]) / 2,
        height + 1,
        (b.max[2] + b.min[2]) / 2,
    );
    let mut pieces = vec![piece];
    if max_depth > 0 {
        let config = JigsawConfig {
            start_pool: &def.pool,
            start_jigsaw: None,
            max_depth,
            use_expansion_hack: false,
            project_to: None,
            max_distance: [distance, distance],
            aliases: &Default::default(),
            padding: [0, 0],
            waterlogging: true,
        };
        let container = BoundingBox::new(
            [cx - distance, cy - distance, cz - distance],
            [cx + distance, cy + distance, cz + distance],
        );
        add_children(
            &config,
            context,
            pools,
            resources,
            bindings,
            random,
            &mut pieces,
            container,
        );
    }
    Some(([cx, cy, cz], pieces))
}

/// The four corners (min x min z, min x max z, max x max z, max x min z)
/// and the centre, as Formations' cornersAndCenter.
fn corners_and_center(b: BoundingBox) -> [[i32; 2]; 5] {
    let c = b.center();
    [
        [b.min[0], b.min[2]],
        [b.min[0], b.max[2]],
        [b.max[0], b.max[2]],
        [b.max[0], b.min[2]],
        [c[0], c[2]],
    ]
}

/// ChunkGenerator.getFirstOccupiedHeight on WORLD_SURFACE_WG.
fn first_occupied(context: &mut impl StartContext, x: i32, z: i32) -> i32 {
    context.base_height(x, z, HeightmapType::WorldSurfaceWg) - 1
}

/// (int) Math.round of the heights' mean.
fn average(heights: &[i32]) -> i32 {
    let mean = heights.iter().map(|h| f64::from(*h)).sum::<f64>() / heights.len() as f64;
    (mean + 0.5).floor() as i32
}

/// The largest and the mean distance of the heights from `average`.
fn spread(heights: &[i32], average: i32) -> (i32, f64) {
    let d: Vec<i32> = heights.iter().map(|h| (average - h).abs()).collect();
    let max = d.iter().copied().max().unwrap_or(0);
    let mean = d.iter().map(|v| f64::from(*v)).sum::<f64>() / d.len() as f64;
    (max, mean)
}

fn find_height(
    placement: Placement,
    context: &mut impl StartContext,
    b: BoundingBox,
    random: &mut impl RandomSource,
) -> Option<i32> {
    let solid = NoiseBlock::is_solid;
    match placement {
        Placement::Surface => from_top(context, b, 10, 3.0, solid),
        Placement::OnWater => from_top(context, b, 1, 1.0, |n| n == NoiseBlock::Water),
        Placement::OnLava => from_top(context, b, 1, 1.0, |n| n == NoiseBlock::Lava),
        Placement::Ceiling => {
            let min = context.min_y();
            let top = context.max_y() + 1;
            let mut heights = Vec::with_capacity(5);
            for [x, z] in corners_and_center(b) {
                let highest = first_occupied(context, x, z).min(top);
                let column = context.noise_column(x, z);
                heights.push(((min + 1)..highest).find(|y| column.get(*y) != NoiseBlock::Air)?);
            }
            let average = average(&heights);
            (spread(&heights, average).1 <= 4.0).then_some(average)
        }
        _ => underground(placement, context, b, random),
    }
}

/// findFromTop: every column's top block (WORLD_SURFACE_WG) passes
/// `target`, and the heights are close enough.
fn from_top(
    context: &mut impl StartContext,
    b: BoundingBox,
    max_offset: i32,
    max_average_offset: f64,
    target: impl Fn(NoiseBlock) -> bool,
) -> Option<i32> {
    let min = context.min_y();
    let points = corners_and_center(b);
    let heights: Vec<i32> = points
        .iter()
        .map(|[x, z]| first_occupied(context, *x, *z))
        .collect();
    if heights.iter().any(|y| *y <= min) {
        return None;
    }
    for ([x, z], y) in points.iter().zip(&heights) {
        if !target(context.noise_column(*x, *z).get(*y)) {
            return None;
        }
    }
    let average = average(&heights);
    let (max, mean) = spread(&heights, average);
    (max <= max_offset && mean <= max_average_offset).then_some(average)
}

/// The underground placements: the space between the highest first block
/// above the bottom and the lowest surface of the five columns, then a
/// random height in it, which each column adjusts to its floor (or
/// ceiling, or lava), within limits.
fn underground(
    placement: Placement,
    context: &mut impl StartContext,
    b: BoundingBox,
    random: &mut impl RandomSource,
) -> Option<i32> {
    let (bottom, top) = (context.min_y(), context.max_y() + 1);
    let mut columns: Vec<(NoiseColumn, i32, i32)> = Vec::with_capacity(5);
    for [x, z] in corners_and_center(b) {
        let highest = first_occupied(context, x, z).min(top);
        let column = context.noise_column(x, z);
        let lowest = ((bottom + 1)..highest).find(|y| column.get(*y) != NoiseBlock::Air)?;
        columns.push((column, highest, lowest));
    }
    let min = columns.iter().map(|c| c.2).max()?;
    let max = columns.iter().map(|c| c.1).min()?;
    let span = b.span()[1];
    if max - min - 2 <= span {
        return None;
    }
    let draw = random.next_int_bound(max - min - span - 2).ok()?;
    let air = |c: &NoiseColumn, y: i32| c.get(y) == NoiseBlock::Air;
    let solid = |c: &NoiseColumn, y: i32| c.get(y).is_solid();
    // A floor (or lava) under air near `height`: down through up to 10
    // blocks (`step` -1) or up (`step` 1, the ceiling), then through up to
    // `reach` blocks of air.
    let floor = |c: &NoiseColumn, height: i32, step: i32, reach: i32, lava: bool| {
        let mut y = height;
        let mut i = 0;
        while i < 10 && y > min && !air(c, y) {
            y += step;
            i += 1;
        }
        if !air(c, y) {
            return None;
        }
        y += step;
        let mut i = 0;
        while i < reach && air(c, y) {
            y += step;
            i += 1;
        }
        let ok = if lava {
            c.get(y) == NoiseBlock::Lava
        } else {
            solid(c, y)
        };
        ok.then_some(y)
    };
    let heights: Option<Vec<i32>> = match placement {
        Placement::Underground => return Some(min + 1 + draw),
        Placement::UndergroundSurface => {
            let h = min + 1 + draw;
            columns
                .iter()
                .map(|c| floor(&c.0, h, -1, 20, false))
                .collect()
        }
        Placement::UndergroundCeiling => {
            let h = min + 1 + draw;
            columns
                .iter()
                .map(|c| floor(&c.0, h, 1, 30, false))
                .collect()
        }
        Placement::UndergroundOnLava => {
            let h = min + 1 + draw;
            columns
                .iter()
                .map(|c| floor(&c.0, h, -1, 40, true))
                .collect()
        }
        Placement::UndergroundBuried => {
            let h = min + span + 1 + draw;
            let heights: Option<Vec<i32>> = columns
                .iter()
                .map(|(c, _, _)| {
                    let mut y = h;
                    let mut i = 0;
                    while i < 10 && y > min && !solid(c, y) {
                        y -= 1;
                        i += 1;
                    }
                    if !solid(c, y) {
                        return None;
                    }
                    y -= 2;
                    for _ in 0..span {
                        if !solid(c, y) {
                            return None;
                        }
                        y -= 1;
                    }
                    (solid(c, y) && solid(c, y - 1)).then_some(y)
                })
                .collect();
            let heights = heights?;
            let average = average(&heights);
            let held = columns
                .iter()
                .all(|(c, _, _)| solid(c, average) && solid(c, average + span));
            return held.then_some(average);
        }
        _ => None,
    };
    let heights = heights?;
    let average = average(&heights);
    let (max, mean) = spread(&heights, average);
    (max <= 5 && mean <= 3.0).then_some(average)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_round_as_math_round() {
        assert_eq!(average(&[1, 2]), 2);
        assert_eq!(average(&[-1, -2]), -1);
        assert_eq!(average(&[64, 64, 65, 66, 64]), 65);
        assert_eq!(spread(&[61, 64, 66], 63), (3, 2.0));
    }

    #[test]
    fn corners_then_centre() {
        let b = BoundingBox::new([0, 0, 10], [6, 4, 13]);
        assert_eq!(
            corners_and_center(b),
            [[0, 10], [0, 13], [6, 13], [6, 10], [3, 12]]
        );
    }

    #[test]
    fn placements_by_name() {
        assert_eq!(
            Placement::decode("underground_on_lava").unwrap(),
            Placement::UndergroundOnLava
        );
        assert!(Placement::decode("floating").is_err());
        let def = Def::decode(&serde_json::json!({"template": "a:b", "placement": "surface"}));
        let def = def.unwrap();
        assert_eq!((def.placement, def.pieced), (Placement::Surface, None));
        let pieced = Def::decode(
            &serde_json::json!({"template": "a:b", "placement": "surface",
            "horizon:pieced": {"max_depth": 2, "max_distance_from_center": 116}}),
        );
        assert_eq!(pieced.unwrap().pieced, Some((2, 116)));
        let broken = serde_json::json!({"template": "a:b", "placement": "surface",
            "horizon:pieced": {"max_depth": 2}});
        assert!(Def::decode(&broken).is_err());
    }
}
