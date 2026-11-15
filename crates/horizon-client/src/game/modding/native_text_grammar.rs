//! The component grammar of `ComponentSerialization.CODEC`, as `text_from_json` reads it: a
//! component is checked and rebuilt in one pass, so what the text capture sees is exactly
//! the value the codec would have decoded, never a partial or ambiguous one.
use horizon_client::command::Registries;
use horizon_client::component;
use serde_json::{Map, Value};

/// Content kinds in the order the codec tries them without a `type`.
const KINDS: [&str; 7] = [
    "text",
    "translatable",
    "keybind",
    "score",
    "selector",
    "nbt",
    "object",
];
const STYLE_FLAGS: [&str; 5] = [
    "bold",
    "italic",
    "underlined",
    "strikethrough",
    "obfuscated",
];

/// `value` as the codec decodes it: a string; a non-empty list of components; an object with
/// one content (the one its `type` names, else the first kind its keys satisfy) plus
/// well-formed style fields and `extra`, holding only the keys that decode. None when it
/// does not decode.
pub(super) fn build(value: &Value, reg: &Registries, snbt: bool) -> Option<Value> {
    match value {
        Value::String(_) => Some(value.clone()),
        Value::Array(items) if !items.is_empty() => items
            .iter()
            .map(|item| build(item, reg, snbt))
            .collect::<Option<_>>()
            .map(Value::Array),
        Value::Object(fields) => build_object(fields, reg, snbt).map(Value::Object),
        _ => None,
    }
}

fn build_object(
    fields: &Map<String, Value>,
    reg: &Registries,
    snbt: bool,
) -> Option<Map<String, Value>> {
    // A null `type` is no `type`; any other non-string refuses.
    let mut out = match fields.get("type") {
        None | Some(Value::Null) => KINDS
            .iter()
            .find_map(|kind| content(kind, fields, reg, snbt))?,
        Some(Value::String(kind)) => content(kind, fields, reg, snbt)?,
        Some(_) => return None,
    };
    for flag in STYLE_FLAGS {
        if let Some(v) = present(fields, flag) {
            if snbt {
                component::style_flag(Some(v))?;
            } else {
                v.as_bool()?;
            }
            out.insert(flag.into(), v.clone());
        }
    }
    if let Some(v) = present(fields, "color") {
        component::is_color(v.as_str()?).then_some(())?;
        out.insert("color".into(), v.clone());
    }
    if let Some(v) = present(fields, "shadow_color") {
        component::shadow_color(v)?;
        out.insert("shadow_color".into(), v.clone());
    }
    if let Some(v) = present(fields, "font") {
        horizon_client::text::font_id(v)?;
        out.insert("font".into(), v.clone());
    }
    if let Some(v) = present(fields, "insertion") {
        v.as_str()?;
        out.insert("insertion".into(), v.clone());
    }
    if let Some(v) = present(fields, "click_event") {
        component::parse_click_event(v.as_object()?).ok()??;
        out.insert("click_event".into(), v.clone());
    }
    if let Some(v) = present(fields, "hover_event") {
        out.insert("hover_event".into(), hover(v.as_object()?, reg, snbt)?);
    }
    if let Some(v) = present(fields, "extra") {
        let items = v.as_array().filter(|items| !items.is_empty())?;
        let built = items
            .iter()
            .map(|i| build(i, reg, snbt))
            .collect::<Option<_>>()?;
        out.insert("extra".into(), Value::Array(built));
    }
    Some(out)
}

/// A field that is there and not null: null counts as absent.
fn present<'a>(fields: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    fields.get(key).filter(|v| !v.is_null())
}

