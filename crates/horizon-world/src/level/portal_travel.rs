//! Nether portal travel: `PortalForcer` (finding and building exit portals),
//! `BlockUtil.getLargestRectangleAround`, `PortalShape.getRelativePosition`
//! and `NetherPortalBlock.createDimensionTransition`.
use super::{Behaviours, LevelAccess, Sim, Ticks};
use crate::fluid::FluidKind;
use crate::{
    java_hash_map::{JavaHashMap, JavaKey},
    registry::BlockStateId,
};
use horizon_core::{
    geometry::{Aabb, Axis, Direction, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};
use std::cmp::Ordering;

/// `BlockUtil.FoundRectangle`: the low corner and the sizes along the two axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoundRectangle {
    pub min_corner: BlockPos,
    pub axis1_size: i32,
    pub axis2_size: i32,
}

/// The world border's extent (`getMinX` .. `getMaxX`, `getMinZ` .. `getMaxZ`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderBounds {
    pub min_x: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_z: f64,
}
impl BorderBounds {
    /// `WorldBorder.isWithinBounds(x, z)`.
    pub fn contains(&self, x: f64, z: f64) -> bool {
        x >= self.min_x && x < self.max_x && z >= self.min_z && z < self.max_z
    }
    /// `WorldBorder.isWithinBounds(BlockPos)`.
    pub fn contains_block(&self, pos: BlockPos) -> bool {
        self.contains(f64::from(pos.x), f64::from(pos.z))
    }
    /// `WorldBorder.clampToBounds(x, y, z)`.
    pub fn clamp(&self, x: f64, y: f64, z: f64) -> BlockPos {
        let margin = f64::from(1.0e-5_f32);
        let clamp = |v: f64, lo: f64, hi: f64| if v < lo { lo } else { v.min(hi) };
        BlockPos::new(
            clamp(x, self.min_x, self.max_x - margin).floor() as i32,
            y.floor() as i32,
            clamp(z, self.min_z, self.max_z - margin).floor() as i32,
        )
    }
}

fn step(pos: BlockPos, direction: Direction, n: i32) -> BlockPos {
    let [dx, dy, dz] = direction.normal();
    pos.offset(dx * n, dy * n, dz * n)
}
fn negative(axis: Axis) -> Direction {
    match axis {
        Axis::X => Direction::West,
        Axis::Y => Direction::Down,
        Axis::Z => Direction::North,
    }
}
fn component(pos: BlockPos, axis: Axis) -> i32 {
    match axis {
        Axis::X => pos.x,
        Axis::Y => pos.y,
        Axis::Z => pos.z,
    }
}
/// `Direction.getClockWise` for the horizontal directions.
fn clockwise(direction: Direction) -> Direction {
    match direction {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        other => other,
    }
}

/// `BlockUtil.getLimit`: how many steps from `start` along `direction` pass `test`.
fn limit(
    test: &mut impl FnMut(BlockPos) -> bool,
    start: BlockPos,
    direction: Direction,
    limit: i32,
) -> i32 {
    let mut max = 0;
    let mut pos = start;
    while max < limit {
        pos = step(pos, direction, 1);
        if !test(pos) {
            break;
        }
        max += 1;
    }
    max
}

/// `BlockUtil.getMaxRectangleLocation`: the widest-area run of `columns`
/// (start, end inclusive) and its height.
fn max_rectangle_location(columns: &[i32]) -> (i32, i32, i32) {
    let (mut max_start, mut max_end, mut max_height) = (0usize, 0usize, 0i32);
    let mut stack: Vec<usize> = vec![0];
    for column in 1..=columns.len() {
        let height = if column == columns.len() {
            0
        } else {
            columns[column]
        };
        while let Some(&top) = stack.last() {
            let stack_height = columns[top];
            if height >= stack_height {
                stack.push(column);
                break;
            }
            stack.pop();
            let start = stack.last().map_or(0, |t| t + 1);
            if i64::from(stack_height) * (column - start) as i64
                > i64::from(max_height) * (max_end - max_start) as i64
            {
                max_end = column;
                max_start = start;
                max_height = stack_height;
            }
        }
        if stack.is_empty() {
            stack.push(column);
        }
    }
    (max_start as i32, max_end as i32 - 1, max_height)
}

