//! Foliage placers (FoliagePlacer subclasses), in native random-draw order.
use super::{
    Layer, Run, TreeDecoder, TreeFeature, TreeResult, kind, read, sample_state, unit_float,
};
use crate::worldgen::{
    DecodeError, Object,
    feature::FeatureWorld,
    int_range,
    placement::PlacementContext,
    required,
    tree_blocks::{self as tb, Axis},
    value_provider::IntProvider,
};
use horizon_core::{geometry::Direction, random::RandomSource};
use serde_json::value::RawValue;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FoliageAttachment {
    pub pos: [i32; 3],
    pub radius_offset: i32,
    pub height_offset: i32,
    size: [i32; 2],
}
impl FoliageAttachment {
    pub(crate) fn new(pos: [i32; 3], radius_offset: i32, double_trunk: bool) -> Self {
        let size = if double_trunk { 2 } else { 1 };
        Self {
            pos,
            radius_offset,
            height_offset: 0,
            size: [size; 2],
        }
    }
    fn double_trunk(&self) -> bool {
        self.size == [2, 2]
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Blob(i32),
    Fancy(i32),
    Bush(i32),
    Acacia,
    Spruce(IntProvider),
    Pine(IntProvider),
    MegaPine(IntProvider),
    MegaJungle(i32),
    DarkOak,
    RandomSpread {
        foliage_height: IntProvider,
        attempts: i32,
    },
    Cherry {
        height: IntProvider,
        wide_bottom_hole: f32,
        corner_hole: f32,
        hanging: f32,
        hanging_extension: f32,
    },
    Poplar {
        height: IntProvider,
        side_hole: f32,
    },
}
#[derive(Clone, Debug)]
pub(super) struct FoliagePlacer {
    radius: IntProvider,
    offset: IntProvider,
    kind: Kind,
}

fn abs(v: i32) -> i32 {
    v.wrapping_abs()
}

impl FoliagePlacer {
    pub(super) fn decode(
        raw: &RawValue,
        decoder: &mut TreeDecoder<'_, '_>,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(raw.get())?;
        let radius = decoder.int_provider(required(&object, "radius")?, 0, 16)?;
        let offset = decoder.int_provider(required(&object, "offset")?, 0, 16)?;
        let fixed_height = || int_range(required(&object, "height")?, "foliage height", 0, 16);
        let chance = |name: &'static str| unit_float(required(&object, name)?, name);
        let kind = match kind(&object)?.as_str() {
            "minecraft:blob_foliage_placer" => Kind::Blob(fixed_height()?),
            "minecraft:fancy_foliage_placer" => Kind::Fancy(fixed_height()?),
            "minecraft:bush_foliage_placer" => Kind::Bush(fixed_height()?),
            "minecraft:jungle_foliage_placer" => Kind::MegaJungle(fixed_height()?),
            "minecraft:acacia_foliage_placer" => Kind::Acacia,
            "minecraft:dark_oak_foliage_placer" => Kind::DarkOak,
            "minecraft:spruce_foliage_placer" => {
                Kind::Spruce(decoder.int_provider(required(&object, "trunk_height")?, 0, 24)?)
            }
            "minecraft:pine_foliage_placer" => {
                Kind::Pine(decoder.int_provider(required(&object, "height")?, 0, 24)?)
            }
            "minecraft:mega_pine_foliage_placer" => {
                Kind::MegaPine(decoder.int_provider(required(&object, "crown_height")?, 0, 24)?)
            }
            "minecraft:random_spread_foliage_placer" => Kind::RandomSpread {
                foliage_height: decoder.int_provider(
                    required(&object, "foliage_height")?,
                    1,
                    512,
                )?,
                attempts: int_range(
                    required(&object, "leaf_placement_attempts")?,
                    "leaf_placement_attempts",
                    0,
                    256,
                )?,
            },
            "minecraft:cherry_foliage_placer" => Kind::Cherry {
                height: decoder.int_provider(required(&object, "height")?, 4, 16)?,
                wide_bottom_hole: chance("wide_bottom_layer_hole_chance")?,
                corner_hole: chance("corner_hole_chance")?,
                hanging: chance("hanging_leaves_chance")?,
                hanging_extension: chance("hanging_leaves_extension_chance")?,
            },
            "minecraft:poplar_foliage_placer" => Kind::Poplar {
                height: decoder.int_provider(required(&object, "height")?, 5, 16)?,
                side_hole: chance("side_hole_chance")?,
            },
            _ => return Err(DecodeError::Field("foliage placer type")),
        };
        Ok(Self {
            radius,
            offset,
            kind,
        })
    }
    pub(super) fn foliage_height<E>(
        &self,
        random: &mut impl RandomSource,
        tree_height: i32,
    ) -> TreeResult<i32, E> {
        Ok(match &self.kind {
            Kind::Blob(h) | Kind::Fancy(h) | Kind::Bush(h) | Kind::MegaJungle(h) => *h,
            Kind::Acacia => 0,
            Kind::DarkOak => 4,
            Kind::Spruce(trunk) => 4.max(tree_height - trunk.sample(random)?),
            Kind::Pine(h) | Kind::MegaPine(h) => h.sample(random)?,
            Kind::RandomSpread { foliage_height, .. } => foliage_height.sample(random)?,
            Kind::Cherry { height, .. } | Kind::Poplar { height, .. } => height.sample(random)?,
        })
    }
    pub(super) fn foliage_radius<E>(
        &self,
        random: &mut impl RandomSource,
        trunk_height: i32,
    ) -> TreeResult<i32, E> {
        let radius = self.radius.sample(random)?;
        if matches!(self.kind, Kind::Pine(_)) {
            return Ok(radius + random.next_int_bound(trunk_height.wrapping_add(1).max(1))?);
        }
        Ok(radius)
    }

