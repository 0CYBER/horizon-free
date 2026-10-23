//! Feature world transport retaining ordered mutations and partial failures.
use crate::support::common::tag;
use horizon_core::random::{LegacyRandom, RandomSource, WorldgenRandom, XoroshiroRandom};
use horizon_world::{
    DATA_VERSION,
    heightmap::HeightmapType,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        block_feature::FeatureBlockBindings,
        feature::{FeatureBlocks, FeatureWorld},
        feature_catalog::FeatureCatalog,
        feature_graph::FeatureDefinition,
        placement::{FeaturePlacer, PlacementContext, PlacementModifier, PlacementQueryError},
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
    blocks: [BlockStateId; 5],
    calls: i32,
    fail: i32,
    reject: i32,
    out: &'a mut W,
}
impl<W: Write> PlacementContext for World<'_, W> {
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.block(position)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(319)
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(-64)
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
            .rem_euclid(5) as usize;
        let state = self.states.get(&p).copied().unwrap_or(self.blocks[hash]);
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
    catalog: &FeatureCatalog,
    bindings: &FeatureBlockBindings<'_>,
    mut random: impl RandomSource,
    blocks: [BlockStateId; 5],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let root = if f["placed_root"].as_bool().unwrap_or(false) {
        catalog
            .placed_feature_with_blocks(f["definition"].as_str().unwrap(), bindings)
            .map(|placed| FeatureDefinition::Sequence(vec![placed]))
    } else {
        catalog.feature_with_blocks(f["definition"].as_str().unwrap(), bindings)
    };
    let decoded = root.and_then(|feature| {
        let modifiers = f
            .get("placement")
            .map(|v| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| PlacementModifier::from_json_with_blocks(&v.to_string(), bindings))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        Ok((feature, modifiers))
    });
    match decoded {
        Err(_) => writeln!(out, "decode error")?,
        Ok((feature, modifiers)) => {
            writeln!(out, "decode ok")?;
            let mut world = World {
                states: BTreeMap::new(),
                blocks,
                calls: 0,
                fail: f["fail"].as_i64().unwrap() as i32,
                reject: f["reject"].as_i64().unwrap() as i32,
                out,
            };
            let blocks = FeatureBlocks {
                air: blocks[0],
                stone: blocks[1],
                cobblestone: blocks[2],
                end_stone: blocks[3],
                obsidian: blocks[4],
            };
            let p = &f["origin"];
            let origin = [
                p[0].as_i64().unwrap() as i32,
                p[1].as_i64().unwrap() as i32,
                p[2].as_i64().unwrap() as i32,
            ];
            let mut placer = FeaturePlacer::new();
            for _ in 0..f["repeats"].as_u64().unwrap() {
                world.calls = 0;
                let result = if let Some(modifiers) = &modifiers {
                    placer
                        .place_with_context(
                            modifiers,
                            &mut world,
                            &mut random,
                            origin,
                            |world, random, position| {
                                feature.place(world, random, position, blocks)
                            },
                        )
                        .map_err(|_| ())
                } else {
                    feature
                        .place(&mut world, &mut random, origin, blocks)
                        .map_err(|_| ())
                };
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
    let resources: Value = serde_json::from_str(&fs::read_to_string(dir.join("catalog.json"))?)?;
    let definitions = |name: &str| {
        resources[name]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect()
    };
    let mut tags = TagDefinitions::default();
    for (name, value) in resources["tags"].as_object().unwrap() {
        tags.apply_json(name, "fixture", value.as_str().unwrap())?;
    }
    let catalog =
        FeatureCatalog::from_resources(&definitions("features"), &definitions("placed"), &tags)?;
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
    let bindings = FeatureBlockBindings::new(&registry, integers)?;
    let blocks = ["air", "stone", "cobblestone", "end_stone", "obsidian"].map(|name| {
        registry
            .default_state(&format!("minecraft:{name}"))
            .unwrap()
    });
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        match (
            f["legacy"].as_bool().unwrap(),
            f["worldgen"].as_bool().unwrap(),
        ) {
            (true, false) => fixture(
                &f,
                &catalog,
                &bindings,
                LegacyRandom::new(seed),
                blocks,
                &mut out,
            )?,
            (true, true) => fixture(
                &f,
                &catalog,
                &bindings,
                WorldgenRandom::new(LegacyRandom::new(seed)),
                blocks,
                &mut out,
            )?,
            (false, false) => fixture(
                &f,
                &catalog,
                &bindings,
                XoroshiroRandom::new(seed),
                blocks,
                &mut out,
            )?,
            (false, true) => fixture(
                &f,
                &catalog,
                &bindings,
                WorldgenRandom::new(XoroshiroRandom::new(seed)),
                blocks,
                &mut out,
            )?,
        }
    }
    Ok(())
}