/// `BlockUtil.getLargestRectangleAround`: the largest rectangle of positions
/// passing `test` in the plane of `axis1` and `axis2` that contains `center`.
pub fn largest_rectangle_around(
    center: BlockPos,
    axis1: Axis,
    limit1: i32,
    axis2: Axis,
    limit2: i32,
    mut test: impl FnMut(BlockPos) -> bool,
) -> FoundRectangle {
    let negative1 = negative(axis1);
    let positive1 = negative1.opposite();
    let negative2 = negative(axis2);
    let positive2 = negative2.opposite();
    let negative_delta1 = limit(&mut test, center, negative1, limit1);
    let positive_delta1 = limit(&mut test, center, positive1, limit1);
    let center_index1 = negative_delta1 as usize;
    let mut bounds = vec![(0, 0); center_index1 + 1 + positive_delta1 as usize];
    bounds[center_index1] = (
        limit(&mut test, center, negative2, limit2),
        limit(&mut test, center, positive2, limit2),
    );
    let center_index2 = bounds[center_index1].0;
    for i in 1..=negative_delta1 as usize {
        let last = bounds[center_index1 - (i - 1)];
        let at = step(center, negative1, i as i32);
        bounds[center_index1 - i] = (
            limit(&mut test, at, negative2, last.0),
            limit(&mut test, at, positive2, last.1),
        );
    }
    for i in 1..=positive_delta1 as usize {
        let last = bounds[center_index1 + i - 1];
        let at = step(center, positive1, i as i32);
        bounds[center_index1 + i] = (
            limit(&mut test, at, negative2, last.0),
            limit(&mut test, at, positive2, last.1),
        );
    }
    let (mut min_axis1, mut min_axis2, mut size_axis1, mut size_axis2) = (0, 0, 0, 0);
    let mut columns = vec![0; bounds.len()];
    for i2 in (0..=center_index2).rev() {
        for (i1, b) in bounds.iter().enumerate() {
            let min2 = center_index2 - b.0;
            let max2 = center_index2 + b.1;
            columns[i1] = if i2 >= min2 && i2 <= max2 {
                max2 + 1 - i2
            } else {
                0
            };
        }
        let (start, end, height) = max_rectangle_location(&columns);
        let new_size1 = 1 + end - start;
        if new_size1 * height > size_axis1 * size_axis2 {
            min_axis1 = start;
            min_axis2 = i2;
            size_axis1 = new_size1;
            size_axis2 = height;
        }
    }
    let corner = step(
        center,
        negative1.opposite(),
        min_axis1 - center_index1 as i32,
    );
    let corner = step(corner, negative2.opposite(), min_axis2 - center_index2);
    FoundRectangle {
        min_corner: corner,
        axis1_size: size_axis1,
        axis2_size: size_axis2,
    }
}

/// `PortalShape.getRelativePosition`: where in the portal rectangle an entity
/// of `width` by `height` at `position` stands (right and up as fractions,
/// forward in blocks; living entities, minecarts and boats then drop the
/// forward part, `LivingEntity.resetForwardDirectionOfRelativePortalPosition`).
pub fn relative_position(
    rectangle: FoundRectangle,
    x_axis: bool,
    position: Vec3,
    width: f32,
    height: f32,
) -> Vec3 {
    let axis = if x_axis { Axis::X } else { Axis::Z };
    let room_width = f64::from(rectangle.axis1_size) - f64::from(width);
    let room_height = f64::from(rectangle.axis2_size) - f64::from(height);
    let bottom = rectangle.min_corner;
    let clamp = |v: f64| if v < 0.0 { 0.0 } else { v.min(1.0) };
    let right = if room_width > 0.0 {
        let start = f64::from(component(bottom, axis)) + f64::from(width) / 2.0;
        clamp((position.component(axis) - start) / room_width)
    } else {
        0.5
    };
    let up = if room_height > 0.0 {
        clamp((position.y - f64::from(bottom.y)) / room_height)
    } else {
        0.0
    };
    let forward_axis = if x_axis { Axis::Z } else { Axis::X };
    let forward =
        position.component(forward_axis) - (f64::from(component(bottom, forward_axis)) + 0.5);
    Vec3::new(right, up, forward)
}

