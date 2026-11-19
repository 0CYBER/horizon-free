#[path = "api_text/text_style_capture.rs"]
mod text_style_capture;
use horizon_client_core::{component, text as engine};
use horizon_core::{
    json_nbt,
    nbt::{self, Tag},
};
use mod_api::text::{
    Text, TextClick, TextContent, TextFormError, TextHover, TextNbtSource, TextObject, TextProfile,
    TextProfileProperty, TextRelation, TextStyle,
};
use serde_json::{Map, Value};
use std::collections::HashMap;
pub use text_style_capture::style_to_api;

type Error = TextFormError;
type Key = (u8, usize);
#[derive(Clone, Copy)]
enum Source<'a> {
    Engine(&'a engine::Text),
    Span(&'a engine::Span),
    Json(&'a Value),
    Str(&'a str),
}
impl Source<'_> {
    fn key(self) -> Key {
        match self {
            Self::Engine(v) => (0, v as *const _ as usize),
            Self::Span(v) => (1, v as *const _ as usize),
            Self::Json(v) => (2, v as *const _ as usize),
            Self::Str(v) => (3, v.as_ptr() as usize),
        }
    }
}
#[derive(Clone, Copy)]
enum NbtSource<'a> {
    Block(&'a str),
    Entity(&'a str),
    Storage(&'a str),
}
enum Content<'a> {
    Literal(&'a str),
    Scalar(&'a Value),
    Translate {
        key: &'a str,
        fallback: Option<&'a str>,
        arguments: &'a [Value],
    },
    Keybind(&'a str),
    Selector {
        pattern: &'a str,
        separator: Option<Source<'a>>,
    },
    Score {
        name: &'a str,
        objective: &'a str,
        fields: &'a Map<String, Value>,
    },
    Nbt {
        path: &'a str,
        interpret: bool,
        source: NbtSource<'a>,
        separator: Option<Source<'a>>,
    },
    Player {
        profile: &'a Value,
        hat: bool,
        fallback: Option<Source<'a>>,
    },
    Atlas {
        atlas: &'a str,
        sprite: &'a str,
        fallback: Option<Source<'a>>,
    },
}
enum Hover<'a> {
    Text(Source<'a>),
    Item {
        id: &'a str,
        count: u32,
        components: Option<&'a Value>,
    },
    Entity {
        kind: &'a str,
        id: [u64; 2],
        name: Option<Source<'a>>,
    },
}
#[derive(Clone, Copy)]
enum Style<'a> {
    Json(&'a Map<String, Value>),
    Native(&'a engine::Style),
    Empty,
}
struct Node<'a> {
    source: Source<'a>,
    content: Content<'a>,
    style: Style<'a>,
    hover: Option<Hover<'a>>,
    hover_fields: Option<&'a Map<String, Value>>,
    fields: Option<&'a Map<String, Value>>,
    edges: Vec<(Source<'a>, TextRelation, bool)>,
    merge: bool,
}

fn uuid(value: &Value) -> Option<[u64; 2]> {
    let words = if let Some(s) = value.as_str() {
        horizon_client_core::uuid::uuid_from_text(s)?
    } else {
        let values: &[Value; 4] = value.as_array()?.as_slice().try_into().ok()?;
        let mut words = [0i32; 4];
        for (word, value) in words.iter_mut().zip(values) {
            let n = value.as_number()?;
            *word = if let Some(n) = n.as_i64() {
                n as i32
            } else if let Some(n) = n.as_u64() {
                n as i32
            } else {
                n.as_f64()? as i32
            };
        }
        words
    }
    .map(|v| v as u32 as u64);
    Some([words[0] << 32 | words[1], words[2] << 32 | words[3]])
}
fn hover(fields: &Map<String, Value>) -> Option<Hover<'_>> {
    Some(match fields.get("action")?.as_str()? {
        "show_text" => Hover::Text(Source::Json(
            fields
                .get("value")
                .or_else(|| fields.get("contents"))
                .or_else(|| fields.get("text"))?,
        )),
        "show_item" => Hover::Item {
            id: fields.get("id")?.as_str()?,
            count: fields
                .get("count")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(1),
            components: fields.get("components"),
        },
        "show_entity" => Hover::Entity {
            kind: fields.get("id")?.as_str()?,
            id: uuid(fields.get("uuid")?)?,
            name: fields.get("name").map(Source::Json),
        },
        _ => return None,
    })
}
fn content(fields: &Map<String, Value>) -> Result<Content<'_>, Error> {
    use component::InlineObjectFields as Object;
    if let Some(object) = component::inline_object_fields(fields) {
        let fallback = fields.get("fallback").map(Source::Json);
        return Ok(match object {
            Object::Player { profile, hat } => Content::Player {
                profile,
                hat,
                fallback,
            },
            Object::Atlas { atlas, sprite } => Content::Atlas {
                atlas,
                sprite,
                fallback,
            },
        });
    }
    if let Some(value) = fields.get("text") {
        return Ok(match value {
            Value::String(s) => Content::Literal(s),
            Value::Number(_) | Value::Bool(_) | Value::Null => Content::Scalar(value),
            _ => return Err(Error::Malformed),
        });
    }
    if let Some(key) = fields.get("translate").and_then(Value::as_str) {
        return Ok(Content::Translate {
            key,
            fallback: fields.get("fallback").and_then(Value::as_str),
            arguments: fields
                .get("with")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        });
    }
    if let Some(pattern) = fields.get("selector").and_then(Value::as_str) {
        return Ok(Content::Selector {
            pattern,
            separator: fields.get("separator").map(Source::Json),
        });
    }
    if let Some(key) = fields.get("keybind").and_then(Value::as_str) {
        return Ok(Content::Keybind(key));
    }
    if let Some(score) = fields.get("score").and_then(Value::as_object)
        && let (Some(name), Some(objective)) = (
            score.get("name").and_then(Value::as_str),
            score.get("objective").and_then(Value::as_str),
        )
    {
        return Ok(Content::Score {
            name,
            objective,
            fields: score,
        });
    }
    if let Some(path) = fields.get("nbt").and_then(Value::as_str) {
        let source = fields
            .get("block")
            .and_then(Value::as_str)
            .map(NbtSource::Block)
            .or_else(|| {
                fields
                    .get("entity")
                    .and_then(Value::as_str)
                    .map(NbtSource::Entity)
            })
            .or_else(|| {
                fields
                    .get("storage")
                    .and_then(Value::as_str)
                    .map(NbtSource::Storage)
            });
        if let Some(source) = source {
            return Ok(Content::Nbt {
                path,
                interpret: component::style_flag(fields.get("interpret")).unwrap_or(false),
                source,
                separator: fields.get("separator").map(Source::Json),
            });
        }
    }
    Ok(Content::Literal(""))
}
fn node(source: Source<'_>) -> Result<Node<'_>, Error> {
    let mut node = Node {
        source,
        content: Content::Literal(""),
        style: Style::Empty,
        hover: None,
        hover_fields: None,
        fields: None,
        edges: Vec::new(),
        merge: false,
    };
    match source {
        Source::Str(s) => node.content = Content::Literal(s),
        Source::Engine(text) => {
            node.merge = true;
            if let Some(source) = text.source_component() {
                node.edges
                    .push((Source::Json(source), TextRelation::Child, true));
            } else {
                node.edges.extend(text.spans.iter().map(|span| {
                    (
                        Source::Span(span),
                        TextRelation::Child,
                        text.spans.len() == 1,
                    )
                }));
                // One span is the component itself; multiple spans need a parent.
                node.merge = text.spans.len() <= 1;
            }
        }
        Source::Json(Value::Array(values)) => {
            node.merge = true;
            node.edges.extend(
                values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (Source::Json(v), TextRelation::Child, i == 0)),
            );
        }
        Source::Json(Value::Object(fields)) => {
            node.content = content(fields)?;
            node.fields = Some(fields);
            node.style = Style::Json(fields);
            node.hover_fields = component::style_event_object(fields, "hover_event", "hoverEvent");
            node.hover = node.hover_fields.and_then(hover);
            if let Some(values) = fields.get("extra").and_then(Value::as_array) {
                node.edges.extend(
                    values
                        .iter()
                        .map(|v| (Source::Json(v), TextRelation::Child, false)),
                );
            }
        }
        Source::Json(Value::String(s)) => node.content = Content::Literal(s),
        Source::Json(value @ (Value::Bool(_) | Value::Number(_))) => {
            node.content = Content::Scalar(value)
        }
        Source::Json(Value::Null) => {}
        Source::Span(span) => {
            node.content = match &span.style.inline_object {
                Some(engine::InlineObject::Player {
                    profile,
                    hat,
                    fallback,
                }) => Content::Player {
                    profile,
                    hat: *hat,
                    fallback: Some(Source::Str(fallback)),
                },
                Some(engine::InlineObject::Atlas {
                    atlas,
                    sprite,
                    fallback,
                }) => Content::Atlas {
                    atlas,
                    sprite,
                    fallback: Some(Source::Str(fallback)),
                },
                None => Content::Literal(&span.text),
            };
            node.style = Style::Native(&span.style);
            if let Some(text) = &span.style.hover {
                node.hover = Some(Hover::Text(Source::Engine(text)));
            } else {
                node.hover_fields = span
                    .style
                    .hover_item
                    .as_deref()
                    .or(span.style.hover_entity.as_deref())
                    .and_then(Value::as_object);
                // Native retained item/entity events may omit action.
                if let Some(fields) = node.hover_fields {
                    node.hover = if span.style.hover_item.is_some() {
                        fields
                            .get("id")
                            .and_then(Value::as_str)
                            .map(|id| Hover::Item {
                                id,
                                count: fields
                                    .get("count")
                                    .and_then(Value::as_u64)
                                    .and_then(|v| u32::try_from(v).ok())
                                    .unwrap_or(1),
                                components: fields.get("components"),
                            })
                    } else {
                        match (
                            fields.get("id").and_then(Value::as_str),
                            fields.get("uuid").and_then(uuid),
                        ) {
                            (Some(kind), Some(id)) => Some(Hover::Entity {
                                kind,
                                id,
                                name: fields.get("name").map(Source::Json),
                            }),
                            _ => None,
                        }
                    };
                }
            }
        }
    }
    match &node.content {
        Content::Translate { arguments, .. } => node.edges.extend(
            arguments
                .iter()
                .map(|v| (Source::Json(v), TextRelation::Argument, false)),
        ),
        Content::Selector { separator, .. } | Content::Nbt { separator, .. } => node.edges.extend(
            separator
                .iter()
                .map(|&v| (v, TextRelation::Separator, false)),
        ),
        Content::Player { fallback, .. } | Content::Atlas { fallback, .. } => node
            .edges
            .extend(fallback.iter().map(|&v| (v, TextRelation::Fallback, false))),
        _ => {}
    }
    match &node.hover {
        Some(Hover::Text(v)) => node.edges.push((*v, TextRelation::HoverText, false)),
        Some(Hover::Entity { name: Some(v), .. }) => {
            node.edges.push((*v, TextRelation::HoverEntityName, false))
        }
        _ => {}
    }
    Ok(node)
}
// Plan borrowed topology completely before cloning any data. Array aliases and
// a single native span stay at the same logical component depth.
fn plan(root: Source<'_>) -> Result<Vec<Node<'_>>, Error> {
    enum Work<'a> {
        Enter(Source<'a>, usize),
        Finish(Node<'a>),
    }
    let mut work = vec![Work::Enter(root, 1)];
    let mut nodes = Vec::new();
    while let Some(task) = work.pop() {
        match task {
            Work::Finish(node) => nodes.push(node),
            Work::Enter(source, depth) => {
                if depth > mod_api::text::MAX_TEXT_DEPTH {
                    return Err(Error::TooDeep);
                }
                let node = node(source)?;
                let children: Vec<_> = node
                    .edges
                    .iter()
                    .map(|&(source, _, same)| (source, depth + usize::from(!same)))
                    .collect();
                work.push(Work::Finish(node));
                work.extend(children.into_iter().rev().map(|(s, d)| Work::Enter(s, d)));
            }
        }
    }
    Ok(nodes)
}

fn error(error: json_nbt::JsonNbtError) -> Error {
    match error {
        json_nbt::JsonNbtError::TooDeep => Error::TooDeep,
        json_nbt::JsonNbtError::NestedNull => Error::NestedNull,
        json_nbt::JsonNbtError::NonFinite => Error::NonFinite,
        json_nbt::JsonNbtError::InvalidString => Error::InvalidString,
    }
}
fn metadata<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a Value)>,
    parent_depth: usize,
) -> Result<Tag, Error> {
    json_nbt::json_fields_to_compound(fields, parent_depth)
        .map(Tag::Compound)
        .map_err(error)
}
fn reserved(key: &str) -> bool {
    [
        "type",
        "text",
        "translate",
        "with",
        "fallback",
        "keybind",
        "selector",
        "separator",
        "score",
        "nbt",
        "interpret",
        "block",
        "entity",
        "storage",
        "object",
        "player",
        "hat",
        "atlas",
        "sprite",
        "extra",
        "color",
        "shadow_color",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "font",
        "insertion",
        "click_event",
        "hover_event",
        "clickEvent",
        "hoverEvent",
    ]
    .contains(&key)
}
fn click_kind(click: &TextClick) -> &'static str {
    match click {
        TextClick::OpenUrl(_) => "open_url",
        TextClick::ChangePage(_) => "change_page",
        TextClick::SuggestCommand(_) => "suggest_command",
        TextClick::RunCommand(_) => "run_command",
        TextClick::CopyToClipboard(_) => "copy_to_clipboard",
        TextClick::ShowDialog(_) => "show_dialog",
        TextClick::Custom { .. } => "custom",
    }
}
fn event_reserved(kind: &str, key: &str) -> bool {
    key == "action"
        || match kind {
            "open_url" => ["url", "value"].contains(&key),
            "change_page" => ["page", "value"].contains(&key),
            "suggest_command" | "run_command" => ["command", "value"].contains(&key),
            "copy_to_clipboard" => key == "value",
            "show_dialog" => key == "dialog",
            "custom" => ["id", "payload"].contains(&key),
            "show_text" => ["value", "contents", "text"].contains(&key),
            "show_item" => ["id", "count", "components"].contains(&key),
            "show_entity" => ["id", "uuid", "name"].contains(&key),
            _ => false,
        }
}
fn profile(value: &Value) -> Result<TextProfile, Error> {
    fn name(value: &str) -> Result<String, Error> {
        if value.len() > 16 || value.bytes().any(|b| b <= 32 || b >= 127) {
            return Err(Error::Malformed);
        }
        Ok(value.into())
    }
    fn field<'a>(fields: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, Error> {
        fields
            .get(key)
            .filter(|v| !v.is_null())
            .map(|v| v.as_str().ok_or(Error::Malformed))
            .transpose()
    }
    if let Value::String(value) = value {
        let mut profile = TextProfile::default();
        profile.name = Some(name(value)?);
        return Ok(profile);
    }
    let fields = value.as_object().ok_or(Error::Malformed)?;
    let name = field(fields, "name")?.map(name).transpose()?;
    let id = fields
        .get("id")
        .filter(|v| !v.is_null())
        .map(|value| {
            if !value.is_array() {
                return Err(Error::Malformed);
            }
            uuid(value).ok_or(Error::Malformed)
        })
        .transpose()?;
    let mut properties = Vec::new();
    match fields.get("properties").filter(|v| !v.is_null()) {
        None => {}
        Some(Value::Object(map)) => {
            if map.len() > 16 {
                return Err(Error::Malformed);
            }
            for (name, values) in map {
                for value in values.as_array().ok_or(Error::Malformed)? {
                    let mut property = TextProfileProperty::default();
                    property.name = name.clone();
                    property.value = value.as_str().ok_or(Error::Malformed)?.into();
                    properties.push(property);
                }
            }
        }
        Some(Value::Array(records)) => {
            if records.len() > 16 {
                return Err(Error::Malformed);
            }
            for record in records {
                let record = record.as_object().ok_or(Error::Malformed)?;
                let name = field(record, "name")?.ok_or(Error::Malformed)?;
                let value = field(record, "value")?.ok_or(Error::Malformed)?;
                let signature = field(record, "signature")?;
                if name.encode_utf16().count() > 64
                    || value.encode_utf16().count() > 32767
                    || signature.is_some_and(|s| s.encode_utf16().count() > 1024)
                {
                    return Err(Error::Malformed);
                }
                let extra = metadata(
                    record
                        .iter()
                        .filter(|(k, _)| !["name", "value", "signature"].contains(&k.as_str()))
                        .map(|(k, v)| (k.as_str(), v)),
                    0,
                )?;
                let mut property = TextProfileProperty::default();
                property.name = name.into();
                property.value = value.into();
                property.signature = signature.map(str::to_owned);
                property.metadata = extra;
                properties.push(property);
            }
        }
        _ => return Err(Error::Malformed),
    }
    let metadata = metadata(
        fields
            .iter()
            .filter(|(k, _)| !["name", "id", "properties"].contains(&k.as_str()))
            .map(|(k, v)| (k.as_str(), v)),
        0,
    )?;
    let mut profile = TextProfile::default();
    profile.name = name;
    profile.id = id;
    profile.properties = properties;
    profile.metadata = metadata;
    Ok(profile)
}
fn take_data(tag: std::sync::Arc<Tag>) -> Result<Tag, Error> {
    if !nbt::within_depth(&tag, mod_api::text::MAX_TEXT_DEPTH) {
        return Err(Error::TooDeep);
    }
    Ok(std::sync::Arc::try_unwrap(tag).unwrap_or_else(|tag| nbt::clone_tag(&tag)))
}
fn native_click(click: engine::ClickEvent) -> Result<Option<TextClick>, Error> {
    use engine::ClickEvent as E;
    Ok(Some(match click {
        E::OpenUrl(v) => {
            if !component::untrusted_http_url(&v) {
                return Ok(None);
            }
            TextClick::OpenUrl(v)
        }
        E::ChangePage(v) => {
            if v < 1 {
                return Ok(None);
            }
            TextClick::ChangePage(v)
        }
        E::SuggestCommand(v) => TextClick::SuggestCommand(v),
        E::RunCommand(v) => TextClick::RunCommand(v),
        E::CopyToClipboard(v) => TextClick::CopyToClipboard(v),
        E::ShowDialog(v) => TextClick::ShowDialog(take_data(v)?),
        E::Custom { id, payload } => TextClick::Custom {
            id,
            payload: payload.map(take_data).transpose()?,
        },
        E::OpenFile(_) | E::Profiler(_) => return Ok(None),
    }))
}
fn captured_style(node: &Node<'_>) -> Result<TextStyle, Error> {
    let mut style = TextStyle::default();
    match node.style {
        Style::Empty => {}
        Style::Native(s) => {
            style.color = s.color.map(|v| format!("#{v:06X}"));
            style.shadow_color = s.shadow_color;
            style.bold = s.bold;
            style.italic = s.italic;
            style.underlined = s.underlined;
            style.strikethrough = s.strikethrough;
            style.obfuscated = s.obfuscated;
            style.font = s.font.clone();
            style.insertion = s.insertion.clone();
            style.click = s.click.clone().map(native_click).transpose()?.flatten();
        }
        Style::Json(fields) => {
            style.color = fields
                .get("color")
                .and_then(Value::as_str)
                .map(str::to_owned);
            style.shadow_color = fields.get("shadow_color").and_then(component::shadow_color);
            style.bold = component::style_flag(fields.get("bold"));
            style.italic = component::style_flag(fields.get("italic"));
            style.underlined = component::style_flag(fields.get("underlined"));
            style.strikethrough = component::style_flag(fields.get("strikethrough"));
            style.obfuscated = component::style_flag(fields.get("obfuscated"));
            style.font = fields
                .get("font")
                .and_then(Value::as_str)
                .map(str::to_owned);
            style.insertion = fields
                .get("insertion")
                .and_then(Value::as_str)
                .map(str::to_owned);
            style.click = component::style_event_object(fields, "click_event", "clickEvent")
                .map(component::parse_click_event)
                .transpose()
                .map_err(error)?
                .flatten()
                .map(native_click)
                .transpose()?
                .flatten();
        }
    }
    style.hover = match &node.hover {
        None | Some(Hover::Text(_)) => None, // Child attachment supplies text.
        Some(Hover::Item {
            id,
            count,
            components,
        }) => Some(TextHover::Item {
            id: (*id).into(),
            count: *count,
            components: components
                .map(json_nbt::json_to_tag)
                .transpose()
                .map_err(error)?,
        }),
        Some(Hover::Entity { kind, id, .. }) => Some(TextHover::Entity {
            kind: (*kind).into(),
            id: *id,
            name: None,
        }),
    };
    let click_fields = match node.style {
        Style::Json(fields) if style.click.is_some() => {
            component::style_event_object(fields, "click_event", "clickEvent")
        }
        _ => None,
    };
    let hover_fields = node.hover_fields.filter(|_| node.hover.is_some());
    let Tag::Compound(extra) = &mut style.metadata else {
        unreachable!()
    };
    let hover_kind = match &node.hover {
        Some(Hover::Text(_)) => "show_text",
        Some(Hover::Item { .. }) => "show_item",
        Some(Hover::Entity { .. }) => "show_entity",
        None => "",
    };
    for (key, fields, kind) in [
        (
            "click_event",
            click_fields,
            style.click.as_ref().map(click_kind).unwrap_or(""),
        ),
        ("hover_event", hover_fields, hover_kind),
    ] {
        if let Some(fields) = fields {
            let tag = metadata(
                fields
                    .iter()
                    .filter(|(k, _)| !event_reserved(kind, k))
                    .map(|(k, v)| (k.as_str(), v)),
                1,
            )?;
            if !matches!(&tag, Tag::Compound(v) if v.is_empty()) {
                extra.insert(key.into(), tag);
            }
        }
    }
    Ok(style)
}
fn captured_shallow(node: &Node<'_>) -> Result<Text, Error> {
    let meta = node
        .fields
        .map(|fields| {
            metadata(
                fields
                    .iter()
                    .filter(|(k, _)| !reserved(k))
                    .map(|(k, v)| (k.as_str(), v)),
                0,
            )
        })
        .transpose()?
        .unwrap_or_else(|| Tag::Compound(Default::default()));
    let mut text = Text::new(match &node.content {
        Content::Literal(s) => TextContent::Literal((*s).into()),
        Content::Scalar(value) => TextContent::Literal(value.to_string()),
        Content::Translate { key, fallback, .. } => TextContent::Translatable {
            key: (*key).into(),
            fallback: fallback.map(str::to_owned),
            arguments: Vec::new(),
        },
        Content::Keybind(key) => TextContent::Keybind((*key).into()),
        Content::Selector { pattern, .. } => TextContent::Selector {
            pattern: (*pattern).into(),
            separator: None,
        },
        Content::Score {
            name, objective, ..
        } => TextContent::Score {
            name: (*name).into(),
            objective: (*objective).into(),
        },
        Content::Nbt {
            path,
            interpret,
            source,
            ..
        } => TextContent::Nbt {
            path: (*path).into(),
            interpret: *interpret,
            source: match source {
                NbtSource::Block(v) => TextNbtSource::Block((*v).into()),
                NbtSource::Entity(v) => TextNbtSource::Entity((*v).into()),
                NbtSource::Storage(v) => TextNbtSource::Storage((*v).into()),
            },
            separator: None,
        },
        Content::Player {
            profile: value,
            hat,
            ..
        } => TextContent::Object(TextObject::Player {
            profile: profile(value)?,
            hat: *hat,
            fallback: None,
        }),
        Content::Atlas { atlas, sprite, .. } => TextContent::Object(TextObject::Atlas {
            atlas: (*atlas).into(),
            sprite: (*sprite).into(),
            fallback: None,
        }),
    });
    text.metadata = meta;
    if let Content::Score { fields, .. } = &node.content {
        let score = metadata(
            fields
                .iter()
                .filter(|(k, _)| !["name", "objective"].contains(&k.as_str()))
                .map(|(k, v)| (k.as_str(), v)),
            1,
        )?;
        if !matches!(&score, Tag::Compound(v) if v.is_empty()) {
            let Tag::Compound(meta) = &mut text.metadata else {
                unreachable!()
            };
            meta.insert("score".into(), score);
        }
    }
    text.style = captured_style(node)?;
    Ok(text)
}
fn attach(text: &mut Text, relation: TextRelation, child: Text) {
    match relation {
        TextRelation::Child => text.children.push(child),
        TextRelation::Argument => match &mut text.content {
            TextContent::Translatable { arguments, .. } => arguments.push(child),
            _ => unreachable!(),
        },
        TextRelation::Separator => match &mut text.content {
            TextContent::Selector { separator, .. } | TextContent::Nbt { separator, .. } => {
                *separator = Some(Box::new(child))
            }
            _ => unreachable!(),
        },
        TextRelation::Fallback => match &mut text.content {
            TextContent::Object(
                TextObject::Player { fallback, .. } | TextObject::Atlas { fallback, .. },
            ) => *fallback = Some(Box::new(child)),
            _ => unreachable!(),
        },
        TextRelation::HoverText => text.style.hover = Some(TextHover::Text(Box::new(child))),
        TextRelation::HoverEntityName => match &mut text.style.hover {
            Some(TextHover::Entity { name, .. }) => *name = Some(Box::new(child)),
            _ => unreachable!(),
        },
        _ => unreachable!("component visitor supplied an unknown relation"),
    }
}
fn capture(root: Source<'_>) -> Result<Text, Error> {
    let nodes = plan(root)?;
    let mut ready: HashMap<Key, Text> = HashMap::new();
    for node in nodes {
        let mut text = if node.merge {
            node.edges
                .first()
                .map(|(child, _, _)| ready.remove(&child.key()).expect("child was built"))
                .unwrap_or_default()
        } else {
            match captured_shallow(&node) {
                Ok(value) => value,
                Err(error) => {
                    for text in ready.into_values() {
                        discard_api_text(text);
                    }
                    return Err(error);
                }
            }
        };
        for &(child, relation, _) in node.edges.iter().skip(usize::from(node.merge)) {
            attach(
                &mut text,
                relation,
                ready.remove(&child.key()).expect("child was built"),
            );
        }
        ready.insert(node.source.key(), text);
    }
    Ok(ready.remove(&root.key()).expect("root was built"))
}
pub fn captured_text(value: &Value) -> Result<Text, Error> {
    capture(Source::Json(value))
}
/// A span's hover as a mod reads it (`GuiGraphicsExtractor.componentHoverEffect`'s style): the
/// same capture as the span's own, from the style's text, item or entity hover.
pub fn style_hover(style: &engine::Style) -> Option<TextHover> {
    if style.hover.is_none() && style.hover_item.is_none() && style.hover_entity.is_none() {
        return None;
    }
    let span = engine::Span::new(String::new(), style.clone());
    capture(Source::Span(&span)).ok()?.style.hover.take()
}
/// Capture an engine text without losing structured content or style.
pub fn to_api(text: &engine::Text) -> Result<Text, TextFormError> {
    capture(Source::Engine(text))
}

fn discard_api_text(root: Text) {
    let mut pending = vec![root];
    while let Some(mut text) = pending.pop() {
        pending.append(&mut text.children);
        nbt::discard_tag(text.metadata);
        nbt::discard_tag(text.style.metadata);
        match text.content {
            TextContent::Translatable { arguments, .. } => pending.extend(arguments),
            TextContent::Selector { separator, .. } | TextContent::Nbt { separator, .. } => {
                pending.extend(separator.map(|t| *t))
            }
            TextContent::Object(object) => {
                let fallback = match object {
                    TextObject::Player {
                        profile, fallback, ..
                    } => {
                        nbt::discard_tag(profile.metadata);
                        for property in profile.properties {
                            nbt::discard_tag(property.metadata);
                        }
                        fallback
                    }
                    TextObject::Atlas { fallback, .. } => fallback,
                };
                pending.extend(fallback.map(|t| *t));
            }
            _ => {}
        }
        match text.style.hover {
            Some(TextHover::Text(t)) => pending.push(*t),
            Some(TextHover::Entity { name, .. }) => pending.extend(name.map(|t| *t)),
            Some(TextHover::Item {
                components: Some(tag),
                ..
            }) => nbt::discard_tag(tag),
            _ => {}
        }
        #[allow(clippy::single_match)] // TODO(arismoko/horizon#2273): rewrite as `if let`.
        match text.style.click {
            Some(
                TextClick::ShowDialog(tag)
                | TextClick::Custom {
                    payload: Some(tag), ..
                },
            ) => nbt::discard_tag(tag),
            _ => {}
        }
    }
}
fn api_nodes(root: &Text) -> Vec<&Text> {
    let mut work = vec![(root, false)];
    let mut nodes = Vec::new();
    while let Some((text, done)) = work.pop() {
        if done {
            nodes.push(text);
        } else {
            work.push((text, true));
            let mut children = Vec::new();
            text.visit_children(|text, _| children.push((text, false)));
            work.extend(children.into_iter().rev());
        }
    }
    nodes
}
/// What stands in a custom click's `payload` where the typed tag is kept aside (`exact_payloads`): this prefix,
/// the call's random nonce and the slot, so text that merely looks like a slot is never taken for one.
const PAYLOAD_SLOT: &str = "\0payload:";
/// A random number for one conversion: a mod cannot know it, so it cannot write a slot of its own.
fn payload_nonce() -> u64 {
    use std::hash::BuildHasher;
    std::collections::hash_map::RandomState::new().hash_one(std::time::Instant::now())
}
struct Payloads<'a> {
    nonce: u64,
    tags: HashMap<usize, json_nbt::JsonTag<'a>>,
    /// With `exact_payloads`, the custom click payloads taken out of the JSON, in slot order.
    slots: Option<std::cell::RefCell<Vec<&'a Tag>>>,
}
impl<'a> Payloads<'a> {
    fn new(nodes: &[&'a Text], exact: bool) -> Result<Self, Error> {
        let mut tags = HashMap::new();
        let mut keep = |tag: &'a Tag| -> Result<(), Error> {
            if matches!(tag, Tag::Compound(v) if v.is_empty())
                || matches!(tag, Tag::List(v) if v.is_empty())
            {
                return Ok(());
            }
            tags.insert(
                tag as *const _ as usize,
                json_nbt::JsonTag::new(tag).map_err(error)?,
            );
            Ok(())
        };
        for text in nodes {
            keep(&text.metadata)?;
            keep(&text.style.metadata)?;
            if let TextContent::Object(TextObject::Player { profile, .. }) = &text.content {
                keep(&profile.metadata)?;
                for p in &profile.properties {
                    keep(&p.metadata)?;
                }
            }
            #[allow(clippy::single_match)] // TODO(arismoko/horizon#2273): rewrite as `if let`.
            match &text.style.click {
                Some(
                    TextClick::ShowDialog(tag)
                    | TextClick::Custom {
                        payload: Some(tag), ..
                    },
                ) => keep(tag)?,
                _ => {}
            }
            if let Some(TextHover::Item {
                components: Some(tag),
                ..
            }) = &text.style.hover
            {
                keep(tag)?;
            }
        }
        Ok(Self {
            nonce: payload_nonce(),
            tags,
            slots: exact.then(Default::default),
        })
    }
    /// A custom click's payload: its JSON, or with `exact_payloads` a slot for the tag itself.
    fn emit_payload(&self, tag: &'a Tag) -> Value {
        let Some(slots) = &self.slots else {
            return self.emit(tag);
        };
        let mut slots = slots.borrow_mut();
        slots.push(tag);
        format!("{PAYLOAD_SLOT}{:016x}:{}", self.nonce, slots.len() - 1).into()
    }
    fn emit(&self, tag: &Tag) -> Value {
        match tag {
            Tag::Compound(v) if v.is_empty() => Value::Object(Map::new()),
            Tag::List(v) if v.is_empty() => Value::Array(Vec::new()),
            _ => self.tags[&(tag as *const _ as usize)].to_json(),
        }
    }
    fn object(&self, tag: &Tag) -> Map<String, Value> {
        let Value::Object(map) = self.emit(tag) else {
            unreachable!("validated metadata is a compound")
        };
        map
    }
}
fn filter_metadata(
    fields: Map<String, Value>,
    reserved: impl Fn(&str) -> bool,
) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in fields {
        if reserved(&key) {
            component::discard_component(value);
        } else {
            out.insert(key, value);
        }
    }
    out
}
fn object_or_empty(value: Option<Value>) -> Map<String, Value> {
    match value {
        Some(Value::Object(map)) => map,
        Some(value) => {
            component::discard_component(value);
            Map::new()
        }
        None => Map::new(),
    }
}
fn profile_json(profile: &TextProfile, payloads: &Payloads<'_>) -> Value {
    let mut out = filter_metadata(payloads.object(&profile.metadata), |k| {
        ["name", "id", "properties"].contains(&k)
    });
    if let Some(name) = &profile.name {
        out.insert("name".into(), name.clone().into());
    }
    if let Some([hi, lo]) = profile.id {
        out.insert(
            "id".into(),
            Value::Array(
                [(hi >> 32) as i32, hi as i32, (lo >> 32) as i32, lo as i32]
                    .map(Value::from)
                    .into(),
            ),
        );
    }
    if !profile.properties.is_empty() {
        let unsigned = profile.properties.iter().all(|p| {
            p.signature.is_none() && matches!(&p.metadata, Tag::Compound(m) if m.is_empty())
        });
        let use_map = unsigned
            && (profile.properties.len() > 16
                || profile.properties.iter().any(|p| {
                    p.name.encode_utf16().take(65).count() > 64
                        || p.value.encode_utf16().take(32768).count() > 32767
                }));
        let properties = if use_map {
            let mut map = Map::new();
            for p in &profile.properties {
                let value = map
                    .entry(p.name.clone())
                    .or_insert_with(|| Value::Array(Vec::new()));
                let Value::Array(values) = value else {
                    unreachable!()
                };
                values.push(p.value.clone().into());
            }
            Value::Object(map)
        } else {
            Value::Array(
                profile
                    .properties
                    .iter()
                    .map(|p| {
                        let mut fields = filter_metadata(payloads.object(&p.metadata), |k| {
                            ["name", "value", "signature"].contains(&k)
                        });
                        fields.insert("name".into(), p.name.clone().into());
                        fields.insert("value".into(), p.value.clone().into());
                        if let Some(signature) = &p.signature {
                            fields.insert("signature".into(), signature.clone().into());
                        }
                        Value::Object(fields)
                    })
                    .collect(),
            )
        };
        out.insert("properties".into(), properties);
    }
    Value::Object(out)
}
fn action(action: &str, field: &str, value: Value) -> Map<String, Value> {
    [("action".into(), action.into()), (field.into(), value)]
        .into_iter()
        .collect()
}
fn emitted_style<'a>(
    style: &'a TextStyle,
    payloads: &Payloads<'a>,
    take: &mut impl FnMut(&'a Text) -> Value,
) -> Map<String, Value> {
    let mut metadata = payloads.object(&style.metadata);
    let click = object_or_empty(metadata.remove("click_event"));
    let hover = object_or_empty(metadata.remove("hover_event"));
    let mut out = filter_metadata(metadata, reserved);
    for (key, value) in [
        ("color", style.color.clone().map(Value::String)),
        (
            "shadow_color",
            style.shadow_color.map(|v| Value::from(v as i32)),
        ),
        ("bold", style.bold.map(Value::Bool)),
        ("italic", style.italic.map(Value::Bool)),
        ("underlined", style.underlined.map(Value::Bool)),
        ("strikethrough", style.strikethrough.map(Value::Bool)),
        ("obfuscated", style.obfuscated.map(Value::Bool)),
        ("font", style.font.clone().map(Value::String)),
        ("insertion", style.insertion.clone().map(Value::String)),
    ] {
        if let Some(v) = value {
            out.insert(key.into(), v);
        }
    }
    if let Some(value) = &style.click {
        let kind = click_kind(value);
        let mut fields = filter_metadata(click, |key| event_reserved(kind, key));
        fields.extend(match value {
            TextClick::OpenUrl(v) => action("open_url", "url", v.clone().into()),
            TextClick::ChangePage(v) => action("change_page", "page", (*v).into()),
            TextClick::SuggestCommand(v) => action("suggest_command", "command", v.clone().into()),
            TextClick::RunCommand(v) => action("run_command", "command", v.clone().into()),
            TextClick::CopyToClipboard(v) => action("copy_to_clipboard", "value", v.clone().into()),
            TextClick::ShowDialog(v) => action("show_dialog", "dialog", payloads.emit(v)),
            TextClick::Custom { id, payload } => {
                let mut fields = action("custom", "id", id.clone().into());
                if let Some(tag) = payload {
                    fields.insert("payload".into(), payloads.emit_payload(tag));
                }
                fields
            }
        });
        out.insert("click_event".into(), Value::Object(fields));
    } else {
        component::discard_component(Value::Object(click));
    }
    if let Some(value) = &style.hover {
        let kind = match value {
            TextHover::Text(_) => "show_text",
            TextHover::Item { .. } => "show_item",
            TextHover::Entity { .. } => "show_entity",
        };
        let mut fields = filter_metadata(hover, |key| event_reserved(kind, key));
        match value {
            TextHover::Text(text) => {
                fields.insert("action".into(), "show_text".into());
                fields.insert("value".into(), take(text));
            }
            TextHover::Item {
                id,
                count,
                components,
            } => {
                fields.extend([
                    ("action".into(), "show_item".into()),
                    ("id".into(), id.clone().into()),
                    ("count".into(), (*count).into()),
                ]);
                if let Some(tag) = components {
                    fields.insert("components".into(), payloads.emit(tag));
                }
            }
            TextHover::Entity {
                kind,
                id: [hi, lo],
                name,
            } => {
                fields.extend([
                    ("action".into(), "show_entity".into()),
                    ("id".into(), kind.clone().into()),
                    (
                        "uuid".into(),
                        Value::Array(
                            [
                                (*hi >> 32) as i32,
                                *hi as i32,
                                (*lo >> 32) as i32,
                                *lo as i32,
                            ]
                            .map(Value::from)
                            .into(),
                        ),
                    ),
                ]);
                if let Some(name) = name {
                    fields.insert("name".into(), take(name));
                }
            }
        }
        out.insert("hover_event".into(), Value::Object(fields));
    } else {
        component::discard_component(Value::Object(hover));
    }
    out
}
fn emitted_node<'a>(
    text: &'a Text,
    payloads: &Payloads<'a>,
    take: &mut impl FnMut(&'a Text) -> Value,
) -> Value {
    let mut metadata = payloads.object(&text.metadata);
    let mut score = metadata.remove("score");
    let mut out = filter_metadata(metadata, reserved);
    match &text.content {
        TextContent::Literal(v) => {
            out.insert("text".into(), v.clone().into());
        }
        TextContent::Translatable {
            key,
            fallback,
            arguments,
        } => {
            out.insert("translate".into(), key.clone().into());
            if let Some(fallback) = fallback {
                out.insert("fallback".into(), fallback.clone().into());
            }
            if !arguments.is_empty() {
                out.insert(
                    "with".into(),
                    Value::Array(
                        arguments
                            .iter()
                            .map(|arg| engine::argument_component(take(arg)))
                            .collect(),
                    ),
                );
            }
        }
        TextContent::Keybind(v) => {
            out.insert("keybind".into(), v.clone().into());
        }
        TextContent::Selector { pattern, separator } => {
            out.insert("selector".into(), pattern.clone().into());
            if let Some(separator) = separator {
                out.insert("separator".into(), take(separator));
            }
        }
        TextContent::Score { name, objective } => {
            let mut fields = filter_metadata(object_or_empty(score.take()), |k| {
                ["name", "objective"].contains(&k)
            });
            fields.insert("name".into(), name.clone().into());
            fields.insert("objective".into(), objective.clone().into());
            out.insert("score".into(), Value::Object(fields));
        }
        TextContent::Nbt {
            path,
            interpret,
            source,
            separator,
        } => {
            out.insert("nbt".into(), path.clone().into());
            out.insert("interpret".into(), (*interpret).into());
            let (key, value) = match source {
                TextNbtSource::Block(v) => ("block", v),
                TextNbtSource::Entity(v) => ("entity", v),
                TextNbtSource::Storage(v) => ("storage", v),
            };
            out.insert(key.into(), value.clone().into());
            if let Some(separator) = separator {
                out.insert("separator".into(), take(separator));
            }
        }
        TextContent::Object(object) => {
            let fallback = match object {
                TextObject::Player {
                    profile,
                    hat,
                    fallback,
                } => {
                    out.extend([
                        ("object".into(), "player".into()),
                        ("player".into(), profile_json(profile, payloads)),
                        ("hat".into(), (*hat).into()),
                    ]);
                    fallback
                }
                TextObject::Atlas {
                    atlas,
                    sprite,
                    fallback,
                } => {
                    out.extend([
                        ("object".into(), "atlas".into()),
                        ("atlas".into(), atlas.clone().into()),
                        ("sprite".into(), sprite.clone().into()),
                    ]);
                    fallback
                }
            };
            if let Some(fallback) = fallback {
                out.insert("fallback".into(), take(fallback));
            }
        }
    }
    if let Some(score) = score {
        component::discard_component(score);
    }
    out.extend(emitted_style(&text.style, payloads, take));
    if !text.children.is_empty() {
        out.insert(
            "extra".into(),
            Value::Array(text.children.iter().map(take).collect()),
        );
    }
    Value::Object(out)
}
fn owned_text_component(text: &Text) -> Result<Value, Error> {
    component_and_payloads(text, false).map(|(component, _)| component)
}
/// The component JSON, and with `exact` the custom click payloads it leaves for their own tags.
fn component_and_payloads(text: &Text, exact: bool) -> Result<(Value, PayloadSlots<'_>), Error> {
    text.validate()?;
    let nodes = api_nodes(text);
    let payloads = Payloads::new(&nodes, exact)?; // Refuse all representation errors before any JSON clone.
    let mut ready = HashMap::new();
    for node in nodes {
        let value = emitted_node(node, &payloads, &mut |child| {
            ready
                .remove(&(child as *const _ as usize))
                .expect("child was built")
        });
        ready.insert(node as *const _ as usize, value);
    }
    let component = ready
        .remove(&(text as *const _ as usize))
        .expect("root was built");
    let nonce = payloads.nonce;
    let tags = payloads
        .slots
        .map(|slots| slots.into_inner())
        .unwrap_or_default();
    Ok((component, PayloadSlots { nonce, tags }))
}
/// The custom click payloads kept aside by [`component_with_payload_slots`], in slot order, with the nonce of
/// the slots that stand for them.
pub struct PayloadSlots<'a> {
    nonce: u64,
    tags: Vec<&'a Tag>,
}
/// The component JSON of `text` as `to_engine` builds it, except that each custom click's
/// payload is a slot, with the payload tags themselves in slot order. JSON has no tag widths
/// or typed arrays, so a payload only keeps its type when [`restore_payloads`] puts the tag
/// back into the NBT of this component.
pub fn component_with_payload_slots(
    text: &Text,
) -> Result<(Value, PayloadSlots<'_>), TextFormError> {
    component_and_payloads(text, true)
}
/// Put the payload tags of [`component_with_payload_slots`] back where the NBT of its component
/// holds their slots: the `payload` of a custom `click_event`.
pub fn restore_payloads(root: &mut Tag, payloads: &PayloadSlots<'_>) {
    let wanted = format!("{PAYLOAD_SLOT}{:016x}:", payloads.nonce);
    let slot = |event: &Tag| -> Option<usize> {
        let (Some(Tag::String(action)), Some(Tag::String(payload))) =
            (event.get("action"), event.get("payload"))
        else {
            return None;
        };
        if action.to_string().ok()? != "custom" {
            return None;
        }
        payload
            .to_string()
            .ok()?
            .strip_prefix(wanted.as_str())?
            .parse()
            .ok()
    };
    let click_event = nbt::JavaString::from("click_event");
    let mut work = vec![root];
    while let Some(tag) = work.pop() {
        match tag {
            Tag::List(values) => work.extend(values.iter_mut()),
            Tag::Compound(entries) => {
                for (key, value) in entries.iter_mut() {
                    // The restored payload is the mod's own data: never read as more components.
                    if *key == click_event
                        && let Some(original) = slot(value).and_then(|slot| payloads.tags.get(slot))
                    {
                        if let Some(payload) = value.get_mut("payload") {
                            *payload = nbt::clone_tag(original);
                        }
                    } else {
                        work.push(value);
                    }
                }
            }
            _ => {}
        }
    }
}
/// Encode API text, preserving the boundary's validation and representation errors.
pub fn to_engine(text: &Text) -> Result<engine::Text, TextFormError> {
    owned_text_component(text).map(Into::into)
}

