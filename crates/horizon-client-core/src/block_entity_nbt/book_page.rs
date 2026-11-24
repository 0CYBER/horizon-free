//! The text component inside a written-book page predicate.
use super::{boolean, string, text};
use crate::{component_codec::nbt_bool, identifier::normalize_id};
use horizon_core::nbt::{Compound, Tag};
use horizon_world::datafix::dynamic::{Number, as_number};

/// ComponentSerialization keeps partial lists and styles, but requires valid contents.
pub(super) fn decode(tag: &Tag, valid_nbt_source: &dyn Fn(&str, &str) -> bool) -> Option<Tag> {
    let child = |tag: &Tag| decode(tag, valid_nbt_source);
    let mut fields = match tag {
        Tag::String(_) => return Some(tag.clone()),
        Tag::List(entries) => {
            let mut entries = entries.iter().filter_map(child);
            let first = entries.next()?;
            let extra: Vec<_> = entries.collect();
            if extra.is_empty() {
                return Some(first);
            }
            let mut fields = match first {
                Tag::String(_) => Compound::from([("text".into(), first)]),
                Tag::Compound(fields) => fields,
                _ => unreachable!("a decoded component is a string or compound"),
            };
            let siblings = fields
                .entry("extra".into())
                .or_insert_with(|| Tag::List(Vec::new()));
            if let Tag::List(siblings) = siblings {
                siblings.extend(extra);
            }
            fields
        }
        Tag::Compound(_) => {
            let mut fields = if let Some(kind) = tag.get("type") {
                contents(tag, &text(Some(kind))?, valid_nbt_source)?
            } else {
                [
                    "text",
                    "translatable",
                    "keybind",
                    "score",
                    "selector",
                    "nbt",
                    "object",
                ]
                .into_iter()
                .find_map(|kind| contents(tag, kind, valid_nbt_source))?
            };
            style(tag, &mut fields, &child);
            if let Some(Tag::List(extra)) = tag.get("extra") {
                let extra: Vec<_> = extra.iter().filter_map(child).collect();
                if !extra.is_empty() {
                    fields.insert("extra".into(), Tag::List(extra));
                }
            }
            fields
        }
        _ => return None,
    };
    if fields.len() == 1
        && let Some(literal) = fields.remove(&"text".into())
    {
        return Some(literal);
    }
    Some(Tag::Compound(fields))
}

fn contents(
    tag: &Tag,
    kind: &str,
    valid_nbt_source: &dyn Fn(&str, &str) -> bool,
) -> Option<Compound> {
    let child = |tag: &Tag| decode(tag, valid_nbt_source);
    let string_field = |key| tag.get(key).and_then(string_tag);
    let mut fields = Compound::new();
    match kind {
        "text" | "keybind" | "selector" => {
            fields.insert(kind.into(), string_field(kind)?);
        }
        "translatable" => {
            fields.insert("translate".into(), string_field("translate")?);
            if let Some(fallback) = string_field("fallback") {
                fields.insert("fallback".into(), fallback);
            }
            if let Some(Tag::List(args)) = tag.get("with") {
                let args: Vec<_> = args
                    .iter()
                    .filter_map(|arg| {
                        if as_number(arg).is_some() {
                            Some(arg.clone())
                        } else {
                            child(arg)
                        }
                    })
                    .collect();
                if !args.is_empty() {
                    fields.insert("with".into(), Tag::List(args));
                }
            }
        }
        "score" => {
            let score = tag.get("score")?;
            let name = string_tag(score.get("name")?)?;
            let objective = string_tag(score.get("objective")?)?;
            fields.insert(
                "score".into(),
                Tag::compound([("name", name), ("objective", objective)]),
            );
        }
        "nbt" => {
            let path = string_field("nbt")?;
            crate::nbt_path::NbtPath::parse(&text(Some(&path))?).ok()?;
            fields.insert("nbt".into(), path);
            let (source, value) = nbt_source(tag, valid_nbt_source)?;
            fields.insert(source.into(), value);
            let interpret = tag.get("interpret").and_then(nbt_bool).unwrap_or(false);
            let plain = tag.get("plain").and_then(nbt_bool).unwrap_or(false);
            if interpret && plain {
                return None;
            }
            if interpret {
                fields.insert("interpret".into(), boolean(true));
            }
            if plain {
                fields.insert("plain".into(), boolean(true));
            }
        }
        "object" => {
            let object = tag.get("object");
            let kind = object.and_then(|v| text(Some(v)));
            if kind.as_deref() == Some("atlas") || object.is_none() && tag.get("sprite").is_some() {
                fields.insert("sprite".into(), identifier(tag.get("sprite")?)?);
                if let Some(atlas) = tag.get("atlas").and_then(identifier)
                    && text(Some(&atlas)).as_deref() != Some("minecraft:blocks")
                {
                    fields.insert("atlas".into(), atlas);
                }
            } else if kind.as_deref() == Some("player") || object.is_none() {
                let profile = match tag.get("player")? {
                    name @ Tag::String(_) => Tag::compound([("name", name.clone())]),
                    profile @ Tag::Compound(_)
                        if profile
                            .get("name")
                            .is_none_or(|name| matches!(name, Tag::String(_))) =>
                    {
                        profile.clone()
                    }
                    _ => return None,
                };
                fields.insert("player".into(), profile);
                if tag.get("hat").and_then(nbt_bool) == Some(false) {
                    fields.insert("hat".into(), boolean(false));
                }
            } else {
                return None;
            }
            if let Some(fallback) = tag.get("fallback").and_then(child) {
                fields.insert("fallback".into(), fallback);
            }
        }
        _ => return None,
    }
    if matches!(kind, "selector" | "nbt")
        && let Some(separator) = tag.get("separator").and_then(child)
    {
        fields.insert("separator".into(), separator);
    }
    Some(fields)
}

