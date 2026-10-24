//! Number formats and JSON readers several probes share.
use horizon_core::position::BlockPos;
use horizon_world::nbt::Tag;
use serde_json::Value;

/// An f64's bits, every NaN as Java's canonical one (Double.doubleToLongBits).
pub fn f64_bits(v: f64) -> u64 {
    if v.is_nan() {
        f64::NAN.to_bits()
    } else {
        v.to_bits()
    }
}

/// An f32's bits, every NaN as Java's canonical one (Float.floatToIntBits).
pub fn f32_bits(v: f32) -> u32 {
    if v.is_nan() {
        f32::NAN.to_bits()
    } else {
        v.to_bits()
    }
}

/// f64_bits as 16 hex digits.
pub fn f64_hex(v: f64) -> String {
    format!("{:016x}", f64_bits(v))
}

/// Registry metadata from the oracle's JSON as NBT: strings, ints, lists and compounds.
pub fn tag(v: &Value) -> Tag {
    match v {
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Number(n) => Tag::Int(n.as_i64().unwrap() as i32),
        Value::Array(a) => Tag::List(a.iter().map(tag).collect()),
        Value::Object(m) => {
            Tag::Compound(m.iter().map(|(k, v)| (k.as_str().into(), tag(v))).collect())
        }
        _ => panic!("invalid registry metadata"),
    }
}

/// A block position from a JSON `[x, y, z]`.
pub fn position(v: &Value) -> BlockPos {
    BlockPos::new(
        v[0].as_i64().unwrap() as i32,
        v[1].as_i64().unwrap() as i32,
        v[2].as_i64().unwrap() as i32,
    )
}
