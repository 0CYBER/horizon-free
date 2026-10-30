//! Block entity data on items for tools/test_pick_block_data_parity.py: reads
//! one JSON case per line (see VanillaPickBlockDataOracle) and prints the
//! picked item's component patch, a tab, then the placed block entity.
//!
//! usage: pick_block_data_fixture INPUT OUTPUT
use horizon_client::block_entity_item::{pick, place};
use horizon_client::block_entity_nbt::canonical;
use horizon_client::java_text::{double_to_string, float_to_string};
use horizon_world::nbt::{Tag, parse_snbt};
use serde_json::Value;
use std::fs;

/// ParityData.canon: sorted keys, typed numbers, Java's float text.
fn canon(tag: &Tag, out: &mut String) {
    let join = |out: &mut String, prefix: &str, items: Vec<String>| {
        out.push_str(prefix);
        out.push_str(&items.join(","));
        out.push(']');
    };
    match tag {
        Tag::Float(v) => out.push_str(&format!("{}f", float_to_string(f32::from_bits(*v)))),
        Tag::Double(v) => out.push_str(&format!("{}d", double_to_string(f64::from_bits(*v)))),
        Tag::List(items) => join(
            out,
            "[",
            items
                .iter()
                .map(|t| {
                    let mut s = String::new();
                    canon(t, &mut s);
                    s
                })
                .collect(),
        ),
        Tag::Compound(map) => {
            let mut keys: Vec<_> = map.iter().map(|(k, v)| (k.to_string_lossy(), v)).collect();
            keys.sort_by(|a, b| a.0.cmp(&b.0));
            out.push('{');
            for (i, (k, v)) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&k);
                out.push(':');
                canon(v, out);
            }
            out.push('}');
        }
        other => out.push_str(&horizon_client::item_data::canon_tag(other)),
    }
}

fn snbt(value: &Value) -> Option<Tag> {
    parse_snbt(value.as_str()?).ok().map(|(tag, _)| tag)
}

fn case(line: &str) -> Option<String> {
    let case: Value = serde_json::from_str(line).ok()?;
    let saved = canonical(&snbt(&case["nbt"])?)?;
    let picked = pick(&saved);
    let patch = match case.get("patch") {
        Some(p) => match snbt(p)? {
            Tag::Compound(c) => c,
            _ => return None,
        },
        None => picked.clone(),
    };
    let target = canonical(&snbt(&case["target"])?)?;
    let Tag::Compound(mut placed) = place(&target, &patch, case["op"].as_bool()?) else {
        return None;
    };
    // keepPacked is the chunk's, not saveWithFullMetadata's.
    placed.remove(&"keepPacked".into());
    let mut out = String::new();
    canon(&Tag::Compound(picked), &mut out);
    out.push('\t');
    canon(&Tag::Compound(placed), &mut out);
    Some(out)
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = fs::read_to_string(
        args.get(1)
            .ok_or("usage: pick_block_data_fixture INPUT OUTPUT")?,
    )?;
    let lines: Vec<String> = input
        .lines()
        .map(|line| case(line).unwrap_or_else(|| "error".to_owned()))
        .collect();
    fs::write(
        args.get(2).ok_or("missing OUTPUT")?,
        lines.join("\n") + "\n",
    )?;
    Ok(())
}
