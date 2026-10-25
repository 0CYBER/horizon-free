//! Ocean monuments (OceanMonumentStructure, OceanMonumentPieces).
use super::{
    BoundingBox,
    legacy::{LegacyPiece, PieceContext, PieceFrame, Result, make_bounding_box},
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, random::RandomSource};

const DOWN: usize = 0;
const UP: usize = 1;
const NORTH: usize = 2;
const SOUTH: usize = 3;
const WEST: usize = 4;
const EAST: usize = 5;
const OPPOSITE: [usize; 6] = [UP, DOWN, SOUTH, NORTH, EAST, WEST];
const STEPS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

#[derive(Clone, Debug)]
struct Room {
    index: i32,
    conn: [Option<usize>; 6],
    open: [bool; 6],
    claimed: bool,
    source: bool,
    scan: i32,
}
impl Room {
    fn new(index: i32) -> Self {
        Self {
            index,
            conn: [None; 6],
            open: [false; 6],
            claimed: false,
            source: false,
            scan: 0,
        }
    }
}
fn room_index(x: i32, y: i32, z: i32) -> i32 {
    y * 25 + z * 5 + x
}
struct Graph {
    rooms: Vec<Room>,
}
impl Graph {
    fn connect(&mut self, a: usize, d: usize, b: usize) {
        self.rooms[a].conn[d] = Some(b);
        self.rooms[b].conn[OPPOSITE[d]] = Some(a);
    }
    fn update_openings(&mut self, a: usize) {
        let conn = self.rooms[a].conn;
        self.rooms[a].open = conn.map(|c| c.is_some());
    }
    fn find_source(&mut self, a: usize, scan: i32) -> bool {
        if self.rooms[a].source {
            return true;
        }
        self.rooms[a].scan = scan;
        for i in 0..6 {
            if let Some(n) = self.rooms[a].conn[i]
                && self.rooms[a].open[i]
                && self.rooms[n].scan != scan
                && self.find_source(n, scan)
            {
                return true;
            }
        }
        false
    }
    fn conn(&self, a: usize, d: usize) -> usize {
        self.rooms[a].conn[d].unwrap_or(a)
    }
    fn open(&self, a: usize, d: usize) -> bool {
        self.rooms[a].open[d]
    }
    fn claim(&mut self, a: usize) {
        self.rooms[a].claimed = true;
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Entry,
    Core,
    DoubleX,
    DoubleXY,
    DoubleY,
    DoubleYZ,
    DoubleZ,
    Simple(i32),
    SimpleTop,
    Wing(i32),
    Penthouse,
}
#[derive(Debug)]
struct Child {
    kind: Kind,
    frame: PieceFrame,
    room: usize,
}

#[derive(Debug)]
pub struct MonumentBuilding {
    frame: PieceFrame,
    rooms: Vec<Room>,
    children: Vec<Child>,
}
fn room_box(direction: Direction, index: i32, w: i32, h: i32, d: i32) -> BoundingBox {
    let (rx, rz, ry) = (index % 5, index / 5 % 5, index / 25);
    let b = make_bounding_box(0, 0, 0, direction, w * 8, h * 4, d * 8);
    b.moved(match direction {
        Direction::North => [rx * 8, ry * 4, -(rz + d) * 8 + 1],
        Direction::South => [rx * 8, ry * 4, rz * 8],
        Direction::West => [-(rz + d) * 8 + 1, ry * 4, rx * 8],
        _ => [rz * 8, ry * 4, rx * 8],
    })
}
impl MonumentBuilding {
    pub fn new(
        random: &mut impl RandomSource,
        west: i32,
        north: i32,
        direction: Direction,
    ) -> Self {
        let frame = PieceFrame::new(
            make_bounding_box(west, 39, north, direction, 58, 23, 58),
            Some(direction),
        );
        let mut g = Graph { rooms: Vec::new() };
        let mut grid: [Option<usize>; 75] = [None; 75];
        let mut add = |g: &mut Graph, x: i32, y: i32, z: i32| {
            let i = room_index(x, y, z);
            grid[i as usize] = Some(g.rooms.len());
            g.rooms.push(Room::new(i));
        };
        for x in 0..5 {
            for z in 0..4 {
                add(&mut g, x, 0, z);
            }
        }
        for x in 0..5 {
            for z in 0..4 {
                add(&mut g, x, 1, z);
            }
        }
        for x in 1..4 {
            for z in 0..2 {
                add(&mut g, x, 2, z);
            }
        }
        let source = grid[room_index(2, 0, 0) as usize].unwrap();
        for x in 0..5 {
            for z in 0..5 {
                for y in 0..3 {
                    let Some(a) = grid[room_index(x, y, z) as usize] else {
                        continue;
                    };
                    for (d, s) in STEPS.iter().enumerate() {
                        let (nx, ny, nz) = (x + s[0], y + s[1], z + s[2]);
                        if !(0..5).contains(&nx) || !(0..5).contains(&nz) || !(0..3).contains(&ny) {
                            continue;
                        }
                        if let Some(b) = grid[room_index(nx, ny, nz) as usize] {
                            let dir = if nz == z { d } else { OPPOSITE[d] };
                            g.connect(a, dir, b);
                        }
                    }
                }
            }
        }
        let roof = g.rooms.len();
        g.rooms.push(Room::new(1003));
        let left = g.rooms.len();
        g.rooms.push(Room::new(1001));
        let right = g.rooms.len();
        g.rooms.push(Room::new(1002));
        g.connect(grid[room_index(2, 2, 0) as usize].unwrap(), UP, roof);
        g.connect(grid[room_index(0, 1, 0) as usize].unwrap(), SOUTH, left);
        g.connect(grid[room_index(4, 1, 0) as usize].unwrap(), SOUTH, right);
        g.claim(roof);
        g.claim(left);
        g.claim(right);
        g.rooms[source].source = true;
        let core = grid[room_index(random.next_int_bound(4).unwrap_or(0), 0, 2) as usize].unwrap();
        let (ce, cn, cu) = (g.conn(core, EAST), g.conn(core, NORTH), g.conn(core, UP));
        let cen = g.conn(ce, NORTH);
        for r in [
            core,
            ce,
            cn,
            cen,
            cu,
            g.conn(ce, UP),
            g.conn(cn, UP),
            g.conn(cen, UP),
        ] {
            g.claim(r);
        }
        let mut defs = Vec::new();
        for r in grid.iter().flatten() {
            g.update_openings(*r);
            defs.push(*r);
        }
        g.update_openings(roof);
        for i in (2..=defs.len()).rev() {
            let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
            defs.swap(i - 1, j);
        }
        let mut scan = 1;
        for &def in &defs {
            let (mut closed, mut attempts) = (0, 0);
            while closed < 2 && attempts < 5 {
                attempts += 1;
                let f = random.next_int_bound(6).unwrap_or(0) as usize;
                if !g.rooms[def].open[f] {
                    continue;
                }
                let of = OPPOSITE[f];
                let other = g.conn(def, f);
                g.rooms[def].open[f] = false;
                g.rooms[other].open[of] = false;
                let a = g.find_source(def, scan);
                scan += 1;
                let ok = a && {
                    let b = g.find_source(other, scan);
                    scan += 1;
                    b
                };
                if ok {
                    closed += 1;
                } else {
                    g.rooms[def].open[f] = true;
                    g.rooms[other].open[of] = true;
                }
            }
        }
        defs.extend([roof, left, right]);
        g.claim(source);
        let mut children = Vec::new();
        let child = |kind: Kind, room: usize, g: &Graph, w: i32, h: i32, d: i32| Child {
            kind,
            frame: PieceFrame::new(
                room_box(direction, g.rooms[room].index, w, h, d),
                Some(direction),
            ),
            room,
        };
        children.push(child(Kind::Entry, source, &g, 1, 1, 1));
        children.push(child(Kind::Core, core, &g, 2, 2, 2));
        for def in defs {
            if g.rooms[def].claimed || g.rooms[def].index >= 75 {
                continue;
            }
            let free =
                |g: &Graph, r: usize, d: usize| g.open(r, d) && !g.rooms[g.conn(r, d)].claimed;
            if free(&g, def, EAST) && free(&g, def, UP) && free(&g, g.conn(def, EAST), UP) {
                let e = g.conn(def, EAST);
                for r in [def, e, g.conn(def, UP), g.conn(e, UP)] {
                    g.claim(r);
                }
                children.push(child(Kind::DoubleXY, def, &g, 2, 2, 1));
            } else if free(&g, def, NORTH) && free(&g, def, UP) && free(&g, g.conn(def, NORTH), UP)
            {
                let n = g.conn(def, NORTH);
                for r in [def, n, g.conn(def, UP), g.conn(n, UP)] {
                    g.claim(r);
                }
                children.push(child(Kind::DoubleYZ, def, &g, 1, 2, 2));
            } else if free(&g, def, NORTH) {
                g.claim(def);
                g.claim(g.conn(def, NORTH));
                children.push(child(Kind::DoubleZ, def, &g, 1, 1, 2));
            } else if free(&g, def, EAST) {
                g.claim(def);
                g.claim(g.conn(def, EAST));
                children.push(child(Kind::DoubleX, def, &g, 2, 1, 1));
            } else if free(&g, def, UP) {
                g.claim(def);
                g.claim(g.conn(def, UP));
                children.push(child(Kind::DoubleY, def, &g, 1, 2, 1));
            } else if [WEST, EAST, NORTH, SOUTH, UP]
                .iter()
                .all(|&d| !g.open(def, d))
            {
                g.claim(def);
                children.push(child(Kind::SimpleTop, def, &g, 1, 1, 1));
            } else {
                g.claim(def);
                let design = random.next_int_bound(3).unwrap_or(0);
                children.push(child(Kind::Simple(design), def, &g, 1, 1, 1));
            }
        }
        let offset = frame.world_pos(9, 0, 22);
        for c in &mut children {
            c.frame.bounding_box = c.frame.bounding_box.moved(offset);
        }
        let corners = |a: [i32; 3], b: [i32; 3]| {
            BoundingBox::from_corners(
                frame.world_pos(a[0], a[1], a[2]),
                frame.world_pos(b[0], b[1], b[2]),
            )
        };
        let wing_random = random.next_int();
        for (i, b) in [
            corners([1, 1, 1], [23, 8, 21]),
            corners([34, 1, 1], [56, 8, 21]),
        ]
        .into_iter()
        .enumerate()
        {
            children.push(Child {
                kind: Kind::Wing(wing_random.wrapping_add(i as i32) & 1),
                frame: PieceFrame::new(b, Some(direction)),
                room: source,
            });
        }
        children.push(Child {
            kind: Kind::Penthouse,
            frame: PieceFrame::new(corners([22, 13, 22], [35, 17, 35]), Some(direction)),
            room: source,
        });
        Self {
            frame,
            rooms: g.rooms,
            children,
        }
    }
}

/// OceanMonumentPiece helpers bound to one frame and chunk.
struct W<'x, 'c> {
    f: PieceFrame,
    c: &'x mut PieceContext<'c>,
    sea: i32,
    water: BlockStateId,
}
impl W<'_, '_> {
    /// OceanMonumentPiece.spawnElder.
    fn elder(&mut self, x: i32, y: i32, z: i32) -> Result<()> {
        let pos = self.f.world_pos(x, y, z);
        if self.c.chunk_box.is_inside(pos) {
            self.c.world.entity(super::legacy::spawn_nbt(
                "minecraft:elder_guardian",
                pos,
                true,
                // The ElderGuardian constructor's setPersistenceRequired.
                true,
            ))?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn bx(
        &mut self,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        s: BlockStateId,
    ) -> Result<()> {
        self.f
            .generate_box(self.c, [x0, y0, z0], [x1, y1, z1], s, s, false)
    }
    fn put(&mut self, s: BlockStateId, x: i32, y: i32, z: i32) -> Result<()> {
        self.f.place(self.c, s, x, y, z)
    }
    fn water(&mut self, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32) -> Result<()> {
        let air = self.c.state("air");
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    let s = self.f.get(self.c, x, y, z)?;
                    if matches!(
                        self.c.name(s),
                        "minecraft:ice"
                            | "minecraft:packed_ice"
                            | "minecraft:blue_ice"
                            | "minecraft:water"
                    ) {
                        continue;
                    }
                    let state = if self.f.world_y(y) >= self.sea && s != self.water {
                        air
                    } else {
                        self.water
                    };
                    self.f.place(self.c, state, x, y, z)?;
                }
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn fill_only(
        &mut self,
        x0: i32,
        y0: i32,
        z0: i32,
        x1: i32,
        y1: i32,
        z1: i32,
        s: BlockStateId,
    ) -> Result<()> {
        for y in y0..=y1 {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if self.f.get(self.c, x, y, z)? == self.water {
                        self.f.place(self.c, s, x, y, z)?;
                    }
                }
            }
        }
        Ok(())
    }
    fn floor(&mut self, xo: i32, zo: i32, down: bool) -> Result<()> {
        let (g, l) = (
            self.c.state("prismarine"),
            self.c.state("prismarine_bricks"),
        );
        if down {
            self.bx(xo, 0, zo, xo + 2, 0, zo + 7, g)?;
            self.bx(xo + 5, 0, zo, xo + 7, 0, zo + 7, g)?;
            self.bx(xo + 3, 0, zo, xo + 4, 0, zo + 2, g)?;
            self.bx(xo + 3, 0, zo + 5, xo + 4, 0, zo + 7, g)?;
            self.bx(xo + 3, 0, zo + 2, xo + 4, 0, zo + 2, l)?;
            self.bx(xo + 3, 0, zo + 5, xo + 4, 0, zo + 5, l)?;
            self.bx(xo + 2, 0, zo + 3, xo + 2, 0, zo + 4, l)?;
            self.bx(xo + 5, 0, zo + 3, xo + 5, 0, zo + 4, l)
        } else {
            self.bx(xo, 0, zo, xo + 7, 0, zo + 7, g)
        }
    }
    fn hits(&self, x0: i32, z0: i32, x1: i32, z1: i32) -> bool {
        let (ax, az) = (self.f.world_x(x0, z0), self.f.world_z(x0, z0));
        let (bx, bz) = (self.f.world_x(x1, z1), self.f.world_z(x1, z1));
        let b = self.c.chunk_box;
        b.max[0] >= ax.min(bx)
            && b.min[0] <= ax.max(bx)
            && b.max[2] >= az.min(bz)
            && b.min[2] <= az.max(bz)
    }
}
struct Palette {
    g: BlockStateId,
    l: BlockStateId,
    k: BlockStateId,
    lamp: BlockStateId,
}

impl LegacyPiece for MonumentBuilding {
    fn bounding_box(&self) -> BoundingBox {
        self.frame.bounding_box
    }
    fn type_name(&self) -> &'static str {
        "minecraft:omb"
    }
    /// `MonumentBuilding`: the base fields only; vanilla builds its rooms
    /// again after loading (`OceanMonumentStructure.regeneratePiecesAfterLoad`).
    fn save(&self) -> Option<crate::nbt::Compound> {
        Some(super::legacy::piece_tag(
            "minecraft:omb",
            self.frame.bounding_box,
            self.frame.orientation,
            0,
        ))
    }
    fn post_process(&self, c: &mut PieceContext<'_>) -> Result<()> {
        let sea = c.world.sea_level()?;
        let water = c.state("water");
        let p = Palette {
            g: c.state("prismarine"),
            l: c.state("prismarine_bricks"),
            k: c.state("dark_prismarine"),
            lamp: c.state("sea_lantern"),
        };
        let mut w = W {
            f: self.frame,
            c,
            sea,
            water,
        };
        let water_height = sea.max(64) - self.frame.bounding_box.min[1];
        w.water(0, 0, 0, 58, water_height, 58)?;
        building(&mut w, &p)?;
        let c = w.c;
        for child in &self.children {
            if child.frame.bounding_box.intersects(&c.chunk_box) {
                let mut w = W {
                    f: child.frame,
                    c: &mut *c,
                    sea,
                    water,
                };
                room(&mut w, &p, child, &self.rooms)?;
            }
        }
        Ok(())
    }
}