    /// shouldSkipLocation with already-unsigned offsets.
    fn skip(
        &self,
        random: &mut impl RandomSource,
        dx: i32,
        y: i32,
        dz: i32,
        r: i32,
        double: bool,
    ) -> Result<bool, horizon_core::random::InvalidBound> {
        Ok(match &self.kind {
            Kind::Blob(_) => dx == r && dz == r && (random.next_int_bound(2)? == 0 || y == 0),
            Kind::Bush(_) => dx == r && dz == r && random.next_int_bound(2)? == 0,
            Kind::Fancy(_) => {
                let a = dx as f32 + 0.5;
                let b = dz as f32 + 0.5;
                a * a + b * b > r.wrapping_mul(r) as f32
            }
            Kind::Acacia => {
                if y == 0 {
                    (dx > 1 || dz > 1) && dx != 0 && dz != 0
                } else {
                    dx == r && dz == r && r > 0
                }
            }
            Kind::Spruce(_) | Kind::Pine(_) => dx == r && dz == r && r > 0,
            Kind::MegaPine(_) | Kind::MegaJungle(_) => {
                dx.wrapping_add(dz) >= 7
                    || dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) > r.wrapping_mul(r)
            }
            Kind::DarkOak => {
                if y == -1 && !double {
                    dx == r && dz == r
                } else if y == 1 {
                    dx.wrapping_add(dz) > r.wrapping_mul(2).wrapping_sub(2)
                } else {
                    false
                }
            }
            Kind::RandomSpread { .. } => false,
            Kind::Cherry {
                wide_bottom_hole,
                corner_hole,
                ..
            } => {
                if y == -1 && (dx == r || dz == r) && random.next_float() < *wide_bottom_hole {
                    return Ok(true);
                }
                let corner = dx == r && dz == r;
                if r > 2 {
                    corner
                        || (dx.wrapping_add(dz) > r.wrapping_mul(2).wrapping_sub(2)
                            && random.next_float() < *corner_hole)
                } else {
                    corner && random.next_float() < *corner_hole
                }
            }
            Kind::Poplar { .. } => unreachable!("poplar uses its own row placement"),
        })
    }
    fn skip_signed(
        &self,
        random: &mut impl RandomSource,
        dx: i32,
        y: i32,
        dz: i32,
        r: i32,
        double: bool,
    ) -> Result<bool, horizon_core::random::InvalidBound> {
        if matches!(self.kind, Kind::DarkOak) {
            let defer = y != 0
                || !double
                || (dx != r.wrapping_neg() && dx < r)
                || (dz != r.wrapping_neg() && dz < r);
            if !defer {
                return Ok(true);
            }
        }
        let (mx, mz) = if double {
            (
                abs(dx).min(abs(dx.wrapping_sub(1))),
                abs(dz).min(abs(dz.wrapping_sub(1))),
            )
        } else {
            (abs(dx), abs(dz))
        };
        self.skip(random, mx, y, mz, r, double)
    }
    #[allow(clippy::too_many_arguments)]
    fn leaves_row<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        tree: &TreeFeature,
        origin: [i32; 3],
        r: i32,
        y: i32,
        double: bool,
    ) -> TreeResult<(), W::Error> {
        let offset = i32::from(double);
        let mut dx = r.wrapping_neg();
        while dx <= r.wrapping_add(offset) {
            let mut dz = r.wrapping_neg();
            while dz <= r.wrapping_add(offset) {
                if !self.skip_signed(run.random, dx, y, dz, r, double)? {
                    tree.try_place_leaf(run, tb::offset(origin, dx, y, dz))?;
                }
                dz += 1;
            }
            dx += 1;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn leaves_row_with_hanging<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        tree: &TreeFeature,
        origin: [i32; 3],
        r: i32,
        y: i32,
        double: bool,
        chance: f32,
        extension: f32,
    ) -> TreeResult<(), W::Error> {
        self.leaves_row(run, tree, origin, r, y, double)?;
        let offset = i32::from(double);
        let log = tb::offset(origin, 0, -1, 0);
        for along in tb::HORIZONTAL {
            let to_edge = tb::clockwise(along);
            let positive = matches!(to_edge, Direction::East | Direction::South);
            let to_edge_offset = if positive { r + offset } else { r };
            let mut pos = tb::relative(
                tb::relative(tb::offset(origin, 0, y - 1, 0), to_edge, to_edge_offset),
                along,
                r.wrapping_neg(),
            );
            let mut along_offset = r.wrapping_neg();
            while along_offset < r + offset {
                let above = run.foliage_contains(tb::offset(pos, 0, 1, 0));
                if above && Self::try_extension(run, tree, chance, log, pos)? {
                    Self::try_extension(run, tree, extension, log, tb::offset(pos, 0, -1, 0))?;
                }
                along_offset += 1;
                pos = tb::relative(pos, along, 1);
            }
        }
        Ok(())
    }
    fn try_extension<W: FeatureWorld + PlacementContext, R: RandomSource>(
        run: &mut Run<'_, W, R>,
        tree: &TreeFeature,
        chance: f32,
        log: [i32; 3],
        pos: [i32; 3],
    ) -> TreeResult<bool, W::Error> {
        if tb::dist_manhattan(pos, log) >= 7 {
            return Ok(false);
        }
        if run.random.next_float() > chance {
            return Ok(false);
        }
        tree.try_place_leaf(run, pos)
    }

    pub(super) fn create_foliage<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        tree: &TreeFeature,
        _tree_height: i32,
        attachment: FoliageAttachment,
        foliage_height: i32,
        leaf_radius: i32,
    ) -> TreeResult<(), W::Error> {
        let offset = self.offset.sample(run.random)?;
        let double = attachment.double_trunk();
        let pos = attachment.pos;
        let with_offset = foliage_height + attachment.height_offset;
        match &self.kind {
            Kind::Blob(_) => {
                let mut yo = offset;
                while yo >= offset - with_offset {
                    let r = 0.max(leaf_radius + attachment.radius_offset - 1 - yo / 2);
                    self.leaves_row(run, tree, pos, r, yo, double)?;
                    yo -= 1;
                }
            }
            Kind::Fancy(_) => {
                let mut yo = offset;
                while yo >= offset - foliage_height {
                    let r = leaf_radius + i32::from(yo != offset && yo != offset - foliage_height);
                    self.leaves_row(run, tree, pos, r, yo, double)?;
                    yo -= 1;
                }
            }
            Kind::Bush(_) => {
                let mut yo = offset;
                while yo >= offset - with_offset {
                    let r = leaf_radius + attachment.radius_offset - 1 - yo;
                    self.leaves_row(run, tree, pos, r, yo, double)?;
                    yo -= 1;
                }
            }
            Kind::Acacia => {
                let p = tb::offset(pos, 0, offset, 0);
                self.leaves_row(
                    run,
                    tree,
                    p,
                    leaf_radius + attachment.radius_offset,
                    -1 - with_offset,
                    double,
                )?;
                self.leaves_row(run, tree, p, leaf_radius - 1, -with_offset, double)?;
                self.leaves_row(
                    run,
                    tree,
                    p,
                    leaf_radius + attachment.radius_offset - 1,
                    0,
                    double,
                )?;
            }
            Kind::Spruce(_) => {
                let mut r = run.random.next_int_bound(2)?;
                let mut max_r = 1;
                let mut min_r = 0;
                let mut yo = offset;
                while yo >= -with_offset {
                    self.leaves_row(run, tree, pos, r, yo, double)?;
                    if r >= max_r {
                        r = min_r;
                        min_r = 1;
                        max_r = (max_r + 1).min(leaf_radius + attachment.radius_offset);
                    } else {
                        r += 1;
                    }
                    yo -= 1;
                }
            }
            Kind::Pine(_) => {
                let mut r = 0;
                let mut yo = offset;
                while yo >= offset - with_offset {
                    self.leaves_row(run, tree, pos, r, yo, double)?;
                    if r >= 1 && yo == offset - with_offset + 1 {
                        r -= 1;
                    } else if r < leaf_radius + attachment.radius_offset {
                        r += 1;
                    }
                    yo -= 1;
                }
            }
            Kind::MegaPine(_) => {
                let mut previous = 0;
                let mut yy = pos[1] - with_offset + offset;
                while yy <= pos[1] + offset {
                    let yo = pos[1] - yy;
                    let smooth = leaf_radius
                        .wrapping_add(attachment.radius_offset)
                        .wrapping_add((yo as f32 / with_offset as f32 * 3.5f32).floor() as i32);
                    let jagged = if yo > 0 && smooth == previous && (yy & 1) == 0 {
                        smooth.wrapping_add(1)
                    } else {
                        smooth
                    };
                    self.leaves_row(run, tree, [pos[0], yy, pos[2]], jagged, 0, double)?;
                    previous = smooth;
                    yy += 1;
                }
            }
            Kind::MegaJungle(_) => {
                let leaf_height = (if double {
                    foliage_height
                } else {
                    1 + run.random.next_int_bound(2)?
                }) + attachment.height_offset;
                let mut yo = offset;
                while yo >= offset - leaf_height {
                    let r = leaf_radius + attachment.radius_offset + 1 - yo;
                    self.leaves_row(run, tree, pos, r, yo, double)?;
                    yo -= 1;
                }
            }
            Kind::DarkOak => {
                let p = tb::offset(pos, 0, offset, 0);
                if double {
                    self.leaves_row(run, tree, p, leaf_radius + 2, -1, double)?;
                    self.leaves_row(run, tree, p, leaf_radius + 3, 0, double)?;
                    self.leaves_row(run, tree, p, leaf_radius + 2, 1, double)?;
                    if run.random.next_boolean() {
                        self.leaves_row(run, tree, p, leaf_radius, 2, double)?;
                    }
                } else {
                    self.leaves_row(run, tree, p, leaf_radius + 2, -1, double)?;
                    self.leaves_row(run, tree, p, leaf_radius + 1, 0, double)?;
                }
            }
            Kind::RandomSpread { attempts, .. } => {
                for _ in 0..*attempts {
                    let dx = run.random.next_int_bound(leaf_radius)?
                        - run.random.next_int_bound(leaf_radius)?;
                    let dy = run.random.next_int_bound(foliage_height)?
                        - run.random.next_int_bound(foliage_height)?;
                    let dz = run.random.next_int_bound(leaf_radius)?
                        - run.random.next_int_bound(leaf_radius)?;
                    tree.try_place_leaf(run, tb::offset(pos, dx, dy, dz))?;
                }
            }
            Kind::Cherry {
                hanging,
                hanging_extension,
                ..
            } => {
                let p = tb::offset(pos, 0, offset, 0);
                let r = leaf_radius + attachment.radius_offset - 1;
                self.leaves_row(run, tree, p, r - 2, with_offset - 3, double)?;
                self.leaves_row(run, tree, p, r - 1, with_offset - 4, double)?;
                let mut y = with_offset - 5;
                while y >= 0 {
                    self.leaves_row(run, tree, p, r, y, double)?;
                    y -= 1;
                }
                self.leaves_row_with_hanging(
                    run,
                    tree,
                    p,
                    r,
                    -1,
                    double,
                    *hanging,
                    *hanging_extension,
                )?;
                self.leaves_row_with_hanging(
                    run,
                    tree,
                    p,
                    r - 1,
                    -2,
                    double,
                    *hanging,
                    *hanging_extension,
                )?;
            }
            Kind::Poplar { side_hole, .. } => {
                let poplar = Poplar {
                    side_hole: *side_hole,
                    flip: run.random.next_boolean(),
                    height: with_offset,
                    double,
                };
                let p = tb::offset(pos, 0, offset, 0);
                let r = leaf_radius + attachment.radius_offset - 1;
                poplar.row(run, tree, p, r - 2, with_offset - 1)?;
                poplar.row(run, tree, p, r - 1, with_offset - 2)?;
                poplar.row(run, tree, p, r - 1, with_offset - 3)?;
                let mut y = with_offset - 4;
                while y >= 1 {
                    poplar.row(run, tree, p, r, y)?;
                    y -= 1;
                }
                poplar.replace_with_log(run, tree, p, r, with_offset - 4)?;
                poplar.row(run, tree, p, r - 1, 0)?;
                poplar.row(run, tree, p, (r - 2).clamp(1, 2), -1)?;
            }
        }
        Ok(())
    }
}

