use horizon_world::{nbt::Tag, retrogen::BelowZeroRetrogen};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
};
fn tag(row: &Value) -> Tag {
    let value = &row[1];
    match row[0].as_str().unwrap() {
        "end" => Tag::End,
        "byte" => Tag::Byte(value.as_i64().unwrap() as i8),
        "short" => Tag::Short(value.as_i64().unwrap() as i16),
        "int" => Tag::Int(value.as_i64().unwrap() as i32),
        "long" => Tag::Long(value.as_i64().unwrap()),
        "float" => Tag::Float(u32::from_str_radix(value.as_str().unwrap(), 16).unwrap()),
        "double" => Tag::Double(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap()),
        "string" => Tag::String(value.as_str().unwrap().into()),
        "list" => Tag::List(value.as_array().unwrap().iter().map(tag).collect()),
        "compound" => Tag::Compound(
            value
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, value)| (name.as_str().into(), tag(value)))
                .collect(),
        ),
        "long_array" => Tag::LongArray(
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect(),
        ),
        "int_array" => Tag::IntArray(
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i32)
                .collect(),
        ),
        "byte_array" => Tag::ByteArray(
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as u8)
                .collect(),
        ),
        _ => panic!("unknown tag"),
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let mut out = io::BufWriter::new(fs::File::create(&args[2])?);
    for (index, line) in fs::read_to_string(&args[1])?.lines().enumerate() {
        writeln!(out, "case {index}")?;
        let row: Value = serde_json::from_str(line)?;
        let Ok(data) = BelowZeroRetrogen::from_nbt(&tag(&row)) else {
            writeln!(out, "decoded error")?;
            continue;
        };
        let encoded = data.to_nbt();
        let Tag::Compound(fields) = &encoded else {
            unreachable!()
        };
        writeln!(
            out,
            "decoded {} {} {}",
            data.target_status().name(),
            data.has_bedrock_holes(),
            fields.len()
        )?;
        let Some(Tag::String(target)) = encoded.get("target_status") else {
            unreachable!()
        };
        writeln!(out, "encoded_target {}", target.to_string()?)?;
        let words = match encoded.get("missing_bedrock") {
            Some(Tag::LongArray(words)) => words
                .iter()
                .map(|v| format!("{:x}", *v as u64))
                .collect::<Vec<_>>()
                .join(","),
            None => "absent".into(),
            _ => unreachable!(),
        };
        writeln!(out, "mask {words}")?;
        let mut holes = String::new();
        for z in -17..=17 {
            for x in -17..=17 {
                holes.push(if data.has_bedrock_hole(x, z) {
                    '1'
                } else {
                    '0'
                });
            }
        }
        for z in [i32::MIN, i32::MAX] {
            for x in [i32::MIN, i32::MAX] {
                holes.push(if data.has_bedrock_hole(x, z) {
                    '1'
                } else {
                    '0'
                });
            }
        }
        writeln!(out, "holes {holes}")?;
    }
    out.flush()?;
    Ok(())
}