fn wing(w: &mut W<'_, '_>, p: &Palette, flipped: bool, xo: i32) -> Result<()> {
    let (g, l) = (p.g, p.l);
    if !w.hits(xo, 0, xo + 23, 20) {
        return Ok(());
    }
    w.bx(xo, 0, 0, xo + 24, 0, 20, g)?;
    w.water(xo, 1, 0, xo + 24, 10, 20)?;
    for i in 0..4 {
        w.bx(xo + i, i + 1, i, xo + i, i + 1, 20, l)?;
        w.bx(xo + i + 7, i + 5, i + 7, xo + i + 7, i + 5, 20, l)?;
        w.bx(xo + 17 - i, i + 5, i + 7, xo + 17 - i, i + 5, 20, l)?;
        w.bx(xo + 24 - i, i + 1, i, xo + 24 - i, i + 1, 20, l)?;
        w.bx(xo + i + 1, i + 1, i, xo + 23 - i, i + 1, i, l)?;
        w.bx(xo + i + 8, i + 5, i + 7, xo + 16 - i, i + 5, i + 7, l)?;
    }
    w.bx(xo + 4, 4, 4, xo + 6, 4, 20, g)?;
    w.bx(xo + 7, 4, 4, xo + 17, 4, 6, g)?;
    w.bx(xo + 18, 4, 4, xo + 20, 4, 20, g)?;
    w.bx(xo + 11, 8, 11, xo + 13, 8, 20, g)?;
    w.put(l, xo + 12, 9, 12)?;
    w.put(l, xo + 12, 9, 15)?;
    w.put(l, xo + 12, 9, 18)?;
    let left = xo + if flipped { 19 } else { 5 };
    let right = xo + if flipped { 5 } else { 19 };
    for z in (5..=20).rev().step_by(3) {
        w.put(l, left, 5, z)?;
    }
    for z in (7..=19).rev().step_by(3) {
        w.put(l, right, 5, z)?;
    }
    for i in 0..4 {
        let x = if flipped {
            xo + 24 - (17 - i * 3)
        } else {
            xo + 17 - i * 3
        };
        w.put(l, x, 5, 5)?;
    }
    w.put(l, right, 5, 5)?;
    w.bx(xo + 11, 1, 12, xo + 13, 7, 12, g)?;
    w.bx(xo + 12, 1, 11, xo + 12, 7, 13, g)
}
/// MonumentBuilding.postProcess after the first water box (without children).
fn building(w: &mut W<'_, '_>, p: &Palette) -> Result<()> {
    let (g, l, k, lamp) = (p.g, p.l, p.k, p.lamp);
    wing(w, p, false, 0)?;
    wing(w, p, true, 33)?;
    if w.hits(22, 5, 35, 17) {
        w.water(25, 0, 0, 32, 8, 20)?;
        for i in 0..4 {
            let z = 5 + i * 4;
            w.bx(24, 2, z, 24, 4, z, l)?;
            w.bx(22, 4, z, 23, 4, z, l)?;
            w.put(l, 25, 5, z)?;
            w.put(l, 26, 6, z)?;
            w.put(lamp, 26, 5, z)?;
            w.bx(33, 2, z, 33, 4, z, l)?;
            w.bx(34, 4, z, 35, 4, z, l)?;
            w.put(l, 32, 5, z)?;
            w.put(l, 31, 6, z)?;
            w.put(lamp, 31, 5, z)?;
            w.bx(27, 6, z, 30, 6, z, g)?;
        }
    }
    if w.hits(15, 20, 42, 21) {
        w.bx(15, 0, 21, 42, 0, 21, g)?;
        w.water(26, 1, 21, 31, 3, 21)?;
        for [x0, y, x1] in [[21, 12, 36], [17, 11, 40], [16, 10, 41]] {
            w.bx(x0, y, 21, x1, y, 21, g)?;
        }
        w.bx(15, 7, 21, 42, 9, 21, g)?;
        for [x0, y, x1] in [
            [16, 6, 41],
            [17, 5, 40],
            [21, 4, 36],
            [22, 3, 26],
            [31, 3, 35],
            [23, 2, 25],
            [32, 2, 34],
        ] {
            w.bx(x0, y, 21, x1, y, 21, g)?;
        }
        w.bx(28, 4, 20, 29, 4, 21, l)?;
        for [x, y] in [[27, 3], [30, 3], [26, 2], [31, 2], [25, 1], [32, 1]] {
            w.put(l, x, y, 21)?;
        }
        for i in 0..7 {
            w.put(k, 28 - i, 6 + i, 21)?;
            w.put(k, 29 + i, 6 + i, 21)?;
        }
        for i in 0..4 {
            w.put(k, 28 - i, 9 + i, 21)?;
            w.put(k, 29 + i, 9 + i, 21)?;
        }
        w.put(k, 28, 12, 21)?;
        w.put(k, 29, 12, 21)?;
        for i in 0..3 {
            w.put(k, 22 - i * 2, 8, 21)?;
            w.put(k, 22 - i * 2, 9, 21)?;
            w.put(k, 35 + i * 2, 8, 21)?;
            w.put(k, 35 + i * 2, 9, 21)?;
        }
        w.water(15, 13, 21, 42, 15, 21)?;
        for [x0, y1, x1] in [
            [15, 6, 15],
            [16, 5, 16],
            [17, 4, 20],
            [21, 3, 21],
            [22, 2, 22],
            [23, 1, 24],
            [42, 6, 42],
            [41, 5, 41],
            [37, 4, 40],
            [36, 3, 36],
            [33, 1, 34],
            [35, 2, 35],
        ] {
            w.water(x0, 1, 21, x1, y1, 21)?;
        }
    }
    if w.hits(21, 21, 36, 36) {
        w.bx(21, 0, 22, 36, 0, 36, g)?;
        w.water(21, 1, 22, 36, 23, 36)?;
        for i in 0..4 {
            w.bx(21 + i, 13 + i, 21 + i, 36 - i, 13 + i, 21 + i, l)?;
            w.bx(21 + i, 13 + i, 36 - i, 36 - i, 13 + i, 36 - i, l)?;
            w.bx(21 + i, 13 + i, 22 + i, 21 + i, 13 + i, 35 - i, l)?;
            w.bx(36 - i, 13 + i, 22 + i, 36 - i, 13 + i, 35 - i, l)?;
        }
        w.bx(25, 16, 25, 32, 16, 32, g)?;
        w.bx(25, 17, 25, 25, 19, 25, l)?;
        w.bx(32, 17, 25, 32, 19, 25, l)?;
        w.bx(25, 17, 32, 25, 19, 32, l)?;
        w.bx(32, 17, 32, 32, 19, 32, l)?;
        for [a, b, cx, cz] in [
            [26, 26, 27, 27],
            [26, 31, 27, 30],
            [31, 31, 30, 30],
            [31, 26, 30, 27],
        ] {
            w.put(l, a, 20, b)?;
            w.put(l, cx, 21, cz)?;
            w.put(lamp, cx, 20, cz)?;
        }
        w.bx(28, 21, 27, 29, 21, 27, g)?;
        w.bx(27, 21, 28, 27, 21, 29, g)?;
        w.bx(28, 21, 30, 29, 21, 30, g)?;
        w.bx(30, 21, 28, 30, 21, 29, g)?;
    }
    // generateLowerWall
    if w.hits(0, 21, 6, 58) {
        w.bx(0, 0, 21, 6, 0, 57, g)?;
        w.water(0, 1, 21, 6, 7, 57)?;
        w.bx(4, 4, 21, 6, 4, 53, g)?;
        for i in 0..4 {
            w.bx(i, i + 1, 21, i, i + 1, 57 - i, l)?;
        }
        for z in (23..53).step_by(3) {
            w.put(l, 5, 5, z)?;
        }
        w.put(l, 5, 5, 52)?;
        for i in 0..4 {
            w.bx(i, i + 1, 21, i, i + 1, 57 - i, l)?;
        }
        w.bx(4, 1, 52, 6, 3, 52, g)?;
        w.bx(5, 1, 51, 5, 3, 53, g)?;
    }
    if w.hits(51, 21, 58, 58) {
        w.bx(51, 0, 21, 57, 0, 57, g)?;
        w.water(51, 1, 21, 57, 7, 57)?;
        w.bx(51, 4, 21, 53, 4, 53, g)?;
        for i in 0..4 {
            w.bx(57 - i, i + 1, 21, 57 - i, i + 1, 57 - i, l)?;
        }
        for z in (23..53).step_by(3) {
            w.put(l, 52, 5, z)?;
        }
        w.put(l, 52, 5, 52)?;
        w.bx(51, 1, 52, 53, 3, 52, g)?;
        w.bx(52, 1, 51, 52, 3, 53, g)?;
    }
    if w.hits(0, 51, 57, 57) {
        w.bx(7, 0, 51, 50, 0, 57, g)?;
        w.water(7, 1, 51, 50, 10, 57)?;
        for i in 0..4 {
            w.bx(i + 1, i + 1, 57 - i, 56 - i, i + 1, 57 - i, l)?;
        }
    }
    // generateMiddleWall
    if w.hits(7, 21, 13, 50) {
        w.bx(7, 0, 21, 13, 0, 50, g)?;
        w.water(7, 1, 21, 13, 10, 50)?;
        w.bx(11, 8, 21, 13, 8, 53, g)?;
        for i in 0..4 {
            w.bx(i + 7, i + 5, 21, i + 7, i + 5, 54, l)?;
        }
        for z in (21..=45).step_by(3) {
            w.put(l, 12, 9, z)?;
        }
    }
    if w.hits(44, 21, 50, 54) {
        w.bx(44, 0, 21, 50, 0, 50, g)?;
        w.water(44, 1, 21, 50, 10, 50)?;
        w.bx(44, 8, 21, 46, 8, 53, g)?;
        for i in 0..4 {
            w.bx(50 - i, i + 5, 21, 50 - i, i + 5, 54, l)?;
        }
        for z in (21..=45).step_by(3) {
            w.put(l, 45, 9, z)?;
        }
    }
    if w.hits(8, 44, 49, 54) {
        w.bx(14, 0, 44, 43, 0, 50, g)?;
        w.water(14, 1, 44, 43, 10, 50)?;
        for x in (12..=45).step_by(3) {
            w.put(l, x, 9, 45)?;
            w.put(l, x, 9, 52)?;
            if matches!(x, 12 | 18 | 24 | 33 | 39 | 45) {
                for [y, z] in [
                    [9, 47],
                    [9, 50],
                    [10, 45],
                    [10, 46],
                    [10, 51],
                    [10, 52],
                    [11, 47],
                    [11, 50],
                    [12, 48],
                    [12, 49],
                ] {
                    w.put(l, x, y, z)?;
                }
            }
        }
        for i in 0..3 {
            w.bx(8 + i, 5 + i, 54, 49 - i, 5 + i, 54, g)?;
        }
        w.bx(11, 8, 54, 46, 8, 54, l)?;
        w.bx(14, 8, 44, 43, 8, 53, g)?;
    }
    // generateUpperWall
    if w.hits(14, 21, 20, 43) {
        w.bx(14, 0, 21, 20, 0, 43, g)?;
        w.water(14, 1, 22, 20, 14, 43)?;
        w.bx(18, 12, 22, 20, 12, 39, g)?;
        w.bx(18, 12, 21, 20, 12, 21, l)?;
        for i in 0..4 {
            w.bx(i + 14, i + 9, 21, i + 14, i + 9, 43 - i, l)?;
        }
        for z in (23..=39).step_by(3) {
            w.put(l, 19, 13, z)?;
        }
    }
    if w.hits(37, 21, 43, 43) {
        w.bx(37, 0, 21, 43, 0, 43, g)?;
        w.water(37, 1, 22, 43, 14, 43)?;
        w.bx(37, 12, 22, 39, 12, 39, g)?;
        w.bx(37, 12, 21, 39, 12, 21, l)?;
        for i in 0..4 {
            w.bx(43 - i, i + 9, 21, 43 - i, i + 9, 43 - i, l)?;
        }
        for z in (23..=39).step_by(3) {
            w.put(l, 38, 13, z)?;
        }
    }
    if w.hits(15, 37, 42, 43) {
        w.bx(21, 0, 37, 36, 0, 43, g)?;
        w.water(21, 1, 37, 36, 14, 43)?;
        w.bx(21, 12, 37, 36, 12, 39, g)?;
        for i in 0..4 {
            w.bx(15 + i, i + 9, 43 - i, 42 - i, i + 9, 43 - i, l)?;
        }
        for x in (21..=36).step_by(3) {
            w.put(l, x, 13, 38)?;
        }
    }
    for px in 0..7 {
        let mut pz = 0;
        while pz < 7 {
            if pz == 0 && px == 3 {
                pz = 6;
            }
            let (bx, bz) = (px * 9, pz * 9);
            for dx in 0..4 {
                for dz in 0..4 {
                    w.put(l, bx + dx, 0, bz + dz)?;
                    w.f.fill_column_down(w.c, l, bx + dx, -1, bz + dz)?;
                }
            }
            pz += if px != 0 && px != 6 { 6 } else { 1 };
        }
    }
    for i in 0..5 {
        w.water(-1 - i, i * 2, -1 - i, -1 - i, 23, 58 + i)?;
        w.water(58 + i, i * 2, -1 - i, 58 + i, 23, 58 + i)?;
        w.water(-i, i * 2, -1 - i, 57 + i, 23, -1 - i)?;
        w.water(-i, i * 2, 58 + i, 57 + i, 23, 58 + i)?;
    }
    Ok(())
}

