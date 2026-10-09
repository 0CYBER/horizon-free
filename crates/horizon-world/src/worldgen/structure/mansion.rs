//! Woodland mansions (WoodlandMansionStructure, WoodlandMansionPieces).
use super::{
    BoundingBox, Mirror, Rotation, StructureResources,
    legacy::{
        HORIZONTAL, PieceContext, Result, TemplateHandler, TemplatePiece, clockwise,
        counter_clockwise, create_chest, direction_name, offset, opposite,
    },
    processor::ProcessorList,
};
use horizon_core::{geometry::Direction, random::RandomSource};
use std::sync::Arc;

const CORRIDOR_FLAG: i32 = 8_388_608;
const STAIRS_FLAG: i32 = 4_194_304;
const DOOR_FLAG: i32 = 2_097_152;
const ORIGIN_FLAG: i32 = 1_048_576;
const TYPE_MASK: i32 = 983_040;
const ID_MASK: i32 = 65_535;
const ROOM_1X1: i32 = 65_536;
const ROOM_1X2: i32 = 131_072;
const ROOM_2X2: i32 = 262_144;

#[derive(Clone)]
struct Grid {
    cells: Vec<i32>,
    width: i32,
    height: i32,
    outside: i32,
}
impl Grid {
    fn new(width: i32, height: i32, outside: i32) -> Self {
        Self {
            cells: vec![0; (width * height) as usize],
            width,
            height,
            outside,
        }
    }
    fn set(&mut self, x: i32, y: i32, value: i32) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            self.cells[(x * self.height + y) as usize] = value;
        }
    }
    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, value: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, value);
            }
        }
    }
    fn get(&self, x: i32, y: i32) -> i32 {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            self.cells[(x * self.height + y) as usize]
        } else {
            self.outside
        }
    }
    fn set_if(&mut self, x: i32, y: i32, if_value: i32, value: i32) {
        if self.get(x, y) == if_value {
            self.set(x, y, value);
        }
    }
    fn edges_to(&self, x: i32, y: i32, value: i32) -> bool {
        self.get(x - 1, y) == value
            || self.get(x + 1, y) == value
            || self.get(x, y + 1) == value
            || self.get(x, y - 1) == value
    }
}
fn is_house(grid: &Grid, x: i32, y: i32) -> bool {
    matches!(grid.get(x, y), 1..=4)
}
fn step(d: Direction) -> (i32, i32) {
    let n = d.normal();
    (n[0], n[2])
}
fn from_2d(value: i32) -> Direction {
    [
        Direction::South,
        Direction::West,
        Direction::North,
        Direction::East,
    ][value as usize & 3]
}
fn shuffle<T>(list: &mut [T], random: &mut impl RandomSource) {
    for i in (2..=list.len()).rev() {
        let j = random.next_int_bound(i as i32).unwrap_or(0) as usize;
        list.swap(i - 1, j);
    }
}

