use horizon_assets::blockstate::{
    BlockStateDefinition, Selector, Variant, VariantSet, bind::SelectedModels,
    choice::CompiledChoices,
};
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom};
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};
fn choice(value: &Value, index: usize) -> VariantSet {
    let variant = |suffix| Variant {
        model: format!("horizon:part{index}_{suffix}"),
        rotations: [0; 3],
        uv_lock: false,
    };
    if value.is_null() {
        VariantSet::Single(variant("single".to_owned()))
    } else {
        VariantSet::Weighted(
            value
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, w)| (variant(format!("variant{i}")), w.as_i64().unwrap() as i32))
                .collect(),
        )
    }
}
fn trace(
    mut random: impl RandomSource,
    compiled: &CompiledChoices,
    selected: &SelectedModels,
    repeats: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut result = Vec::new();
    let mut output = Vec::new();
    for _ in 0..repeats {
        output.clear();
        compiled.collect(selected, &mut random, &mut output)?;
        result.push(json!({"models":output.iter().map(|v|&v.model).collect::<Vec<_>>(),"next_long":random.next_long(),"next_int":random.next_int()}));
    }
    Ok(json!(result))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut output = BufWriter::new(File::create(&args[2])?);
    for line in input.lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let parts = row["parts"].as_array().unwrap();
        let (definition, selected) = if row["multipart"].as_bool().unwrap() {
            (
                BlockStateDefinition {
                    variants: None,
                    multipart: Some(
                        parts
                            .iter()
                            .enumerate()
                            .map(|(i, v)| Selector {
                                condition: None,
                                variant: choice(v, i),
                            })
                            .collect(),
                    ),
                },
                SelectedModels::Multipart(
                    row["selected"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_u64().unwrap() as usize)
                        .collect(),
                ),
            )
        } else {
            (
                BlockStateDefinition {
                    variants: Some(vec![(String::new(), choice(&parts[0], 0))]),
                    multipart: None,
                },
                SelectedModels::Variant(0),
            )
        };
        let compiled = CompiledChoices::new(&definition)?;
        let seed = row["seed"].as_i64().unwrap();
        let repeats = row["repeats"].as_u64().unwrap();
        let result = if row["random"] == "legacy" {
            trace(LegacyRandom::new(seed), &compiled, &selected, repeats)?
        } else {
            trace(XoroshiroRandom::new(seed), &compiled, &selected, repeats)?
        };
        writeln!(output, "{}", json!({"name":row["name"],"trace":result}))?;
    }
    Ok(())
}
