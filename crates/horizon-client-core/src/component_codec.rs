//! NBT conversion for Minecraft 26.3 item data components (`DataComponents`),
//! paired with [`horizon_assets::component_codec`] for `ComponentContents` parity.
use crate::identifier::normalize_id;
use horizon_core::nbt::Tag;
use serde_json::Value;

pub use horizon_assets::component_codec::{canonical_text_component, canonicalize_component_json};

/// Converts an NBT tag to raw JSON, keeping numeric tags numeric (`Tag::Byte`
/// stays a number, unlike `scoreboard::tag_to_json`).
pub fn raw_tag_json(tag: &Tag) -> Value {
    match tag {
        Tag::End => Value::Null,
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(bits) => serde_json::Number::from_f64(f64::from(f32::from_bits(*bits)))
            .map_or(Value::Null, Value::Number),
        Tag::Double(bits) => {
            serde_json::Number::from_f64(f64::from_bits(*bits)).map_or(Value::Null, Value::Number)
        }
        Tag::ByteArray(a) => Value::Array(a.iter().map(|b| Value::from(*b as i8)).collect()),
        Tag::String(s) => Value::String(s.to_string_lossy()),
        Tag::List(l) => Value::Array(l.iter().map(raw_tag_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_string_lossy(), raw_tag_json(v)))
                .collect(),
        ),
        Tag::IntArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
    }
}

/// `NbtOps.getBooleanValue`: every numeric tag is true unless its double value
/// is zero (`NaN` and fractions are true; `-0.0` is false).
pub fn nbt_bool(tag: &Tag) -> Option<bool> {
    Some(match tag {
        Tag::Byte(v) => *v != 0,
        Tag::Short(v) => *v != 0,
        Tag::Int(v) => *v != 0,
        Tag::Long(v) => *v != 0,
        Tag::Float(bits) => f32::from_bits(*bits) != 0.0,
        Tag::Double(bits) => f64::from_bits(*bits) != 0.0,
        _ => return None,
    })
}

/// Converts a stack NBT component tag into the canonical JSON form its
/// `DataComponents` codec produces.
pub fn nbt_component_to_json(component_id: &str, tag: &Tag) -> Value {
    let id = normalize_id(component_id).unwrap_or_else(|| component_id.to_owned());
    match id.as_str() {
        "minecraft:enchantment_glint_override" => nbt_bool(tag)
            .map(Value::Bool)
            .unwrap_or_else(|| raw_tag_json(tag)),
        "minecraft:custom_model_data" => custom_model_data_from_nbt(tag),
        _ => {
            let mut value = raw_tag_json(tag);
            apply_nbt_bool_fields(&id, tag, &mut value);
            canonicalize_component_json(&id, value)
        }
    }
}

fn custom_model_data_from_nbt(tag: &Tag) -> Value {
    let mut value = raw_tag_json(tag);
    let flags: Option<Vec<Value>> = match tag.get("flags") {
        Some(Tag::List(flags)) => flags.iter().map(|f| nbt_bool(f).map(Value::Bool)).collect(),
        Some(Tag::ByteArray(flags)) => flags
            .iter()
            .map(|f| nbt_bool(&Tag::Byte(*f as i8)).map(Value::Bool))
            .collect(),
        Some(Tag::IntArray(flags)) => flags
            .iter()
            .map(|f| nbt_bool(&Tag::Int(*f)).map(Value::Bool))
            .collect(),
        Some(Tag::LongArray(flags)) => flags
            .iter()
            .map(|f| nbt_bool(&Tag::Long(*f)).map(Value::Bool))
            .collect(),
        _ => None,
    };
    if let (Some(flags), Some(obj)) = (flags, value.as_object_mut()) {
        obj.insert("flags".into(), Value::Array(flags));
    }
    canonicalize_component_json("minecraft:custom_model_data", value)
}