#[cfg(test)]
mod text_capture_tests {
    use super::{TryIntoApi, TryIntoEngine};

    #[test]
    fn missing_translation_template_survives_typed_iterative_boundary() {
        let language = horizon_client_core::lang::Language::of(
            "en_us",
            [("Hi %s".into(), "Wrong translation".into())],
        );
        horizon_client_core::lang::scoped(language, || {
            let engine = horizon_client_core::text::Text::translate_with_fallback(
                "test.missing",
                "Hi %s",
                &[horizon_client_core::text::Text::literal("Steve")],
            );
            let api = engine.try_into_api().unwrap();
            assert!(
                matches!(&api.content, mod_api::text::TextContent::Translatable { fallback: Some(template), .. } if template == "Hi %s")
            );
            assert_eq!(api.try_into_engine(&()).unwrap().plain(), "Hi Steve");
        });
    }

    #[test]
    fn native_translation_fallback_survives_noop_owned_round_trip() {
        let language = horizon_client_core::lang::Language::of(
            "en_us",
            [("test.fallback".into(), "Wrong translation".into())],
        );
        horizon_client_core::lang::scoped(language, || {
            let native = horizon_client_core::text::Text::translate_with_fallback(
                "test.missing",
                "test.fallback",
                &[],
            );
            let mut api = native.try_into_api().unwrap();
            let listener = |_: &mut mod_api::text::Text| {};
            listener(&mut api);
            let actual = api.try_into_engine(&()).unwrap();
            assert_eq!(actual.plain(), "test.fallback");
            assert_eq!(actual.spans, native.spans);
            assert_eq!(actual.component(), native.component());
        });
    }
    use horizon_client_core::text::Text as EngineText;
    use mod_api::text::{TextContent as C, TextHover};
    use serde_json::json;