/// `NetherPortalBlock.createDimensionTransition` before
/// `PortalShape.findCollisionFreePosition`: the bottom centre of the entity in
/// the exit portal and the yaw it turns by (0 or 90).
pub fn exit_position(
    exit: FoundRectangle,
    exit_x_axis: bool,
    entry_x_axis: bool,
    offset: Vec3,
    width: f32,
    height: f32,
) -> (Vec3, f32) {
    let rotation = if entry_x_axis == exit_x_axis {
        0.0
    } else {
        90.0
    };
    let right = f64::from(width) / 2.0 + (f64::from(exit.axis1_size) - f64::from(width)) * offset.x;
    let up = (f64::from(exit.axis2_size) - f64::from(height)) * offset.y;
    let forward = 0.5 + offset.z;
    let b = exit.min_corner;
    let target = Vec3::new(
        f64::from(b.x) + if exit_x_axis { right } else { forward },
        f64::from(b.y) + up,
        f64::from(b.z) + if exit_x_axis { forward } else { right },
    );
    (target, rotation)
}

impl BorderBounds {
    /// `WorldBorder.isWithinBounds(AABB)`: both corners, the far one pulled
    /// in by 1.0E-5F.
    pub fn contains_box(&self, b: Aabb) -> bool {
        let margin = f64::from(1.0e-5_f32);
        self.contains(b.min(Axis::X), b.min(Axis::Z))
            && self.contains(b.max(Axis::X) - margin, b.max(Axis::Z) - margin)
    }
}

/// `CollisionGetter.findFreePosition` without an entity's collision context:
/// within `allowed`, the point nearest `preferred` where a box of the size
/// centred there meets no block collision inside the border. `shape(pos)`
/// is the block's collision shape there, in world coordinates.
pub fn find_free_position(
    allowed: Aabb,
    preferred: Vec3,
    size: Vec3,
    border: &BorderBounds,
    mut shape: impl FnMut(BlockPos) -> Option<VoxelShape>,
) -> Option<Vec3> {
    let allowed_shape = VoxelShape::from_box(allowed).ok()?;
    if allowed_shape.is_empty() {
        return None;
    }
    let search = allowed_shape.bounds()?.inflate(size);
    let half = size.scale(0.5);
    let mut expanded = VoxelShape::empty();
    // BlockCollisions over the search area: every block whose shape meets it.
    let lo = |v: f64| (v - 1.0e-7).floor() as i32 - 1;
    let hi = |v: f64| (v + 1.0e-7).floor() as i32 + 1;
    for z in lo(search.min(Axis::Z))..=hi(search.max(Axis::Z)) {
        for y in lo(search.min(Axis::Y))..=hi(search.max(Axis::Y)) {
            for x in lo(search.min(Axis::X))..=hi(search.max(Axis::X)) {
                let Some(block) = shape(BlockPos::new(x, y, z)) else {
                    continue;
                };
                if block.is_empty()
                    || !block.boxes().iter().any(|b| b.intersects(search))
                    || !block.bounds().is_some_and(|b| border.contains_box(b))
                {
                    continue;
                }
                for b in block.boxes() {
                    let grown = VoxelShape::from_box(b.inflate(half)).ok()?;
                    expanded = expanded.join(&grown, ShapeOp::OR).ok()?;
                }
            }
        }
    }
    let free = allowed_shape.join(&expanded, ShapeOp::ONLY_FIRST).ok()?;
    free.closest_point(preferred)
}

/// `PortalShape.findCollisionFreePosition`: `bottom_center` moved to the
/// nearest spot within its column of the entity's width (and a block up)
/// where it fits; entities over 4 blocks stay put.
pub fn collision_free_position(
    bottom_center: Vec3,
    width: f32,
    height: f32,
    border: &BorderBounds,
    shape: impl FnMut(BlockPos) -> Option<VoxelShape>,
) -> Vec3 {
    if width > 4.0 || height > 4.0 {
        return bottom_center;
    }
    let half_height = f64::from(height) / 2.0;
    let center = bottom_center.add_vector(Vec3::new(0.0, half_height, 0.0));
    let w = f64::from(width);
    let allowed = Aabb::of_size(center, Vec3::new(w, 0.0, w))
        .expand_towards(Vec3::new(0.0, 1.0, 0.0))
        .inflate(Vec3::new(1.0e-6, 1.0e-6, 1.0e-6));
    find_free_position(
        allowed,
        center,
        Vec3::new(w, f64::from(height), w),
        border,
        shape,
    )
    .map_or(bottom_center, |p| {
        p.subtract(Vec3::new(0.0, half_height, 0.0))
    })
}

