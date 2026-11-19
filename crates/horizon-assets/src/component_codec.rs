//! Canonical JSON normalization for Minecraft 26.3 text and item data
//! components (`ComponentSerialization`, `DataComponents`), so resource pack
//! `items/*.json` component cases compare with `ComponentContents` parity.
use crate::json::identifier;
use serde_json::{Map, Value};

/// `TextColor.parseColor` for `#...`: `Integer.parseInt(rest, 16)` (so a sign
/// and any digit count are accepted), kept only within `0..=0xFFFFFF`.
pub fn hex_color(c: &str) -> Option<u32> {
    let value = i64::from_str_radix(c.strip_prefix('#')?, 16).ok()?;
    (0..=0xFF_FFFF).contains(&value).then_some(value as u32)
}

/// `Style.shadow_color`: a packed ARGB int or normalized RGBA array.
pub fn shadow_color(value: &Value) -> Option<u32> {
    if let Some(packed) = value.as_i64() {
        return i32::try_from(packed).ok().map(|n| n as u32);
    }
    let rgba = value.as_array()?;
    if rgba.len() != 4 {
        return None;
    }
    let channels = rgba
        .iter()
        .map(|channel| {
            // ExtraCodecs.ARGB_COLOR_CODEC reads floats; ARGB.as8BitChannel
            // floors each normalized channel multiplied by 255.
            let value = channel.as_f64()? as f32;
            (value.is_finite() && (0.0..=1.0).contains(&value))
                .then_some((value * 255.0_f32).floor() as u32)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(channels[3] << 24 | channels[0] << 16 | channels[1] << 8 | channels[2])
}

/// A style object as `Style.Serializer.MAP_CODEC` re-encodes it.
pub fn canonical_style(style: &Value) -> Value {
    let (mut node, _) = decompose(style);
    node.remove("text");
    Value::Object(node)
}

/// A component as vanilla decodes and re-encodes it (`ComponentSerialization`):
/// a literal without style or siblings becomes a plain string, a list becomes
/// its first element with the others appended as siblings, and the content
/// type marker is left out.
pub fn canonical_component(c: &Value) -> Value {
    let (node, extra) = decompose(c);
    finish(node, extra)
}

/// A component's own fields (content and style) and its siblings.
fn decompose(c: &Value) -> (Map<String, Value>, Vec<Value>) {
    match c {
        Value::Array(list) if !list.is_empty() => {
            let (mut node, mut extra) = decompose(&list[0]);
            extra.extend(list[1..].iter().map(canonical_component));
            node.remove("extra");
            (node, extra)
        }
        Value::Object(o) => {
            let mut node = Map::new();
            let mut extra = Vec::new();
            for (k, v) in o {
                match k.as_str() {
                    "type" => {}
                    "extra" => {
                        if let Value::Array(list) = v {
                            extra.extend(list.iter().map(canonical_component));
                        }
                    }
                    "bold" | "italic" | "underlined" | "strikethrough" | "obfuscated" => {
                        let b = match v {
                            Value::Bool(b) => *b,
                            Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
                            _ => continue,
                        };
                        node.insert(k.clone(), Value::Bool(b));
                    }
                    "color" => {
                        let color = v.as_str().map(|s| match s.strip_prefix('#') {
                            Some(_) => match hex_color(s) {
                                Some(rgb) => format!("#{rgb:06X}"),
                                None => s.to_owned(),
                            },
                            None => s.to_owned(),
                        });
                        if let Some(color) = color {
                            node.insert(k.clone(), Value::String(color));
                        }
                    }
                    "with" => {
                        if let Value::Array(args) = v {
                            let args = args
                                .iter()
                                .map(|a| match a {
                                    Value::Object(_) | Value::Array(_) => canonical_component(a),
                                    other => other.clone(),
                                })
                                .collect();
                            node.insert(k.clone(), Value::Array(args));
                        }
                    }
                    "separator" => {
                        node.insert(k.clone(), canonical_component(v));
                    }
                    "hover_event" => {
                        let mut h = v.clone();
                        if let Some(value) = h.get("value").cloned()
                            && h.get("action").and_then(Value::as_str) == Some("show_text")
                        {
                            h["value"] = canonical_component(&value);
                        }
                        node.insert(k.clone(), h);
                    }
                    _ => {
                        node.insert(k.clone(), v.clone());
                    }
                }
            }
            (node, extra)
        }
        Value::String(s) => {
            let mut node = Map::new();
            node.insert("text".into(), Value::String(s.clone()));
            (node, Vec::new())
        }
        other => {
            let mut node = Map::new();
            node.insert("text".into(), Value::String(other.to_string()));
            (node, Vec::new())
        }
    }
}

fn finish(mut node: Map<String, Value>, extra: Vec<Value>) -> Value {
    if extra.is_empty()
        && node.len() == 1
        && let Some(Value::String(s)) = node.get("text")
    {
        return Value::String(s.clone());
    }
    if !extra.is_empty() {
        node.insert("extra".into(), Value::Array(extra));
    }
    Value::Object(node)
}

/// Normalizes a JSON component value to the canonical form its `DataComponents`
/// codec produces under `JsonOps`.
pub fn canonicalize_component_json(component_id: &str, value: Value) -> Value {
    let id = normalize_id(component_id).unwrap_or_else(|| component_id.to_owned());
    match id.as_str() {
        "minecraft:custom_name" | "minecraft:item_name" => canonical_text_component(&value),
        "minecraft:lore" => match value {
            Value::Array(lines) => {
                Value::Array(lines.iter().map(canonical_text_component).collect())
            }
            other => other,
        },
        "minecraft:enchantment_glint_override" => json_bool(&value).map_or(value, Value::Bool),
        "minecraft:custom_model_data" => canonical_custom_model_data(value),
        "minecraft:written_book_content" => canonical_written_book(value),
        "minecraft:writable_book_content" => canonical_writable_book(value),
        "minecraft:sign_text_front" | "minecraft:sign_text_back" => canonical_sign_text(value),
        "minecraft:attribute_modifiers" => canonical_attribute_modifiers(value),
        "minecraft:entity_data" | "minecraft:block_entity_data" => canonical_typed_entity(value),
        "minecraft:lodestone_tracker" => canonical_lodestone_tracker(value),
        "minecraft:tooltip_display" => canonical_tooltip_display(value),
        "minecraft:equippable" => canonical_equippable(value),
        "minecraft:consumable" => canonical_consumable(value),
        "minecraft:food" => canonical_food(value),
        "minecraft:use_effects" => canonical_use_effects(value),
        "minecraft:firework_explosion" => canonical_firework_explosion(value),
        "minecraft:fireworks" => canonical_fireworks(value),
        "minecraft:potion_contents" => canonical_potion_contents(value),
        "minecraft:enchantments" | "minecraft:stored_enchantments" => canonical_id_map(value),
        "minecraft:trim" => canonical_trim(value),
        "minecraft:item_model"
        | "minecraft:tooltip_style"
        | "minecraft:note_block_sound"
        | "minecraft:damage_type"
        | "minecraft:block_transformer"
        | "minecraft:break_sound"
        | "minecraft:provides_pottery_pattern" => normalize_string_id(value),
        "minecraft:instrument" | "minecraft:provides_trim_material" => {
            canonical_holder_with_description(value)
        }
        "minecraft:painting/variant" => canonical_painting_variant(value),
        "minecraft:jukebox_playable" => canonical_jukebox_playable(value),
        _ => value,
    }
}

fn normalize_id(s: &str) -> Option<String> {
    identifier(s).ok()
}

fn json_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        _ => None,
    }
}