    #[test]
    fn item_and_entity_hovers_survive_a_round_trip() {
        for hover in [
            json!({"action":"show_item", "id":"minecraft:stick", "count":3, "components":{"minecraft:rarity":"epic"}}),
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[0,1,0,2], "name":{"translate":"test.name"}}),
        ] {
            let source = json!({"text":"x", "hover_event":hover});
            let api = EngineText::from(source).try_into_api().unwrap();
            let engine = api.try_into_engine(&()).unwrap();
            assert_eq!(engine.component()["hover_event"], hover);
        }
    }

    #[test]
    fn a_span_style_hover_matches_the_component_capture() {
        for hover in [
            json!({"action":"show_text", "value":{"text":"hi", "bold":true}}),
            json!({"action":"show_item", "id":"minecraft:stick", "count":3, "components":{"minecraft:rarity":"epic"}}),
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[0,1,0,2], "name":"Babe"}),
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":"11223344-5566-7788-99aa-bbccddeeff00"}),
        ] {
            let source = json!({"text":"x", "hover_event":hover});
            let engine = EngineText::from(source);
            let expected = engine.try_into_api().unwrap().style.hover;
            assert!(expected.is_some());
            assert_eq!(super::style_hover(&engine.spans[0].style), expected);
        }
        let plain = EngineText::from(json!({"text":"x"}));
        assert_eq!(super::style_hover(&plain.spans[0].style), None);
    }

    #[test]
    fn untrusted_file_action_cannot_become_local_after_a_noop_value_round_trip() {
        let source = json!({"text":"server", "click_event":{"action":"open_file", "path":"test-file"},
            "extra":[{"text":"child", "clickEvent":{"action":"open_file", "value":"test-file"}}]});
        let mut api = EngineText::from(source).try_into_api().unwrap();
        assert!(api.style.click.is_none());
        assert!(api.metadata.get("click_event").is_none());
        assert!(api.style.metadata.get("click_event").is_none());
        assert!(api.children[0].style.click.is_none());
        let listener = |_: &mut mod_api::text::Text| {};
        listener(&mut api);
        let engine = api.try_into_engine(&()).unwrap();
        assert!(engine.spans.iter().all(|span| span.style.click.is_none()));
        let component = engine.component();
        assert!(component.get("click_event").is_none());
        assert!(component["extra"][0].get("clickEvent").is_none());
    }

    #[test]
    fn component_tree_capture_keeps_dynamic_contents_and_unknown_metadata() {
        let source = json!({"translate":"test.message", "fallback":"%s", "with":[{"text":"name", "color":"red"}],
            "bold":false, "test:meta":{"n":7}, "click_event":{"action":"custom", "id":"test:open", "payload":{"n":9}, "test:trace":1},
            "extra":[{"nbt":"value", "storage":"test:data", "interpret":true, "separator":{"text":","}},
                {"score":{"name":"@s", "objective":"test", "test:trace":2}}]});
        let api = EngineText::from(source.clone()).try_into_api().unwrap();
        assert!(
            matches!(&api.content, C::Translatable { key, fallback:Some(fallback), arguments } if key == "test.message" && fallback == "%s" && arguments.len() == 1)
        );
        assert_eq!(api.style.bold, Some(false));
        assert!(
            matches!(&api.children[0].content, C::Nbt { path, interpret:true, .. } if path == "value")
        );
        assert_eq!(
            api.style.metadata.get("click_event"),
            Some(&mod_api::Tag::compound([(
                "test:trace",
                mod_api::Tag::Byte(1)
            )]))
        );
        assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
    }

    #[test]
    fn object_fallback_round_trip_keeps_omitted_styled_and_translatable() {
        for object in [
            json!({"object":"player", "player":{"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}, "hat":true}),
            json!({"object":"atlas", "atlas":"minecraft:blocks", "sprite":"test:icon"}),
        ] {
            for fallback in [
                None,
                Some(
                    json!({"text":"label", "italic":false, "extra":[{"text":"child", "bold":true}]}),
                ),
                Some(json!({"translate":"test.head", "with":[{"text":"name"}]})),
            ] {
                let mut source = object.clone();
                if let Some(fallback) = fallback {
                    source["fallback"] = fallback;
                }
                let api = EngineText::from(source.clone()).try_into_api().unwrap();
                assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
            }
        }
    }

    #[test]
    fn hover_uuid_forms_keep_identity_and_unresolved_name() {
        for uuid in [
            json!("11223344-5566-7788-99aa-bbccddeeff00"),
            json!([287454020, 1432778632, -1716864052_i64, -571539712]),
        ] {
            let source = json!({"text":"entity", "hover_event":{"action":"show_entity", "id":"minecraft:pig", "uuid":uuid, "name":{"translate":"test.name"}, "test:trace":true}});
            let api = EngineText::from(source).try_into_api().unwrap();
            let Some(TextHover::Entity {
                id,
                name: Some(name),
                ..
            }) = &api.style.hover
            else {
                panic!("entity hover")
            };
            assert_eq!(*id, [0x1122334455667788, 0x99aabbccddeeff00]);
            assert!(matches!(&name.content, C::Translatable { key, .. } if key == "test.name"));
            assert_eq!(
                api.try_into_engine(&()).unwrap().component()["hover_event"],
                json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[287454020,1432778632,-1716864052_i64,-571539712], "name":{"translate":"test.name"}, "test:trace":1})
            );
        }
    }

    #[test]
    fn style_capture_reuses_snbt_flags_and_rgba_shadow_rules() {
        let source = json!({"text":"styled", "bold":0, "italic":1, "shadow_color":[1,0,0.5,1]});
        let api = EngineText::from(source).try_into_api().unwrap();
        assert_eq!(api.style.bold, Some(false));
        assert_eq!(api.style.italic, Some(true));
        assert_eq!(api.style.shadow_color, Some(0xffff007f));
    }
}