/// `HoverEvent.CODEC`'s three actions; a `show_text` value is itself a component.
fn hover(event: &Map<String, Value>, reg: &Registries, snbt: bool) -> Option<Value> {
    // Recursive component slots are rebuilt below, never cloned before checking.
    let mut out: Map<String, Value> = event
        .iter()
        .filter(|(key, _)| !matches!(key.as_str(), "value" | "name"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    match event.get("action")?.as_str()? {
        "show_text" => {
            out.insert("value".into(), build(event.get("value")?, reg, snbt)?);
        }
        "show_item" => {
            event.get("id")?.as_str()?;
        }
        "show_entity" => {
            mod_api::Identifier::parse(event.get("id")?.as_str()?).ok()?;
            match event.get("uuid")? {
                Value::String(id) => {
                    horizon_client_core::uuid::uuid_from_text(id)?;
                }
                Value::Array(words) if words.len() == 4 => {
                    for word in words {
                        i32::try_from(word.as_i64()?).ok()?;
                    }
                }
                _ => return None,
            }
            if let Some(name) = present(event, "name") {
                out.insert("name".into(), build(name, reg, snbt)?);
            } else {
                out.remove("name");
            }
        }
        _ => return None,
    }
    Some(Value::Object(out))
}

/// One content kind of `fields` alone, as its `MapCodec` decodes it: None when it does not.
fn content(
    kind: &str,
    fields: &Map<String, Value>,
    reg: &Registries,
    snbt: bool,
) -> Option<Map<String, Value>> {
    let string = |key: &str| fields.get(key).and_then(Value::as_str);
    let mut out = Map::new();
    let keep = |out: &mut Map<String, Value>, key: &str| {
        out.insert(key.into(), fields.get(key).cloned().unwrap_or(Value::Null));
    };
    match kind {
        "text" => {
            // A string only: a number is not coerced.
            string("text")?;
            keep(&mut out, "text");
        }
        "translatable" => {
            string("translate")?;
            keep(&mut out, "translate");
            // `fallback` is lenient: a wrong kind is dropped.
            if string("fallback").is_some() {
                keep(&mut out, "fallback");
            }
            if let Some(with) = present(fields, "with") {
                let args = with.as_array()?;
                let built = args
                    .iter()
                    .map(|arg| {
                        build(arg, reg, snbt)
                            .or_else(|| (arg.is_number() || arg.is_boolean()).then(|| arg.clone()))
                    })
                    .collect::<Option<_>>()?;
                out.insert("with".into(), Value::Array(built));
            }
        }
        "keybind" => {
            string("keybind")?;
            keep(&mut out, "keybind");
        }
        "score" => {
            let score = fields.get("score")?.as_object()?;
            score.get("name")?.as_str()?;
            score.get("objective")?.as_str()?;
            keep(&mut out, "score");
        }
        "selector" => {
            selector_parses(string("selector")?, reg).then_some(())?;
            keep(&mut out, "selector");
            // A selector's `separator` stays strict.
            if let Some(separator) = present(fields, "separator") {
                out.insert("separator".into(), build(separator, reg, snbt)?);
            }
        }
        "nbt" => {
            path_parses(string("nbt")?).then_some(())?;
            keep(&mut out, "nbt");
            let source = match present(fields, "source") {
                Some(Value::String(source)) => {
                    data_source(source, fields, reg).map(|key| (key, fields[key].clone()))?
                }
                Some(_) => return None,
                None => ["entity", "block", "storage"].into_iter().find_map(|key| {
                    data_source(key, fields, reg).map(|k| (k, fields[k].clone()))
                })?,
            };
            out.insert(source.0.into(), source.1);
            // `interpret`, `plain` and `separator` are lenient: a wrong kind is dropped.
            let flag =
                |key: &str| present(fields, key).and_then(|v| component::style_flag(Some(v)));
            if flag("interpret") == Some(true) && flag("plain") == Some(true) {
                return None;
            }
            for key in ["interpret", "plain"] {
                if flag(key).is_some() {
                    keep(&mut out, key);
                }
            }
            if let Some(separator) = present(fields, "separator").and_then(|s| build(s, reg, snbt))
            {
                out.insert("separator".into(), separator);
            }
        }
        "object" => {
            component::inline_object_fields(fields)?;
            for key in ["object", "player", "hat", "atlas", "sprite"] {
                if fields.contains_key(key) {
                    keep(&mut out, key);
                }
            }
            if let Some(fallback) = present(fields, "fallback") {
                out.insert("fallback".into(), build(fallback, reg, snbt)?);
            }
        }
        _ => return None,
    }
    Some(out)
}

/// The data source `source` (`entity`, `block` or `storage`) as `fields` holds it: the key
/// when its value is valid (a selector, a position string, a storage identifier).
fn data_source(
    source: &str,
    fields: &Map<String, Value>,
    reg: &Registries,
) -> Option<&'static str> {
    let value = fields.get(source)?.as_str()?;
    match source {
        "entity" => selector_parses(value, reg).then_some("entity"),
        "block" => Some("block"),
        "storage" => mod_api::Identifier::parse(value).ok().map(|_| "storage"),
        _ => None,
    }
}

/// `EntitySelectorParser.parse` on all of `source`.
fn selector_parses(source: &str, reg: &Registries) -> bool {
    let mut reader = horizon_client::command::Reader::new(source);
    horizon_client::command::selector::parse(&mut reader, reg, true).is_ok() && !reader.can_read()
}

/// `NbtPathArgument.parse` on all of `path`.
fn path_parses(path: &str) -> bool {
    horizon_client::nbt_path::NbtPath::parse_prefix(path).is_ok_and(|(_, used)| used == path.len())
}