fn normalize_string_id(v: Value) -> Value {
    match v {
        Value::String(ref s) => normalize_id(s).map_or(v, Value::String),
        other => other,
    }
}

fn normalize_id_field(map: &mut Map<String, Value>, key: &str) {
    if let Some(Value::String(s)) = map.get(key)
        && let Some(id) = normalize_id(s)
    {
        map.insert(key.to_owned(), Value::String(id));
    }
}

fn normalize_bool_field(map: &mut Map<String, Value>, key: &str, default: Option<bool>) {
    let Some(v) = map.get(key) else {
        return;
    };
    if let Some(b) = json_bool(v) {
        if default == Some(b) {
            map.remove(key);
        } else {
            map.insert(key.to_owned(), Value::Bool(b));
        }
    }
}

fn set_always_present_bool(map: &mut Map<String, Value>, key: &str, default: bool) {
    let b = map.get(key).and_then(json_bool).unwrap_or(default);
    map.insert(key.to_owned(), Value::Bool(b));
}

fn remove_empty_array(map: &mut Map<String, Value>, key: &str) {
    if map
        .get(key)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        map.remove(key);
    }
}

fn remove_default_i64(map: &mut Map<String, Value>, key: &str, default: i64) {
    if map.get(key).and_then(Value::as_i64) == Some(default) {
        map.remove(key);
    }
}

