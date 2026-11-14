//! Iterative JSON/NBT conversion, following 26.3 JsonOps and NbtOps.
//!
//! Call at boundaries, not on the tick. NbtAccounter's 512-container limit
//! applies before cloning payloads; scalar and typed-array tags do not add
//! nesting. Conversion is intentionally lossy: JSON has no numeric tag widths,
//! typed arrays become lists, and byte booleans become JSON numbers.

use crate::nbt::{Compound, JavaString, Tag};
use serde_json::{Map, Number, Value};

const MAX_DEPTH: usize = 512;

/// A value that cannot cross the shared JSON/NBT representation boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonNbtError {
    /// JsonOps exposes nested JsonNull as Java null and refuses conversion.
    NestedNull,
    /// More than vanilla NBT's 512 nested list/compound containers.
    TooDeep,
    /// Tag→JSON only: JSON Value cannot hold NaN or infinity. Gson's strict
    /// JSON writer also refuses these numbers. Never substitute null or text.
    NonFinite,
    /// Tag→JSON only: a string or compound key contains an unpaired UTF-16
    /// surrogate. JSON Value uses UTF-8; never replace invalid units lossily.
    InvalidString,
}
impl std::fmt::Display for JsonNbtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NestedNull => "JSON-to-NBT does not accept nested null",
            Self::TooDeep => "NBT nesting exceeds 512 containers",
            Self::NonFinite => "JSON cannot represent a non-finite NBT number",
            Self::InvalidString => "JSON cannot represent an unpaired UTF-16 surrogate",
        })
    }
}
impl std::error::Error for JsonNbtError {}

/// Convert JSON metadata using JsonOps→NbtOps numeric and nested-null rules.
/// Top-level null becomes End. Numeric choice uses the JSON number's decimal
/// spelling, already rounded by serde_json. UTF-8 input preserves valid UTF-16
/// pairs; lone surrogates cannot reach this function through JSON Value.
pub fn json_to_tag(value: &Value) -> Result<Tag, JsonNbtError> {
    build_tag(JsonInput::Value(value), 0)
}

/// Convert a borrowed object without cloning the JSON tree. The object itself
/// counts toward the same 512-container limit as an object in json_to_tag.
pub fn json_to_compound(values: &Map<String, Value>) -> Result<Compound, JsonNbtError> {
    let Tag::Compound(values) = build_tag(JsonInput::Object(values), 0)? else {
        unreachable!("an object builds a compound")
    };
    Ok(values)
}

/// Convert a projection of borrowed object fields without cloning JSON payloads.
/// `parent_depth` counts surrounding NBT containers in the caller's output
/// schema. It includes synthetic metadata wrappers not present in the input.
pub fn json_fields_to_compound<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a Value)>,
    parent_depth: usize,
) -> Result<Compound, JsonNbtError> {
    let fields: Vec<_> = fields.into_iter().collect();
    let Tag::Compound(compound) = build_tag(JsonInput::Fields(&fields), parent_depth)? else {
        unreachable!("fields build a compound")
    };
    Ok(compound)
}

