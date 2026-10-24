//! Original differential-test transport. State air/fluid facts are private inputs;
//! tag predicates are resolved by the Rust loader, not imported oracle results.
use horizon_world::heightmap::{Heightmap, HeightmapChunk, HeightmapState, HeightmapType};
use horizon_world::worldgen::tags::TagDefinitions;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::Path,
};

#[derive(Clone, Copy)]
struct State {
    air: bool,
    air_block: bool,
    fluid: bool,
    motion: bool,
    no_leaves: bool,
}
impl HeightmapState for State {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.no_leaves
    }
}
struct Chunk<'a> {
    min: i32,
    height: i32,
    blocks: Vec<usize>,
    counts: Vec<i32>,
    states: &'a [State],
    air: usize,
    void_air: usize,
    trace: bool,
    reads: RefCell<Vec<String>>,
}
impl Chunk<'_> {
    fn index(&self, x: i32, y: i32, z: i32) -> usize {
        ((y - self.min) * 256 + (z & 15) * 16 + (x & 15)) as usize
    }
    fn write(&mut self, x: i32, y: i32, z: i32, id: usize) {
        let index = self.index(x, y, z);
        let old = self.blocks[index];
        self.counts[((y - self.min) >> 4) as usize] +=
            i32::from(!self.states[id].air) - i32::from(!self.states[old].air);
        self.blocks[index] = id;
    }
}
impl HeightmapChunk for Chunk<'_> {
    type State = State;
    fn min_y(&self) -> i32 {
        self.min
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn highest_section_y(&self) -> i32 {
        self.counts
            .iter()
            .rposition(|&n| n != 0)
            .map_or(self.min, |i| self.min + i as i32 * 16)
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> State {
        let id = if y < self.min || y >= self.min + self.height {
            self.void_air
        } else if self.counts[((y - self.min) >> 4) as usize] == 0 {
            self.air
        } else {
            self.blocks[self.index(x, y, z)]
        };
        if self.trace {
            self.reads
                .borrow_mut()
                .push(format!("read {x} {y} {z} {id}"));
        }
        self.states[id]
    }
}
fn num(value: &Value) -> i32 {
    value.as_i64().unwrap() as i32
}
fn fixture(
    input: Value,
    states: &[State],
    defaults: &Value,
    out: &mut impl Write,
) -> io::Result<()> {
    let min = num(&input["min"]);
    let height = num(&input["height"]);
    let air = num(&defaults["minecraft:air"]) as usize;
    let mut chunk = Chunk {
        min,
        height,
        blocks: vec![air; (height * 256) as usize],
        counts: vec![0; (height / 16) as usize],
        states,
        air,
        void_air: num(&defaults["minecraft:void_air"]) as usize,
        trace: input["trace"].as_bool().unwrap(),
        reads: RefCell::default(),
    };
    writeln!(out, "chunk {min} {height}")?;
    for a in input["initial"].as_array().unwrap() {
        chunk.write(num(&a[0]), num(&a[1]), num(&a[2]), num(&a[3]) as usize);
    }
    let mut maps = HeightmapType::ALL.map(|kind| Heightmap::new(kind, min, height).unwrap());
    for a in input["ops"].as_array().unwrap() {
        match a[0].as_str().unwrap() {
            "set" | "update" => {
                let (x, y, z, id) = (num(&a[1]), num(&a[2]), num(&a[3]), num(&a[4]) as usize);
                let mask = if a[0] == "set" {
                    chunk.write(x, y, z, id);
                    num(&a[5])
                } else {
                    1 << num(&a[5])
                };
                for (t, map) in maps.iter_mut().enumerate() {
                    if mask & (1 << t) != 0 {
                        match map.update(&chunk, x, y, z, states[id]) {
                            Ok(changed) => writeln!(out, "update {t} {changed}")?,
                            Err(_) => writeln!(out, "update {t} error")?,
                        }
                    }
                }
            }
            "prime" => {
                let mask = num(&a[1]);
                let mut selected: Vec<_> = maps
                    .iter_mut()
                    .enumerate()
                    .filter_map(|(t, map)| (mask & (1 << t) != 0).then_some(map))
                    .collect();
                Heightmap::prime(&chunk, &mut selected).unwrap();
                writeln!(out, "prime {mask}")?;
            }
            "raw" => {
                let t = num(&a[1]) as usize;
                let words: Vec<_> = a[2]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|word| u64::from_str_radix(word.as_str().unwrap(), 16).unwrap())
                    .collect();
                maps[t].set_raw_data(&chunk, &words).unwrap();
                writeln!(out, "raw {t}")?;
            }
            "query" => {
                let (x, z) = (num(&a[1]), num(&a[2]));
                for (t, map) in maps.iter().enumerate() {
                    match map.first_available(x, z) {
                        Ok(first) => writeln!(
                            out,
                            "height {t} {x} {z} {first} {}",
                            map.highest_taken(x, z).unwrap()
                        )?,
                        Err(_) => writeln!(out, "height {t} {x} {z} error")?,
                    }
                }
            }
            "dump" => {
                writeln!(out, "highest_section {}", chunk.highest_section_y())?;
                for (t, map) in maps.iter().enumerate() {
                    writeln!(out, "words {t} {}", map.raw_data().len())?;
                    for word in map.raw_data() {
                        writeln!(out, "{word:x}")?;
                    }
                }
            }
            _ => panic!("unknown fixture operation"),
        }
        for read in chunk.reads.borrow_mut().drain(..) {
            writeln!(out, "{read}")?;
        }
    }
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&directory);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let tags: Value = serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut definitions = TagDefinitions::default();
    for (name, text) in tags.as_object().unwrap() {
        definitions.apply_json(name, "vanilla", text.as_str().unwrap())?;
    }
    let built = definitions.build(|id, _| facts["defaults"].get(id).map(|_| id.to_owned()))?;
    assert!(built.missing.is_empty());
    let motion: BTreeSet<_> = built.tags["minecraft:blocks_motion_in_heightmap"]
        .iter()
        .map(String::as_str)
        .collect();
    let no_leaves: BTreeSet<_> = built.tags["minecraft:blocks_motion_in_heightmap_no_leaves"]
        .iter()
        .map(String::as_str)
        .collect();
    let states: Vec<_> = facts["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let name = a[0].as_str().unwrap();
            State {
                air: a[1].as_bool().unwrap(),
                air_block: name == "minecraft:air",
                fluid: a[2].as_bool().unwrap(),
                motion: motion.contains(name),
                no_leaves: no_leaves.contains(name),
            }
        })
        .collect();
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for (t, kind) in HeightmapType::ALL.iter().enumerate() {
        writeln!(
            out,
            "type {t} {} {} {}",
            kind.name(),
            kind.send_to_client(),
            kind.keep_after_worldgen()
        )?;
    }
    for (id, &state) in states.iter().enumerate() {
        let mask = HeightmapType::ALL
            .iter()
            .enumerate()
            .fold(0, |mask, (t, kind)| {
                mask | if kind.is_opaque(state) { 1 << t } else { 0 }
            });
        writeln!(out, "predicate {id} {mask}")?;
    }
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        fixture(
            serde_json::from_str(line)?,
            &states,
            &facts["defaults"],
            &mut out,
        )?;
    }
    out.flush()?;
    Ok(())
}
