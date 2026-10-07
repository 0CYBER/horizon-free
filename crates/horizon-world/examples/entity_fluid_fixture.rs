use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use horizon_world::{
    entity_fluid::*,
    fluid::{FluidKind, FluidState},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn bits(v: f64) -> u64 {
    if v.is_nan() {
        f64::NAN.to_bits()
    } else {
        v.to_bits()
    }
}
fn show(v: Vec3) -> String {
    format!("{:016x} {:016x} {:016x}", bits(v.x), bits(v.y), bits(v.z))
}
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
fn position(v: &Value) -> BlockPos {
    BlockPos::new(
        v[0].as_i64().unwrap() as i32,
        v[1].as_i64().unwrap() as i32,
        v[2].as_i64().unwrap() as i32,
    )
}
struct World<'a> {
    op: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    velocity: Vec3,
    block: BlockPos,
    wet: Vec<bool>,
    states: [FluidState; 5],
    player: bool,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.op["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected query failure"));
        }
        if n == self.op["mutate_at"].as_i64().unwrap() {
            self.velocity = vector(&self.op["after_velocity"]);
            self.block = position(&self.op["after_block"]);
        }
        Ok(())
    }
    fn cell(&self, p: BlockPos) -> &Value {
        self.op["cells"].get(pos(p)).unwrap_or(&self.op["default"])
    }
}
impl FluidMembership for World<'_> {
    fn fluid_in_tag(&self, fluid: FluidKind, tag: FluidTagId) -> bool {
        self.op["tags"][tag.0 as usize].as_u64().unwrap() & (1 << (fluid as u8)) != 0
    }
}
impl FluidInteractionWorld for World<'_> {
    type Error = io::Error;
    type Chunk = ();
    fn interaction_box(&mut self) -> io::Result<Option<Aabb>> {
        self.query("box".into())?;
        let b = &self.op["bounds"];
        Ok(if b.is_null() {
            None
        } else {
            Some(Aabb::new(
                Vec3::new(number(&b[0]), number(&b[1]), number(&b[2])),
                Vec3::new(number(&b[3]), number(&b[4]), number(&b[5])),
            ))
        })
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> io::Result<Option<()>> {
        self.query(format!("chunk {x} {z}"))?;
        Ok((!self.op["missing"]
            .as_array()
            .unwrap()
            .contains(&Value::String(format!("{x} {z}"))))
        .then_some(()))
    }
    fn chunk_sections(&mut self, _: ()) -> io::Result<FluidSections<'_>> {
        self.query("sections".into())?;
        Ok(FluidSections {
            minimum_section_y: self.op["min_section"].as_i64().unwrap() as i32,
            has_fluid: &self.wet,
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        number(&self.op["base_y"])
    }
    fn block_x(&self) -> i32 {
        self.block.x
    }
    fn block_z(&self) -> i32 {
        self.block.z
    }
    fn eye_y(&mut self) -> io::Result<f64> {
        self.query("eye".into())?;
        Ok(number(&self.op["eye"]))
    }
    fn fluid_state(&mut self, p: BlockPos) -> io::Result<FluidState> {
        self.query(format!("fluid {}", pos(p)))?;
        Ok(self.states[self.cell(p)["kind"].as_u64().unwrap() as usize])
    }
    fn fluid_height(&mut self, _: FluidState, p: BlockPos) -> io::Result<f32> {
        self.query(format!("height {}", pos(p)))?;
        Ok(single(&self.cell(p)["height"]))
    }
    fn camera_height(&mut self, s: FluidState, p: BlockPos) -> io::Result<f32> {
        if matches!(s.kind(), FluidKind::Water | FluidKind::Lava) {
            self.query(format!(
                "block {}",
                pos(BlockPos::new(p.x, p.y.wrapping_add(1), p.z))
            ))?;
            if self.op["sturdy"].as_bool().unwrap() {
                return Ok(1.0);
            }
        }
        self.fluid_height(s, p)
    }
    fn flow(&mut self, _: FluidState, p: BlockPos) -> io::Result<Vec3> {
        self.query(format!("flow {}", pos(p)))?;
        Ok(vector(&self.cell(p)["flow"]))
    }
}
impl CurrentTarget for World<'_> {
    type Error = io::Error;
    fn is_player_class(&self) -> bool {
        self.player
    }
    fn velocity(&mut self) -> io::Result<Vec3> {
        self.query("velocity".into())?;
        Ok(self.velocity)
    }
    fn add_delta_movement(&mut self, v: Vec3) -> io::Result<()> {
        if [v.x, v.y, v.z].iter().all(|v| v.is_finite()) {
            let old = CurrentTarget::velocity(self)?;
            let next = Vec3::new(old.x + v.x, old.y + v.y, old.z + v.z);
            self.query(format!("set {}", show(next)))?;
            self.velocity = next;
        }
        Ok(())
    }
}
fn limit_checks(template: &Value, states: [FluidState; 5]) {
    let mut op = template.clone();
    op["default"]["kind"] = 2.into();
    op["default"]["height"] = format!("{:08x}", 0.8f32.to_bits()).into();
    op["eye"] = format!("{:016x}", 0.5f64.to_bits()).into();
    let mut sink = io::sink();
    let mut world = World {
        op: &op,
        out: &mut sink,
        calls: 0,
        velocity: Vec3::ZERO,
        block: BlockPos::new(0, 0, 0),
        wet: vec![false, true, false],
        states,
        player: false,
    };
    let mut interaction = EntityFluidInteraction::new([FluidTagId(0)]);
    assert!(matches!(
        interaction.update(&mut world, false, 0),
        Err(FluidInteractionError::BudgetExceeded)
    ));
    assert_eq!(world.calls, 1);
    world.calls = 0;
    assert!(matches!(
        interaction.update(&mut world, false, 1),
        Err(FluidInteractionError::BudgetExceeded)
    ));
    assert_eq!(world.calls, 3);
    world.calls = 0;
    assert!(matches!(
        interaction.update(&mut world, false, 8),
        Err(FluidInteractionError::BudgetExceeded)
    ));
    assert_eq!(world.calls, 10);
    assert!(interaction.trackers().is_empty());
    assert!(interaction.update(&mut world, false, 9).unwrap());
    assert_eq!(interaction.trackers().len(), 1);
    assert_eq!(interaction.currents()[0].1.count, 1);
    let mut invalid = op.clone();
    invalid["bounds"][0] = format!("{:016x}", f64::NAN.to_bits()).into();
    world.op = &invalid;
    world.calls = 0;
    assert!(matches!(
        interaction.update(&mut world, false, 9),
        Err(FluidInteractionError::NonFiniteBounds)
    ));
    assert_eq!(world.calls, 1);
    assert_eq!(interaction.trackers().len(), 1);
    assert_eq!(interaction.trackers()[0].height, 0.0);
    assert!(!interaction.trackers()[0].eyes_inside);
    assert_eq!(interaction.currents()[0].1.count, 0);
    let mut absent = op.clone();
    absent["bounds"] = Value::Null;
    world.op = &absent;
    assert!(!interaction.update(&mut world, false, 0).unwrap());
    assert!(interaction.trackers().is_empty());
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let states = FluidKind::ALL
        .map(|k| FluidState::from_json(&format!("{{\"id\":\"{}\"}}", k.name())).unwrap());
    let input = fs::read_to_string(dir.join("inputs.txt"))?;
    let first: Value = serde_json::from_str(input.lines().next().unwrap())?;
    limit_checks(&first["ops"][0], states);
    for line in input.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut interaction = EntityFluidInteraction::new(
            f["order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| FluidTagId(v.as_u64().unwrap() as u32)),
        );
        for op in f["ops"].as_array().unwrap() {
            let mut w = World {
                op,
                out: &mut out,
                calls: 0,
                velocity: vector(&op["velocity"]),
                block: position(&op["block"]),
                wet: op["wet"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect(),
                states,
                player: f["player"].as_bool().unwrap(),
            };
            if op["mode"].as_str().unwrap() == "update" {
                match interaction.update(&mut w, op["ignore"].as_bool().unwrap(), 1_000_000) {
                    Ok(v) => writeln!(w.out, "result {v}")?,
                    Err(_) => writeln!(w.out, "error")?,
                }
            } else {
                match interaction.apply_current(
                    &mut w,
                    FluidTagId(op["tag"].as_u64().unwrap() as u32),
                    number(&op["speed"]),
                ) {
                    Ok(()) => writeln!(w.out, "result ok")?,
                    Err(_) => writeln!(w.out, "error")?,
                }
            }
            writeln!(w.out, "trackers {}", interaction.trackers().len())?;
            for t in interaction.trackers() {
                writeln!(
                    w.out,
                    "tracker {} {:016x} {}",
                    t.fluid as u8,
                    bits(t.height),
                    t.eyes_inside
                )?;
            }
            for (tag, c) in interaction.currents() {
                writeln!(
                    w.out,
                    "current {} {:016x} {} {}",
                    tag.0,
                    bits(c.height),
                    show(c.accumulated),
                    c.count
                )?;
            }
            for t in 0..4 {
                let tag = FluidTagId(t);
                let height = interaction.fluid_height(&w, tag);
                let inside = interaction.is_in_fluid(&w, tag);
                let eyes = interaction.is_eye_in_fluid(&w, tag);
                writeln!(w.out, "tag {t} {:016x} {inside} {eyes}", bits(height))?;
            }
            writeln!(w.out, "final {}", show(w.velocity))?;
            writeln!(w.out, "queries {}", w.calls)?;
        }
    }
    Ok(())
}