#[derive(Clone, Copy)]
enum JsonInput<'a> {
    Value(&'a Value),
    Object(&'a Map<String, Value>),
    Fields(&'a [(&'a str, &'a Value)]),
}
fn validate_json(root: JsonInput<'_>, parent_depth: usize) -> Result<(), JsonNbtError> {
    let mut pending = vec![(root, parent_depth, false)];
    while let Some((input, depth, nested)) = pending.pop() {
        match input {
            JsonInput::Value(Value::Null) if nested => return Err(JsonNbtError::NestedNull),
            JsonInput::Value(Value::Array(values)) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(
                    values
                        .iter()
                        .rev()
                        .map(|v| (JsonInput::Value(v), depth + 1, true)),
                );
            }
            JsonInput::Value(Value::Object(values)) | JsonInput::Object(values) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(
                    values
                        .values()
                        .rev()
                        .map(|v| (JsonInput::Value(v), depth + 1, true)),
                );
            }
            JsonInput::Fields(fields) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(
                    fields
                        .iter()
                        .rev()
                        .map(|(_, v)| (JsonInput::Value(v), depth + 1, true)),
                );
            }
            _ => {}
        }
    }
    Ok(())
}
fn build_tag(root: JsonInput<'_>, parent_depth: usize) -> Result<Tag, JsonNbtError> {
    validate_json(root, parent_depth)?;
    enum Work<'a> {
        Enter(JsonInput<'a>),
        List(usize),
        Object(Vec<JavaString>),
    }
    let mut work = vec![Work::Enter(root)];
    let mut done = Vec::new();
    while let Some(step) = work.pop() {
        match step {
            Work::Enter(input) => match input {
                JsonInput::Value(Value::Null) => done.push(Tag::End),
                JsonInput::Value(Value::Bool(v)) => done.push(Tag::Byte(i8::from(*v))),
                JsonInput::Value(Value::String(v)) => done.push(Tag::String(v.as_str().into())),
                JsonInput::Value(Value::Number(v)) => done.push(number(v)),
                JsonInput::Value(Value::Array(values)) => {
                    work.push(Work::List(values.len()));
                    work.extend(
                        values
                            .iter()
                            .rev()
                            .map(|v| Work::Enter(JsonInput::Value(v))),
                    );
                }
                JsonInput::Value(Value::Object(values)) | JsonInput::Object(values) => {
                    work.push(Work::Object(
                        values.keys().map(|k| k.as_str().into()).collect(),
                    ));
                    work.extend(
                        values
                            .values()
                            .rev()
                            .map(|v| Work::Enter(JsonInput::Value(v))),
                    );
                }
                JsonInput::Fields(fields) => {
                    work.push(Work::Object(
                        fields.iter().map(|(key, _)| (*key).into()).collect(),
                    ));
                    work.extend(
                        fields
                            .iter()
                            .rev()
                            .map(|(_, value)| Work::Enter(JsonInput::Value(value))),
                    );
                }
            },
            Work::List(len) => {
                let values = done.split_off(done.len() - len);
                done.push(Tag::List(values));
            }
            Work::Object(keys) => {
                let values = done.split_off(done.len() - keys.len());
                done.push(Tag::Compound(keys.into_iter().zip(values).collect()));
            }
        }
    }
    Ok(done.pop().expect("one root was built"))
}