#[cfg(test)]
mod text_encoding_tests {
    fn test_profile(signed: bool) -> mod_api::text::TextProfile {
        let mut profile = mod_api::text::TextProfile::default();
        profile.name = Some("Test".into());
        if signed {
            let mut p = mod_api::text::TextProfileProperty::default();
            p.name = "textures".into();
            p.value = "data".into();
            p.signature = Some("signed".into());
            profile.properties.push(p);
        }
        profile
    }
    use super::TryIntoEngine;
    use mod_api::text::{Text, TextClick, TextContent as C, TextHover, TextNbtSource, TextObject};
    use serde_json::json;

    #[test]
    fn owned_text_keeps_unresolved_translation_style_and_ordered_children() {
        let mut text = Text::translate("test.message", vec![Text::literal("7")]);
        text.style.bold = Some(false);
        text.style.color = Some("red".into());
        text.style.shadow_color = Some(0xff001122);
        text.style.click = Some(TextClick::RunCommand("/test".into()));
        text.style.metadata = mod_api::Tag::compound([(
            "click_event",
            mod_api::Tag::compound([("test:extra", mod_api::Tag::Byte(1))]),
        )]);
        text.metadata = mod_api::Tag::compound([(
            "test:context",
            mod_api::Tag::compound([("value", mod_api::Tag::Int(7))]),
        )]);
        text.children.push(Text::literal("first"));
        text.children.push(Text::literal("second"));
        let engine: Result<horizon_client_core::text::Text, mod_api::text::TextFormError> =
            text.try_into_engine(&());
        assert_eq!(
            engine.unwrap().component(),
            json!({
                "translate":"test.message", "with":[7], "bold":false, "color":"red",
                "shadow_color":-16772830, "click_event":{"action":"run_command", "command":"/test", "test:extra":1},
                "test:context":{"value":7}, "extra":[{"text":"first"}, {"text":"second"}]
            })
        );
    }

