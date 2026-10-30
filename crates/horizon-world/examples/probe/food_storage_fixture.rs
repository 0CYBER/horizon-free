use crate::support::common::f32_bits;
use horizon_world::{
    food::FoodData,
    nbt::{Compound, Tag},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
const KEYS: [&str; 4] = [
    "foodLevel",
    "foodTickTimer",
    "foodSaturationLevel",
    "foodExhaustionLevel",
];
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn tag(v: &Value) -> Option<Tag> {
    let value = &v["value"];
    Some(match v["type"].as_i64().unwrap() {
        0 => return None,
        1 => Tag::Byte(value.as_i64().unwrap() as i8),
        2 => Tag::Short(value.as_i64().unwrap() as i16),
        3 => Tag::Int(value.as_i64().unwrap() as i32),
        4 => Tag::Long(value.as_str().unwrap().parse().unwrap()),
        5 => {
            let f = single(value);
            Tag::Float(if f == 0.0 { 0 } else { f.to_bits() })
        }
        6 => {
            let f = f64::from_bits(u64::from_str_radix(value.as_str().unwrap(), 16).unwrap());
            Tag::Double(if f == 0.0 { 0 } else { f.to_bits() })
        }
        7 => Tag::ByteArray(vec![1]),
        8 => Tag::String("17".into()),
        9 => Tag::List(vec![]),
        10 => Tag::Compound(Compound::new()),
        11 => Tag::IntArray(vec![2]),
        12 => Tag::LongArray(vec![3]),
        _ => panic!("tag type"),
    })
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut food = FoodData {
            food: -9,
            saturation: -7.,
            timer: 83,
            exhaustion: 99.,
        };
        if f["mode"] == "read" {
            let mut t = Compound::new();
            for (key, v) in KEYS.iter().zip(f["tags"].as_array().unwrap()) {
                if let Some(v) = tag(v) {
                    t.insert((*key).into(), v);
                }
            }
            food.read_nbt(&t);
        } else {
            food = FoodData {
                food: f["food"].as_i64().unwrap() as i32,
                timer: f["timer"].as_i64().unwrap() as i32,
                saturation: single(&f["saturation"]),
                exhaustion: single(&f["exhaustion"]),
            };
        }
        writeln!(
            out,
            "state {} {} {:08x} {:08x}",
            food.food,
            food.timer,
            f32_bits(food.saturation),
            f32_bits(food.exhaustion)
        )?;
        let mut saved = Compound::new();
        saved.insert("unrelated".into(), Tag::Int(42));
        saved.insert("foodLevel".into(), Tag::String("replace".into()));
        food.write_nbt(&mut saved);
        let Tag::Int(unrelated) = saved[&"unrelated".into()] else {
            panic!("preserve unrelated")
        };
        writeln!(out, "saved {} {unrelated}", saved.len())?;
        for key in KEYS {
            match saved[&key.into()] {
                Tag::Int(v) => writeln!(out, "tag 3 {v}")?,
                Tag::Float(v) => writeln!(out, "tag 5 {:08x}", f32_bits(f32::from_bits(v)))?,
                _ => panic!("saved type"),
            }
        }
        food.read_nbt(&saved);
        writeln!(
            out,
            "roundtrip {} {} {:08x} {:08x}",
            food.food,
            food.timer,
            f32_bits(food.saturation),
            f32_bits(food.exhaustion)
        )?;
    }
    Ok(())
}
