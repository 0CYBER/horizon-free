//! Feature world transport retaining ordered mutations and partial failures.
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::{
    heightmap::HeightmapType,
    registry::BlockStateId,
    worldgen::{
        anchor::WorldGenerationContext,
        feature::{FeatureBlocks, FeatureWorld},
        feature_graph::FeatureDefinition,
        placement::{FeaturePlacer, PlacementContext, PlacementModifier, PlacementQueryError},
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
impl<W> PlacementContext for World<'_, W> {
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
    mut random: impl RandomSource,
    blocks: [BlockStateId; 5],
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    let decoded =
        FeatureDefinition::from_json(f["definition"].as_str().unwrap()).and_then(|feature| {
            let modifiers = f
                .get("placement")
                .map(|v| {
                    v.as_array()
                        .unwrap()
                        .iter()
                        .map(|v| PlacementModifier::from_json(&v.to_string()))
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
    let ids: [u32; 5] = serde_json::from_str(&fs::read_to_string(dir.join("facts-server.json"))?)?;
    let blocks = ids.map(BlockStateId);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, LegacyRandom::new(seed), blocks, &mut out)?;
        } else {
            fixture(&f, XoroshiroRandom::new(seed), blocks, &mut out)?;
        }
    }
    Ok(())
}