struct MansionGrid {
    base: Grid,
    third: Grid,
    rooms: [Grid; 3],
    entrance_x: i32,
    entrance_y: i32,
}
impl MansionGrid {
    fn new(r: &mut impl RandomSource) -> Self {
        let (ex, ey) = (7, 4);
        let mut base = Grid::new(11, 11, 5);
        base.fill(ex, ey, ex + 1, ey + 1, 3);
        base.fill(ex - 1, ey, ex - 1, ey + 1, 2);
        base.fill(ex + 2, ey - 2, ex + 3, ey + 3, 5);
        base.fill(ex + 1, ey - 2, ex + 1, ey - 1, 1);
        base.fill(ex + 1, ey + 2, ex + 1, ey + 3, 1);
        base.set(ex - 1, ey - 1, 1);
        base.set(ex - 1, ey + 2, 1);
        base.fill(0, 0, 11, 1, 5);
        base.fill(0, 9, 11, 11, 5);
        corridor(&mut base, r, ex, ey - 2, Direction::West, 6);
        corridor(&mut base, r, ex, ey + 3, Direction::West, 6);
        corridor(&mut base, r, ex - 2, ey - 1, Direction::West, 3);
        corridor(&mut base, r, ex - 2, ey + 2, Direction::West, 3);
        while clean_edges(&mut base) {}
        let mut rooms = [
            Grid::new(11, 11, 5),
            Grid::new(11, 11, 5),
            Grid::new(11, 11, 5),
        ];
        identify_rooms(&base, &mut rooms[0], r);
        identify_rooms(&base, &mut rooms[1], r);
        rooms[0].fill(ex + 1, ey, ex + 1, ey + 1, CORRIDOR_FLAG);
        rooms[1].fill(ex + 1, ey, ex + 1, ey + 1, CORRIDOR_FLAG);
        let mut grid = Self {
            third: Grid::new(base.width, base.height, 5),
            base,
            rooms,
            entrance_x: ex,
            entrance_y: ey,
        };
        grid.setup_third_floor(r);
        let mut third_rooms = std::mem::replace(&mut grid.rooms[2], Grid::new(0, 0, 0));
        identify_rooms(&grid.third, &mut third_rooms, r);
        grid.rooms[2] = third_rooms;
        grid
    }
    fn is_room_id(&self, x: i32, y: i32, floor: usize, id: i32) -> bool {
        (self.rooms[floor].get(x, y) & ID_MASK) == id
    }
    fn room_direction_1x2(&self, x: i32, y: i32, floor: usize, id: i32) -> Option<Direction> {
        HORIZONTAL.into_iter().find(|&d| {
            let (dx, dz) = step(d);
            self.is_room_id(x + dx, y + dz, floor, id)
        })
    }
    fn setup_third_floor(&mut self, r: &mut impl RandomSource) {
        let mut potential = Vec::new();
        for y in 0..self.third.height {
            for x in 0..self.third.width {
                let data = self.rooms[1].get(x, y);
                if data & TYPE_MASK == ROOM_1X2 && data & DOOR_FLAG == DOOR_FLAG {
                    potential.push((x, y));
                }
            }
        }
        let (w, h) = (self.third.width, self.third.height);
        if potential.is_empty() {
            self.third.fill(0, 0, w, h, 5);
            return;
        }
        let (rx, ry) = potential[r.next_int_bound(potential.len() as i32).unwrap_or(0) as usize];
        let data = self.rooms[1].get(rx, ry);
        self.rooms[1].set(rx, ry, data | STAIRS_FLAG);
        let Some(dir) = self.room_direction_1x2(rx, ry, 1, data & ID_MASK) else {
            return;
        };
        let (dx, dz) = step(dir);
        let (ex, ey) = (rx + dx, ry + dz);
        for y in 0..h {
            for x in 0..w {
                if !is_house(&self.base, x, y) {
                    self.third.set(x, y, 5);
                } else if x == rx && y == ry {
                    self.third.set(x, y, 3);
                } else if x == ex && y == ey {
                    self.third.set(x, y, 3);
                    self.rooms[2].set(x, y, CORRIDOR_FLAG);
                }
            }
        }
        let corridors: Vec<Direction> = HORIZONTAL
            .into_iter()
            .filter(|&d| {
                let (dx, dz) = step(d);
                self.third.get(ex + dx, ey + dz) == 0
            })
            .collect();
        if corridors.is_empty() {
            self.third.fill(0, 0, w, h, 5);
            self.rooms[1].set(rx, ry, data);
        } else {
            let d = corridors[r.next_int_bound(corridors.len() as i32).unwrap_or(0) as usize];
            let (dx, dz) = step(d);
            corridor(&mut self.third, r, ex + dx, ey + dz, d, 4);
            while clean_edges(&mut self.third) {}
        }
    }
}
fn corridor(
    g: &mut Grid,
    r: &mut impl RandomSource,
    x: i32,
    y: i32,
    heading: Direction,
    depth: i32,
) {
    if depth <= 0 {
        return;
    }
    let (hx, hz) = step(heading);
    g.set(x, y, 1);
    g.set_if(x + hx, y + hz, 0, 1);
    for _ in 0..8 {
        let next = from_2d(r.next_int_bound(4).unwrap_or(0));
        if next != opposite(heading) && (next != Direction::East || !r.next_boolean()) {
            let (nx, ny) = (x + hx, y + hz);
            let (dx, dz) = step(next);
            if g.get(nx + dx, ny + dz) == 0 && g.get(nx + dx * 2, ny + dz * 2) == 0 {
                corridor(g, r, x + hx + dx, y + hz + dz, next, depth - 1);
                break;
            }
        }
    }
    let (cx, cz) = step(clockwise(heading));
    let (ax, az) = step(counter_clockwise(heading));
    g.set_if(x + cx, y + cz, 0, 2);
    g.set_if(x + ax, y + az, 0, 2);
    g.set_if(x + hx + cx, y + hz + cz, 0, 2);
    g.set_if(x + hx + ax, y + hz + az, 0, 2);
    g.set_if(x + hx * 2, y + hz * 2, 0, 2);
    g.set_if(x + cx * 2, y + cz * 2, 0, 2);
    g.set_if(x + ax * 2, y + az * 2, 0, 2);
}
fn clean_edges(g: &mut Grid) -> bool {
    let mut touched = false;
    for y in 0..g.height {
        for x in 0..g.width {
            if g.get(x, y) != 0 {
                continue;
            }
            let house = |g: &Grid, x, y| i32::from(is_house(g, x, y));
            let direct =
                house(g, x + 1, y) + house(g, x - 1, y) + house(g, x, y + 1) + house(g, x, y - 1);
            if direct >= 3 {
                g.set(x, y, 2);
                touched = true;
            } else if direct == 2 {
                let diagonal = house(g, x + 1, y + 1)
                    + house(g, x - 1, y + 1)
                    + house(g, x + 1, y - 1)
                    + house(g, x - 1, y - 1);
                if diagonal <= 1 {
                    g.set(x, y, 2);
                    touched = true;
                }
            }
        }
    }
    touched
}
fn identify_rooms(from: &Grid, rooms: &mut Grid, r: &mut impl RandomSource) {
    let mut positions = Vec::new();
    for y in 0..from.height {
        for x in 0..from.width {
            if from.get(x, y) == 2 {
                positions.push((x, y));
            }
        }
    }
    shuffle(&mut positions, r);
    let mut id = 10;
    for (x, y) in positions {
        if rooms.get(x, y) != 0 {
            continue;
        }
        let (mut x0, mut x1, mut y0, mut y1) = (x, x, y, y);
        let mut kind = ROOM_1X1;
        let free =
            |dx: i32, dy: i32| rooms.get(x + dx, y + dy) == 0 && from.get(x + dx, y + dy) == 2;
        if free(1, 0) && free(0, 1) && free(1, 1) {
            x1 += 1;
            y1 += 1;
            kind = ROOM_2X2;
        } else if free(-1, 0) && free(0, 1) && free(-1, 1) {
            x0 -= 1;
            y1 += 1;
            kind = ROOM_2X2;
        } else if free(-1, 0) && free(0, -1) && free(-1, -1) {
            x0 -= 1;
            y0 -= 1;
            kind = ROOM_2X2;
        } else if free(1, 0) {
            x1 += 1;
            kind = ROOM_1X2;
        } else if free(0, 1) {
            y1 += 1;
            kind = ROOM_1X2;
        } else if free(-1, 0) {
            x0 -= 1;
            kind = ROOM_1X2;
        } else if free(0, -1) {
            y0 -= 1;
            kind = ROOM_1X2;
        }
        let mut door_x = if r.next_boolean() { x0 } else { x1 };
        let mut door_y = if r.next_boolean() { y0 } else { y1 };
        let mut door = DOOR_FLAG;
        let swap = |v: i32, a: i32, b: i32| if v == a { b } else { a };
        if !from.edges_to(door_x, door_y, 1) {
            door_x = swap(door_x, x0, x1);
            door_y = swap(door_y, y0, y1);
            if !from.edges_to(door_x, door_y, 1) {
                door_y = swap(door_y, y0, y1);
                if !from.edges_to(door_x, door_y, 1) {
                    door_x = swap(door_x, x0, x1);
                    door_y = swap(door_y, y0, y1);
                    if !from.edges_to(door_x, door_y, 1) {
                        door = 0;
                        door_x = x0;
                        door_y = y0;
                    }
                }
            }
        }
        for ry in y0..=y1 {
            for rx in x0..=x1 {
                if rx == door_x && ry == door_y {
                    rooms.set(rx, ry, ORIGIN_FLAG | door | kind | id);
                } else {
                    rooms.set(rx, ry, kind | id);
                }
            }
        }
        id += 1;
    }
}