/// Normalizes a `ComponentSerialization.CODEC` JSON tree for equality with
/// vanilla `Component.equals` (style booleans, colors, shadow_color, font,
/// inline objects, click/hover events, and collapsed/list forms).
pub fn canonical_text_component(v: &Value) -> Value {
    let c = canonical_component(v);
    normalize_text_fields(c)
}

fn normalize_text_fields(v: Value) -> Value {
    let Value::Object(mut map) = v else {
        return v;
    };
    if let Some(shadow) = map.get("shadow_color")
        && let Some(packed) = shadow_color(shadow)
    {
        map.insert("shadow_color".into(), Value::from(packed as i32));
    }
    normalize_id_field(&mut map, "font");
    if map.contains_key("player") {
        normalize_bool_field(&mut map, "hat", Some(true));
    }
    if map.contains_key("nbt") {
        normalize_bool_field(&mut map, "interpret", Some(false));
        normalize_bool_field(&mut map, "plain", Some(false));
    }
    if let Some(Value::Object(hover)) = map.get_mut("hover_event") {
        match hover.get("action").and_then(Value::as_str) {
            Some("show_text") => {
                if let Some(val) = hover.remove("value") {
                    hover.insert("value".into(), normalize_text_fields(val));
                }
            }
            Some("show_item") | Some("show_entity") => {
                normalize_id_field(hover, "id");
            }
            _ => {}
        }
    }
    if let Some(Value::Object(click)) = map.get_mut("click_event")
        && click.get("action").and_then(Value::as_str) == Some("custom")
    {
        normalize_id_field(click, "id");
    }
    for key in ["extra", "with"] {
        if let Some(Value::Array(items)) = map.remove(key) {
            let items: Vec<Value> = items.into_iter().map(normalize_text_fields).collect();
            map.insert(key.into(), Value::Array(items));
        }
    }
    if let Some(sep) = map.remove("separator") {
        map.insert("separator".into(), normalize_text_fields(sep));
    }
    Value::Object(map)
}

fn canonical_filterable(v: Value, map_inner: fn(&Value) -> Value) -> Value {
    match v {
        Value::Object(mut map) if map.contains_key("raw") => {
            if let Some(raw) = map.remove("raw") {
                map.insert("raw".into(), map_inner(&raw));
            }
            match map.remove("filtered") {
                Some(Value::Null) | None => {}
                Some(filtered) => {
                    map.insert("filtered".into(), map_inner(&filtered));
                }
            }
            Value::Object(map)
        }
        other => Value::Object(Map::from_iter([("raw".into(), map_inner(&other))])),
    }
}

fn canonical_custom_model_data(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(Value::Array(floats)) = map.get_mut("floats") {
        for entry in floats {
            if let Some(f) = entry.as_f64()
                && let Some(n) = serde_json::Number::from_f64(f64::from(f as f32))
            {
                *entry = Value::Number(n);
            }
        }
    }
    if let Some(Value::Array(flags)) = map.get_mut("flags") {
        for flag in flags {
            if let Some(b) = json_bool(flag) {
                *flag = Value::Bool(b);
            }
        }
    }
    for key in ["floats", "flags", "strings", "colors"] {
        remove_empty_array(&mut map, key);
    }
    Value::Object(map)
}

fn canonical_written_book(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(title) = map.remove("title") {
        map.insert("title".into(), canonical_filterable(title, Value::clone));
    }
    if let Some(Value::Array(pages)) = map.remove("pages") {
        let pages: Vec<Value> = pages
            .into_iter()
            .map(|p| canonical_filterable(p, canonical_text_component))
            .collect();
        if !pages.is_empty() {
            map.insert("pages".into(), Value::Array(pages));
        }
    }
    remove_default_i64(&mut map, "generation", 0);
    normalize_bool_field(&mut map, "resolved", Some(false));
    Value::Object(map)
}

fn canonical_writable_book(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(Value::Array(pages)) = map.remove("pages") {
        let pages: Vec<Value> = pages
            .into_iter()
            .map(|p| canonical_filterable(p, Value::clone))
            .collect();
        if !pages.is_empty() {
            map.insert("pages".into(), Value::Array(pages));
        }
    }
    Value::Object(map)
}

