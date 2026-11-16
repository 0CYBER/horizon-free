//! Lossless conversion at the data-map JSON boundary.
use horizon_core::nbt::Tag;
use mod_api::DataMapError;
use serde_json::Value;

pub(super) fn to_tag(value: &Value) -> Result<Tag, DataMapError> {
    Ok(match value {
        Value::Null => return Err(DataMapError::Undecodable),
        Value::Bool(value) => Tag::Byte(i8::from(*value)),
        Value::Number(value) => {
            if let Some(integer) = value.as_i64() {
                i32::try_from(integer).map_or(Tag::Long(integer), Tag::Int)
            } else if value.is_u64() {
                return Err(DataMapError::Undecodable);
            } else {
                Tag::Double(value.as_f64().ok_or(DataMapError::Undecodable)?.to_bits())
            }
        }
        Value::String(value) => Tag::String(value.as_str().into()),
        Value::Array(values) => Tag::List(values.iter().map(to_tag).collect::<Result<_, _>>()?),
        Value::Object(values) => Tag::Compound(
            values
                .iter()
                .map(|(key, value)| Ok((key.as_str().into(), to_tag(value)?)))
                .collect::<Result<_, DataMapError>>()?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn refuses_unrepresentable_values_including_nested_values() {
        for value in [
            json!(null),
            json!({"nested": [null]}),
            json!(9223372036854775809_u64),
            json!(u64::MAX),
            json!([u64::MAX]),
        ] {
            assert_eq!(to_tag(&value), Err(DataMapError::Undecodable), "{value}");
        }
    }

    #[test]
    fn preserves_numeric_width_and_structural_values() {
        for (value, expected) in [
            (json!(i32::MIN), Tag::Int(i32::MIN)),
            (json!(i32::MAX), Tag::Int(i32::MAX)),
            (json!(i64::MIN), Tag::Long(i64::MIN)),
            (json!(i64::MAX), Tag::Long(i64::MAX)),
            (json!(2147483648_i64), Tag::Long(2147483648)),
            (json!(1.25), Tag::Double(1.25_f64.to_bits())),
            (json!(true), Tag::Byte(1)),
            (json!(false), Tag::Byte(0)),
            (json!("text"), Tag::String("text".into())),
            (json!([]), Tag::List(vec![])),
            (
                json!([1, "text", false]),
                Tag::List(vec![Tag::Int(1), Tag::String("text".into()), Tag::Byte(0)]),
            ),
        ] {
            assert_eq!(to_tag(&value), Ok(expected), "{value}");
        }
    }
}