/// A portal block position as a `PoiRecord` in a `PoiSection`'s hash set.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Record(BlockPos);
impl JavaKey for Record {
    /// `Vec3i.hashCode`.
    fn java_hash(&self) -> i32 {
        let p = self.0;
        p.y.wrapping_add(p.z.wrapping_mul(31))
            .wrapping_mul(31)
            .wrapping_add(p.x)
    }
    fn tie_break(&self, other: &Self) -> Ordering {
        (self.0.y, self.0.z, self.0.x)
            .cmp(&(other.0.y, other.0.z, other.0.x))
            .then(Ordering::Less)
    }
}

/// `PortalForcer.findClosestPortalPosition` over the nether portal blocks
/// `portals` (the points of interest, in the order their sections hold
/// them): within 16 blocks of `approximate` going to the Nether, 128 out of
/// it, inside the border, nearest first, then lowest.
pub fn closest_portal(
    portals: &[BlockPos],
    approximate: BlockPos,
    to_nether: bool,
    border: &BorderBounds,
    min_section: i32,
    max_section: i32,
) -> Option<BlockPos> {
    let radius: i32 = if to_nether { 16 } else { 128 };
    // PoiManager.getInSquare: the chunks of ChunkPos.rangeClosed (x fastest),
    // each section from the bottom, each section's records in hash set order.
    let chunk_radius = radius.div_euclid(16) + 1;
    let (cx, cz) = (approximate.x >> 4, approximate.z >> 4);
    let mut sections: std::collections::HashMap<(i32, i32, i32), JavaHashMap<Record, ()>> =
        std::collections::HashMap::new();
    for &p in portals {
        sections
            .entry((p.x >> 4, p.y >> 4, p.z >> 4))
            .or_default()
            .put(Record(p), ());
    }
    let mut best: Option<(f64, BlockPos)> = None;
    for z in cz - chunk_radius..=cz + chunk_radius {
        for x in cx - chunk_radius..=cx + chunk_radius {
            for y in min_section..=max_section {
                let Some(set) = sections.get(&(x, y, z)) else {
                    continue;
                };
                for (record, _) in set.entries() {
                    let p = record.0;
                    if (p.x - approximate.x).abs() > radius
                        || (p.z - approximate.z).abs() > radius
                        || !border.contains_block(p)
                    {
                        continue;
                    }
                    let (dx, dy, dz) = (
                        f64::from(p.x) - f64::from(approximate.x),
                        f64::from(p.y) - f64::from(approximate.y),
                        f64::from(p.z) - f64::from(approximate.z),
                    );
                    let d = dx * dx + dy * dy + dz * dz;
                    if best.is_none_or(|(bd, bp)| d < bd || (d == bd && p.y < bp.y)) {
                        best = Some((d, p));
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

/// `BlockPos.spiralAround(center, radius, first, second)`.
fn spiral_around(
    center: BlockPos,
    radius: i32,
    first: Direction,
    second: Direction,
) -> Vec<BlockPos> {
    let directions = [first, second, first.opposite(), second.opposite()];
    let mut out = Vec::new();
    let mut last = step(center, second, 1);
    let legs = 4 * radius;
    let (mut leg, mut leg_size, mut leg_index) = (-1i32, 0, 0);
    loop {
        let cursor = step(last, directions[((leg + 4) % 4) as usize], 1);
        last = cursor;
        if leg_index >= leg_size {
            if leg >= legs {
                return out;
            }
            leg += 1;
            leg_index = 0;
            leg_size = leg / 2 + 1;
        }
        leg_index += 1;
        out.push(cursor);
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    /// `PortalForcer.canPortalReplaceBlock`.
    fn portal_can_replace(&self, pos: BlockPos) -> bool {
        let state = self.state(pos);
        self.b.is_replaceable(state) && self.b.fluid(state).kind() == FluidKind::Empty
    }
    /// `PortalForcer.canHostFrame`.
    fn can_host_frame(&self, origin: BlockPos, direction: Direction, offset: i32) -> bool {
        let side = clockwise(direction);
        for width in -1..3 {
            for height in -1..4 {
                let at = step(step(origin, direction, width), side, offset).offset(0, height, 0);
                if height < 0 && !self.b.is_solid(self.state(at)) {
                    return false;
                }
                if height >= 0 && !self.portal_can_replace(at) {
                    return false;
                }
            }
        }
        true
    }
    /// `PortalForcer.createPortal`: a 2 by 3 portal in an obsidian frame at the
    /// nearest column around `origin` with room for it (on a platform it
    /// carves out when none has), facing `x_axis`.
    pub fn create_portal(
        &mut self,
        origin: BlockPos,
        x_axis: bool,
        border: &BorderBounds,
        logical_height: i32,
    ) -> Option<FoundRectangle> {
        let direction = if x_axis {
            Direction::East
        } else {
            Direction::South
        };
        let (min_y, max_y) = (self.level.min_y(), self.level.max_y() - 1);
        let max_placeable = max_y.min(min_y + logical_height - 1);
        let distance = |a: BlockPos, b: BlockPos| {
            let (dx, dy, dz) = (
                f64::from(a.x) - f64::from(b.x),
                f64::from(a.y) - f64::from(b.y),
                f64::from(a.z) - f64::from(b.z),
            );
            dx * dx + dy * dy + dz * dz
        };
        let mut full: Option<(f64, BlockPos)> = None;
        let mut partial: Option<(f64, BlockPos)> = None;
        for column in spiral_around(origin, 16, Direction::East, Direction::South) {
            let top = self.b.motion_blocking_top(&*self.level, column).y;
            let height = max_placeable.min(top);
            if !border.contains_block(column) || !border.contains_block(step(column, direction, 1))
            {
                continue;
            }
            let mut y = height;
            while y >= min_y {
                let at = BlockPos::new(column.x, y, column.z);
                if self.portal_can_replace(at) {
                    let first_empty = y;
                    while y > min_y
                        && self.portal_can_replace(BlockPos::new(column.x, y - 1, column.z))
                    {
                        y -= 1;
                    }
                    if y + 4 <= max_placeable {
                        let delta = first_empty - y;
                        if delta <= 0 || delta >= 3 {
                            let at = BlockPos::new(column.x, y, column.z);
                            if self.can_host_frame(at, direction, 0) {
                                let d = distance(origin, at);
                                if self.can_host_frame(at, direction, -1)
                                    && self.can_host_frame(at, direction, 1)
                                    && full.is_none_or(|(fd, _)| fd > d)
                                {
                                    full = Some((d, at));
                                }
                                if full.is_none() && partial.is_none_or(|(pd, _)| pd > d) {
                                    partial = Some((d, at));
                                }
                            }
                        }
                    }
                }
                y -= 1;
            }
        }
        let b = self.b;
        let obsidian = b.default_of(b.named.obsidian);
        let spot = match full.or(partial) {
            Some((_, spot)) => spot,
            None => {
                let min_start = (min_y + 1).max(70);
                let max_start = max_placeable - 9;
                if max_start < min_start {
                    return None;
                }
                let [sx, _, sz] = direction.normal();
                let y = origin.y.clamp(min_start, max_start);
                let spot = border.clamp(
                    f64::from(origin.x - sx),
                    f64::from(y),
                    f64::from(origin.z - sz),
                );
                let side = clockwise(direction);
                let air = b.default_of(b.named.air);
                for side_offset in -1..2 {
                    for width in 0..2 {
                        for height in -1..3 {
                            let at = step(step(spot, direction, width), side, side_offset)
                                .offset(0, height, 0);
                            self.set_block(at, if height < 0 { obsidian } else { air }, 3);
                        }
                    }
                }
                spot
            }
        };
        for width in -1..3 {
            for height in -1..4 {
                if width == -1 || width == 2 || height == -1 || height == 3 {
                    self.set_block(
                        step(spot, direction, width).offset(0, height, 0),
                        obsidian,
                        3,
                    );
                }
            }
        }
        let base = b.default_of(b.named.nether_portal);
        let portal = b
            .table
            .with_text(base, b.props.axis, if x_axis { "x" } else { "z" })
            .unwrap_or(base);
        for width in 0..2 {
            for height in 0..3 {
                self.set_block(
                    step(spot, direction, width).offset(0, height, 0),
                    portal,
                    18,
                );
            }
        }
        Some(FoundRectangle {
            min_corner: spot,
            axis1_size: 2,
            axis2_size: 3,
        })
    }
}

impl Behaviours {
    /// `PortalForcer.createPortal` on `level` (see `Sim::create_portal`).
    pub fn create_portal<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        origin: BlockPos,
        x_axis: bool,
        border: &BorderBounds,
        logical_height: i32,
    ) -> Option<FoundRectangle> {
        self.sim(level, ticks)
            .create_portal(origin, x_axis, border, logical_height)
    }
    /// The axis of a nether portal state: Some(true) for X, Some(false) for
    /// Z, None for other blocks.
    pub fn portal_axis(&self, state: BlockStateId) -> Option<bool> {
        self.is_block(state, self.named.nether_portal)
            .then(|| self.table.text(state, self.props.axis) == "x")
    }
}