    #[test]
    fn owned_text_retains_dynamic_sources_and_inline_profile_properties() {
        for (content, expected) in [
            (C::Keybind("key.jump".into()), json!({"keybind":"key.jump"})),
            (
                C::Selector {
                    pattern: "@p".into(),
                    separator: Some(Box::new(Text::literal(","))),
                },
                json!({"selector":"@p", "separator":{"text":","}}),
            ),
            (
                C::Score {
                    name: "@s".into(),
                    objective: "test".into(),
                },
                json!({"score":{"name":"@s", "objective":"test"}}),
            ),
            (
                C::Nbt {
                    path: "value".into(),
                    interpret: true,
                    source: TextNbtSource::Storage("test:data".into()),
                    separator: None,
                },
                json!({"nbt":"value", "interpret":true, "storage":"test:data"}),
            ),
            (
                C::Object(TextObject::Player {
                    profile: test_profile(true),
                    hat: false,
                    fallback: Some(Box::new(Text::literal("head"))),
                }),
                json!({"object":"player", "player":{"name":"Test", "properties":[{"name":"textures", "value":"data", "signature":"signed"}]}, "hat":false, "fallback":{"text":"head"}}),
            ),
        ] {
            let engine = Text::new(content).try_into_engine(&()).unwrap();
            assert_eq!(engine.component(), expected);
        }
    }

