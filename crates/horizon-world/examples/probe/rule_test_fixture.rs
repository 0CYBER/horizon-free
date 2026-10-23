//! Differential transport for replacement rules over pinned block identities.
use crate::support::common::tag;
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use horizon_world::{
    DATA_VERSION,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{block_feature::FeatureBlockBindings, rule_test::RuleTest, tags::TagDefinitions},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
fn fixture(
    f: &Value,
    bindings: &FeatureBlockBindings<'_>,
    mut random: impl RandomSource,
    out: &mut impl Write,
) -> Result<(), Box<dyn std::error::Error>> {
    match RuleTest::from_json(f["definition"].as_str().unwrap(), bindings) {
        Err(_) => writeln!(out, "decode error")?,
        Ok(rule) => {
            writeln!(out, "decode ok")?;
            let mut index = 0;
            let heights = f["heights"].as_array().unwrap();
            for _ in 0..f["repeats"].as_u64().unwrap() {
                for state in f["states"].as_array().unwrap() {
                    let y = heights[index % heights.len()].as_i64().unwrap() as i32;
                    index += 1;
                    let result = rule.test(
                        BlockStateId(state.as_u64().unwrap() as u32),
                        [17, y, -33],
                        &mut random,
                    );
                    writeln!(out, "test {result} {}", random.next_long())?;
                }
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
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let seed = f["seed"].as_i64().unwrap();
        if f["legacy"].as_bool().unwrap() {
            fixture(&f, &bindings, LegacyRandom::new(seed), &mut out)?;
        } else {
            fixture(&f, &bindings, XoroshiroRandom::new(seed), &mut out)?;
        }
    }
    Ok(())
}