struct Piece {
    name: String,
    position: [i32; 3],
    rotation: Rotation,
    mirror: Mirror,
}
struct Placement {
    position: [i32; 3],
    rotation: Rotation,
    wall: &'static str,
}
fn up(p: [i32; 3], n: i32) -> [i32; 3] {
    [p[0], p[1] + n, p[2]]
}
fn rel(p: [i32; 3], rotation: Rotation, d: Direction, n: i32) -> [i32; 3] {
    offset(p, rotation.rotate_direction(d), n)
}
/// One floor's room-template choices (First/Second/ThirdFloorRoomCollection).
fn room_name(floor: usize, kind: &str, r: &mut impl RandomSource, stairs: bool) -> String {
    let mut n = |b: i32| r.next_int_bound(b).unwrap_or(0) + 1;
    if floor == 0 {
        match kind {
            "1x1" => format!("1x1_a{}", n(5)),
            "1x1s" => format!("1x1_as{}", n(4)),
            "side" => format!("1x2_a{}", n(9)),
            "front" => format!("1x2_b{}", n(5)),
            "1x2s" => format!("1x2_s{}", n(2)),
            "2x2" => format!("2x2_a{}", n(4)),
            _ => "2x2_s1".into(),
        }
    } else {
        match kind {
            "1x1" => format!("1x1_b{}", n(5)),
            "1x1s" => format!("1x1_as{}", n(4)),
            "side" if stairs => "1x2_c_stairs".into(),
            "side" => format!("1x2_c{}", n(4)),
            "front" if stairs => "1x2_d_stairs".into(),
            "front" => format!("1x2_d{}", n(5)),
            "1x2s" => format!("1x2_se{}", n(1)),
            "2x2" => format!("2x2_b{}", n(5)),
            _ => "2x2_s1".into(),
        }
    }
}
struct Placer<'a, R: RandomSource> {
    random: &'a mut R,
    pieces: Vec<Piece>,
    start_x: i32,
    start_y: i32,
}
impl<R: RandomSource> Placer<'_, R> {
    fn add(
        &mut self,
        name: impl Into<String>,
        position: [i32; 3],
        rotation: Rotation,
        mirror: Mirror,
    ) {
        self.pieces.push(Piece {
            name: name.into(),
            position,
            rotation,
            mirror,
        });
    }
    fn grid_pos(
        &self,
        origin: [i32; 3],
        rotation: Rotation,
        x: i32,
        y: i32,
        east: i32,
    ) -> [i32; 3] {
        let p = rel(
            origin,
            rotation,
            Direction::South,
            8 + (y - self.start_y) * 8,
        );
        rel(p, rotation, Direction::East, east + (x - self.start_x) * 8)
    }
    fn create(&mut self, origin: [i32; 3], rotation: Rotation, mansion: &MansionGrid) {
        let mut data = Placement {
            position: origin,
            rotation,
            wall: "wall_flat",
        };
        let west = rotation.rotate_direction(Direction::West);
        self.add(
            "entrance",
            offset(data.position, west, 9),
            rotation,
            Mirror::None,
        );
        data.position = rel(data.position, rotation, Direction::South, 16);
        let mut second = Placement {
            position: up(data.position, 8),
            rotation: data.rotation,
            wall: "wall_window",
        };
        let (base, third) = (&mansion.base, &mansion.third);
        self.start_x = mansion.entrance_x + 1;
        self.start_y = mansion.entrance_y + 1;
        let (end_x, end_y) = (mansion.entrance_x + 1, mansion.entrance_y);
        let (sx, sy) = (self.start_x, self.start_y);
        self.outer_walls(&mut data, base, Direction::South, sx, sy, end_x, end_y);
        self.outer_walls(&mut second, base, Direction::South, sx, sy, end_x, end_y);
        let mut third_data = Placement {
            position: up(data.position, 19),
            rotation: data.rotation,
            wall: "wall_window",
        };
        'done: for y in 0..third.height {
            for x in (0..third.width).rev() {
                if is_house(third, x, y) {
                    third_data.position = self.grid_pos(third_data.position, rotation, x, y, 0);
                    self.wall_piece(&mut third_data);
                    self.outer_walls(&mut third_data, third, Direction::South, x, y, x, y);
                    break 'done;
                }
            }
        }
        self.roof(up(origin, 16), rotation, base, Some(third));
        self.roof(up(origin, 27), rotation, third, None);
        for floor in 0..3 {
            let floor_origin = up(origin, 8 * floor as i32 + if floor == 2 { 3 } else { 0 });
            let rooms = &mansion.rooms[floor];
            let grid = if floor == 2 { third } else { base };
            let south_piece = if floor == 0 {
                "carpet_south_1"
            } else {
                "carpet_south_2"
            };
            let west_piece = if floor == 0 {
                "carpet_west_1"
            } else {
                "carpet_west_2"
            };
            let corridor =
                |x, y| grid.get(x, y) == 1 || rooms.get(x, y) & CORRIDOR_FLAG == CORRIDOR_FLAG;
            for y in 0..grid.height {
                for x in 0..grid.width {
                    if grid.get(x, y) != 1 {
                        continue;
                    }
                    let pos = self.grid_pos(floor_origin, rotation, x, y, 0);
                    self.add("corridor_floor", pos, rotation, Mirror::None);
                    if corridor(x, y - 1) {
                        let p = up(rel(pos, rotation, Direction::East, 1), 1);
                        self.add("carpet_north", p, rotation, Mirror::None);
                    }
                    if corridor(x + 1, y) {
                        let p = rel(pos, rotation, Direction::South, 1);
                        let p = up(rel(p, rotation, Direction::East, 5), 1);
                        self.add("carpet_east", p, rotation, Mirror::None);
                    }
                    if corridor(x, y + 1) {
                        let p = rel(pos, rotation, Direction::South, 5);
                        let p = rel(p, rotation, Direction::West, 1);
                        self.add(south_piece, p, rotation, Mirror::None);
                    }
                    if corridor(x - 1, y) {
                        let p = rel(pos, rotation, Direction::West, 1);
                        let p = rel(p, rotation, Direction::North, 1);
                        self.add(west_piece, p, rotation, Mirror::None);
                    }
                }
            }
            let wall = if floor == 0 {
                "indoors_wall_1"
            } else {
                "indoors_wall_2"
            };
            let door = if floor == 0 {
                "indoors_door_1"
            } else {
                "indoors_door_2"
            };
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let mut third_start = floor == 2 && grid.get(x, y) == 3;
                    if grid.get(x, y) != 2 && !third_start {
                        continue;
                    }
                    let data = rooms.get(x, y);
                    let kind = data & TYPE_MASK;
                    let id = data & ID_MASK;
                    third_start = third_start && data & CORRIDOR_FLAG == CORRIDOR_FLAG;
                    let mut doors = Vec::new();
                    if data & DOOR_FLAG == DOOR_FLAG {
                        for d in HORIZONTAL {
                            let (dx, dz) = step(d);
                            if grid.get(x + dx, y + dz) == 1 {
                                doors.push(d);
                            }
                        }
                    }
                    let door_dir = if !doors.is_empty() {
                        Some(
                            doors[self.random.next_int_bound(doors.len() as i32).unwrap_or(0)
                                as usize],
                        )
                    } else if data & ORIGIN_FLAG == ORIGIN_FLAG {
                        Some(Direction::Up)
                    } else {
                        None
                    };
                    let room_pos = self.grid_pos(floor_origin, rotation, x, y, -1);
                    let pick = |d: Direction| if door_dir == Some(d) { door } else { wall };
                    if is_house(grid, x - 1, y) && !mansion.is_room_id(x - 1, y, floor, id) {
                        self.add(pick(Direction::West), room_pos, rotation, Mirror::None);
                    }
                    if grid.get(x + 1, y) == 1 && !third_start {
                        let p = rel(room_pos, rotation, Direction::East, 8);
                        self.add(pick(Direction::East), p, rotation, Mirror::None);
                    }
                    let cw90 = rotation.get_rotated(Rotation::Clockwise90);
                    if is_house(grid, x, y + 1) && !mansion.is_room_id(x, y + 1, floor, id) {
                        let p = rel(room_pos, rotation, Direction::South, 7);
                        let p = rel(p, rotation, Direction::East, 7);
                        self.add(pick(Direction::South), p, cw90, Mirror::None);
                    }
                    if grid.get(x, y - 1) == 1 && !third_start {
                        let p = rel(room_pos, rotation, Direction::North, 1);
                        let p = rel(p, rotation, Direction::East, 7);
                        self.add(pick(Direction::North), p, cw90, Mirror::None);
                    }
                    if kind == ROOM_1X1 {
                        self.room_1x1(room_pos, rotation, door_dir, floor);
                    } else if kind == ROOM_1X2 && door_dir.is_some() {
                        let room_dir = mansion.room_direction_1x2(x, y, floor, id);
                        let stairs = data & STAIRS_FLAG == STAIRS_FLAG;
                        if let (Some(room_dir), Some(door_dir)) = (room_dir, door_dir) {
                            self.room_1x2(room_pos, rotation, room_dir, door_dir, floor, stairs);
                        }
                    } else if kind == ROOM_2X2 && door_dir.is_some_and(|d| d != Direction::Up) {
                        let door_dir = door_dir.unwrap();
                        let mut room_dir = clockwise(door_dir);
                        let (dx, dz) = step(room_dir);
                        if !mansion.is_room_id(x + dx, y + dz, floor, id) {
                            room_dir = opposite(room_dir);
                        }
                        self.room_2x2(room_pos, rotation, room_dir, door_dir, floor);
                    } else if kind == ROOM_2X2 && door_dir == Some(Direction::Up) {
                        let name = room_name(floor, "2x2s", self.random, false);
                        let p = rel(room_pos, rotation, Direction::East, 1);
                        self.add(name, p, rotation, Mirror::None);
                    }
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn outer_walls(
        &mut self,
        data: &mut Placement,
        grid: &Grid,
        direction: Direction,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) {
        let (mut gx, mut gy, mut dir) = (start_x, start_y, direction);
        let start_dir = direction;
        loop {
            let (dx, dz) = step(dir);
            let (cx, cz) = step(counter_clockwise(dir));
            let inner_corner = is_house(grid, gx + dx + cx, gy + dz + cz);
            if !is_house(grid, gx + dx, gy + dz) {
                data.position = rel(data.position, data.rotation, Direction::South, -1);
                self.add("wall_corner", data.position, data.rotation, Mirror::None);
                data.position = rel(data.position, data.rotation, Direction::South, -7);
                data.position = rel(data.position, data.rotation, Direction::West, -6);
                data.rotation = data.rotation.get_rotated(Rotation::Clockwise90);
                dir = clockwise(dir);
                if gx != end_x || gy != end_y || start_dir != dir {
                    self.wall_piece(data);
                }
            } else if inner_corner {
                data.position = rel(data.position, data.rotation, Direction::South, 6);
                data.position = rel(data.position, data.rotation, Direction::East, 8);
                data.rotation = data.rotation.get_rotated(Rotation::Counterclockwise90);
                gx += dx;
                gy += dz;
                dir = counter_clockwise(dir);
            } else {
                gx += dx;
                gy += dz;
                if gx != end_x || gy != end_y || start_dir != dir {
                    self.wall_piece(data);
                }
            }
            if gx == end_x && gy == end_y && start_dir == dir {
                break;
            }
        }
    }
    fn wall_piece(&mut self, data: &mut Placement) {
        let p = rel(data.position, data.rotation, Direction::East, 7);
        self.add(data.wall, p, data.rotation, Mirror::None);
        data.position = rel(data.position, data.rotation, Direction::South, 8);
    }
    fn roof(&mut self, origin: [i32; 3], rotation: Rotation, grid: &Grid, above: Option<&Grid>) {
        use Direction::{East, North, South, West};
        let r = |rot: Rotation| rotation.get_rotated(rot);
        let (cw90, cw180, ccw90) = (
            r(Rotation::Clockwise90),
            r(Rotation::Clockwise180),
            r(Rotation::Counterclockwise90),
        );
        let none = Mirror::None;
        for y in 0..grid.height {
            for x in 0..grid.width {
                let pos = self.grid_pos(origin, rotation, x, y, 0);
                let is_above = above.is_some_and(|a| is_house(a, x, y));
                if !is_house(grid, x, y) || is_above {
                    continue;
                }
                self.add("roof", up(pos, 3), rotation, none);
                if !is_house(grid, x + 1, y) {
                    self.add("roof_front", rel(pos, rotation, East, 6), rotation, none);
                }
                if !is_house(grid, x - 1, y) {
                    let p = rel(rel(pos, rotation, East, 0), rotation, South, 7);
                    self.add("roof_front", p, cw180, none);
                }
                if !is_house(grid, x, y - 1) {
                    self.add("roof_front", rel(pos, rotation, West, 1), ccw90, none);
                }
                if !is_house(grid, x, y + 1) {
                    let p = rel(rel(pos, rotation, East, 6), rotation, South, 6);
                    self.add("roof_front", p, cw90, none);
                }
            }
        }
        if let Some(above) = above {
            for y in 0..grid.height {
                for x in 0..grid.width {
                    let pos = self.grid_pos(origin, rotation, x, y, 0);
                    if !(is_house(grid, x, y) && is_house(above, x, y)) {
                        continue;
                    }
                    if !is_house(grid, x + 1, y) {
                        self.add("small_wall", rel(pos, rotation, East, 7), rotation, none);
                    }
                    if !is_house(grid, x - 1, y) {
                        let p = rel(rel(pos, rotation, West, 1), rotation, South, 6);
                        self.add("small_wall", p, cw180, none);
                    }
                    if !is_house(grid, x, y - 1) {
                        let p = rel(rel(pos, rotation, West, 0), rotation, North, 1);
                        self.add("small_wall", p, ccw90, none);
                    }
                    if !is_house(grid, x, y + 1) {
                        let p = rel(rel(pos, rotation, East, 6), rotation, South, 7);
                        self.add("small_wall", p, cw90, none);
                    }
                    if !is_house(grid, x + 1, y) {
                        if !is_house(grid, x, y - 1) {
                            let p = rel(rel(pos, rotation, East, 7), rotation, North, 2);
                            self.add("small_wall_corner", p, rotation, none);
                        }
                        if !is_house(grid, x, y + 1) {
                            let p = rel(rel(pos, rotation, East, 8), rotation, South, 7);
                            self.add("small_wall_corner", p, cw90, none);
                        }
                    }
                    if !is_house(grid, x - 1, y) {
                        if !is_house(grid, x, y - 1) {
                            let p = rel(rel(pos, rotation, West, 2), rotation, North, 1);
                            self.add("small_wall_corner", p, ccw90, none);
                        }
                        if !is_house(grid, x, y + 1) {
                            let p = rel(rel(pos, rotation, West, 1), rotation, South, 8);
                            self.add("small_wall_corner", p, cw180, none);
                        }
                    }
                }
            }
        }
        for y in 0..grid.height {
            for x in 0..grid.width {
                let pos = self.grid_pos(origin, rotation, x, y, 0);
                let is_above = above.is_some_and(|a| is_house(a, x, y));
                if !is_house(grid, x, y) || is_above {
                    continue;
                }
                if !is_house(grid, x + 1, y) {
                    let p2 = rel(pos, rotation, East, 6);
                    if !is_house(grid, x, y + 1) {
                        self.add("roof_corner", rel(p2, rotation, South, 6), rotation, none);
                    } else if is_house(grid, x + 1, y + 1) {
                        self.add(
                            "roof_inner_corner",
                            rel(p2, rotation, South, 5),
                            rotation,
                            none,
                        );
                    }
                    if !is_house(grid, x, y - 1) {
                        self.add("roof_corner", p2, ccw90, none);
                    } else if is_house(grid, x + 1, y - 1) {
                        let p3 = rel(rel(pos, rotation, East, 9), rotation, North, 2);
                        self.add("roof_inner_corner", p3, cw90, none);
                    }
                }
                if !is_house(grid, x - 1, y) {
                    let p2 = rel(rel(pos, rotation, East, 0), rotation, South, 0);
                    if !is_house(grid, x, y + 1) {
                        self.add("roof_corner", rel(p2, rotation, South, 6), cw90, none);
                    } else if is_house(grid, x - 1, y + 1) {
                        let p3 = rel(rel(p2, rotation, South, 8), rotation, West, 3);
                        self.add("roof_inner_corner", p3, ccw90, none);
                    }
                    if !is_house(grid, x, y - 1) {
                        self.add("roof_corner", p2, cw180, none);
                    } else if is_house(grid, x - 1, y - 1) {
                        self.add(
                            "roof_inner_corner",
                            rel(p2, rotation, South, 1),
                            cw180,
                            none,
                        );
                    }
                }
            }
        }
    }
    fn room_1x1(
        &mut self,
        room_pos: [i32; 3],
        rotation: Rotation,
        door: Option<Direction>,
        floor: usize,
    ) {
        let mut piece_rot = Rotation::None;
        let mut name = room_name(floor, "1x1", self.random, false);
        if door != Some(Direction::East) {
            match door {
                Some(Direction::North) => piece_rot = Rotation::Counterclockwise90,
                Some(Direction::West) => piece_rot = Rotation::Clockwise180,
                Some(Direction::South) => piece_rot = Rotation::Clockwise90,
                _ => name = room_name(floor, "1x1s", self.random, false),
            }
        }
        let o = match piece_rot {
            Rotation::Counterclockwise90 => [1, 0, 6],
            Rotation::Clockwise90 => [7, 0, 0],
            Rotation::Clockwise180 => [7, 0, 6],
            Rotation::None => [1, 0, 0],
        };
        let piece_rot = piece_rot.get_rotated(rotation);
        let o = match rotation {
            Rotation::Clockwise90 => [-o[2], o[1], o[0]],
            Rotation::Clockwise180 => [-o[0], o[1], -o[2]],
            Rotation::Counterclockwise90 => [o[2], o[1], -o[0]],
            Rotation::None => o,
        };
        let pos = [room_pos[0] + o[0], room_pos[1], room_pos[2] + o[2]];
        self.add(name, pos, piece_rot, Mirror::None);
    }
    fn room_1x2(
        &mut self,
        room_pos: [i32; 3],
        rotation: Rotation,
        room: Direction,
        door: Direction,
        floor: usize,
        stairs: bool,
    ) {
        use Direction::{East, North, South, Up, West};
        let r = |rot: Rotation| rotation.get_rotated(rot);
        let at = |east: i32, south: i32| {
            rel(rel(room_pos, rotation, East, east), rotation, South, south)
        };
        let (kind, pos, rot, mirror) = match (door, room) {
            (East, South) => ("side", at(1, 0), rotation, Mirror::None),
            (East, North) => ("side", at(1, 6), rotation, Mirror::LeftRight),
            (West, North) => ("side", at(7, 6), r(Rotation::Clockwise180), Mirror::None),
            (West, South) => ("side", at(7, 0), rotation, Mirror::FrontBack),
            (South, East) => (
                "side",
                at(1, 0),
                r(Rotation::Clockwise90),
                Mirror::LeftRight,
            ),
            (South, West) => ("side", at(7, 0), r(Rotation::Clockwise90), Mirror::None),
            (North, West) => (
                "side",
                at(7, 6),
                r(Rotation::Clockwise90),
                Mirror::FrontBack,
            ),
            (North, East) => (
                "side",
                at(1, 6),
                r(Rotation::Counterclockwise90),
                Mirror::None,
            ),
            (South, North) => ("front", at(1, -8), rotation, Mirror::None),
            (North, South) => ("front", at(7, 14), r(Rotation::Clockwise180), Mirror::None),
            (West, East) => ("front", at(15, 0), r(Rotation::Clockwise90), Mirror::None),
            (East, West) => (
                "front",
                at(-7, 6),
                r(Rotation::Counterclockwise90),
                Mirror::None,
            ),
            (Up, East) => ("1x2s", at(15, 0), r(Rotation::Clockwise90), Mirror::None),
            (Up, South) => ("1x2s", at(1, 0), rotation, Mirror::None),
            _ => return,
        };
        let name = room_name(floor, kind, self.random, stairs);
        self.add(name, pos, rot, mirror);
    }
    fn room_2x2(
        &mut self,
        room_pos: [i32; 3],
        rotation: Rotation,
        room: Direction,
        door: Direction,
        floor: usize,
    ) {
        use Direction::{East, North, South, West};
        let r = |rot: Rotation| rotation.get_rotated(rot);
        let (east, south, rot, mirror) = match (door, room) {
            (East, South) => (-7, 0, rotation, Mirror::None),
            (East, North) => (-7, 6, rotation, Mirror::LeftRight),
            (North, East) => (1, 14, r(Rotation::Counterclockwise90), Mirror::None),
            (North, West) => (7, 14, r(Rotation::Counterclockwise90), Mirror::LeftRight),
            (South, West) => (7, -8, r(Rotation::Clockwise90), Mirror::None),
            (South, East) => (1, -8, r(Rotation::Clockwise90), Mirror::LeftRight),
            (West, North) => (15, 6, r(Rotation::Clockwise180), Mirror::None),
            (West, South) => (15, 0, rotation, Mirror::FrontBack),
            _ => (0, 0, rotation, Mirror::None),
        };
        let pos = rel(rel(room_pos, rotation, East, east), rotation, South, south);
        let name = room_name(floor, "2x2", self.random, false);
        self.add(name, pos, rot, mirror);
    }
}