/// Preserves `NbtOps.getBooleanValue` for non-finite float/double tags (such
/// as `NaN`, which `raw_tag_json` maps to `Value::Null`) on known boolean keys.
fn apply_nbt_bool_fields(component_id: &str, tag: &Tag, value: &mut Value) {
    match component_id {
        "minecraft:written_book_content" => {
            patch_bool_key(tag, value, "resolved");
        }
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => {
            patch_bool_key(tag, value, "has_glowing_text");
        }
        "minecraft:lodestone_tracker" => patch_bool_key(tag, value, "tracked"),
        "minecraft:tooltip_display" => patch_bool_key(tag, value, "hide_tooltip"),
        "minecraft:equippable" => {
            for key in [
                "dispensable",
                "swappable",
                "damage_on_hurt",
                "equip_on_interact",
                "can_be_sheared",
            ] {
                patch_bool_key(tag, value, key);
            }
        }
        "minecraft:consumable" => patch_bool_key(tag, value, "has_consume_particles"),
        "minecraft:food" => patch_bool_key(tag, value, "can_always_eat"),
        "minecraft:use_effects" => {
            patch_bool_key(tag, value, "can_sprint");
            patch_bool_key(tag, value, "interact_vibrations");
        }
        "minecraft:firework_explosion" => {
            patch_bool_key(tag, value, "has_trail");
            patch_bool_key(tag, value, "has_twinkle");
        }
        "minecraft:fireworks" => {
            if let (Some(Tag::List(tags)), Some(vals)) = (
                tag.get("explosions"),
                value.get_mut("explosions").and_then(|v| v.as_array_mut()),
            ) {
                for (t, v) in tags.iter().zip(vals.iter_mut()) {
                    patch_bool_key(t, v, "has_trail");
                    patch_bool_key(t, v, "has_twinkle");
                }
            }
        }
        _ => {}
    }
}

fn patch_bool_key(tag: &Tag, value: &mut Value, key: &str) {
    if let Some(child) = tag.get(key)
        && let Some(b) = nbt_bool(child)
        && let Some(obj) = value.as_object_mut()
    {
        obj.insert(key.to_owned(), Value::Bool(b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::parse_snbt;
    use serde_json::json;

    #[test]
    fn custom_name_nbt_booleans_and_shorthands_match_pack_json() {
        let (nbt, _) = parse_snbt(
            r#"{text:"",extra:[{text:"Styled Item",strikethrough:0b,obfuscated:0b,bold:0b,italic:1b,underlined:0b,color:"gold"}]}"#,
        )
        .unwrap();
        let pack_when = json!({
            "text": "",
            "extra": [{
                "text": "Styled Item",
                "strikethrough": false,
                "obfuscated": false,
                "bold": false,
                "italic": true,
                "underlined": false,
                "color": "gold"
            }]
        });
        assert_eq!(
            nbt_component_to_json("minecraft:custom_name", &nbt),
            canonicalize_component_json("minecraft:custom_name", pack_when)
        );
    }

    #[test]
    fn written_book_nbt_normalizes_filterable_resolved_and_defaults() {
        let nbt = Tag::compound([
            ("title", Tag::String("Ledger".into())),
            ("author", Tag::String("Alice".into())),
            ("generation", Tag::Int(1)),
            (
                "pages",
                Tag::List(vec![Tag::String("Page 1\nLine 2".into())]),
            ),
            ("resolved", Tag::Byte(1)),
        ]);
        let pack_when = json!({
            "generation": 1,
            "pages": [{"raw": "Page 1\nLine 2"}],
            "resolved": true,
            "title": {"raw": "Ledger"},
            "author": "Alice"
        });
        assert_eq!(
            nbt_component_to_json("minecraft:written_book_content", &nbt),
            canonicalize_component_json("minecraft:written_book_content", pack_when)
        );
    }

    #[test]
    fn entity_data_nbt_normalizes_id_while_custom_data_keeps_numeric_bytes() {
        let entity_nbt = Tag::compound([
            ("id", Tag::String("minecraft:frog".into())),
            ("variant", Tag::String("cold".into())),
        ]);
        let entity_when = json!({"id": "frog", "variant": "cold"});
        assert_eq!(
            nbt_component_to_json("entity_data", &entity_nbt),
            canonicalize_component_json("entity_data", entity_when)
        );

        let custom = Tag::compound([
            ("flag", Tag::Byte(1)),
            ("count", Tag::Byte(0)),
            ("val", Tag::Double(0.1_f64.to_bits())),
        ]);
        assert_eq!(
            nbt_component_to_json("minecraft:custom_data", &custom),
            json!({"flag": 1, "count": 0, "val": 0.1})
        );
    }

    #[test]
    fn custom_model_data_floats_normalize_through_f32() {
        let cmd_nbt = Tag::compound([(
            "floats",
            Tag::List(vec![
                Tag::Float(0.1_f32.to_bits()),
                Tag::Float(1.0_f32.to_bits()),
            ]),
        )]);
        assert_eq!(
            nbt_component_to_json("minecraft:custom_model_data", &cmd_nbt),
            canonicalize_component_json("minecraft:custom_model_data", json!({"floats": [0.1, 1]}))
        );
    }
}