struct Poplar {
    side_hole: f32,
    flip: bool,
    height: i32,
    double: bool,
}
impl Poplar {
    fn partial_row(&self, y: i32) -> bool {
        self.height - 1 == y || self.height - 2 == y
    }
    fn corner_cut(&self, dx: i32, dz: i32, r: i32, partial: bool) -> i32 {
        let small = if self.flip {
            (dx > 0 && dz > 0) || (dz < 0 && dx < 0)
        } else {
            (dx > 0 && dz < 0) || (dz > 0 && dx < 0)
        };
        if small {
            r - 1
        } else if partial {
            r + 1
        } else {
            r
        }
    }
    fn within(r: i32, adx: i32, adz: i32, cut: i32, extra: i32) -> bool {
        adx.wrapping_add(adz) <= r.wrapping_mul(2).wrapping_sub(cut.wrapping_add(extra))
    }
    fn skip(
        &self,
        random: &mut impl RandomSource,
        dx: i32,
        y: i32,
        dz: i32,
        r: i32,
    ) -> Result<bool, horizon_core::random::InvalidBound> {
        let partial = self.partial_row(y);
        let cut = self.corner_cut(dx, dz, r, partial);
        let (adx, adz) = (abs(dx), abs(dz));
        if partial && (adx == r || adz == r) {
            return Ok(true);
        }
        let extra = i32::from(random.next_float() <= self.side_hole);
        Ok(!Self::within(r, adx, adz, cut, extra))
    }
    fn row<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        tree: &TreeFeature,
        origin: [i32; 3],
        r: i32,
        y: i32,
    ) -> TreeResult<(), W::Error> {
        let offset = i32::from(self.double);
        let mut dx = r.wrapping_neg();
        while dx <= r + offset {
            let mut dz = r.wrapping_neg();
            while dz <= r + offset {
                if !self.skip(run.random, dx, y, dz, r)? {
                    tree.try_place_leaf(run, tb::offset(origin, dx, y, dz))?;
                }
                dz += 1;
            }
            dx += 1;
        }
        Ok(())
    }
    fn replace_with_log<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        run: &mut Run<'_, W, R>,
        tree: &TreeFeature,
        origin: [i32; 3],
        r: i32,
        y: i32,
    ) -> TreeResult<(), W::Error> {
        let offset = i32::from(self.double);
        let mut dx = r.wrapping_neg();
        while dx <= r + offset {
            let mut dz = r.wrapping_neg();
            while dz <= r + offset {
                let (adx, adz) = (abs(dx), abs(dz));
                let cut = self.corner_cut(dx, dz, r, self.partial_row(y));
                if Self::within(r, adx, adz, cut, 2)
                    && ((adz == 0 && r - adx >= 4) || (adx == 0 && r - adz >= 4))
                {
                    let pos = tb::offset(origin, dx, y, dz);
                    let axis = if adz == 0 { Axis::X } else { Axis::Z };
                    // The comparison state is sampled from the foliage provider.
                    let here = read(run.world, pos)?;
                    let foliage = sample_state(&tree.foliage_provider, run.world, run.random, pos)?;
                    if here == foliage {
                        let log = sample_state(&tree.trunk_provider, run.world, run.random, pos)?;
                        run.set(Layer::Foliage, pos, tree.blocks.try_axis(log, axis))?;
                    }
                }
                dz += 1;
            }
            dx += 1;
        }
        Ok(())
    }
}
