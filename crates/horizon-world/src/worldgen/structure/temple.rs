//! Desert pyramids and jungle temples (DesertPyramidPiece, JungleTemplePiece).
use super::{
    BoundingBox,
    legacy::{HORIZONTAL, LegacyPiece, PieceContext, Result, random_horizontal},
    simple::Scattered,
};
use crate::nbt::{Compound, Tag};
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, PositionalRandom, RandomSource},
};
use std::sync::Mutex;

#[derive(Debug)]
pub struct DesertPyramidPiece {
    pub base: Scattered,
    placed_chest: Mutex<[bool; 4]>,
}
impl DesertPyramidPiece {
    pub fn new(random: &mut impl RandomSource, west: i32, north: i32) -> Self {
        let direction = random_horizontal(random);
        Self {
            base: Scattered::new(west, 64, north, 21, 15, 21, direction),
            placed_chest: Mutex::new([false; 4]),
        }
    }
    /// getPotentialSuspiciousSandWorldPositions, in placement order.
    fn sand_positions(&self) -> Vec<[i32; 3]> {
        let f = self.base.frame();
        let (x, y, z) = (16, -4, 13);
        let mut out = Vec::new();
        for py in y + 1..=y + 3 {
            for px in x - 2..=x + 2 {
                for pz in z - 2..=z + 2 {
                    out.push(f.world_pos(px, py, pz));
                }
            }
        }
        for [px, pz] in [[x + 3, z], [x - 3, z], [x, z + 3], [x, z - 3]] {
            out.push(f.world_pos(px, y + 1, pz));
            out.push(f.world_pos(px, y + 2, pz));
        }
        out
    }
    fn collapsed_roof(&self, seed: i64) -> [i32; 3] {
        let f = self.base.frame();
        let [x0, y0, z0] = f.world_pos(14, 0, 11);
        let mut random = LegacyRandom::new(seed)
            .fork_positional()
            .at(BlockPos::new(x0, y0, z0));
        let rx = 14 + random.next_int_bound(5).unwrap_or(0);
        let rz = 11 + random.next_int_bound(5).unwrap_or(0);
        f.world_pos(rx, 0, rz)
    }
}
pub fn union(boxes: &[BoundingBox]) -> BoundingBox {
    let mut out = boxes[0];
    for b in &boxes[1..] {
        out.encapsulate(b);
    }
    out
}
fn suspicious_sand(c: &mut PieceContext<'_>, pos: [i32; 3]) -> Result<()> {
    if c.chunk_box.is_inside(pos) {
        c.world.set(pos, c.state("suspicious_sand"), 2)?;
        let mut nbt = Compound::new();
        nbt.insert("id".into(), Tag::String("minecraft:brushable_block".into()));
        nbt.insert(
            "LootTable".into(),
            Tag::String("minecraft:archaeology/desert_pyramid".into()),
        );
        nbt.insert(
            "LootTableSeed".into(),
            Tag::Long(BlockPos::new(pos[0], pos[1], pos[2]).pack()),
        );
        c.world.block_entity(pos, nbt)?;
    }
    Ok(())
}
impl LegacyPiece for DesertPyramidPiece {
    fn bounding_box(&self) -> BoundingBox {
        self.base.frame().bounding_box
    }
    fn save(&self) -> Option<crate::nbt::Compound> {
        let mut tag = self.base.saved("minecraft:tedp");
        for (i, placed) in self.placed_chest.lock().unwrap().iter().enumerate() {
            tag.insert(
                format!("hasPlacedChest{i}").as_str().into(),
                crate::nbt::Tag::Byte(i8::from(*placed)),
            );
        }
        Some(tag)
    }
    fn type_name(&self) -> &'static str {
        "minecraft:tedp"
    }
    fn post_process(&self, c: &mut PieceContext<'_>) -> Result<()> {
        let offset = -c.random.int(3);
        if !self.base.lowest_ground(c, offset)? {
            return Ok(());
        }
        let f = self.base.frame();
        let (w, d) = (21, 21);
        let sandstone = c.state("sandstone");
        let cut = c.state("cut_sandstone");
        let chiseled = c.state("chiseled_sandstone");
        let air = c.state("air");
        let orange = c.state("orange_terracotta");
        let blue = c.state("blue_terracotta");
        let stairs = |c: &PieceContext<'_>, facing: &str| {
            c.state_with("sandstone_stairs", &[("facing", facing)])
        };
        let north = stairs(c, "north");
        let south = stairs(c, "south");
        let east = stairs(c, "east");
        let west = stairs(c, "west");
        f.generate_box(
            c,
            [0, -4, 0],
            [w - 1, 0, d - 1],
            sandstone,
            sandstone,
            false,
        )?;
        for p in 1..=9 {
            f.generate_box(
                c,
                [p, p, p],
                [w - 1 - p, p, d - 1 - p],
                sandstone,
                sandstone,
                false,
            )?;
            f.generate_box(
                c,
                [p + 1, p, p + 1],
                [w - 2 - p, p, d - 2 - p],
                air,
                air,
                false,
            )?;
        }
        for x in 0..w {
            for z in 0..d {
                f.fill_column_down(c, sandstone, x, -5, z)?;
            }
        }
        f.generate_box(c, [0, 0, 0], [4, 9, 4], sandstone, air, false)?;
        f.generate_box(c, [1, 10, 1], [3, 10, 3], sandstone, sandstone, false)?;
        f.place(c, north, 2, 10, 0)?;
        f.place(c, south, 2, 10, 4)?;
        f.place(c, east, 0, 10, 2)?;
        f.place(c, west, 4, 10, 2)?;
        f.generate_box(c, [w - 5, 0, 0], [w - 1, 9, 4], sandstone, air, false)?;
        f.generate_box(
            c,
            [w - 4, 10, 1],
            [w - 2, 10, 3],
            sandstone,
            sandstone,
            false,
        )?;
        f.place(c, north, w - 3, 10, 0)?;
        f.place(c, south, w - 3, 10, 4)?;
        f.place(c, east, w - 5, 10, 2)?;
        f.place(c, west, w - 1, 10, 2)?;
        f.generate_box(c, [8, 0, 0], [12, 4, 4], sandstone, air, false)?;
        f.generate_box(c, [9, 1, 0], [11, 3, 4], air, air, false)?;
        for [x, y] in [[9, 1], [9, 2], [9, 3], [10, 3], [11, 3], [11, 2], [11, 1]] {
            f.place(c, cut, x, y, 1)?;
        }
        f.generate_box(c, [4, 1, 1], [8, 3, 3], sandstone, air, false)?;
        f.generate_box(c, [4, 1, 2], [8, 2, 2], air, air, false)?;
        f.generate_box(c, [12, 1, 1], [16, 3, 3], sandstone, air, false)?;
        f.generate_box(c, [12, 1, 2], [16, 2, 2], air, air, false)?;
        f.generate_box(c, [5, 4, 5], [w - 6, 4, d - 6], sandstone, sandstone, false)?;
        f.generate_box(c, [9, 4, 9], [11, 4, 11], air, air, false)?;
        for [x, z] in [[8, 8], [12, 8], [8, 12], [12, 12]] {
            f.generate_box(c, [x, 1, z], [x, 3, z], cut, cut, false)?;
        }
        f.generate_box(c, [1, 1, 5], [4, 4, 11], sandstone, sandstone, false)?;
        f.generate_box(
            c,
            [w - 5, 1, 5],
            [w - 2, 4, 11],
            sandstone,
            sandstone,
            false,
        )?;
        f.generate_box(c, [6, 7, 9], [6, 7, 11], sandstone, sandstone, false)?;
        f.generate_box(
            c,
            [w - 7, 7, 9],
            [w - 7, 7, 11],
            sandstone,
            sandstone,
            false,
        )?;
        f.generate_box(c, [5, 5, 9], [5, 7, 11], cut, cut, false)?;
        f.generate_box(c, [w - 6, 5, 9], [w - 6, 7, 11], cut, cut, false)?;
        f.place(c, air, 5, 5, 10)?;
        f.place(c, air, 5, 6, 10)?;
        f.place(c, air, 6, 6, 10)?;
        f.place(c, air, w - 6, 5, 10)?;
        f.place(c, air, w - 6, 6, 10)?;
        f.place(c, air, w - 7, 6, 10)?;
        f.generate_box(c, [2, 4, 4], [2, 6, 4], air, air, false)?;
        f.generate_box(c, [w - 3, 4, 4], [w - 3, 6, 4], air, air, false)?;
        f.place(c, north, 2, 4, 5)?;
        f.place(c, north, 2, 3, 4)?;
        f.place(c, north, w - 3, 4, 5)?;
        f.place(c, north, w - 3, 3, 4)?;
        f.generate_box(c, [1, 1, 3], [2, 2, 3], sandstone, sandstone, false)?;
        f.generate_box(c, [w - 3, 1, 3], [w - 2, 2, 3], sandstone, sandstone, false)?;
        f.place(c, sandstone, 1, 1, 2)?;
        f.place(c, sandstone, w - 2, 1, 2)?;
        let slab = c.state("sandstone_slab");
        f.place(c, slab, 1, 2, 2)?;
        f.place(c, slab, w - 2, 2, 2)?;
        f.place(c, west, 2, 1, 2)?;
        f.place(c, east, w - 3, 1, 2)?;
        f.generate_box(c, [4, 3, 5], [4, 3, 17], sandstone, sandstone, false)?;
        f.generate_box(
            c,
            [w - 5, 3, 5],
            [w - 5, 3, 17],
            sandstone,
            sandstone,
            false,
        )?;
        f.generate_box(c, [3, 1, 5], [4, 2, 16], air, air, false)?;
        f.generate_box(c, [w - 6, 1, 5], [w - 5, 2, 16], air, air, false)?;
        for z in (5..=17).step_by(2) {
            f.place(c, cut, 4, 1, z)?;
            f.place(c, chiseled, 4, 2, z)?;
            f.place(c, cut, w - 5, 1, z)?;
            f.place(c, chiseled, w - 5, 2, z)?;
        }
        for [x, z] in [
            [10, 7],
            [10, 8],
            [9, 9],
            [11, 9],
            [8, 10],
            [12, 10],
            [7, 10],
            [13, 10],
            [9, 11],
            [11, 11],
            [10, 12],
            [10, 13],
        ] {
            f.place(c, orange, x, 0, z)?;
        }
        f.place(c, blue, 10, 0, 10)?;
        // Rows of the corner towers (y 2..=8, three blocks each).
        let rows = [
            [cut, orange, cut],
            [cut, orange, cut],
            [orange, chiseled, orange],
            [cut, orange, cut],
            [orange, chiseled, orange],
            [orange, orange, orange],
            [cut, cut, cut],
        ];
        for x in [0, w - 1] {
            for (i, row) in rows.iter().enumerate() {
                for (j, &state) in row.iter().enumerate() {
                    f.place(c, state, x, 2 + i as i32, 1 + j as i32)?;
                }
            }
        }
        for x in [2, w - 3] {
            for (i, row) in rows.iter().enumerate() {
                for (j, &state) in row.iter().enumerate() {
                    f.place(c, state, x - 1 + j as i32, 2 + i as i32, 0)?;
                }
            }
        }
        f.generate_box(c, [8, 4, 0], [12, 6, 0], cut, cut, false)?;
        f.place(c, air, 8, 6, 0)?;
        f.place(c, air, 12, 6, 0)?;
        f.place(c, orange, 9, 5, 0)?;
        f.place(c, chiseled, 10, 5, 0)?;
        f.place(c, orange, 11, 5, 0)?;
        f.generate_box(c, [8, -14, 8], [12, -11, 12], cut, cut, false)?;
        f.generate_box(c, [8, -10, 8], [12, -10, 12], chiseled, chiseled, false)?;
        f.generate_box(c, [8, -9, 8], [12, -9, 12], cut, cut, false)?;
        f.generate_box(c, [8, -8, 8], [12, -1, 12], sandstone, sandstone, false)?;
        f.generate_box(c, [9, -11, 9], [11, -1, 11], air, air, false)?;
        f.place(c, c.state("stone_pressure_plate"), 10, -11, 10)?;
        f.generate_box(c, [9, -13, 9], [11, -13, 11], c.state("tnt"), air, false)?;
        for [x, z, ox, oz] in [
            [8, 10, 7, 10],
            [12, 10, 13, 10],
            [10, 8, 10, 7],
            [10, 12, 10, 13],
        ] {
            f.place(c, air, x, -11, z)?;
            f.place(c, air, x, -10, z)?;
            f.place(c, chiseled, ox, -10, oz)?;
            f.place(c, cut, ox, -11, oz)?;
        }
        for (i, d) in HORIZONTAL.into_iter().enumerate() {
            if !self.placed_chest.lock().unwrap()[i] {
                let s = d.normal();
                let placed = f.create_chest(
                    c,
                    10 + s[0] * 2,
                    -11,
                    10 + s[2] * 2,
                    "minecraft:chests/desert_pyramid",
                )?;
                self.placed_chest.lock().unwrap()[i] = placed;
            }
        }
        // addCellar: stairs, then the room.
        let (x, y, z) = (16, -4, 13);
        let ccw = stairs(c, "west");
        f.place(c, ccw, 13, -1, 17)?;
        f.place(c, ccw, 14, -2, 17)?;
        f.place(c, ccw, 15, -3, 17)?;
        let sand = c.state("sand");
        let variant = c.world.level_boolean()?;
        for dx in -4..=0 {
            f.place(c, sand, x + dx, y + 4, z + 4)?;
        }
        f.place(c, sand, x - 2, y + 3, z + 4)?;
        f.place(
            c,
            if variant { sand } else { sandstone },
            x - 1,
            y + 3,
            z + 4,
        )?;
        f.place(c, if variant { sandstone } else { sand }, x, y + 3, z + 4)?;
        f.place(c, sand, x - 1, y + 2, z + 4)?;
        f.place(c, sandstone, x, y + 2, z + 4)?;
        f.place(c, sand, x, y + 1, z + 4)?;
        for (state, yy) in [(cut, y + 1), (chiseled, y + 2), (cut, -1)] {
            f.generate_box(
                c,
                [x - 3, yy, z - 3],
                [x - 3, yy, z + 2],
                state,
                state,
                true,
            )?;
            f.generate_box(
                c,
                [x + 3, yy, z - 3],
                [x + 3, yy, z + 2],
                state,
                state,
                true,
            )?;
            f.generate_box(
                c,
                [x - 3, yy, z - 3],
                [x + 3, yy, z - 2],
                state,
                state,
                true,
            )?;
            f.generate_box(
                c,
                [x - 3, yy, z + 3],
                [x + 3, yy, z + 3],
                state,
                state,
                true,
            )?;
        }
        for px in x - 2..=x + 2 {
            for pz in z - 2..=z + 2 {
                let state = if c.world.level_float()? < 0.33 {
                    sandstone
                } else {
                    sand
                };
                f.place(c, state, px, y + 4, pz)?;
            }
        }
        f.place(c, blue, x, y, z)?;
        for [dx, dz] in [
            [1, -1],
            [1, 1],
            [-1, -1],
            [-1, 1],
            [2, 0],
            [-2, 0],
            [0, 2],
            [0, -2],
        ] {
            f.place(c, orange, x + dx, y, z + dz)?;
        }
        f.place(c, orange, x + 3, y, z)?;
        f.place(c, cut, x + 4, y + 1, z)?;
        f.place(c, chiseled, x + 4, y + 2, z)?;
        f.place(c, orange, x - 3, y, z)?;
        f.place(c, cut, x - 4, y + 1, z)?;
        f.place(c, chiseled, x - 4, y + 2, z)?;
        f.place(c, orange, x, y, z + 3)?;
        f.place(c, orange, x, y, z - 3)?;
        f.place(c, cut, x, y + 1, z - 4)?;
        f.place(c, chiseled, x, -2, z - 4)?;
        Ok(())
    }
    /// DesertPyramidStructure.afterPlace.
    fn after_place(&self, c: &mut PieceContext<'_>, pieces: &[BoundingBox]) -> Result<()> {
        if *self.base.height_position.lock().unwrap() < 0 {
            return Ok(());
        }
        let seed = c.world.seed()?;
        let mut positions = self.sand_positions();
        positions.sort_by_key(|p| (p[1], p[2], p[0]));
        positions.dedup();
        suspicious_sand(c, self.collapsed_roof(seed))?;
        let center = union(pieces).center();
        let mut random = LegacyRandom::new(seed)
            .fork_positional()
            .at(BlockPos::new(center[0], center[1], center[2]));
        for i in (2..=positions.len()).rev() {
            let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
            positions.swap(i - 1, j);
        }
        let mut remaining = (positions.len() as i32).min(5 + random.next_int_bound(3).unwrap_or(0));
        let sand = c.state("sand");
        for pos in positions {
            if remaining > 0 {
                remaining -= 1;
                suspicious_sand(c, pos)?;
            } else if c.chunk_box.is_inside(pos) {
                c.world.set(pos, sand, 2)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct JungleTemplePiece {
    pub base: Scattered,
    /// placedMainChest, placedHiddenChest, placedTrap1, placedTrap2.
    placed: Mutex<[bool; 4]>,
}
impl JungleTemplePiece {
    pub fn new(random: &mut impl RandomSource, west: i32, north: i32) -> Self {
        let direction = random_horizontal(random);
        Self {
            base: Scattered::new(west, 64, north, 12, 10, 15, direction),
            placed: Mutex::new([false; 4]),
        }
    }
}
impl LegacyPiece for JungleTemplePiece {
    fn bounding_box(&self) -> BoundingBox {
        self.base.frame().bounding_box
    }
    fn save(&self) -> Option<crate::nbt::Compound> {
        let mut tag = self.base.saved("minecraft:tejp");
        let placed = *self.placed.lock().unwrap();
        for (key, value) in [
            "placedMainChest",
            "placedHiddenChest",
            "placedTrap1",
            "placedTrap2",
        ]
        .into_iter()
        .zip(placed)
        {
            tag.insert(key.into(), crate::nbt::Tag::Byte(i8::from(value)));
        }
        Some(tag)
    }
    fn type_name(&self) -> &'static str {
        "minecraft:tejp"
    }
    fn post_process(&self, c: &mut PieceContext<'_>) -> Result<()> {
        if !self.base.average_ground(c, 0)? {
            return Ok(());
        }
        let f = self.base.frame();
        let cobble = c.state("cobblestone");
        let mossy = c.state("mossy_cobblestone");
        let air = c.state("air");
        let stone = |c: &mut PieceContext<'_>, a: [i32; 3], b: [i32; 3]| {
            f.generate_box_selector(c, a, b, false, &mut |c, _, _, _, _| {
                if c.random.float() < 0.4 {
                    cobble
                } else {
                    mossy
                }
            })
        };
        stone(c, [0, -4, 0], [11, 0, 14])?;
        for (a, b) in [
            ([2, 1, 2], [9, 2, 2]),
            ([2, 1, 12], [9, 2, 12]),
            ([2, 1, 3], [2, 2, 11]),
            ([9, 1, 3], [9, 2, 11]),
            ([1, 3, 1], [10, 6, 1]),
            ([1, 3, 13], [10, 6, 13]),
            ([1, 3, 2], [1, 6, 12]),
            ([10, 3, 2], [10, 6, 12]),
            ([2, 3, 2], [9, 3, 12]),
            ([2, 6, 2], [9, 6, 12]),
            ([3, 7, 3], [8, 7, 11]),
            ([4, 8, 4], [7, 8, 10]),
        ] {
            stone(c, a, b)?;
        }
        for (a, b) in [
            ([3, 1, 3], [8, 2, 11]),
            ([4, 3, 6], [7, 3, 9]),
            ([2, 4, 2], [9, 5, 12]),
            ([4, 6, 5], [7, 6, 9]),
            ([5, 7, 6], [6, 7, 8]),
            ([5, 1, 2], [6, 2, 2]),
            ([5, 2, 12], [6, 2, 12]),
            ([5, 5, 1], [6, 5, 1]),
            ([5, 5, 13], [6, 5, 13]),
        ] {
            f.generate_box(c, a, b, air, air, false)?;
        }
        f.place(c, air, 1, 5, 5)?;
        f.place(c, air, 10, 5, 5)?;
        f.place(c, air, 1, 5, 9)?;
        f.place(c, air, 10, 5, 9)?;
        for z in [0, 14] {
            for x in [2, 4, 7, 9] {
                stone(c, [x, 4, z], [x, 5, z])?;
            }
        }
        stone(c, [5, 6, 0], [6, 6, 0])?;
        for x in [0, 11] {
            for z in (2..=12).step_by(2) {
                stone(c, [x, 4, z], [x, 5, z])?;
            }
            stone(c, [x, 6, 5], [x, 6, 5])?;
            stone(c, [x, 6, 9], [x, 6, 9])?;
        }
        for (a, b) in [
            ([2, 7, 2], [2, 9, 2]),
            ([9, 7, 2], [9, 9, 2]),
            ([2, 7, 12], [2, 9, 12]),
            ([9, 7, 12], [9, 9, 12]),
            ([4, 9, 4], [4, 9, 4]),
            ([7, 9, 4], [7, 9, 4]),
            ([4, 9, 10], [4, 9, 10]),
            ([7, 9, 10], [7, 9, 10]),
            ([5, 9, 7], [6, 9, 7]),
        ] {
            stone(c, a, b)?;
        }
        let stairs = |c: &PieceContext<'_>, facing: &str| {
            c.state_with("cobblestone_stairs", &[("facing", facing)])
        };
        let east = stairs(c, "east");
        let west = stairs(c, "west");
        let south = stairs(c, "south");
        let north = stairs(c, "north");
        f.place(c, north, 5, 9, 6)?;
        f.place(c, north, 6, 9, 6)?;
        f.place(c, south, 5, 9, 8)?;
        f.place(c, south, 6, 9, 8)?;
        for [x, y, z] in [
            [4, 0, 0],
            [5, 0, 0],
            [6, 0, 0],
            [7, 0, 0],
            [4, 1, 8],
            [4, 2, 9],
            [4, 3, 10],
            [7, 1, 8],
            [7, 2, 9],
            [7, 3, 10],
        ] {
            f.place(c, north, x, y, z)?;
        }
        stone(c, [4, 1, 9], [4, 1, 9])?;
        stone(c, [7, 1, 9], [7, 1, 9])?;
        stone(c, [4, 1, 10], [7, 2, 10])?;
        stone(c, [5, 4, 5], [6, 4, 5])?;
        f.place(c, east, 4, 4, 5)?;
        f.place(c, west, 7, 4, 5)?;
        for i in 0..4 {
            f.place(c, south, 5, -i, 6 + i)?;
            f.place(c, south, 6, -i, 6 + i)?;
            f.generate_box(c, [5, -i, 7 + i], [6, -i, 9 + i], air, air, false)?;
        }
        f.generate_box(c, [1, -3, 12], [10, -1, 13], air, air, false)?;
        f.generate_box(c, [1, -3, 1], [3, -1, 13], air, air, false)?;
        f.generate_box(c, [1, -3, 1], [9, -1, 5], air, air, false)?;
        for z in (1..=13).step_by(2) {
            stone(c, [1, -3, z], [1, -2, z])?;
        }
        for z in (2..=12).step_by(2) {
            stone(c, [1, -1, z], [3, -1, z])?;
        }
        stone(c, [2, -2, 1], [5, -2, 1])?;
        stone(c, [7, -2, 1], [9, -2, 1])?;
        stone(c, [6, -3, 1], [6, -3, 1])?;
        stone(c, [6, -1, 1], [6, -1, 1])?;
        let hook = |c: &PieceContext<'_>, facing: &str| {
            c.state_with("tripwire_hook", &[("facing", facing), ("attached", "true")])
        };
        let wire = |c: &PieceContext<'_>, a: &str, b: &str| {
            c.state_with(
                "tripwire",
                &[(a, "true"), (b, "true"), ("attached", "true")],
            )
        };
        let redstone =
            |c: &PieceContext<'_>, sides: &[(&str, &str)]| c.state_with("redstone_wire", sides);
        f.place(c, hook(c, "east"), 1, -3, 8)?;
        f.place(c, hook(c, "west"), 4, -3, 8)?;
        f.place(c, wire(c, "east", "west"), 2, -3, 8)?;
        f.place(c, wire(c, "east", "west"), 3, -3, 8)?;
        let ns = redstone(c, &[("north", "side"), ("south", "side")]);
        for z in (2..=7).rev() {
            f.place(c, ns, 5, -3, z)?;
        }
        f.place(
            c,
            redstone(c, &[("north", "side"), ("west", "side")]),
            5,
            -3,
            1,
        )?;
        f.place(
            c,
            redstone(c, &[("east", "side"), ("west", "side")]),
            4,
            -3,
            1,
        )?;
        f.place(c, mossy, 3, -3, 1)?;
        let dispenser = "minecraft:chests/jungle_temple_dispenser";
        if !self.placed.lock().unwrap()[2] {
            let placed = f.create_dispenser(c, 3, -2, 1, Direction::North, dispenser)?;
            self.placed.lock().unwrap()[2] = placed;
        }
        f.place(c, c.state_with("vine", &[("south", "true")]), 3, -2, 2)?;
        f.place(c, hook(c, "north"), 7, -3, 1)?;
        f.place(c, hook(c, "south"), 7, -3, 5)?;
        for z in 2..=4 {
            f.place(c, wire(c, "north", "south"), 7, -3, z)?;
        }
        f.place(
            c,
            redstone(c, &[("east", "side"), ("west", "side")]),
            8,
            -3,
            6,
        )?;
        f.place(
            c,
            redstone(c, &[("west", "side"), ("south", "side")]),
            9,
            -3,
            6,
        )?;
        f.place(
            c,
            redstone(c, &[("north", "side"), ("south", "up")]),
            9,
            -3,
            5,
        )?;
        f.place(c, mossy, 9, -3, 4)?;
        f.place(c, ns, 9, -2, 4)?;
        if !self.placed.lock().unwrap()[3] {
            let placed = f.create_dispenser(c, 9, -2, 3, Direction::West, dispenser)?;
            self.placed.lock().unwrap()[3] = placed;
        }
        let vine_east = c.state_with("vine", &[("east", "true")]);
        f.place(c, vine_east, 8, -1, 3)?;
        f.place(c, vine_east, 8, -2, 3)?;
        if !self.placed.lock().unwrap()[0] {
            let placed = f.create_chest(c, 8, -3, 3, "minecraft:chests/jungle_temple")?;
            self.placed.lock().unwrap()[0] = placed;
        }
        for [x, y, z] in [
            [9, -3, 2],
            [8, -3, 1],
            [4, -3, 5],
            [5, -2, 5],
            [5, -1, 5],
            [6, -3, 5],
            [7, -2, 5],
            [7, -1, 5],
            [8, -3, 5],
        ] {
            f.place(c, mossy, x, y, z)?;
        }
        stone(c, [9, -1, 1], [9, -1, 5])?;
        f.generate_box(c, [8, -3, 8], [10, -1, 10], air, air, false)?;
        let chiseled = c.state("chiseled_stone_bricks");
        let lever = c.state_with("lever", &[("facing", "north"), ("face", "wall")]);
        for x in 8..=10 {
            f.place(c, chiseled, x, -2, 11)?;
        }
        for x in 8..=10 {
            f.place(c, lever, x, -2, 12)?;
        }
        stone(c, [8, -3, 8], [8, -3, 10])?;
        stone(c, [10, -3, 8], [10, -3, 10])?;
        f.place(c, mossy, 10, -2, 9)?;
        f.place(c, ns, 8, -2, 9)?;
        f.place(c, ns, 8, -2, 10)?;
        f.place(
            c,
            redstone(
                c,
                &[
                    ("north", "side"),
                    ("south", "side"),
                    ("east", "side"),
                    ("west", "side"),
                ],
            ),
            10,
            -1,
            9,
        )?;
        f.place(
            c,
            c.state_with("sticky_piston", &[("facing", "up")]),
            9,
            -2,
            8,
        )?;
        let piston_west = c.state_with("sticky_piston", &[("facing", "west")]);
        f.place(c, piston_west, 10, -2, 8)?;
        f.place(c, piston_west, 10, -1, 8)?;
        f.place(
            c,
            c.state_with("repeater", &[("facing", "north")]),
            10,
            -2,
            10,
        )?;
        if !self.placed.lock().unwrap()[1] {
            let placed = f.create_chest(c, 9, -3, 10, "minecraft:chests/jungle_temple")?;
            self.placed.lock().unwrap()[1] = placed;
        }
        Ok(())
    }
}