fn nbt_source(
    tag: &Tag,
    valid_nbt_source: &dyn Fn(&str, &str) -> bool,
) -> Option<(&'static str, Tag)> {
    let decode = |source| {
        let value = tag.get(source)?;
        match source {
            "entity" | "block" => {
                valid_nbt_source(source, &text(Some(value))?).then(|| value.clone())
            }
            "storage" => identifier(value),
            _ => None,
        }
    };
    // DataSources uses the explicit type when present, and tries the legacy
    // fields in order otherwise. Only the selected field is written back.
    let sources = ["entity", "block", "storage"];
    if let Some(kind) = tag.get("source") {
        let kind = text(Some(kind))?;
        let source = sources.into_iter().find(|&source| source == kind)?;
        return Some((source, decode(source)?));
    }
    sources
        .into_iter()
        .find_map(|source| Some((source, decode(source)?)))
}

fn style(tag: &Tag, fields: &mut Compound, child: &dyn Fn(&Tag) -> Option<Tag>) {
    for key in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        if let Some(value) = tag.get(key).and_then(nbt_bool) {
            fields.insert(key.into(), boolean(value));
        }
    }
    if let Some(color) = text(tag.get("color")) {
        if let Some(rgb) = horizon_assets::component_codec::hex_color(&color) {
            fields.insert("color".into(), string(&format!("#{rgb:06X}")));
        } else if crate::text::named_color(&color).is_some() {
            fields.insert("color".into(), string(&color));
        }
    }
    for (key, decode) in [
        ("font", identifier as fn(&Tag) -> Option<Tag>),
        ("insertion", string_tag),
        ("shadow_color", shadow_color),
        ("click_event", click_event),
    ] {
        if let Some(value) = tag.get(key).and_then(decode) {
            fields.insert(key.into(), value);
        }
    }
    if let Some(value) = tag
        .get("hover_event")
        .and_then(|value| hover_event(value, child))
    {
        fields.insert("hover_event".into(), value);
    }
}

fn string_tag(tag: &Tag) -> Option<Tag> {
    matches!(tag, Tag::String(_)).then(|| tag.clone())
}

fn shadow_color(tag: &Tag) -> Option<Tag> {
    if let Some(number) = as_number(tag) {
        return Some(Tag::Int(number.int_value()));
    }
    horizon_assets::component_codec::shadow_color(&crate::component_codec::raw_tag_json(tag))
        .map(|argb| Tag::Int(argb as i32))
}

fn click_event(tag: &Tag) -> Option<Tag> {
    let action = text(tag.get("action"))?;
    let key = match action.as_str() {
        "open_url"
            if text(tag.get("url"))
                .is_some_and(|url| crate::component::untrusted_http_url(&url)) =>
        {
            "url"
        }
        "run_command" | "suggest_command" => "command",
        "copy_to_clipboard" => "value",
        "change_page" => {
            let page = tag
                .get("page")
                .and_then(as_number)
                .map(Number::int_value)
                .filter(|&page| page > 0)?;
            return Some(Tag::compound([
                ("action", string(&action)),
                ("page", Tag::Int(page)),
            ]));
        }
        "show_dialog" => {
            let dialog = tag.get("dialog")?;
            return matches!(dialog, Tag::String(_) | Tag::Compound(_)).then(|| tag.clone());
        }
        "custom" => {
            let id = identifier(tag.get("id")?)?;
            let mut event = Compound::from([("action".into(), string(&action)), ("id".into(), id)]);
            if let Some(payload) = tag.get("payload") {
                event.insert("payload".into(), payload.clone());
            }
            return Some(Tag::Compound(event));
        }
        _ => return None,
    };
    Some(Tag::compound([
        ("action", string(&action)),
        (key, string_tag(tag.get(key)?)?),
    ]))
}

fn hover_event(tag: &Tag, child: &dyn Fn(&Tag) -> Option<Tag>) -> Option<Tag> {
    let action = text(tag.get("action"))?;
    match action.as_str() {
        "show_text" => Some(Tag::compound([
            ("action", string(&action)),
            ("value", child(tag.get("value")?)?),
        ])),
        "show_item" => {
            let mut event = Compound::from([
                ("action".into(), string(&action)),
                ("id".into(), identifier(tag.get("id")?)?),
            ]);
            if let Some(count) = tag
                .get("count")
                .and_then(as_number)
                .map(Number::int_value)
                .filter(|&count| count != 1)
            {
                event.insert("count".into(), Tag::Int(count));
            }
            if let Some(components @ Tag::Compound(fields)) = tag.get("components")
                && !fields.is_empty()
            {
                event.insert("components".into(), components.clone());
            }
            Some(Tag::Compound(event))
        }
        "show_entity" => {
            let id = identifier(tag.get("id")?)?;
            let uuid = match tag.get("uuid")? {
                uuid @ Tag::IntArray(values) if values.len() == 4 => uuid.clone(),
                Tag::String(uuid) => {
                    Tag::IntArray(crate::uuid::uuid_from_text(&uuid.to_string_lossy())?.to_vec())
                }
                _ => return None,
            };
            let mut event = Compound::from([
                ("action".into(), string(&action)),
                ("id".into(), id),
                ("uuid".into(), uuid),
            ]);
            if let Some(name) = tag.get("name").and_then(child) {
                event.insert("name".into(), name);
            }
            Some(Tag::Compound(event))
        }
        _ => None,
    }
}

fn identifier(tag: &Tag) -> Option<Tag> {
    normalize_id(&text(Some(tag))?).map(|id| string(&id))
}