fn validate_tag(root: &Tag) -> Result<(), JsonNbtError> {
    let valid_string = |s: &JavaString| {
        if char::decode_utf16(s.0.iter().copied()).any(|c| c.is_err()) {
            Err(JsonNbtError::InvalidString)
        } else {
            Ok(())
        }
    };
    let mut pending = vec![(root, 0)];
    while let Some((tag, depth)) = pending.pop() {
        match tag {
            Tag::Float(bits) if !f32::from_bits(*bits).is_finite() => {
                return Err(JsonNbtError::NonFinite);
            }
            Tag::Double(bits) if !f64::from_bits(*bits).is_finite() => {
                return Err(JsonNbtError::NonFinite);
            }
            Tag::String(s) => valid_string(s)?,
            Tag::List(values) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                pending.extend(values.iter().rev().map(|v| (v, depth + 1)));
            }
            Tag::Compound(values) => {
                if depth >= MAX_DEPTH {
                    return Err(JsonNbtError::TooDeep);
                }
                for key in values.keys() {
                    valid_string(key)?;
                }
                pending.extend(values.values().rev().map(|v| (v, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Convert using NbtOps→JsonOps, preserving its intended loss of numeric widths,
/// boolean identity and typed-array identity. Float numbers use Java's decimal
/// spelling, not the expanded f64 value of a promoted f32.
///
/// NonFinite and InvalidString are explicit representation refusals: serde_json
/// cannot hold those numbers or UTF-16 strings, and Gson's strict writer also
/// refuses non-finite numbers. No replacement with null, strings or U+FFFD.
pub fn tag_to_json(root: &Tag) -> Result<Value, JsonNbtError> {
    JsonTag::new(root).map(JsonTag::to_json)
}

/// Borrowed proof that a tag fits the JSON representation and nesting bound.
/// Construct all proofs before building a larger output to avoid partial JSON
/// temporaries when a later payload is refused.
#[derive(Clone, Copy)]
pub struct JsonTag<'a> {
    root: &'a Tag,
}
impl<'a> JsonTag<'a> {
    /// Check without cloning payloads or constructing JSON.
    pub fn new(root: &'a Tag) -> Result<Self, JsonNbtError> {
        validate_tag(root)?;
        Ok(Self { root })
    }
    /// Construct JSON from the validated, immutably borrowed tag.
    pub fn to_json(self) -> Value {
        enum Work<'a> {
            Enter(&'a Tag),
            List(usize),
            Object(Vec<String>),
        }
        let mut work = vec![Work::Enter(self.root)];
        let mut done = Vec::new();
        while let Some(step) = work.pop() {
            match step {
                Work::Enter(tag) => {
                    let value = match tag {
                        Tag::End => Value::Null,
                        Tag::Byte(v) => Value::from(*v),
                        Tag::Short(v) => Value::from(*v),
                        Tag::Int(v) => Value::from(*v),
                        Tag::Long(v) => Value::from(*v),
                        Tag::Float(bits) => Value::Number(
                            Number::from_f64(
                                crate::java_number::float_to_string(f32::from_bits(*bits))
                                    .parse::<f64>()
                                    .expect("validated finite float"),
                            )
                            .expect("validated finite float"),
                        ),
                        // Do not reparse through serde_json: its optional parser
                        // features must not affect an already typed double's bits.
                        Tag::Double(bits) => Value::Number(
                            Number::from_f64(f64::from_bits(*bits))
                                .expect("validated finite double"),
                        ),
                        Tag::String(s) => Value::String(s.to_string().expect("validated UTF-16")),
                        Tag::ByteArray(values) => {
                            Value::Array(values.iter().map(|v| Value::from(*v as i8)).collect())
                        }
                        Tag::IntArray(values) => {
                            Value::Array(values.iter().copied().map(Value::from).collect())
                        }
                        Tag::LongArray(values) => {
                            Value::Array(values.iter().copied().map(Value::from).collect())
                        }
                        Tag::List(values) => {
                            work.push(Work::List(values.len()));
                            work.extend(values.iter().rev().map(Work::Enter));
                            continue;
                        }
                        Tag::Compound(values) => {
                            work.push(Work::Object(
                                values
                                    .keys()
                                    .map(|k| k.to_string().expect("validated UTF-16"))
                                    .collect(),
                            ));
                            work.extend(values.values().rev().map(Work::Enter));
                            continue;
                        }
                    };
                    done.push(value);
                }
                Work::List(len) => {
                    let values = done.split_off(done.len() - len);
                    done.push(Value::Array(values));
                }
                Work::Object(keys) => {
                    let values = done.split_off(done.len() - keys.len());
                    done.push(Value::Object(keys.into_iter().zip(values).collect()));
                }
            }
        }
        done.pop().expect("one root was built")
    }
}

fn number(number: &Number) -> Tag {
    // BigDecimal.longValueExact in JsonOps: do not round a fraction or a u64
    // through f64 into an apparently integral signed long.
    if let Some(value) = exact_long(&number.to_string()) {
        if let Ok(value) = i8::try_from(value) {
            return Tag::Byte(value);
        }
        if let Ok(value) = i16::try_from(value) {
            return Tag::Short(value);
        }
        if let Ok(value) = i32::try_from(value) {
            return Tag::Int(value);
        }
        return Tag::Long(value);
    }
    let value = number
        .as_f64()
        .expect("a JSON number is representable as f64");
    let float = value as f32;
    if f64::from(float) == value {
        Tag::Float(if float == 0.0 { 0 } else { float.to_bits() })
    } else {
        Tag::Double(if value == 0.0 { 0 } else { value.to_bits() })
    }
}

/// Decimal integer extraction without rounding. The input is Number's valid
/// decimal spelling; this is not a second JSON parser.
fn exact_long(decimal: &str) -> Option<i64> {
    let (mantissa, exponent) = decimal
        .split_once(['e', 'E'])
        .map_or((decimal, 0), |(m, e)| {
            (m, e.parse::<i32>().expect("JSON exponent"))
        });
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let fractional = mantissa.split_once('.').map_or(0, |(_, f)| f.len() as i32);
    let mut digits: Vec<u8> = mantissa.bytes().filter(|b| *b != b'.').collect();
    let mut scale = exponent - fractional;
    while scale < 0 && digits.last() == Some(&b'0') {
        digits.pop();
        scale += 1;
    }
    if digits.iter().all(|b| *b == b'0') {
        return Some(0);
    }
    if !(0..=18).contains(&scale) {
        return None;
    }
    // Accumulate negative so i64::MIN is representable as well.
    let mut value = 0i64;
    for digit in digits {
        value = value
            .checked_mul(10)?
            .checked_sub(i64::from(digit - b'0'))?;
    }
    for _ in 0..scale {
        value = value.checked_mul(10)?;
    }
    if negative {
        Some(value)
    } else {
        value.checked_neg()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn tag(value: &Value) -> Tag {
        json_to_tag(value).unwrap()
    }

    #[test]
    fn refuses_nested_null_without_inventing_end_payloads() {
        assert_eq!(json_to_tag(&Value::Null), Ok(Tag::End));
        for value in [json!([null]), json!({"x": null}), json!({"x": [1, null]})] {
            assert_eq!(json_to_tag(&value), Err(JsonNbtError::NestedNull));
        }
    }

    #[test]
    fn numeric_boundaries_do_not_round_through_double() {
        for (source, expected) in [
            ("-128", Tag::Byte(-128)),
            ("127", Tag::Byte(127)),
            ("128", Tag::Short(128)),
            ("-129", Tag::Short(-129)),
            ("32767", Tag::Short(32767)),
            ("32768", Tag::Int(32768)),
            ("-32769", Tag::Int(-32769)),
            ("2147483647", Tag::Int(i32::MAX)),
            ("2147483648", Tag::Long(2147483648)),
            ("-9223372036854775808", Tag::Long(i64::MIN)),
            ("9223372036854775807", Tag::Long(i64::MAX)),
            (
                "9223372036854775808",
                Tag::Float((9223372036854775808.0f32).to_bits()),
            ),
            ("1.0", Tag::Byte(1)),
            ("1e3", Tag::Short(1000)),
            ("1.5", Tag::Float(1.5f32.to_bits())),
            ("0.1", Tag::Double(0.1f64.to_bits())),
            ("-0.0", Tag::Byte(0)),
        ] {
            assert_eq!(
                tag(&serde_json::from_str::<Value>(source).unwrap()),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn json_arrays_are_never_nbt_arrays_or_compound_wrappers() {
        assert_eq!(tag(&json!([])), Tag::List(vec![]));
        assert_eq!(
            tag(&json!([1, 2])),
            Tag::List(vec![Tag::Byte(1), Tag::Byte(2)])
        );
        assert_eq!(
            tag(&json!([32768, 2147483648i64])),
            Tag::List(vec![Tag::Int(32768), Tag::Long(2147483648)])
        );
        assert_eq!(
            tag(&json!([true, "x", {}])),
            Tag::List(vec![
                Tag::Byte(1),
                Tag::String("x".into()),
                Tag::compound([] as [(&str, Tag); 0])
            ])
        );
    }

    #[test]
    fn metadata_preserves_boolean_strings_and_nested_list() {
        let tag =
            tag(&json!({"custom": {"enabled": false, "note": "present"}, "lore": ["a", "b"]}));
        assert_eq!(
            tag.get("custom").unwrap().get("enabled"),
            Some(&Tag::Byte(0))
        );
        assert_eq!(
            tag.get("custom").unwrap().get("note"),
            Some(&Tag::String("present".into()))
        );
        assert_eq!(
            tag.get("lore"),
            Some(&Tag::List(vec![
                Tag::String("a".into()),
                Tag::String("b".into())
            ]))
        );
    }
    #[test]
    fn reverse_follows_vanilla_types_and_java_decimal_spelling() {
        for (source, expected) in [
            (Tag::End, "null"),
            (Tag::Byte(1), "1"),
            (Tag::Short(-129), "-129"),
            (Tag::Int(i32::MAX), "2147483647"),
            (Tag::Long(i64::MAX), "9223372036854775807"),
            (Tag::Float(0.1f32.to_bits()), "0.1"),
            (Tag::Float(1), "1.4e-45"),
            (Tag::Double(0.1f64.to_bits()), "0.1"),
            (Tag::Double(1), "4.9e-324"),
            (Tag::ByteArray(vec![0, 127, 128, 255]), "[0,127,-128,-1]"),
            (Tag::IntArray(vec![i32::MIN, 4]), "[-2147483648,4]"),
            (
                Tag::LongArray(vec![i64::MIN, i64::MAX]),
                "[-9223372036854775808,9223372036854775807]",
            ),
        ] {
            assert_eq!(
                tag_to_json(&source).unwrap(),
                serde_json::from_str::<Value>(expected).unwrap()
            );
        }
        assert_eq!(
            tag_to_json(&Tag::Float((-0.0f32).to_bits()))
                .unwrap()
                .to_string(),
            "-0.0"
        );
        let source = Tag::compound([
            (
                "first",
                Tag::List(vec![Tag::Byte(3), Tag::String("s".into())]),
            ),
            ("second", Tag::Compound(Compound::new())),
        ]);
        assert_eq!(
            tag_to_json(&source).unwrap(),
            json!({"first": [3, "s"], "second": {}})
        );
        // NbtOps→JsonOps is not an NBT identity codec.
        for (source, normalized) in [
            (Tag::Long(1), Tag::Byte(1)),
            (Tag::Float(0.1f32.to_bits()), Tag::Double(0.1f64.to_bits())),
            (Tag::ByteArray(vec![255]), Tag::List(vec![Tag::Byte(-1)])),
        ] {
            assert_eq!(
                json_to_tag(&tag_to_json(&source).unwrap()).unwrap(),
                normalized
            );
        }
        assert_eq!(
            tag_to_json(&json_to_tag(&json!(true)).unwrap()).unwrap(),
            json!(1)
        );
    }

    #[test]
    fn reverse_numbers_do_not_depend_on_json_parser_features() {
        for value in [
            123456789012345.67,
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::from_bits(1),
            -0.0,
        ] {
            let json = tag_to_json(&Tag::Double(value.to_bits())).unwrap();
            assert_eq!(json.as_f64().unwrap().to_bits(), value.to_bits());
        }
        for (value, expected) in [
            (0.1f32, 0.1f64),
            (f32::from_bits(1), 1.4e-45f64),
            (f32::MAX, 3.4028235e38f64),
        ] {
            let json = tag_to_json(&Tag::Float(value.to_bits())).unwrap();
            assert_eq!(json.as_f64().unwrap().to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn reverse_refuses_nonfinite_and_unpaired_utf16_without_replacement() {
        for source in [
            Tag::Float(f32::NAN.to_bits()),
            Tag::Double(f64::INFINITY.to_bits()),
            Tag::Float(f32::NEG_INFINITY.to_bits()),
            Tag::Double(0x7ff0000000000001),
            Tag::List(vec![Tag::Double(f64::NEG_INFINITY.to_bits())]),
        ] {
            assert_eq!(tag_to_json(&source), Err(JsonNbtError::NonFinite));
        }
        for source in [
            Tag::String(JavaString(vec![0xd800])),
            Tag::String(JavaString(vec![0xdc00])),
            Tag::Compound([(JavaString(vec![0xdbff]), Tag::Byte(1))].into()),
        ] {
            assert_eq!(tag_to_json(&source), Err(JsonNbtError::InvalidString));
        }
        let pair = JavaString(vec![0xd83d, 0xde00]);
        assert_eq!(
            tag_to_json(&Tag::String(pair.clone())).unwrap(),
            json!("😀")
        );
        assert_eq!(
            tag_to_json(&Tag::Compound([(pair.clone(), Tag::Byte(7))].into())).unwrap(),
            json!({"😀": 7})
        );
        assert_eq!(json_to_tag(&json!("😀")).unwrap(), Tag::String(pair));
    }

    fn discard_json(root: Value) {
        let mut work = vec![root];
        while let Some(value) = work.pop() {
            match value {
                Value::Array(v) => work.extend(v),
                Value::Object(v) => work.extend(v.into_iter().map(|(_, v)| v)),
                _ => {}
            }
        }
    }
    fn discard_tag(root: Tag) {
        let mut work = vec![root];
        while let Some(value) = work.pop() {
            match value {
                Tag::List(v) => work.extend(v),
                Tag::Compound(v) => work.extend(v.into_values()),
                _ => {}
            }
        }
    }
    fn nested_json(depth: usize) -> Value {
        (0..depth).fold(json!(1), |value, n| {
            if n % 2 == 0 {
                Value::Array(vec![value])
            } else {
                Value::Object([("child".into(), value)].into_iter().collect())
            }
        })
    }
    fn nested_tag(depth: usize, leaf: Tag) -> Tag {
        (0..depth).fold(leaf, |value, n| {
            if n % 2 == 0 {
                Tag::List(vec![value])
            } else {
                Tag::compound([("child", value)])
            }
        })
    }

    #[test]
    fn conversion_accepts_512_containers_and_preflights_513_on_256_kib() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let source = nested_json(512);
                let converted = json_to_tag(&source).unwrap();
                let restored = tag_to_json(&converted).unwrap();
                let mut leaf = &restored;
                for _ in 0..512 {
                    leaf = match leaf {
                        Value::Array(v) => &v[0],
                        Value::Object(v) => &v["child"],
                        _ => panic!("missing container"),
                    };
                }
                assert_eq!(leaf, &json!(1));
                discard_json(source);
                discard_tag(converted);
                discard_json(restored);
                for depth in [513, 4096] {
                    let json = nested_json(depth);
                    assert!(matches!(json_to_tag(&json), Err(JsonNbtError::TooDeep)));
                    discard_json(json);
                    let tag = nested_tag(depth, Tag::Byte(1));
                    assert!(matches!(tag_to_json(&tag), Err(JsonNbtError::TooDeep)));
                    discard_tag(tag);
                }
                // json_to_compound must count its already borrowed root object.
                for (children, accepted) in [(511, true), (512, false)] {
                    let map = [("child".into(), nested_json(children))]
                        .into_iter()
                        .collect();
                    let result = json_to_compound(&map);
                    if accepted {
                        discard_tag(Tag::Compound(result.unwrap()));
                    } else {
                        assert!(matches!(result, Err(JsonNbtError::TooDeep)));
                    }
                    discard_json(Value::Object(map));
                }
                // Primitive arrays have no recursive NBT containers of their own.
                let tag = nested_tag(512, Tag::ByteArray(vec![255]));
                let json = tag_to_json(&tag).unwrap();
                discard_tag(tag);
                discard_json(json);
            })
            .unwrap()
            .join()
            .unwrap();
    }
    #[test]
    fn borrowed_projection_counts_wrappers_and_skips_unselected_data() {
        let map: Map<_, _> = [
            ("selected".into(), nested_json(511)),
            ("ignored".into(), nested_json(4096)),
        ]
        .into_iter()
        .collect();
        let selected = || {
            map.iter()
                .filter(|(k, _)| k.as_str() == "selected")
                .map(|(k, v)| (k.as_str(), v))
        };
        let compound = json_fields_to_compound(selected(), 0).unwrap();
        discard_tag(Tag::Compound(compound));
        assert!(matches!(
            json_fields_to_compound(selected(), 1),
            Err(JsonNbtError::TooDeep)
        ));
        discard_json(Value::Object(map));
    }
}