fn canonical_sign_text(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(Value::Array(msgs)) = map.remove("messages") {
        map.insert(
            "messages".into(),
            Value::Array(msgs.iter().map(canonical_text_component).collect()),
        );
    }
    if let Some(Value::Array(filtered)) = map.remove("filtered_messages") {
        let filtered: Vec<Value> = filtered.iter().map(canonical_text_component).collect();
        if map.get("messages").and_then(Value::as_array) != Some(&filtered) {
            map.insert("filtered_messages".into(), Value::Array(filtered));
        }
    }
    if !map.contains_key("color") {
        map.insert("color".into(), Value::String("black".into()));
    }
    set_always_present_bool(&mut map, "has_glowing_text", false);
    Value::Object(map)
}

fn canonical_attribute_modifiers(value: Value) -> Value {
    let Value::Array(entries) = value else {
        return value;
    };
    let entries = entries
        .into_iter()
        .map(|entry| {
            let Value::Object(mut map) = entry else {
                return entry;
            };
            normalize_id_field(&mut map, "type");
            normalize_id_field(&mut map, "id");
            if map.get("slot").and_then(Value::as_str) == Some("any") {
                map.remove("slot");
            }
            if let Some(Value::Object(display)) = map.get_mut("display") {
                match display.get("type").and_then(Value::as_str) {
                    Some("default") => {
                        map.remove("display");
                    }
                    Some("override") => {
                        if let Some(val) = display.remove("value") {
                            display.insert("value".into(), canonical_text_component(&val));
                        }
                    }
                    _ => {}
                }
            }
            Value::Object(map)
        })
        .collect();
    Value::Array(entries)
}

fn canonical_typed_entity(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_id_field(&mut map, "id");
    Value::Object(map)
}

fn canonical_lodestone_tracker(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "tracked", Some(true));
    if let Some(Value::Object(target)) = map.get_mut("target") {
        normalize_id_field(target, "dimension");
    }
    Value::Object(map)
}

fn canonical_tooltip_display(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "hide_tooltip", Some(false));
    if let Some(Value::Array(hidden)) = map.remove("hidden_components") {
        let hidden: Vec<Value> = hidden.into_iter().map(normalize_string_id).collect();
        if !hidden.is_empty() {
            map.insert("hidden_components".into(), Value::Array(hidden));
        }
    }
    Value::Object(map)
}

fn canonical_equippable(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in [
        "equip_sound",
        "asset_id",
        "camera_overlay",
        "shearing_sound",
    ] {
        normalize_id_field(&mut map, key);
    }
    if let Some(Value::String(allowed)) = map.get("allowed_entities")
        && !allowed.starts_with('#')
    {
        normalize_id_field(&mut map, "allowed_entities");
    } else if let Some(Value::Array(allowed)) = map.get_mut("allowed_entities") {
        for item in allowed {
            if let Value::String(s) = item
                && let Some(id) = normalize_id(s)
            {
                *s = id;
            }
        }
    }
    for key in ["dispensable", "swappable", "damage_on_hurt"] {
        normalize_bool_field(&mut map, key, Some(true));
    }
    for key in ["equip_on_interact", "can_be_sheared"] {
        normalize_bool_field(&mut map, key, Some(false));
    }
    Value::Object(map)
}

fn canonical_consumable(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "has_consume_particles", Some(true));
    normalize_id_field(&mut map, "sound");
    remove_empty_array(&mut map, "on_consume_effects");
    Value::Object(map)
}

fn canonical_food(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "can_always_eat", Some(false));
    Value::Object(map)
}

fn canonical_use_effects(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_bool_field(&mut map, "can_sprint", Some(false));
    normalize_bool_field(&mut map, "interact_vibrations", Some(true));
    Value::Object(map)
}

fn canonical_firework_explosion(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    remove_empty_array(&mut map, "colors");
    remove_empty_array(&mut map, "fade_colors");
    normalize_bool_field(&mut map, "has_trail", Some(false));
    normalize_bool_field(&mut map, "has_twinkle", Some(false));
    Value::Object(map)
}

fn canonical_fireworks(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    remove_default_i64(&mut map, "flight_duration", 0);
    if let Some(Value::Array(explosions)) = map.remove("explosions") {
        let explosions: Vec<Value> = explosions
            .into_iter()
            .map(canonical_firework_explosion)
            .collect();
        if !explosions.is_empty() {
            map.insert("explosions".into(), Value::Array(explosions));
        }
    }
    Value::Object(map)
}

fn canonical_mob_effect_instance(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    normalize_id_field(&mut map, "id");
    canonical_mob_effect_details(&mut map);
    Value::Object(map)
}

