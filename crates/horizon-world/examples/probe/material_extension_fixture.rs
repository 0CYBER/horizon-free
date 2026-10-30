//! Original column-extension comparison transport.
use horizon_core::{
    noise::{NanRange, Noise},
    random::{LegacyRandom, PositionalRandom, RandomSource, XoroshiroRandom},
};
use horizon_world::{
    registry::BlockStateId,
    worldgen::{
        biome_temperature::{BiomeTemperature, BiomeTemperatureSettings},
        decode_noise,
        material_extensions::{
            ExtensionBlocks, ExtensionNoises, MaterialColumn, MaterialExtensions,
        },
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};
const NAMES: [&str; 6] = [
    "badlands_surface",
    "badlands_pillar",
    "badlands_pillar_roof",
    "iceberg_surface",
    "iceberg_pillar",
    "iceberg_pillar_roof",
];
#[derive(Debug)]
struct TracedNoise {
    name: &'static str,
    real: Arc<dyn Noise>,
    constant: Option<f32>,
    trace: Arc<Mutex<Vec<String>>>,
}
impl Noise for TracedNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok([f32::NEG_INFINITY, f32::INFINITY])
    }
    fn get2(&self, _: f64, _: f64) -> f32 {
        panic!("unexpected 2D noise query")
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        self.trace.lock().unwrap().push(format!(
            "noise {} {:x} {:x} {:x}",
            self.name,
            x.to_bits(),
            y.to_bits(),
            z.to_bits()
        ));
        self.constant.unwrap_or_else(|| self.real.get3(x, y, z))
    }
}
struct Column {
    min: i32,
    values: Vec<BlockStateId>,
    remaining: i32,
    trace: Arc<Mutex<Vec<String>>>,
}
impl Column {
    fn index(&self, y: i32) -> Option<usize> {
        let i = i64::from(y) - i64::from(self.min);
        (i >= 0 && i < self.values.len() as i64).then_some(i as usize)
    }
}
impl MaterialColumn for Column {
    type Error = io::Error;
    fn block(&mut self, y: i32) -> io::Result<BlockStateId> {
        if self.remaining == 0 {
            self.trace.lock().unwrap().push(format!("read error {y}"));
            return Err(io::Error::other("fixture read limit"));
        }
        self.remaining -= 1;
        let id = self
            .index(y)
            .map(|i| self.values[i])
            .unwrap_or(BlockStateId(2));
        self.trace
            .lock()
            .unwrap()
            .push(format!("read {y} {}", id.0));
        Ok(id)
    }
    fn set_block(&mut self, y: i32, state: BlockStateId) -> io::Result<()> {
        self.trace
            .lock()
            .unwrap()
            .push(format!("write {y} {}", state.0));
        if let Some(i) = self.index(y) {
            self.values[i] = state;
        }
        Ok(())
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        state.0 <= 2
    }
    fn same_block(&self, a: BlockStateId, b: BlockStateId) -> bool {
        let kind = |id: BlockStateId| match id.0 {
            5 => 4,
            9 => 8,
            other => other,
        };
        kind(a) == kind(b)
    }
}
fn fixture<R: RandomSource>(
    f: &Value,
    noises: &BTreeMap<String, String>,
    biomes: &BTreeMap<String, String>,
    mut random: R,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>>
where
    R::Positional: Clone,
{
    let root = random.fork_positional();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let mut sources = Vec::<Arc<dyn Noise>>::new();
    for (i, name) in NAMES.iter().enumerate() {
        let id = format!("minecraft:{name}");
        sources.push(Arc::new(TracedNoise {
            name,
            real: Arc::new(decode_noise(&noises[&id])?.create(&mut root.from_hash_of(&id))?),
            constant: f
                .get("noise_bits")
                .map(|v| f32::from_bits(u32::from_str_radix(v[i].as_str().unwrap(), 16).unwrap())),
            trace: trace.clone(),
        }));
    }
    let system = MaterialExtensions::new(
        root,
        ExtensionBlocks {
            default: BlockStateId(f["default"].as_u64().unwrap() as u32),
            water: BlockStateId(4),
            packed_ice: BlockStateId(7),
            snow: BlockStateId(6),
        },
        ExtensionNoises {
            badlands_surface: sources[0].clone(),
            badlands_pillar: sources[1].clone(),
            badlands_roof: sources[2].clone(),
            iceberg_surface: sources[3].clone(),
            iceberg_pillar: sources[4].clone(),
            iceberg_roof: sources[5].clone(),
        },
        f["sea"].as_i64().unwrap() as i32,
    );
    let mut biome = BiomeTemperature::new(BiomeTemperatureSettings::from_json(
        &biomes[f["biome"].as_str().unwrap()],
    )?);
    let mut column = Column {
        min: f["min"].as_i64().unwrap() as i32,
        values: f["initial"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| BlockStateId(v.as_u64().unwrap() as u32))
            .collect(),
        remaining: 0,
        trace: trace.clone(),
    };
    for op in f["ops"].as_array().unwrap() {
        column.remaining = f["limit"].as_i64().unwrap() as i32;
        let x = op[1].as_i64().unwrap() as i32;
        let z = op[2].as_i64().unwrap() as i32;
        let surface = op[3].as_i64().unwrap() as i32;
        let minimum = op[4].as_i64().unwrap() as i32;
        let result = match op[0].as_str().unwrap() {
            "badlands" => system.eroded_badlands(&mut column, x, z, surface, minimum),
            "frozen" => system.frozen_ocean(&mut column, &mut biome, [x, z], surface, minimum),
            _ => panic!("unknown extension"),
        };
        for event in trace.lock().unwrap().drain(..) {
            writeln!(out, "{event}")?;
        }
        writeln!(
            out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        write!(out, "column")?;
        for id in &column.values {
            write!(out, " {}", id.0)?;
        }
        writeln!(out)?;
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let noises = serde_json::from_str(&fs::read_to_string(dir.join("noises.json"))?)?;
    let biomes = serde_json::from_str(&fs::read_to_string(dir.join("biomes.json"))?)?;
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, &noises, &biomes, LegacyRandom::new(seed), &mut out)?;
        } else {
            fixture(&f, &noises, &biomes, XoroshiroRandom::new(seed), &mut out)?;
        }
    }
    Ok(())
}
