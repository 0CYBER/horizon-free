use horizon_core::position::BlockPos;
use horizon_world::swimming::{self, PlayerSwimmingWorld, SwimmingWorld};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    swimming: bool,
    water: bool,
    sprint: bool,
    passenger: bool,
    eye: bool,
    position: BlockPos,
}
impl World<'_> {
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected swimming failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.swimming = !self.swimming;
            self.water = !self.water;
            self.sprint = !self.sprint;
            self.passenger = !self.passenger;
            self.eye = !self.eye;
            self.position = BlockPos::new(31, 32, 33);
        }
        Ok(())
    }
}
impl SwimmingWorld for World<'_> {
    type Error = io::Error;
    fn is_swimming(&mut self) -> io::Result<bool> {
        self.query("swimming".into())?;
        Ok(self.swimming)
    }
    fn is_sprinting(&mut self) -> io::Result<bool> {
        self.query("sprint".into())?;
        Ok(self.sprint)
    }
    fn is_in_water(&mut self) -> io::Result<bool> {
        self.query("water".into())?;
        Ok(self.water)
    }
    fn is_under_water(&mut self) -> io::Result<bool> {
        self.query("under".into())?;
        swimming::under_water(self)
    }
    fn is_passenger(&mut self) -> io::Result<bool> {
        self.query("passenger".into())?;
        Ok(self.passenger)
    }
    fn was_eye_in_water(&self) -> bool {
        self.eye
    }
    fn block_position(&self) -> BlockPos {
        self.position
    }
    fn water_at(&mut self, p: BlockPos) -> io::Result<bool> {
        self.query(format!("fluid {} {} {}", p.x, p.y, p.z))?;
        Ok(self.f["water_mask"].as_u64().unwrap() & (1 << self.f["fluid"].as_u64().unwrap()) != 0)
    }
    fn set_swimming(&mut self, v: bool) -> io::Result<()> {
        self.query(format!("set {v}"))?;
        self.swimming = v;
        Ok(())
    }
}
impl PlayerSwimmingWorld for World<'_> {
    fn flying_ability(&self) -> bool {
        self.f["flying"].as_bool().unwrap()
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            swimming: f["swimming"].as_bool().unwrap(),
            water: f["water"].as_bool().unwrap(),
            sprint: f["sprint"].as_bool().unwrap(),
            passenger: f["passenger"].as_bool().unwrap(),
            eye: f["eye"].as_bool().unwrap(),
            position: BlockPos::new(17, -64, -33),
        };
        let player = f["player"].as_bool().unwrap();
        let result: io::Result<String> = match f["mode"].as_str().unwrap() {
            "update" => if player {
                swimming::update_player(&mut w)
            } else {
                swimming::update(&mut w)
            }
            .map(|()| "result ok".into()),
            "under" => w.is_under_water().map(|v| format!("result {v}")),
            "shallow" => swimming::shallow_water(&mut w).map(|v| format!("result {v}")),
            "threshold" => Ok(format!(
                "threshold {:016x}",
                swimming::fluid_jump_threshold(f32::from_bits(u32::from_str_radix(
                    f["eye_height"].as_str().unwrap(),
                    16
                )?))
                .to_bits()
            )),
            _ => panic!("fixture mode"),
        };
        writeln!(w.out, "{}", result.unwrap_or_else(|_| "error".into()))?;
        writeln!(
            w.out,
            "final {} {} {} {} {}",
            w.swimming, w.eye, w.position.x, w.position.y, w.position.z
        )?;
        writeln!(w.out, "queries {}", w.calls)?;
    }
    Ok(())
}