    #[test]
    fn object_fallback_encoder_keeps_default_styling_and_translation_tree() {
        let mut styled = Text::literal("styled");
        styled.style.italic = Some(false);
        for player in [true, false] {
            for (fallback, expected) in [
                (None, None),
                (
                    Some(Box::new(styled.clone())),
                    Some(json!({"text":"styled", "italic":false})),
                ),
                (
                    Some(Box::new(Text::translate("test.head", vec![]))),
                    Some(json!({"translate":"test.head"})),
                ),
            ] {
                let object = if player {
                    TextObject::Player {
                        profile: test_profile(false),
                        hat: true,
                        fallback,
                    }
                } else {
                    TextObject::Atlas {
                        atlas: "minecraft:blocks".into(),
                        sprite: "test:icon".into(),
                        fallback,
                    }
                };
                let engine = Text::new(C::Object(object)).try_into_engine(&()).unwrap();
                match expected {
                    None => {
                        assert!(engine.component().get("fallback").is_none());
                        assert_eq!(
                            engine.plain(),
                            if player { "[Test head]" } else { "[test:icon]" }
                        );
                    }
                    Some(expected) => assert_eq!(engine.component()["fallback"], expected),
                }
            }
        }
    }

    #[test]
    fn raw_file_action_metadata_does_not_reach_the_engine() {
        let mut text = Text::literal("remote");
        for metadata in [&mut text.metadata, &mut text.style.metadata] {
            for field in ["click_event", "clickEvent"] {
                let mod_api::Tag::Compound(fields) = metadata else {
                    unreachable!()
                };
                fields.insert(
                    field.into(),
                    mod_api::Tag::compound([
                        ("action", mod_api::Tag::String("open_file".into())),
                        ("path", mod_api::Tag::String("test-file".into())),
                    ]),
                );
            }
        }
        let engine = text.try_into_engine(&()).unwrap();
        assert!(engine.spans.iter().all(|span| span.style.click.is_none()));
        assert!(engine.component().get("click_event").is_none());
        assert!(engine.component().get("clickEvent").is_none());
    }

