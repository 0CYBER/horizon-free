//! Text adapter for differential checks; no vanilla logic lives in the adapter.
use horizon_core::geometry::{
    Aabb, Axis, BlockHit, Direction, Vec3, VoxelShape, block_floor_height, candidate_step_heights,
    entity_collision_bounds, resolve_shape_movement, resolve_step_movement,
    sweep_intersected_blocks,
};
use horizon_core::position::BlockPos;
use std::io::{self, BufRead, Write};

struct Input<'a>(std::str::SplitWhitespace<'a>);
impl Input<'_> {
    fn word(&mut self) -> &str {
        self.0.next().expect("missing input")
    }
    fn integer(&mut self) -> i32 {
        self.word().parse().expect("integer")
    }
    fn number(&mut self) -> f64 {
        f64::from_bits(u64::from_str_radix(self.word(), 16).expect("double bits"))
    }
    fn vector(&mut self) -> Vec3 {
        Vec3::new(self.number(), self.number(), self.number())
    }
    fn aabb(&mut self) -> Aabb {
        Aabb::new(self.vector(), self.vector())
    }
    fn pos(&mut self) -> BlockPos {
        BlockPos::new(self.integer(), self.integer(), self.integer())
    }
}
fn bits(value: f64) -> String {
    format!(
        "{:016x}",
        if value.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            value.to_bits()
        }
    )
}
fn positions(
    out: &mut impl Write,
    label: &str,
    iter: impl Iterator<Item = BlockPos>,
    limit: usize,
) -> io::Result<()> {
    let mut count = 0;
    for pos in iter.take(limit) {
        writeln!(out, "{label} {} {} {}", pos.x, pos.y, pos.z)?;
        count += 1;
    }
    writeln!(out, "{label}-count {count}")
}
fn values(
    out: &mut impl Write,
    label: &str,
    values: impl IntoIterator<Item = f64>,
) -> io::Result<()> {
    write!(out, "{label}")?;
    for value in values {
        write!(out, " {}", bits(value))?;
    }
    writeln!(out)
}
fn vector(out: &mut impl Write, label: &str, v: Vec3) -> io::Result<()> {
    values(out, label, v.components())
}
fn aabb(out: &mut impl Write, label: &str, b: Aabb) -> io::Result<()> {
    values(
        out,
        label,
        b.min_position()
            .components()
            .into_iter()
            .chain(b.max_position().components()),
    )
}
fn point(out: &mut impl Write, label: &str, p: Option<Vec3>) -> io::Result<()> {
    match p {
        Some(v) => vector(out, label, v),
        None => writeln!(out, "{label} none"),
    }
}
fn hit(out: &mut impl Write, h: Option<BlockHit>) -> io::Result<()> {
    match h {
        Some(h) => {
            write!(
                out,
                "hit {} {} {} {} {}",
                h.direction as u8,
                u8::from(h.inside),
                h.block.x,
                h.block.y,
                h.block.z
            )?;
            for c in h.location.components() {
                write!(out, " {}", bits(c))?;
            }
            writeln!(out)
        }
        None => writeln!(out, "hit none"),
    }
}
fn shape_values(out: &mut impl Write, id: usize, shape: &VoxelShape) -> io::Result<()> {
    values(
        out,
        "axis-extrema",
        Axis::ALL
            .map(|axis| shape.min(axis))
            .into_iter()
            .chain(Axis::ALL.map(|axis| shape.max(axis))),
    )?;
    writeln!(out, "shape {id} {}", shape.boxes().len())?;
    let flags = shape.cube_coordinate_axes();
    writeln!(
        out,
        "cube {} {}",
        u8::from(shape.is_cube()),
        u8::from(flags[0]) + 2 * u8::from(flags[1]) + 4 * u8::from(flags[2])
    )?;
    for (axis, label) in Axis::ALL
        .into_iter()
        .zip(["x-coords", "y-coords", "z-coords"])
    {
        values(out, label, shape.coordinates(axis).iter().copied())?;
    }
    if let Some(b) = shape.bounds() {
        aabb(out, "shape-bounds", b)?;
    } else {
        writeln!(out, "shape-bounds none")?;
    }
    for b in shape.boxes() {
        aabb(out, "part", *b)?;
    }
    Ok(())
}
fn main() -> io::Result<()> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut shapes = Vec::<VoxelShape>::new();
    for (line_number, line) in io::stdin().lock().lines().enumerate() {
        let line = line?;
        let mut input = Input(line.split_whitespace());
        let operation = input.word().to_owned();
        writeln!(out, "command {} {operation}", line_number + 1)?;
        match operation.as_str() {
            "make-box" => {
                let shape = VoxelShape::from_box(input.aabb()).expect("valid box");
                shape_values(&mut out, shapes.len(), &shape)?;
                shapes.push(shape);
            }
            "optimize" => {
                let shape = shapes[input.integer() as usize]
                    .optimized()
                    .expect("bounded optimization");
                shape_values(&mut out, shapes.len(), &shape)?;
                shapes.push(shape);
            }
            "join" | "join-optimized" => {
                let optimized = operation == "join-optimized";
                let a = input.integer() as usize;
                let b = input.integer() as usize;
                let operation =
                    horizon_core::geometry::ShapeOp::from_truth_table(input.integer() as u8);
                let result = if optimized {
                    shapes[a].join(&shapes[b], operation)
                } else {
                    shapes[a].join_unoptimized(&shapes[b], operation)
                };
                match result {
                    Err(horizon_core::geometry::ShapeError::UnboundedBoolean) => {
                        writeln!(out, "join invalid")?
                    }
                    Err(error) => panic!("join failed: {error}"),
                    Ok(shape) => {
                        writeln!(
                            out,
                            "overlap {}",
                            u8::from(
                                shapes[a]
                                    .join_is_not_empty(&shapes[b], operation)
                                    .expect("bounded Boolean")
                            )
                        )?;
                        shape_values(&mut out, shapes.len(), &shape)?;
                        shapes.push(shape);
                    }
                }
            }
            "traverse" => {
                let start = input.vector();
                let end = input.vector();
                let stop = input.integer();
                let mut count = 0;
                let result = horizon_core::geometry::traverse_blocks(start, end, 10000, |pos| {
                    count += 1;
                    writeln!(out, "visit {} {} {}", pos.x, pos.y, pos.z).expect("write visit");
                    (count == stop).then_some(pos)
                })
                .expect("finite bounded corpus");
                match result {
                    Some(pos) => writeln!(out, "traverse {} {} {} {count}", pos.x, pos.y, pos.z)?,
                    None => writeln!(out, "traverse none {count}")?,
                }
            }
            "trig-table" => {
                for (i, value) in horizon_core::geometry::trig::sine_table()
                    .iter()
                    .enumerate()
                {
                    writeln!(out, "sine {i} {:08x}", value.to_bits())?;
                }
            }
            "rotation" => {
                let v = input.vector();
                let angle = input.number();
                let pitch = input.number() as f32;
                let yaw = input.number() as f32;
                let axes = input.integer();
                let axis = Axis::ALL[input.integer() as usize];
                let direction = Direction::ALL[input.integer() as usize];
                let amount = input.number();
                values(
                    &mut out,
                    "trig",
                    [
                        horizon_core::geometry::trig::sin(angle) as f64,
                        horizon_core::geometry::trig::cos(angle) as f64,
                    ],
                )?;
                vector(&mut out, "rotate-x", v.rotate_x(angle as f32))?;
                vector(&mut out, "rotate-y", v.rotate_y(angle as f32))?;
                vector(&mut out, "rotate-z", v.rotate_z(angle as f32))?;
                vector(&mut out, "clockwise", v.rotate_clockwise_90())?;
                vector(&mut out, "look", Vec3::direction_from_rotation(pitch, yaw))?;
                values(&mut out, "rotation", v.rotation().map(|v| v as f64))?;
                vector(
                    &mut out,
                    "align",
                    v.align([axes & 1 != 0, axes & 2 != 0, axes & 4 != 0]),
                )?;
                vector(&mut out, "with", v.with_component(axis, amount))?;
                vector(&mut out, "relative", v.relative(direction, amount))?;
            }
            "vec" => {
                let a = input.vector();
                let b = input.vector();
                let t = input.number();
                for (label, v) in [
                    ("add", a.add_vector(b)),
                    ("subtract", a.subtract(b)),
                    ("vector-to", a.vector_to(b)),
                    ("multiply", a.multiply(b)),
                    ("scale", a.scale(t)),
                    ("normalize", a.normalize()),
                    ("cross", a.cross(b)),
                    ("lerp", a.lerp(b, t)),
                    ("projected", a.projected_on(b)),
                    ("reverse", a.reverse()),
                    ("horizontal", a.horizontal()),
                ] {
                    vector(&mut out, label, v)?;
                }
                values(
                    &mut out,
                    "scalar",
                    [
                        a.dot(b),
                        a.length_squared(),
                        a.length(),
                        a.horizontal_distance_squared(),
                        a.horizontal_distance(),
                        a.distance_squared(b),
                        a.distance(b),
                    ],
                )?;
                writeln!(
                    out,
                    "flags {} {} {} {}",
                    u8::from(a == b),
                    u8::from(a.is_finite()),
                    u8::from(a.closer_than(b, t)),
                    Direction::approximate_nearest(a) as u8
                )?;
                let order = Axis::step_order(a);
                writeln!(
                    out,
                    "order {} {} {}",
                    order[0] as u8, order[1] as u8, order[2] as u8
                )?;
            }
            "box" => {
                let a = input.aabb();
                let b = input.aabb();
                let start = input.vector();
                let end = input.vector();
                let amount = input.vector();
                let axis = Axis::ALL[input.integer() as usize];
                let value = input.number();
                for (label, b) in [
                    ("bounds", a),
                    ("move", a.move_by(amount)),
                    ("inflate", a.inflate(amount)),
                    ("deflate", a.deflate(amount)),
                    ("contract", a.contract(amount)),
                    ("expand", a.expand_towards(amount)),
                    ("intersect", a.intersect(b)),
                    ("union", a.union_bounds(b)),
                    ("next", a.next_deflated()),
                    ("set-min", a.with_min(axis, value)),
                    ("set-max", a.with_max(axis, value)),
                    ("of-size", Aabb::of_size(start, amount)),
                    ("unit", Aabb::unit_cube(start)),
                ] {
                    aabb(&mut out, label, b)?;
                }
                vector(&mut out, "center", a.center())?;
                vector(&mut out, "bottom", a.bottom_center())?;
                vector(&mut out, "size", a.size())?;
                values(
                    &mut out,
                    "distance",
                    [
                        a.average_size(),
                        a.distance_squared_to_point(start),
                        a.distance_squared_to_box(b),
                    ],
                )?;
                writeln!(
                    out,
                    "flags {} {} {} {}",
                    u8::from(a.intersects(b)),
                    u8::from(a.contains(start)),
                    u8::from(a.has_nan()),
                    u8::from(a.collided_along_vector(amount, &[b]))
                )?;
                point(&mut out, "clip", a.clip(start, end))?;
                hit(
                    &mut out,
                    Aabb::clip_boxes([a, b], start, end, BlockPos::default()),
                )?;
            }
            "block-box" => {
                let a = input.pos();
                let b = input.pos();
                aabb(&mut out, "block", Aabb::block(a))?;
                aabb(&mut out, "encapsulate", Aabb::encapsulating_blocks(a, b))?;
            }
            "grid" => {
                let cube = input.integer() != 0;
                let sizes = std::array::from_fn(|_| input.integer() as usize);
                let coords = std::array::from_fn(|i| {
                    (0..=sizes[i]).map(|_| input.number()).collect::<Vec<_>>()
                });
                let occupied = input.word().bytes().map(|b| b == b'1').collect::<Vec<_>>();
                let occupied = if sizes.contains(&0) {
                    &[][..]
                } else {
                    &occupied[..]
                };
                let shape = if cube {
                    VoxelShape::from_cube(sizes, occupied)
                } else {
                    VoxelShape::from_grid(coords, occupied)
                }
                .expect("valid shape");
                writeln!(out, "shape {} {}", shapes.len(), shape.boxes().len())?;
                values(
                    &mut out,
                    "axis-extrema",
                    Axis::ALL
                        .map(|axis| shape.min(axis))
                        .into_iter()
                        .chain(Axis::ALL.map(|axis| shape.max(axis))),
                )?;
                if let Some(b) = shape.bounds() {
                    aabb(&mut out, "shape-bounds", b)?;
                } else {
                    writeln!(out, "shape-bounds none")?;
                }
                for b in shape.boxes() {
                    aabb(&mut out, "part", *b)?;
                }
                shapes.push(shape);
            }
            "query" => {
                let id = input.integer() as usize;
                let b = input.aabb();
                let start = input.vector();
                let end = input.vector();
                let p = input.vector();
                let block = input.pos();
                let axis = Axis::ALL[input.integer() as usize];
                let amount = input.number();
                let shape = &shapes[id];
                values(
                    &mut out,
                    "column",
                    [shape.min_at(axis, p.x, p.y), shape.max_at(axis, p.x, p.y)],
                )?;
                values(&mut out, "collide", [shape.collide(axis, b, amount)])?;
                hit(&mut out, shape.clip(start, end, block))?;
                point(&mut out, "closest", shape.closest_point(p))?;
                writeln!(
                    out,
                    "index {} {} {}",
                    shape.find_index(Axis::X, p.x),
                    shape.find_index(Axis::Y, p.y),
                    shape.find_index(Axis::Z, p.z)
                )?;
            }
            "many" => {
                let b = input.aabb();
                let axis = Axis::ALL[input.integer() as usize];
                let amount = input.number();
                let count = input.integer();
                let selected = (0..count)
                    .map(|_| &shapes[input.integer() as usize])
                    .collect::<Vec<_>>();
                values(
                    &mut out,
                    "many",
                    [VoxelShape::collide_all(axis, b, selected, amount)],
                )?;
            }
            "floor-height" => {
                let shape = &shapes[input.integer() as usize];
                let below = &shapes[input.integer() as usize];
                let mut calls = 0;
                let height = block_floor_height(shape, || {
                    calls += 1;
                    below
                });
                values(&mut out, "floor-height", [height])?;
                writeln!(out, "below-calls {calls}")?;
            }
            "block-iterate" => {
                let a = input.pos();
                let b = input.pos();
                let direction = input.vector();
                let limit = input.integer() as usize;
                positions(
                    &mut out,
                    "closed",
                    BlockPos::between_closed(a, b).expect("bounded block box"),
                    limit,
                )?;
                positions(
                    &mut out,
                    "direction",
                    BlockPos::between_corners_in_direction(a, b, direction)
                        .expect("bounded block box"),
                    limit,
                )?;
            }
            "box-iterate" => {
                let b = input.aabb();
                let direction = input.vector();
                let limit = input.integer() as usize;
                positions(
                    &mut out,
                    "closed",
                    BlockPos::between_closed_box(b).expect("bounded block box"),
                    limit,
                )?;
                positions(
                    &mut out,
                    "direction",
                    BlockPos::between_box_corners_in_direction(b, direction)
                        .expect("bounded block box"),
                    limit,
                )?;
            }
            "sweep" => {
                let start = input.vector();
                let end = input.vector();
                let b = input.aabb();
                let stop = input.integer() as usize;
                let mut visits = Vec::new();
                let completed = sweep_intersected_blocks(start, end, b, 1_000_000, |pos, step| {
                    visits.push((pos, step));
                    visits.len() != stop
                })
                .expect("bounded finite sweep");
                for (pos, step) in &visits {
                    writeln!(out, "block-visit {} {} {} {step}", pos.x, pos.y, pos.z)?;
                }
                writeln!(out, "sweep {} {}", u8::from(completed), visits.len())?;
            }
            "step-motion" => {
                let b = input.aabb();
                let movement = input.vector();
                let max_step = input.number() as f32;
                let grounded = input.integer() != 0;
                let mut read_shapes = || {
                    let count = input.integer();
                    (0..count)
                        .map(|_| &shapes[input.integer() as usize])
                        .collect::<Vec<_>>()
                };
                let entities = read_shapes();
                let initial = read_shapes();
                let stepping = read_shapes();
                let border = input.integer();
                aabb(
                    &mut out,
                    "entity-query",
                    entity_collision_bounds(movement, b, max_step),
                )?;
                let mut queries = Vec::new();
                let result = resolve_step_movement(movement, b, max_step, grounded, |query| {
                    let blocks = if queries.is_empty() {
                        &initial
                    } else {
                        &stepping
                    };
                    queries.push(query);
                    let mut selected = entities.clone();
                    if border >= 0 {
                        selected.push(&shapes[border as usize]);
                    }
                    selected.extend(blocks.iter().copied());
                    selected
                });
                for query in &queries {
                    aabb(&mut out, "block-query", *query)?;
                }
                vector(&mut out, "step-motion", result)?;
                writeln!(out, "block-queries {}", queries.len())?;
            }
            "motion" => {
                let b = input.aabb();
                let movement = input.vector();
                let max_step = input.number() as f32;
                let current_y = input.number() as f32;
                let count = input.integer();
                let selected = (0..count)
                    .map(|_| &shapes[input.integer() as usize])
                    .collect::<Vec<_>>();
                vector(
                    &mut out,
                    "motion",
                    resolve_shape_movement(movement, b, &selected),
                )?;
                let candidates = candidate_step_heights(b, &selected, max_step, current_y);
                write!(out, "steps {}", candidates.len())?;
                for value in candidates {
                    write!(
                        out,
                        " {:08x}",
                        if value.is_nan() {
                            0x7fc0_0000
                        } else {
                            value.to_bits()
                        }
                    )?;
                }
                writeln!(out)?;
            }
            _ => panic!("unknown command {operation}"),
        }
        assert!(input.0.next().is_none(), "trailing input");
    }
    out.flush()
}