/// WoodlandMansionPieces.generateMansion.
pub fn pieces(
    resources: &StructureResources,
    origin: [i32; 3],
    rotation: Rotation,
    random: &mut impl RandomSource,
) -> Option<Vec<TemplatePiece>> {
    let grid = MansionGrid::new(random);
    let mut placer = Placer {
        random,
        pieces: Vec::new(),
        start_x: 0,
        start_y: 0,
    };
    placer.create(origin, rotation, &grid);
    let processors = Arc::new(ProcessorList::default());
    placer
        .pieces
        .into_iter()
        .map(|p| {
            let name = format!("minecraft:woodland_mansion/{}", p.name);
            Some(TemplatePiece::new(
                "minecraft:wmp",
                &name,
                resources.template(&name)?,
                p.position,
                p.rotation,
                p.mirror,
                [0; 3],
                processors.clone(),
                true,
                vec!["minecraft:structure_block".into()],
                TemplateHandler::Mansion,
            ))
        })
        .collect()
}

/// WoodlandMansionPiece.postProcess with its data markers.
pub fn post_process(piece: &TemplatePiece, c: &mut PieceContext<'_>) -> Result<()> {
    let Some(markers) = piece.place_at(c, piece.position, &[])? else {
        return Ok(());
    };
    for (marker, pos) in markers {
        if marker.starts_with("Chest") {
            let facing = match marker.as_str() {
                "ChestWest" => Direction::West,
                "ChestEast" => Direction::East,
                "ChestSouth" => Direction::South,
                _ => Direction::North,
            };
            let facing = if marker == "Chest" {
                Direction::North
            } else {
                piece.rotation.rotate_direction(facing)
            };
            let state = c.state_with("chest", &[("facing", direction_name(facing))]);
            create_chest(c, pos, "minecraft:chests/woodland_mansion", Some(state))?;
        } else if matches!(marker.as_str(), "Mage" | "Warrior" | "Group of Allays") {
            let (id, count) = match marker.as_str() {
                "Mage" => ("minecraft:evoker", 1),
                "Warrior" => ("minecraft:vindicator", 1),
                _ => ("minecraft:allay", c.world.level_int(3)? + 1),
            };
            for _ in 0..count {
                c.world
                    .entity(super::legacy::spawn_nbt(id, pos, true, true))?;
            }
            c.world.set(pos, c.state("air"), 2)?;
        }
    }
    Ok(())
}

/// WoodlandMansionStructure.afterPlace: cobblestone columns under the pieces.
pub fn after_place(c: &mut PieceContext<'_>, boxes: &[BoundingBox]) -> Result<()> {
    let min_y = c.world.min_y()?;
    let bb = super::temple::union(boxes);
    let y_start = bb.min[1];
    let cobble = c.state("cobblestone");
    for x in c.chunk_box.min[0]..=c.chunk_box.max[0] {
        for z in c.chunk_box.min[2]..=c.chunk_box.max[2] {
            let pos = [x, y_start, z];
            let state = c.world.block(pos)?;
            if c.is_air(state) || !bb.is_inside(pos) || !boxes.iter().any(|b| b.is_inside(pos)) {
                continue;
            }
            let mut y = y_start - 1;
            while y > min_y {
                let state = c.world.block([x, y, z])?;
                if !c.is_air(state) && !c.is_liquid(state) {
                    break;
                }
                c.world.set([x, y, z], cobble, 2)?;
                y -= 1;
            }
        }
    }
    Ok(())
}