    #[test]
    fn owned_text_keeps_hover_uuid_name_and_component_removal_patch() {
        let mut text = Text::literal("entity");
        text.style.hover = Some(TextHover::Entity {
            kind: "minecraft:pig".into(),
            id: [0x1122334455667788, 0x99aabbccddeeff00],
            name: Some(Box::new(Text::literal("name"))),
        });
        let engine = text.try_into_engine(&()).unwrap();
        assert_eq!(
            engine.component()["hover_event"],
            json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[287454020,1432778632,-1716864052_i64,-571539712], "name":{"text":"name"}})
        );
        text.style.hover = Some(TextHover::Item {
            id: "minecraft:stone".into(),
            count: 3,
            components: Some(mod_api::Tag::compound([(
                "!minecraft:custom_name",
                mod_api::Tag::Compound(Default::default()),
            )])),
        });
        let engine = text.try_into_engine(&()).unwrap();
        assert_eq!(
            engine.component()["hover_event"],
            json!({"action":"show_item", "id":"minecraft:stone", "count":3, "components":{"!minecraft:custom_name":{}}})
        );
    }
}

#[cfg(test)]
mod iterative_tests {
    use super::*;
    use mod_api::text::TextForm;
    use serde_json::json;

    #[test]
    fn default_stack_builders_build_cross_clone_and_drop_512_deep_text() {
        // Integrated and network workers use named builders without stack_size;
        // exercise that default as well as std::thread::spawn's unnamed default.
        for name in [None, Some("Server thread"), Some("horizon-net")] {
            let builder = std::thread::Builder::new();
            let builder = match name {
                Some(name) => builder.name(name.into()),
                None => builder,
            };
            builder
                .spawn(|| {
                    let mut text = mod_api::Text::literal("leaf");
                    for _ in 1..512 {
                        let mut parent = mod_api::Text::literal("parent");
                        parent.children.push(text);
                        text = parent;
                    }
                    let engine = text.try_into_engine(&()).unwrap();
                    let restored = engine.try_into_api().unwrap();
                    assert_eq!(TextForm::try_from(&restored).unwrap().nodes.len(), 512);
                    drop((text.clone(), restored.clone(), engine.clone()));
                    drop((text, restored, engine));
                })
                .unwrap()
                .join()
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn recursive_fields_survive_512_nodes_on_256_kib() {
        std::thread::Builder::new().stack_size(256 * 1024).spawn(|| {
            for (relation, depth) in (0..9).flat_map(|r| [(r, 512), (r, 513)]) {
                let mut source = json!({"text":"leaf", "bold":false, "test:metadata":17});
                for _ in 1..depth {
                    let mut parent = match relation {
                        0 => json!({"text":"parent"}),
                        1 => json!({"translate":"test:translation", "fallback":"%s"}),
                        2 => json!({"selector":"@s"}),
                        3 => json!({"object":"atlas", "sprite":"test:icon"}),
                        6 => json!({"nbt":"name", "storage":"test:data", "interpret":true}),
                        7 => json!({"object":"player", "player":{"name":"test"}}),
                        8 => json!({"text":""}),
                        _ => json!({"text":"parent"}),
                    };
                    let fields = parent.as_object_mut().unwrap();
                    match relation {
                        0 | 1 => { fields.insert(if relation == 0 { "extra" } else { "with" }.into(), serde_json::Value::Array(vec![source])); }
                        2 | 3 | 6 | 7 => { fields.insert(if relation == 2 || relation == 6 { "separator" } else { "fallback" }.into(), source); }
                        _ => {
                            let mut hover = if relation == 4 { json!({"action":"show_text"}) } else { json!({"action":"show_entity", "id":"minecraft:pig", "uuid":[0,0,0,1]}) };
                            hover.as_object_mut().unwrap().insert(if relation == 4 { "value" } else { "name" }.into(), source);
                            fields.insert("hover_event".into(), hover);
                        }
                    }
                    source = parent;
                }
                let engine = horizon_client_core::text::Text::empty().with_component(source);
                if depth == 513 {
                    assert!(matches!(engine.try_into_api(), Err(mod_api::text::TextFormError::TooDeep)));
                    assert!(matches!(captured_text(&engine.component_view()), Err(mod_api::text::TextFormError::TooDeep)));
                    std::mem::forget(engine);
                    continue;
                }
                let api = engine.try_into_api().unwrap();
                let expected = TextForm::try_from(&api).unwrap();
                assert_eq!(expected.nodes.len(), 512);
                assert!(matches!(&expected.nodes.last().unwrap().content, mod_api::text::TextNodeContent::Literal(s) if s == "leaf"));
                let emitted = api.try_into_engine(&()).unwrap();
                let restored = emitted.try_into_api().unwrap();
                let actual = TextForm::try_from(&restored).unwrap();
                assert_eq!(actual, expected);
                // Recursive ownership operations are not the crossing guarantee.
                std::mem::forget((engine, api, emitted, restored));
            }
        }).unwrap().join().unwrap();
    }

    #[test]
    fn too_deep_is_refused_before_cloning_opaque_metadata() {
        // Keep opaque metadata much deeper than the small stack could recursively clone.
        // The component-depth refusal must win before any metadata is copied.
        let mut opaque = Tag::Byte(1);
        for _ in 0..4096 {
            opaque = Tag::List(vec![opaque]);
        }
        let mut api = mod_api::Text::literal("leaf");
        for _ in 1..513 {
            api = mod_api::Text::literal("parent").append(api);
        }
        // Build the source separately without serde's recursive json! value serializer.
        let mut component = json!({"text":"leaf"});
        for _ in 1..513 {
            let mut parent = serde_json::Map::new();
            parent.insert("text".into(), "parent".into());
            parent.insert("extra".into(), serde_json::Value::Array(vec![component]));
            component = serde_json::Value::Object(parent);
        }
        api.metadata = Tag::compound([("test:opaque", opaque)]);
        // A separate opaque value avoids cloning either deep input in the test itself.
        let mut opaque = serde_json::Value::Null;
        for _ in 0..4096 {
            opaque = serde_json::Value::Array(vec![opaque]);
        }
        component
            .as_object_mut()
            .unwrap()
            .insert("test:opaque".into(), opaque);
        let engine = horizon_client_core::text::Text::empty().with_component(component);
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                assert!(matches!(
                    api.try_into_engine(&()),
                    Err(mod_api::text::TextFormError::TooDeep)
                ));
                assert!(matches!(
                    engine.try_into_api(),
                    Err(mod_api::text::TextFormError::TooDeep)
                ));
                std::mem::forget((api, engine));
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn span_only_hover_sources_are_validated_and_materialized_iteratively() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                use horizon_client_core::text::{Span, Style, Text as Engine};
                for depth in [512, 513] {
                    let mut text = Engine::from_spans(vec![Span::new("leaf", Style::default())]);
                    for _ in 1..depth {
                        text = Engine::from_spans(vec![Span::new(
                            "parent",
                            Style {
                                hover: Some(std::sync::Arc::new(text)),
                                ..Style::default()
                            },
                        )]);
                    }
                    let api = text.try_into_api();
                    if depth == 513 {
                        assert!(matches!(api, Err(mod_api::text::TextFormError::TooDeep)));
                    } else {
                        assert_eq!(
                            TextForm::try_from(api.as_ref().unwrap())
                                .unwrap()
                                .nodes
                                .len(),
                            512
                        );
                    }
                    std::mem::forget((text, api));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn array_root_and_children_keep_order_and_metadata() {
        let source = json!([
            {"text":"root", "extra":[{"text":"first"}], "bold":false},
            {"text":"second", "test:metadata":{"id":7}}
        ]);
        let api = captured_text(&source).unwrap();
        assert_eq!(api.children.len(), 2);
        assert_eq!(
            owned_text_component(&api).unwrap(),
            json!({
                "text":"root", "bold":false,
                "extra":[{"text":"first"}, {"text":"second", "test:metadata":{"id":7}}]
            })
        );
    }

    #[test]
    fn component_view_borrows_retained_source() {
        let engine =
            horizon_client_core::text::Text::empty().with_component(json!({"text":"source"}));
        assert!(matches!(
            engine.component_view(),
            std::borrow::Cow::Borrowed(_)
        ));
    }
}

#[cfg(test)]
trait TryIntoApi {
    fn try_into_api(&self) -> Result<Text, TextFormError>;
}
#[cfg(test)]
impl TryIntoApi for engine::Text {
    fn try_into_api(&self) -> Result<Text, TextFormError> {
        to_api(self)
    }
}
#[cfg(test)]
trait TryIntoEngine<C: ?Sized> {
    fn try_into_engine(&self, context: &C) -> Result<engine::Text, TextFormError>;
}
#[cfg(test)]
impl<C: ?Sized> TryIntoEngine<C> for Text {
    fn try_into_engine(&self, _: &C) -> Result<engine::Text, TextFormError> {
        to_engine(self)
    }
}