fn canonical_mob_effect_details(map: &mut Map<String, Value>) {
    remove_default_i64(map, "amplifier", 0);
    remove_default_i64(map, "duration", 0);
    normalize_bool_field(map, "ambient", Some(false));
    normalize_bool_field(map, "show_particles", Some(true));
    let show_particles = map
        .get("show_particles")
        .and_then(json_bool)
        .unwrap_or(true);
    set_always_present_bool(map, "show_icon", show_particles);
    if let Some(Value::Object(hidden)) = map.get_mut("hidden_effect") {
        canonical_mob_effect_details(hidden);
    }
}

fn canonical_potion_contents(value: Value) -> Value {
    match value {
        Value::String(s) => {
            let id = normalize_id(&s).unwrap_or(s);
            Value::Object(Map::from_iter([("potion".into(), Value::String(id))]))
        }
        Value::Object(mut map) => {
            normalize_id_field(&mut map, "potion");
            if let Some(Value::Array(effects)) = map.remove("custom_effects") {
                let effects: Vec<Value> = effects
                    .into_iter()
                    .map(canonical_mob_effect_instance)
                    .collect();
                if !effects.is_empty() {
                    map.insert("custom_effects".into(), Value::Array(effects));
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn canonical_id_map(value: Value) -> Value {
    let Value::Object(map) = value else {
        return value;
    };
    Value::Object(
        map.into_iter()
            .map(|(k, v)| (normalize_id(&k).unwrap_or(k), v))
            .collect(),
    )
}

fn canonical_trim(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    for key in ["material", "pattern"] {
        if let Some(v) = map.remove(key) {
            map.insert(key.into(), canonical_holder_with_description(v));
        }
    }
    Value::Object(map)
}

fn canonical_holder_with_description(value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(normalize_id(&s).unwrap_or(s)),
        Value::Object(mut map) => {
            if let Some(desc) = map.remove("description") {
                map.insert("description".into(), canonical_text_component(&desc));
            }
            normalize_id_field(&mut map, "sound_event");
            normalize_id_field(&mut map, "asset_name");
            normalize_id_field(&mut map, "asset_id");
            normalize_id_field(&mut map, "ingredient");
            Value::Object(map)
        }
        other => other,
    }
}

fn canonical_painting_variant(value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(normalize_id(&s).unwrap_or(s)),
        Value::Object(mut map) => {
            normalize_id_field(&mut map, "asset_id");
            for key in ["title", "author"] {
                if let Some(v) = map.remove(key) {
                    map.insert(key.into(), canonical_text_component(&v));
                }
            }
            Value::Object(map)
        }
        other => other,
    }
}

fn canonical_jukebox_playable(value: Value) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    if let Some(song) = map.remove("song") {
        map.insert("song".into(), canonical_holder_with_description(song));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn custom_name_normalizes_colors_plain_and_list_shorthand() {
        // `SelectItemModel` keys on `TextColor.hashCode()`, which includes `name`,
        // so hex `#ffaa00` normalizes to `#FFAA00` without collapsing to `"gold"`.
        assert_eq!(
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "#ffaa00"})
            ),
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "#FFAA00"})
            )
        );
        assert_ne!(
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "#ffaa00"})
            ),
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "Hex", "color": "gold"})
            )
        );
        assert_eq!(
            canonicalize_component_json("minecraft:custom_name", json!({"text": "Plain Title"})),
            canonicalize_component_json("minecraft:custom_name", json!("Plain Title"))
        );
        assert_eq!(
            canonicalize_component_json(
                "minecraft:custom_name",
                json!(["", {"text": "First", "color": "green", "italic": 1}])
            ),
            canonicalize_component_json(
                "minecraft:custom_name",
                json!({"text": "", "extra": [{"text": "First", "color": "green", "italic": true}]})
            )
        );
    }

    #[test]
    fn written_book_and_entity_data_json_canonicalize_defaults_and_ids() {
        assert_eq!(
            canonicalize_component_json(
                "minecraft:written_book_content",
                json!({
                    "generation": 0,
                    "pages": [],
                    "resolved": false,
                    "title": "Ledger",
                    "author": "Alice"
                })
            ),
            canonicalize_component_json(
                "minecraft:written_book_content",
                json!({
                    "title": {"raw": "Ledger"},
                    "author": "Alice"
                })
            )
        );
        assert_eq!(
            canonicalize_component_json("entity_data", json!({"id": "frog", "variant": "cold"})),
            canonicalize_component_json(
                "minecraft:entity_data",
                json!({"id": "minecraft:frog", "variant": "cold"})
            )
        );
    }
}