/// Openings of the doorways shared by the double rooms: (room, side, water box).
fn doorways(w: &mut W<'_, '_>, list: &[(bool, [i32; 6])]) -> Result<()> {
    for (open, [x0, y0, z0, x1, y1, z1]) in list {
        if *open {
            w.water(*x0, *y0, *z0, *x1, *y1, *z1)?;
        }
    }
    Ok(())
}
fn room(w: &mut W<'_, '_>, p: &Palette, child: &Child, rooms: &[Room]) -> Result<()> {
    let (g, l, k, lamp) = (p.g, p.l, p.k, p.lamp);
    let r = &rooms[child.room];
    let conn = |a: &Room, d: usize| a.conn[d].map(|i| &rooms[i]);
    let upper = r.index / 25 > 0;
    match child.kind {
        Kind::Core => {
            w.fill_only(1, 8, 0, 14, 8, 14, g)?;
            w.bx(0, 7, 0, 0, 7, 15, l)?;
            w.bx(15, 7, 0, 15, 7, 15, l)?;
            w.bx(1, 7, 0, 15, 7, 0, l)?;
            w.bx(1, 7, 15, 14, 7, 15, l)?;
            for y in 1..=6 {
                let b = if y == 2 || y == 6 { g } else { l };
                for x in [0, 15] {
                    w.bx(x, y, 0, x, y, 1, b)?;
                    w.bx(x, y, 6, x, y, 9, b)?;
                    w.bx(x, y, 14, x, y, 15, b)?;
                }
                w.bx(1, y, 0, 1, y, 0, b)?;
                w.bx(6, y, 0, 9, y, 0, b)?;
                w.bx(14, y, 0, 14, y, 0, b)?;
                w.bx(1, y, 15, 14, y, 15, b)?;
            }
            w.bx(6, 3, 6, 9, 6, 9, k)?;
            let gold = w.c.state("gold_block");
            w.bx(7, 4, 7, 8, 5, 8, gold)?;
            for y in [3, 6] {
                for x in [6, 9] {
                    w.put(lamp, x, y, 6)?;
                    w.put(lamp, x, y, 9)?;
                }
            }
            for [x0, y0, z0, x1, y1, z1] in [
                [5, 1, 6, 5, 2, 6],
                [5, 1, 9, 5, 2, 9],
                [10, 1, 6, 10, 2, 6],
                [10, 1, 9, 10, 2, 9],
                [6, 1, 5, 6, 2, 5],
                [9, 1, 5, 9, 2, 5],
                [6, 1, 10, 6, 2, 10],
                [9, 1, 10, 9, 2, 10],
                [5, 2, 5, 5, 6, 5],
                [5, 2, 10, 5, 6, 10],
                [10, 2, 5, 10, 6, 5],
                [10, 2, 10, 10, 6, 10],
                [5, 7, 1, 5, 7, 6],
                [10, 7, 1, 10, 7, 6],
                [5, 7, 9, 5, 7, 14],
                [10, 7, 9, 10, 7, 14],
                [1, 7, 5, 6, 7, 5],
                [1, 7, 10, 6, 7, 10],
                [9, 7, 5, 14, 7, 5],
                [9, 7, 10, 14, 7, 10],
                [2, 1, 2, 2, 1, 3],
                [3, 1, 2, 3, 1, 2],
                [13, 1, 2, 13, 1, 3],
                [12, 1, 2, 12, 1, 2],
                [2, 1, 12, 2, 1, 13],
                [3, 1, 13, 3, 1, 13],
                [13, 1, 12, 13, 1, 13],
                [12, 1, 13, 12, 1, 13],
            ] {
                w.bx(x0, y0, z0, x1, y1, z1, l)?;
            }
        }
        Kind::DoubleX => {
            let east = conn(r, EAST).unwrap_or(r);
            let west = r;
            if upper {
                w.floor(8, 0, east.open[DOWN])?;
                w.floor(0, 0, west.open[DOWN])?;
            }
            if west.conn[UP].is_none() {
                w.fill_only(1, 4, 1, 7, 4, 6, g)?;
            }
            if east.conn[UP].is_none() {
                w.fill_only(8, 4, 1, 14, 4, 6, g)?;
            }
            for (y, b) in [(3, l), (2, g), (1, l)] {
                w.bx(0, y, 0, 0, y, 7, b)?;
                w.bx(15, y, 0, 15, y, 7, b)?;
                w.bx(1, y, 0, 15, y, 0, b)?;
                w.bx(1, y, 7, 14, y, 7, b)?;
            }
            w.bx(5, 1, 0, 10, 1, 4, l)?;
            w.bx(6, 2, 0, 9, 2, 3, g)?;
            w.bx(5, 3, 0, 10, 3, 4, l)?;
            w.put(lamp, 6, 2, 3)?;
            w.put(lamp, 9, 2, 3)?;
            doorways(
                w,
                &[
                    (west.open[SOUTH], [3, 1, 0, 4, 2, 0]),
                    (west.open[NORTH], [3, 1, 7, 4, 2, 7]),
                    (west.open[WEST], [0, 1, 3, 0, 2, 4]),
                    (east.open[SOUTH], [11, 1, 0, 12, 2, 0]),
                    (east.open[NORTH], [11, 1, 7, 12, 2, 7]),
                    (east.open[EAST], [15, 1, 3, 15, 2, 4]),
                ],
            )?;
        }
        Kind::DoubleXY => {
            let east = conn(r, EAST).unwrap_or(r);
            let west = r;
            let west_up = conn(west, UP).unwrap_or(r);
            let east_up = conn(east, UP).unwrap_or(r);
            if upper {
                w.floor(8, 0, east.open[DOWN])?;
                w.floor(0, 0, west.open[DOWN])?;
            }
            if west_up.conn[UP].is_none() {
                w.fill_only(1, 8, 1, 7, 8, 6, g)?;
            }
            if east_up.conn[UP].is_none() {
                w.fill_only(8, 8, 1, 14, 8, 6, g)?;
            }
            for y in 1..=7 {
                let b = if y == 2 || y == 6 { g } else { l };
                w.bx(0, y, 0, 0, y, 7, b)?;
                w.bx(15, y, 0, 15, y, 7, b)?;
                w.bx(1, y, 0, 15, y, 0, b)?;
                w.bx(1, y, 7, 14, y, 7, b)?;
            }
            for [x0, y0, z0, x1, y1, z1] in [
                [2, 1, 3, 2, 7, 4],
                [3, 1, 2, 4, 7, 2],
                [3, 1, 5, 4, 7, 5],
                [13, 1, 3, 13, 7, 4],
                [11, 1, 2, 12, 7, 2],
                [11, 1, 5, 12, 7, 5],
                [5, 1, 3, 5, 3, 4],
                [10, 1, 3, 10, 3, 4],
                [5, 7, 2, 10, 7, 5],
                [5, 5, 2, 5, 7, 2],
                [10, 5, 2, 10, 7, 2],
                [5, 5, 5, 5, 7, 5],
                [10, 5, 5, 10, 7, 5],
            ] {
                w.bx(x0, y0, z0, x1, y1, z1, l)?;
            }
            w.put(l, 6, 6, 2)?;
            w.put(l, 9, 6, 2)?;
            w.put(l, 6, 6, 5)?;
            w.put(l, 9, 6, 5)?;
            w.bx(5, 4, 3, 6, 4, 4, l)?;
            w.bx(9, 4, 3, 10, 4, 4, l)?;
            w.put(lamp, 5, 4, 2)?;
            w.put(lamp, 5, 4, 5)?;
            w.put(lamp, 10, 4, 2)?;
            w.put(lamp, 10, 4, 5)?;
            doorways(
                w,
                &[
                    (west.open[SOUTH], [3, 1, 0, 4, 2, 0]),
                    (west.open[NORTH], [3, 1, 7, 4, 2, 7]),
                    (west.open[WEST], [0, 1, 3, 0, 2, 4]),
                    (east.open[SOUTH], [11, 1, 0, 12, 2, 0]),
                    (east.open[NORTH], [11, 1, 7, 12, 2, 7]),
                    (east.open[EAST], [15, 1, 3, 15, 2, 4]),
                    (west_up.open[SOUTH], [3, 5, 0, 4, 6, 0]),
                    (west_up.open[NORTH], [3, 5, 7, 4, 6, 7]),
                    (west_up.open[WEST], [0, 5, 3, 0, 6, 4]),
                    (east_up.open[SOUTH], [11, 5, 0, 12, 6, 0]),
                    (east_up.open[NORTH], [11, 5, 7, 12, 6, 7]),
                    (east_up.open[EAST], [15, 5, 3, 15, 6, 4]),
                ],
            )?;
        }
        Kind::DoubleY => {
            if upper {
                w.floor(0, 0, r.open[DOWN])?;
            }
            let above = conn(r, UP).unwrap_or(r);
            if above.conn[UP].is_none() {
                w.fill_only(1, 8, 1, 6, 8, 6, g)?;
            }
            for [x0, z0, x1, z1] in [
                [0, 0, 0, 7],
                [7, 0, 7, 7],
                [1, 0, 6, 0],
                [1, 7, 6, 7],
                [2, 1, 2, 2],
                [1, 2, 1, 2],
                [5, 1, 5, 2],
                [6, 2, 6, 2],
                [2, 5, 2, 6],
                [1, 5, 1, 5],
                [5, 5, 5, 6],
                [6, 5, 6, 5],
            ] {
                w.bx(x0, 4, z0, x1, 4, z1, l)?;
            }
            let mut def = r;
            for y in [1, 5] {
                for (side, z) in [(SOUTH, 0), (NORTH, 7)] {
                    if def.open[side] {
                        w.bx(2, y, z, 2, y + 2, z, l)?;
                        w.bx(5, y, z, 5, y + 2, z, l)?;
                        w.bx(3, y + 2, z, 4, y + 2, z, l)?;
                    } else {
                        w.bx(0, y, z, 7, y + 2, z, l)?;
                        w.bx(0, y + 1, z, 7, y + 1, z, g)?;
                    }
                }
                for (side, x) in [(WEST, 0), (EAST, 7)] {
                    if def.open[side] {
                        w.bx(x, y, 2, x, y + 2, 2, l)?;
                        w.bx(x, y, 5, x, y + 2, 5, l)?;
                        w.bx(x, y + 2, 3, x, y + 2, 4, l)?;
                    } else {
                        w.bx(x, y, 0, x, y + 2, 7, l)?;
                        w.bx(x, y + 1, 0, x, y + 1, 7, g)?;
                    }
                }
                def = above;
            }
        }
        Kind::DoubleYZ => {
            let north = conn(r, NORTH).unwrap_or(r);
            let south = r;
            let north_up = conn(north, UP).unwrap_or(r);
            let south_up = conn(south, UP).unwrap_or(r);
            if upper {
                w.floor(0, 8, north.open[DOWN])?;
                w.floor(0, 0, south.open[DOWN])?;
            }
            if south_up.conn[UP].is_none() {
                w.fill_only(1, 8, 1, 6, 8, 7, g)?;
            }
            if north_up.conn[UP].is_none() {
                w.fill_only(1, 8, 8, 6, 8, 14, g)?;
            }
            for y in 1..=7 {
                let b = if y == 2 || y == 6 { g } else { l };
                w.bx(0, y, 0, 0, y, 15, b)?;
                w.bx(7, y, 0, 7, y, 15, b)?;
                w.bx(1, y, 0, 6, y, 0, b)?;
                w.bx(1, y, 15, 6, y, 15, b)?;
            }
            for y in 1..=7 {
                let b = if y == 2 || y == 6 { lamp } else { k };
                w.bx(3, y, 7, 4, y, 8, b)?;
            }
            doorways(
                w,
                &[
                    (south.open[SOUTH], [3, 1, 0, 4, 2, 0]),
                    (south.open[EAST], [7, 1, 3, 7, 2, 4]),
                    (south.open[WEST], [0, 1, 3, 0, 2, 4]),
                    (north.open[NORTH], [3, 1, 15, 4, 2, 15]),
                    (north.open[WEST], [0, 1, 11, 0, 2, 12]),
                    (north.open[EAST], [7, 1, 11, 7, 2, 12]),
                    (south_up.open[SOUTH], [3, 5, 0, 4, 6, 0]),
                ],
            )?;
            if south_up.open[EAST] {
                w.water(7, 5, 3, 7, 6, 4)?;
                w.bx(5, 4, 2, 6, 4, 5, l)?;
                w.bx(6, 1, 2, 6, 3, 2, l)?;
                w.bx(6, 1, 5, 6, 3, 5, l)?;
            }
            if south_up.open[WEST] {
                w.water(0, 5, 3, 0, 6, 4)?;
                w.bx(1, 4, 2, 2, 4, 5, l)?;
                w.bx(1, 1, 2, 1, 3, 2, l)?;
                w.bx(1, 1, 5, 1, 3, 5, l)?;
            }
            if north_up.open[NORTH] {
                w.water(3, 5, 15, 4, 6, 15)?;
            }
            if north_up.open[WEST] {
                w.water(0, 5, 11, 0, 6, 12)?;
                w.bx(1, 4, 10, 2, 4, 13, l)?;
                w.bx(1, 1, 10, 1, 3, 10, l)?;
                w.bx(1, 1, 13, 1, 3, 13, l)?;
            }
            if north_up.open[EAST] {
                w.water(7, 5, 11, 7, 6, 12)?;
                w.bx(5, 4, 10, 6, 4, 13, l)?;
                w.bx(6, 1, 10, 6, 3, 10, l)?;
                w.bx(6, 1, 13, 6, 3, 13, l)?;
            }
        }
        Kind::DoubleZ => {
            let north = conn(r, NORTH).unwrap_or(r);
            let south = r;
            if upper {
                w.floor(0, 8, north.open[DOWN])?;
                w.floor(0, 0, south.open[DOWN])?;
            }
            if south.conn[UP].is_none() {
                w.fill_only(1, 4, 1, 6, 4, 7, g)?;
            }
            if north.conn[UP].is_none() {
                w.fill_only(1, 4, 8, 6, 4, 14, g)?;
            }
            for (y, b) in [(3, l), (2, g), (1, l)] {
                w.bx(0, y, 0, 0, y, 15, b)?;
                w.bx(7, y, 0, 7, y, 15, b)?;
                w.bx(1, y, 0, 7, y, 0, b)?;
                w.bx(1, y, 15, 6, y, 15, b)?;
            }
            for [x0, y0, z0, x1, y1, z1] in [
                [1, 1, 1, 1, 1, 2],
                [6, 1, 1, 6, 1, 2],
                [1, 3, 1, 1, 3, 2],
                [6, 3, 1, 6, 3, 2],
                [1, 1, 13, 1, 1, 14],
                [6, 1, 13, 6, 1, 14],
                [1, 3, 13, 1, 3, 14],
                [6, 3, 13, 6, 3, 14],
                [2, 1, 6, 2, 3, 6],
                [5, 1, 6, 5, 3, 6],
                [2, 1, 9, 2, 3, 9],
                [5, 1, 9, 5, 3, 9],
                [3, 2, 6, 4, 2, 6],
                [3, 2, 9, 4, 2, 9],
                [2, 2, 7, 2, 2, 8],
                [5, 2, 7, 5, 2, 8],
            ] {
                w.bx(x0, y0, z0, x1, y1, z1, l)?;
            }
            for [x, z] in [[2, 5], [5, 5], [2, 10], [5, 10]] {
                w.put(lamp, x, 2, z)?;
            }
            for [x, z] in [[2, 5], [5, 5], [2, 10], [5, 10]] {
                w.put(l, x, 3, z)?;
            }
            doorways(
                w,
                &[
                    (south.open[SOUTH], [3, 1, 0, 4, 2, 0]),
                    (south.open[EAST], [7, 1, 3, 7, 2, 4]),
                    (south.open[WEST], [0, 1, 3, 0, 2, 4]),
                    (north.open[NORTH], [3, 1, 15, 4, 2, 15]),
                    (north.open[WEST], [0, 1, 11, 0, 2, 12]),
                    (north.open[EAST], [7, 1, 11, 7, 2, 12]),
                ],
            )?;
        }
        Kind::Entry => {
            for [x0, y0, z0, x1, y1, z1] in [
                [0, 3, 0, 2, 3, 7],
                [5, 3, 0, 7, 3, 7],
                [0, 2, 0, 1, 2, 7],
                [6, 2, 0, 7, 2, 7],
                [0, 1, 0, 0, 1, 7],
                [7, 1, 0, 7, 1, 7],
                [0, 1, 7, 7, 3, 7],
                [1, 1, 0, 2, 3, 0],
                [5, 1, 0, 6, 3, 0],
            ] {
                w.bx(x0, y0, z0, x1, y1, z1, l)?;
            }
            doorways(
                w,
                &[
                    (r.open[NORTH], [3, 1, 7, 4, 2, 7]),
                    (r.open[WEST], [0, 1, 3, 1, 2, 4]),
                    (r.open[EAST], [6, 1, 3, 7, 2, 4]),
                ],
            )?;
        }
        Kind::Penthouse => {
            w.bx(2, -1, 2, 11, -1, 11, l)?;
            w.bx(0, -1, 0, 1, -1, 11, g)?;
            w.bx(12, -1, 0, 13, -1, 11, g)?;
            w.bx(2, -1, 0, 11, -1, 1, g)?;
            w.bx(2, -1, 12, 11, -1, 13, g)?;
            w.bx(0, 0, 0, 0, 0, 13, l)?;
            w.bx(13, 0, 0, 13, 0, 13, l)?;
            w.bx(1, 0, 0, 12, 0, 0, l)?;
            w.bx(1, 0, 13, 12, 0, 13, l)?;
            for i in (2..=11).step_by(3) {
                w.put(lamp, 0, 0, i)?;
                w.put(lamp, 13, 0, i)?;
                w.put(lamp, i, 0, 0)?;
            }
            w.bx(2, 0, 3, 4, 0, 9, l)?;
            w.bx(9, 0, 3, 11, 0, 9, l)?;
            w.bx(4, 0, 9, 9, 0, 11, l)?;
            w.put(l, 5, 0, 8)?;
            w.put(l, 8, 0, 8)?;
            w.put(l, 10, 0, 10)?;
            w.put(l, 3, 0, 10)?;
            w.bx(3, 0, 3, 3, 0, 7, k)?;
            w.bx(10, 0, 3, 10, 0, 7, k)?;
            w.bx(6, 0, 10, 7, 0, 10, k)?;
            for x in [3, 10] {
                for z in (2..=8).step_by(3) {
                    w.bx(x, 0, z, x, 2, z, l)?;
                }
            }
            w.bx(5, 0, 10, 5, 2, 10, l)?;
            w.bx(8, 0, 10, 8, 2, 10, l)?;
            w.bx(6, -1, 7, 7, -1, 8, k)?;
            w.water(6, -1, 3, 7, -1, 4)?;
            w.elder(6, 1, 6)?;
        }
        Kind::Simple(design) => {
            if upper {
                w.floor(0, 0, r.open[DOWN])?;
            }
            if r.conn[UP].is_none() {
                w.fill_only(1, 4, 1, 6, 4, 6, g)?;
            }
            let pillar = design != 0
                && w.c.random.boolean()
                && !r.open[DOWN]
                && !r.open[UP]
                && r.open.iter().filter(|o| **o).count() > 1;
            match design {
                0 => {
                    for (xa, xb, za, zb, gx, gz, lx, lz) in [
                        (0, 2, 0, 2, [0, 0, 0, 2], [1, 2, 0, 0], 1, 1),
                        (5, 7, 0, 2, [7, 7, 0, 2], [5, 6, 0, 0], 6, 1),
                        (0, 2, 5, 7, [0, 0, 5, 7], [1, 2, 7, 7], 1, 6),
                        (5, 7, 5, 7, [7, 7, 5, 7], [5, 6, 7, 7], 6, 6),
                    ] {
                        w.bx(xa, 1, za, xb, 1, zb, l)?;
                        w.bx(xa, 3, za, xb, 3, zb, l)?;
                        w.bx(gx[0], 2, gx[2], gx[1], 2, gx[3], g)?;
                        w.bx(gz[0], 2, gz[2], gz[1], 2, gz[3], g)?;
                        w.put(lamp, lx, 2, lz)?;
                    }
                    if r.open[SOUTH] {
                        w.bx(3, 3, 0, 4, 3, 0, l)?;
                    } else {
                        w.bx(3, 3, 0, 4, 3, 1, l)?;
                        w.bx(3, 2, 0, 4, 2, 0, g)?;
                        w.bx(3, 1, 0, 4, 1, 1, l)?;
                    }
                    if r.open[NORTH] {
                        w.bx(3, 3, 7, 4, 3, 7, l)?;
                    } else {
                        w.bx(3, 3, 6, 4, 3, 7, l)?;
                        w.bx(3, 2, 7, 4, 2, 7, g)?;
                        w.bx(3, 1, 6, 4, 1, 7, l)?;
                    }
                    if r.open[WEST] {
                        w.bx(0, 3, 3, 0, 3, 4, l)?;
                    } else {
                        w.bx(0, 3, 3, 1, 3, 4, l)?;
                        w.bx(0, 2, 3, 0, 2, 4, g)?;
                        w.bx(0, 1, 3, 1, 1, 4, l)?;
                    }
                    if r.open[EAST] {
                        w.bx(7, 3, 3, 7, 3, 4, l)?;
                    } else {
                        w.bx(6, 3, 3, 7, 3, 4, l)?;
                        w.bx(7, 2, 3, 7, 2, 4, g)?;
                        w.bx(6, 1, 3, 7, 1, 4, l)?;
                    }
                }
                1 => {
                    for [x, z] in [[2, 2], [2, 5], [5, 5], [5, 2]] {
                        w.bx(x, 1, z, x, 3, z, l)?;
                    }
                    for [x, z] in [[2, 2], [2, 5], [5, 5], [5, 2]] {
                        w.put(lamp, x, 2, z)?;
                    }
                    for [x0, z0, x1, z1] in [
                        [0, 0, 1, 0],
                        [0, 1, 0, 1],
                        [0, 7, 1, 7],
                        [0, 6, 0, 6],
                        [6, 7, 7, 7],
                        [7, 6, 7, 6],
                        [6, 0, 7, 0],
                        [7, 1, 7, 1],
                    ] {
                        w.bx(x0, 1, z0, x1, 3, z1, l)?;
                    }
                    for [x, z] in [
                        [1, 0],
                        [0, 1],
                        [1, 7],
                        [0, 6],
                        [6, 7],
                        [7, 6],
                        [6, 0],
                        [7, 1],
                    ] {
                        w.put(g, x, 2, z)?;
                    }
                    for (side, [x0, z0, x1, z1]) in [
                        (SOUTH, [1, 0, 6, 0]),
                        (NORTH, [1, 7, 6, 7]),
                        (WEST, [0, 1, 0, 6]),
                        (EAST, [7, 1, 7, 6]),
                    ] {
                        if !r.open[side] {
                            w.bx(x0, 3, z0, x1, 3, z1, l)?;
                            w.bx(x0, 2, z0, x1, 2, z1, g)?;
                            w.bx(x0, 1, z0, x1, 1, z1, l)?;
                        }
                    }
                }
                2 => {
                    for (y, b) in [(1, l), (2, k), (3, l)] {
                        w.bx(0, y, 0, 0, y, 7, b)?;
                        w.bx(7, y, 0, 7, y, 7, b)?;
                        w.bx(1, y, 0, 6, y, 0, b)?;
                        w.bx(1, y, 7, 6, y, 7, b)?;
                    }
                    w.bx(0, 1, 3, 0, 2, 4, k)?;
                    w.bx(7, 1, 3, 7, 2, 4, k)?;
                    w.bx(3, 1, 0, 4, 2, 0, k)?;
                    w.bx(3, 1, 7, 4, 2, 7, k)?;
                    doorways(
                        w,
                        &[
                            (r.open[SOUTH], [3, 1, 0, 4, 2, 0]),
                            (r.open[NORTH], [3, 1, 7, 4, 2, 7]),
                            (r.open[WEST], [0, 1, 3, 0, 2, 4]),
                            (r.open[EAST], [7, 1, 3, 7, 2, 4]),
                        ],
                    )?;
                }
                _ => {}
            }
            if pillar {
                w.bx(3, 1, 3, 4, 1, 4, l)?;
                w.bx(3, 2, 3, 4, 2, 4, g)?;
                w.bx(3, 3, 3, 4, 3, 4, l)?;
            }
        }
        Kind::SimpleTop => {
            if upper {
                w.floor(0, 0, r.open[DOWN])?;
            }
            if r.conn[UP].is_none() {
                w.fill_only(1, 4, 1, 6, 4, 6, g)?;
            }
            let sponge = w.c.state("wet_sponge");
            for x in 1..=6 {
                for z in 1..=6 {
                    if w.c.random.int(3) != 0 {
                        let y0 = 2 + if w.c.random.int(4) == 0 { 0 } else { 1 };
                        w.bx(x, y0, z, x, 3, z, sponge)?;
                    }
                }
            }
            for (y, b) in [(1, l), (2, k), (3, l)] {
                w.bx(0, y, 0, 0, y, 7, b)?;
                w.bx(7, y, 0, 7, y, 7, b)?;
                w.bx(1, y, 0, 6, y, 0, b)?;
                w.bx(1, y, 7, 6, y, 7, b)?;
            }
            w.bx(0, 1, 3, 0, 2, 4, k)?;
            w.bx(7, 1, 3, 7, 2, 4, k)?;
            w.bx(3, 1, 0, 4, 2, 0, k)?;
            w.bx(3, 1, 7, 4, 2, 7, k)?;
            if r.open[SOUTH] {
                w.water(3, 1, 0, 4, 2, 0)?;
            }
        }
        Kind::Wing(0) => {
            for i in 0..4 {
                w.bx(10 - i, 3 - i, 20 - i, 12 + i, 3 - i, 20, l)?;
            }
            for [x0, y0, z0, x1, y1, z1] in [
                [7, 0, 6, 15, 0, 16],
                [6, 0, 6, 6, 3, 20],
                [16, 0, 6, 16, 3, 20],
                [7, 1, 7, 7, 1, 20],
                [15, 1, 7, 15, 1, 20],
                [7, 1, 6, 9, 3, 6],
                [13, 1, 6, 15, 3, 6],
                [8, 1, 7, 9, 1, 7],
                [13, 1, 7, 14, 1, 7],
                [9, 0, 5, 13, 0, 5],
            ] {
                w.bx(x0, y0, z0, x1, y1, z1, l)?;
            }
            w.bx(10, 0, 7, 12, 0, 7, k)?;
            w.bx(8, 0, 10, 8, 0, 12, k)?;
            w.bx(14, 0, 10, 14, 0, 12, k)?;
            for z in (7..=18).rev().step_by(3) {
                w.put(lamp, 6, 3, z)?;
                w.put(lamp, 16, 3, z)?;
            }
            for [x, y, z] in [
                [10, 0, 10],
                [12, 0, 10],
                [10, 0, 12],
                [12, 0, 12],
                [8, 3, 6],
                [14, 3, 6],
            ] {
                w.put(lamp, x, y, z)?;
            }
            for [x, z] in [[4, 4], [18, 4], [4, 18], [18, 18]] {
                w.put(l, x, 2, z)?;
                w.put(lamp, x, 1, z)?;
                w.put(l, x, 0, z)?;
            }
            w.put(l, 9, 7, 20)?;
            w.put(l, 13, 7, 20)?;
            w.bx(6, 0, 21, 7, 4, 21, l)?;
            w.bx(15, 0, 21, 16, 4, 21, l)?;
            w.elder(11, 2, 16)?;
        }
        Kind::Wing(_) => {
            w.bx(9, 3, 18, 13, 3, 20, l)?;
            w.bx(9, 0, 18, 9, 2, 18, l)?;
            w.bx(13, 0, 18, 13, 2, 18, l)?;
            for x in [9, 13] {
                w.put(l, x, 6, 20)?;
                w.put(lamp, x, 5, 20)?;
                w.put(l, x, 4, 20)?;
            }
            w.bx(7, 3, 7, 15, 3, 14, l)?;
            for x in [10, 12] {
                w.bx(x, 0, 10, x, 6, 10, l)?;
                w.bx(x, 0, 12, x, 6, 12, l)?;
                w.put(lamp, x, 0, 10)?;
                w.put(lamp, x, 0, 12)?;
                w.put(lamp, x, 4, 10)?;
                w.put(lamp, x, 4, 12)?;
            }
            for x in [8, 14] {
                w.bx(x, 0, 7, x, 2, 7, l)?;
                w.bx(x, 0, 14, x, 2, 14, l)?;
            }
            w.bx(8, 3, 8, 8, 3, 13, k)?;
            w.bx(14, 3, 8, 14, 3, 13, k)?;
            w.elder(11, 5, 13)?;
        }
    }
    Ok(())
}
