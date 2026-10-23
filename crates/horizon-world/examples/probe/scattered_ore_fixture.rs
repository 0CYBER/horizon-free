//! Feature world transport retaining ordered mutations and partial failures.
use crate::support::common::tag;
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::{
    DATA_VERSION,
    heightmap::HeightmapType,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        ore_feature::ScatteredOreFeature,
        placement::{PlacementContext, PlacementQueryError},
        tags::TagDefinitions,
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a, W> {
    states: BTreeMap<[i32; 3], BlockStateId>,
    blocks: Vec<BlockStateId>,
    minimum: i32,
    mode: usize,
    center: [i32; 3],
    calls: i32,
    fail: i32,
    reject: i32,
    out: &'a mut W,
}
impl<W: Write> PlacementContext for World<'_, W> {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(self.minimum, 384, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        writeln!(self.out, "min").map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            return Err(PlacementQueryError::Failed("minimum query failure".into()));
        }
        writeln!(self.out, "minimum {}", self.minimum)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        Ok(self.minimum)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
}
impl<W: Write> FeatureWorld for World<'_, W> {
    type Error = io::Error;
    fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, Self::Error> {
        let call = self.calls;
        self.calls += 1;
        writeln!(self.out, "get {} {} {}", p[0], p[1], p[2])?;
        if call == self.fail {
            return Err(io::Error::other("world failure"));
        }
        let hash = p[0]
            .wrapping_mul(31)
            .wrapping_add(p[1].wrapping_mul(7))
            .wrapping_add(p[2].wrapping_mul(17))
            .rem_euclid(7) as usize;
        let mut fallback = if self.mode == 0 {
            self.blocks[hash]
        } else {
            self.blocks[3]
        };
        if self.mode >= 2 {
            let delta = [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ][self.mode - 2];
            let adjacent = std::array::from_fn(|i| self.center[i].wrapping_add(delta[i]));
            if p == adjacent {
                fallback = self.blocks[1];
            }
        }
        let state = self.states.get(&p).copied().unwrap_or(fallback);
        writeln!(self.out, "state {}", state.0)?;
        Ok(state)
    }
    fn set_block(
        &mut self,
        p: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, Self::Error> {
        let call = self.calls;
        self.calls += 1;
        writeln!(
            self.out,
            "set {} {} {} {} {flags}",
            p[0], p[1], p[2], state.0
        )?;
        if call == self.fail {
            return Err(io::Error::other("world failure"));
        }
        let result = self.reject == 0 || call % self.reject != 0;
        if result {
            self.states.insert(p, state);
        }
        writeln!(self.out, "written {result}")?;
        Ok(result)
    }
}
fn fixture(
    f: &Value,
    mut random: impl RandomSource,
    blocks: &[BlockStateId],
    bindings: &FeatureBlockBindings<'_>,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let decoded = ScatteredOreFeature::from_json(f["definition"].as_str().unwrap(), bindings);
    match decoded {
        Err(_) => writeln!(out, "decode error")?,
        Ok(feature) => {
            writeln!(out, "decode ok")?;
            let mut world = World {
                states: BTreeMap::new(),
                blocks: blocks.to_vec(),
                minimum: f["world_min"].as_i64().unwrap() as i32,
                mode: f["mode"].as_u64().unwrap() as usize,
                center: std::array::from_fn(|i| f["origin"][i].as_i64().unwrap() as i32),
                calls: 0,
                fail: f["fail"].as_i64().unwrap() as i32,
                reject: f["reject"].as_i64().unwrap() as i32,
                out,
            };
            let p = &f["origin"];
            let origin = [
                p[0].as_i64().unwrap() as i32,
                p[1].as_i64().unwrap() as i32,
                p[2].as_i64().unwrap() as i32,
            ];
            for _ in 0..f["repeats"].as_u64().unwrap() {
                world.calls = 0;
                let result = feature.place(&mut world, &mut random, origin);
                match result {
                    Ok(value) => writeln!(world.out, "place {value}")?,
                    Err(_) => writeln!(world.out, "place error")?,
                }
                writeln!(world.out, "next {}", random.next_long())?;
                for (p, state) in &world.states {
                    writeln!(world.out, "stored {} {} {} {}", p[0], p[1], p[2], state.0)?;
                }
                writeln!(world.out, "end")?;
            }
        }
    }
    writeln!(out, "tail {}", random.next_long())?;
    Ok(())
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let facts: Value = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", tag(&facts["states"])),
        ("defaults", tag(&facts["defaults"])),
    ]))?;
    let integers = facts["integers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v[0].as_str().unwrap().to_owned(),
                v[1].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let resources: Value = serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut definitions = TagDefinitions::default();
    for (name, text) in resources.as_object().unwrap() {
        definitions.apply_json(name, "vanilla", text.as_str().unwrap())?;
    }
    let built =
        definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    assert!(built.missing.is_empty());
    let bindings = FeatureBlockBindings::with_tags(&registry, integers, &built.tags)?;
    let blocks: Vec<_> = [
        "air",
        "cave_air",
        "void_air",
        "stone",
        "water",
        "oak_leaves",
        "dirt",
    ]
    .map(|name| {
        registry
            .default_state(&format!("minecraft:{name}"))
            .unwrap()
    })
    .to_vec();
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, LegacyRandom::new(seed), &blocks, &bindings, &mut out)?;
        } else {
            fixture(&f, XoroshiroRandom::new(seed), &blocks, &bindings, &mut out)?;
        }
    }
    Ok(())
}
