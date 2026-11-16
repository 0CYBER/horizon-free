//! Text components from commands: `ComponentArgument` reads an SNBT (or JSON)
//! value, `ComponentSerialization` turns it into styled text.
use crate::text::{InlineObject, Origin, Style, Text, named_color};
use horizon_core::nbt::Tag;
use serde_json::Value;

/// `SnbtGrammar` into JSON for the text renderer, using the canonical NBT parser.
pub fn parse_snbt(input: &str) -> Result<(Value, usize), String> {
    let (tag, used) = horizon_core::nbt::parse_snbt(input)?;
    let value = horizon_core::json_nbt::tag_to_json(&tag).map_err(|error| error.to_string())?;
    Ok((value, used))
}
/// `click_event` / `hover_event`, or the pre-datafixer `clickEvent` /
/// `hoverEvent` names SNBT and older chat JSON still use.
pub fn style_event_object<'a>(
    o: &'a serde_json::Map<String, Value>,
    modern: &str,
    legacy: &str,
) -> Option<&'a serde_json::Map<String, Value>> {
    o.get(modern)
        .or_else(|| o.get(legacy))
        .and_then(Value::as_object)
}

/// Style flags from JSON bools, or SNBT `1b`/`0b` which parse as numbers.
pub fn style_flag(v: Option<&Value>) -> Option<bool> {
    match v {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0),
        _ => None,
    }
}

/// ExtraCodecs.UNTRUSTED_URI / Util.parseAndValidateUntrustedUri: http and
/// https only, scheme case-insensitive. Every `open_url` a server sends must
/// pass it: anything else (`file:`, a UNC path, a custom protocol handler)
/// could start a program through the system's URL handler.
pub fn untrusted_http_url(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

/// `ClickEvent.CODEC` plus the legacy `value` field TextComponentHoverAndClickEventFix
/// renames (`url` / `command` / `page`).
pub fn parse_click_event(
    e: &serde_json::Map<String, Value>,
) -> Result<Option<crate::text::ClickEvent>, horizon_core::json_nbt::JsonNbtError> {
    parse_click_event_origin(e, Origin::Remote)
}
fn parse_click_event_origin(
    e: &serde_json::Map<String, Value>,
    origin: Origin,
) -> Result<Option<crate::text::ClickEvent>, horizon_core::json_nbt::JsonNbtError> {
    use crate::text::ClickEvent;
    use horizon_core::json_nbt::json_to_tag;
    let text = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
    let legacy = || text("value");
    let event = match e.get("action").and_then(Value::as_str) {
        Some("open_file") if origin == Origin::Local => text("path").map(ClickEvent::OpenFile),
        Some("open_url") => text("url")
            .or_else(legacy)
            .filter(|u| origin == Origin::Local || untrusted_http_url(u))
            .map(ClickEvent::OpenUrl),
        Some("change_page") => e
            .get("page")
            .and_then(Value::as_i64)
            .or_else(|| e.get("value").and_then(Value::as_i64))
            .or_else(|| {
                e.get("value")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
            })
            .and_then(|p| i32::try_from(p).ok())
            .filter(|p| origin == Origin::Local || *p >= 1)
            .map(ClickEvent::ChangePage),
        Some("suggest_command") => text("command")
            .or_else(legacy)
            .map(ClickEvent::SuggestCommand),
        Some("run_command") => text("command").or_else(legacy).map(ClickEvent::RunCommand),
        Some("copy_to_clipboard") => text("value").map(ClickEvent::CopyToClipboard),
        Some("show_dialog") => e
            .get("dialog")
            .map(json_to_tag)
            .transpose()?
            .map(|tag| ClickEvent::ShowDialog(tag.into())),
        Some("custom") => {
            let Some(id) = e
                .get("id")
                .and_then(Value::as_str)
                .and_then(crate::identifier::parse_identifier)
            else {
                return Ok(None);
            };
            let payload = e
                .get("payload")
                .filter(|v| !v.is_null())
                .map(json_to_tag)
                .transpose()?
                .map(Into::into);
            Some(ClickEvent::Custom { id, payload })
        }
        _ => None,
    };
    Ok(event)
}

// Hover entity names can themselves contain components. Clone their retained
// JSON with a work list rather than Value's recursive Clone implementation.
pub(crate) fn clone_object(object: &serde_json::Map<String, Value>) -> Value {
    clone_json(Work::BorrowedObject(object))
}
pub(crate) fn clone_value(value: &Value) -> Value {
    clone_json(Work::Value(value))
}
/// Release an owned component without recursive JSON container destruction.
pub fn discard_component(value: Value) {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(values) => pending.extend(values),
            Value::Object(values) => pending.extend(values.into_values()),
            _ => {}
        }
    }
}
enum Work<'a> {
    Value(&'a Value),
    BorrowedObject(&'a serde_json::Map<String, Value>),
    Array(usize),
    Object(Vec<String>),
}
fn clone_json(root: Work<'_>) -> Value {
    let mut pending = vec![root];
    let mut done = Vec::new();
    while let Some(work) = pending.pop() {
        match work {
            Work::Value(Value::Array(a)) => {
                pending.push(Work::Array(a.len()));
                pending.extend(a.iter().rev().map(Work::Value));
            }
            Work::Value(Value::Object(o)) | Work::BorrowedObject(o) => {
                pending.push(Work::Object(o.keys().cloned().collect()));
                pending.extend(o.values().rev().map(Work::Value));
            }
            Work::Value(v) => done.push(v.clone()),
            Work::Array(len) => {
                let values = done.split_off(done.len() - len);
                done.push(Value::Array(values));
            }
            Work::Object(keys) => {
                let values = done.split_off(done.len() - keys.len());
                done.push(Value::Object(keys.into_iter().zip(values).collect()));
            }
        }
    }
    done.pop().unwrap_or(Value::Null)
}

/// The style fields of a component object.
fn object_style<'a>(
    o: &'a serde_json::Map<String, Value>,
    child: &mut dyn FnMut(&'a Value) -> Text,
    origin: Origin,
) -> Style {
    let mut style = Style::default();
    if let Some(c) = o.get("color").and_then(Value::as_str) {
        style.color = named_color(c).or_else(|| hex_color(c));
    }
    style.shadow_color = o.get("shadow_color").and_then(shadow_color);
    style.bold = style_flag(o.get("bold"));
    style.italic = style_flag(o.get("italic"));
    style.underlined = style_flag(o.get("underlined"));
    style.strikethrough = style_flag(o.get("strikethrough"));
    style.obfuscated = style_flag(o.get("obfuscated"));
    style.font = o.get("font").and_then(crate::text::font_id);
    // ClickEvent.CODEC: the actions a chat line acts on here. A component
    // from a server never opens a file.
    if let Some(e) = style_event_object(o, "click_event", "clickEvent") {
        style.click = parse_click_event_origin(e, origin).ok().flatten();
    }
    // HoverEvent.ShowText, ShowItem and ShowEntity.
    if let Some(e) = style_event_object(o, "hover_event", "hoverEvent") {
        match e.get("action").and_then(Value::as_str) {
            Some("show_text") => {
                // Modern `value`, or the older `contents` / nested `text`.
                let value = e
                    .get("value")
                    .or_else(|| e.get("contents"))
                    .or_else(|| e.get("text"));
                if let Some(value) = value {
                    style.hover = Some(std::sync::Arc::new(child(value)));
                }
            }
            Some("show_item") if e.get("id").is_some_and(Value::is_string) => {
                style.hover_item = Some(std::sync::Arc::new(clone_object(e)));
            }
            Some("show_entity") if e.get("id").is_some_and(Value::is_string) => {
                style.hover_entity = Some(std::sync::Arc::new(clone_object(e)));
            }
            _ => {}
        }
    }
    style.insertion = o
        .get("insertion")
        .and_then(Value::as_str)
        .map(str::to_owned);
    style
}

/// `TextColor.parseColor` for `#...`: `Integer.parseInt(rest, 16)` (so a sign
/// and any digit count are accepted), kept only within `0..=0xFFFFFF`.
pub(crate) fn hex_color(c: &str) -> Option<u32> {
    let value = i64::from_str_radix(c.strip_prefix('#')?, 16).ok()?;
    (0..=0xFF_FFFF).contains(&value).then_some(value as u32)
}

/// Whether `c` is a style `color` (`TextColor.parseColor`): a named colour or `#` and hex.
pub fn is_color(c: &str) -> bool {
    named_color(c).is_some() || hex_color(c).is_some()
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
/// `ComponentSerialization.CODEC` decoding into text. `selector` components
/// are resolved by `resolve` (ComponentUtils.resolve).
pub fn to_text(v: &Value, resolve: &mut dyn FnMut(&str) -> Text) -> Text {
    to_text_with(v, resolve, &mut |_| None)
}
/// `to_text` with a hook resolving other contents (`nbt`) from the object.
pub fn to_text_with(
    v: &Value,
    resolve: &mut dyn FnMut(&str) -> Text,
    contents: &mut dyn FnMut(&serde_json::Map<String, Value>) -> Option<Text>,
) -> Text {
    build(v, resolve, contents, Sourced(true, Origin::Remote), &|_| {
        Origin::Remote
    })
}

/// The spans of `v` alone, as [`to_text`] decodes them, with `v` itself as the text's
/// component: what a text received whole is (`From<Value> for Text`). It skips building a
/// component for each part, which the whole one replaces.
pub(crate) fn spans_of(v: Value) -> Text {
    let mut text = render_source(&v, &|_| Origin::Remote);
    text.retain_source(v);
    text
}

pub(crate) fn render_source(v: &Value, origin: &dyn Fn(&Value) -> Origin) -> Text {
    build(
        v,
        &mut |s| Text::literal(s),
        &mut |_| None,
        Sourced(false, Origin::Remote),
        origin,
    )
}

/// Whether [`build`] keeps each part's component, or only its spans.
#[derive(Clone, Copy)]
struct Sourced(bool, Origin);

impl Sourced {
    fn literal(self, text: String) -> Text {
        match self.0 {
            true => {
                let mut out = Text::literal(text);
                for span in &mut out.rendered.spans {
                    span.origin = self.1;
                }
                // Raw component builders never confer local origin.
                if self.1 == Origin::Remote {
                    out.clear_origins();
                }
                out
            }
            false => {
                let mut out = Text::empty();
                if !text.is_empty() {
                    out.rendered.spans.push(crate::text::Span {
                        origin: self.1,
                        text,
                        style: Style::default(),
                    });
                }
                out
            }
        }
    }
    /// MutableComponent.append.
    fn append(self, base: &mut Text, other: Text) {
        match self.0 {
            true => base.push(other),
            false => {
                let mut other = other;
                base.rendered.spans.append(&mut other.rendered.spans);
                crate::text::discard_text(other);
            }
        }
    }
    fn styled(self, mut text: Text, style: &Style) -> Text {
        match self.0 {
            true => {
                // The builder already resolved the node's own style before its parent's.
                text.inherit_built_style(style);
                text
            }
            false => {
                for span in &mut text.rendered.spans {
                    span.inherit_style(style);
                }
                text
            }
        }
    }
    fn translate(self, key: &str, fallback: Option<&str>, args: &[Text]) -> Text {
        let mut text = match (self.0, fallback) {
            (true, Some(fallback)) => Text::translate_with_fallback(key, fallback, args),
            (true, None) => Text::translate(key, args),
            (false, fallback) => {
                crate::text::format_translatable_origin(key, fallback, args, self.1)
            }
        };
        if self.0 && self.1 == Origin::Remote {
            text.clear_origins();
            for span in &mut text.rendered.spans {
                span.origin = Origin::Remote;
            }
        }
        text
    }
}

// Continuations preserve lazy content selection and callback order while keeping
// every component relation off the call stack. Hover decoding has its own policy.
fn build(
    v: &Value,
    resolve: &mut dyn FnMut(&str) -> Text,
    contents: &mut dyn FnMut(&serde_json::Map<String, Value>) -> Option<Text>,
    sourced: Sourced,
    origin: &dyn Fn(&Value) -> Origin,
) -> Text {
    use std::collections::HashMap;
    enum Work<'a> {
        Enter(&'a Value, bool),
        Styled(&'a Value, bool),
        Content(&'a Value, bool, Style, Option<Text>),
        Finish(&'a Value, bool, Style, Text),
    }
    let mut pending = vec![Work::Enter(v, false)];
    let mut done = HashMap::<*const Value, (Text, Style)>::new();
    while let Some(work) = pending.pop() {
        match work {
            Work::Enter(value, hover) => match value {
                Value::Object(o) => {
                    pending.push(Work::Styled(value, hover));
                    if let Some(e) = style_event_object(o, "hover_event", "hoverEvent")
                        && e.get("action").and_then(Value::as_str) == Some("show_text")
                        && let Some(v) = e
                            .get("value")
                            .or_else(|| e.get("contents"))
                            .or_else(|| e.get("text"))
                    {
                        pending.push(Work::Enter(v, true));
                    }
                }
                Value::Array(a) => {
                    pending.push(Work::Content(value, hover, Style::default(), None));
                    pending.extend(a.iter().rev().map(|v| Work::Enter(v, hover)));
                }
                _ => pending.push(Work::Content(value, hover, Style::default(), None)),
            },
            Work::Styled(value, hover) => {
                let Value::Object(o) = value else {
                    unreachable!()
                };
                let style = object_style(
                    o,
                    &mut |v| {
                        done.remove(&(v as *const Value))
                            .map(take_built_text)
                            .unwrap_or_default()
                    },
                    origin(value),
                );
                let overridden = if hover { None } else { contents(o) };
                let needs_content = overridden.is_none();
                pending.push(Work::Content(value, hover, style, overridden));
                if needs_content {
                    // inline_object evaluates its fallback before selecting the content.
                    // Arguments are evaluated only for the translation branch.
                    if inline_object_fields(o).is_none()
                        && !o.contains_key("text")
                        && o.get("translate").is_some_and(Value::is_string)
                        && let Some(Value::Array(a)) = o.get("with")
                    {
                        pending.extend(a.iter().rev().map(|v| Work::Enter(v, hover)));
                    }
                    if let Some(v) = o.get("fallback") {
                        pending.push(Work::Enter(v, hover));
                    }
                }
            }
            Work::Content(value, hover, mut style, overridden) => {
                let mode = Sourced(sourced.0 || hover, origin(value));
                if let Value::Array(a) = value
                    && let Some(first) = a.first()
                    && let Some((text, own_style)) = done.get(&(first as *const Value))
                {
                    style = if first.is_object() {
                        own_style.clone()
                    } else {
                        text.rendered
                            .spans
                            .first()
                            .map(|s| s.style.clone())
                            .unwrap_or_default()
                    };
                }
                let mut literal = |s: &str| Text::literal(s);
                let resolver: &mut dyn FnMut(&str) -> Text =
                    if hover { &mut literal } else { resolve };
                let text = build_node(value, resolver, overridden, mode, &style, &mut |v| {
                    done.remove(&(v as *const Value))
                        .map(take_built_text)
                        .unwrap_or_default()
                });
                if let Value::Object(o) = value {
                    pending.push(Work::Finish(value, hover, style, text));
                    if let Some(Value::Array(a)) = o.get("extra") {
                        pending.extend(a.iter().rev().map(|v| Work::Enter(v, hover)));
                    }
                } else {
                    done.insert(value as *const Value, (text, style));
                }
            }
            Work::Finish(value, hover, style, mut text) => {
                let Value::Object(o) = value else {
                    unreachable!()
                };
                let mode = Sourced(sourced.0 || hover, origin(value));
                if let Some(Value::Array(a)) = o.get("extra") {
                    for v in a {
                        let extra = done
                            .remove(&(v as *const Value))
                            .map(take_built_text)
                            .unwrap_or_default();
                        mode.append(&mut text, mode.styled(extra, &style));
                    }
                }
                if mode.0
                    && (o.contains_key("object")
                        || o.contains_key("player")
                        || o.contains_key("sprite"))
                {
                    text.retain_annotated_source(value, origin);
                }
                done.insert(value as *const Value, (text, style));
            }
        }
    }
    done.remove(&(v as *const Value))
        .map(take_built_text)
        .unwrap_or_default()
}

fn take_built_text((text, style): (Text, Style)) -> Text {
    crate::text::discard_style(style);
    text
}

fn build_node<'a>(
    v: &'a Value,
    resolve: &mut dyn FnMut(&str) -> Text,
    overridden: Option<Text>,
    sourced: Sourced,
    style: &Style,
    child: &mut dyn FnMut(&'a Value) -> Text,
) -> Text {
    match v {
        Value::String(s) => sourced.literal(s.clone()),
        Value::Number(n) => sourced.literal(n.to_string()),
        Value::Bool(b) => sourced.literal(b.to_string()),
        // A list is its first element with the rest as siblings.
        Value::Array(list) => {
            let Some((first, rest)) = list.split_first() else {
                return Text::empty();
            };
            let mut base = child(first);
            // The siblings take the first component's own style, even when
            // its text is empty.
            for sibling in rest {
                let sibling = child(sibling);
                sourced.append(&mut base, sourced.styled(sibling, style));
            }
            base
        }
        Value::Object(o) => {
            let content = if let Some(t) = overridden {
                Text::from(t.component())
            } else if let Some(object) =
                inline_object_with_fallback(o, o.get("fallback").map(|v| plain_component(child(v))))
            {
                let mut text = sourced.literal("\u{fffc}".into());
                text.rendered.spans[0].style.inline_object = Some(object);
                text
            } else if let Some(t) = o.get("text") {
                match t {
                    Value::String(s) => sourced.literal(s.clone()),
                    other => sourced.literal(other.to_string()),
                }
            } else if let Some(key) = o.get("translate").and_then(Value::as_str) {
                let args: Vec<Text> = o
                    .get("with")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(child).collect())
                    .unwrap_or_default();
                let text = sourced.translate(key, o.get("fallback").and_then(Value::as_str), &args);
                for arg in args {
                    crate::text::discard_text(arg);
                }
                text
            } else if let Some(sel) = o.get("selector").and_then(Value::as_str) {
                Text::from(resolve(sel).component())
            } else if let Some(key) = o.get("keybind").and_then(Value::as_str) {
                Text::from(crate::key_display::display_name(key).component())
            } else {
                Text::empty()
            };
            sourced.styled(content, style)
        }
        Value::Null => Text::empty(),
    }
}

fn plain_component(text: Text) -> String {
    let plain = text.plain();
    crate::text::discard_text(text);
    plain
}

/// Decode inline object fields, including the engine's profile and atlas defaults.
pub fn inline_object(
    object: &serde_json::Map<String, Value>,
    resolve: &mut dyn FnMut(&str) -> Text,
    contents: &mut dyn FnMut(&serde_json::Map<String, Value>) -> Option<Text>,
) -> Option<InlineObject> {
    let fallback = object
        .get("fallback")
        .map(|value| plain_component(to_text_with(value, resolve, contents)));
    inline_object_with_fallback(object, fallback)
}

/// Decode object identity/defaults without rendering its recursive fallback.
/// Structured component capture retains the fallback as a component instead.
pub enum InlineObjectFields<'a> {
    Player { profile: &'a Value, hat: bool },
    Atlas { atlas: &'a str, sprite: &'a str },
}
pub fn inline_object_fields(
    object: &serde_json::Map<String, Value>,
) -> Option<InlineObjectFields<'_>> {
    match object.get("object").and_then(Value::as_str) {
        Some("player") | None if object.contains_key("player") => {
            Some(InlineObjectFields::Player {
                profile: object.get("player")?,
                hat: object.get("hat").and_then(Value::as_bool).unwrap_or(true),
            })
        }
        Some("atlas") | None if object.contains_key("sprite") => Some(InlineObjectFields::Atlas {
            atlas: object
                .get("atlas")
                .and_then(Value::as_str)
                .unwrap_or("minecraft:blocks"),
            sprite: object.get("sprite")?.as_str()?,
        }),
        _ => None,
    }
}

fn inline_object_with_fallback(
    object: &serde_json::Map<String, Value>,
    fallback: Option<String>,
) -> Option<InlineObject> {
    Some(match inline_object_fields(object)? {
        InlineObjectFields::Player { profile, hat } => {
            let fallback = fallback.unwrap_or_else(|| {
                match profile
                    .as_str()
                    .or_else(|| profile.get("name").and_then(Value::as_str))
                {
                    Some(name) => format!("[{name} head]"),
                    None => "[unknown player head]".to_owned(),
                }
            });
            InlineObject::Player {
                profile: clone_value(profile).into(),
                hat,
                fallback,
            }
        }
        InlineObjectFields::Atlas { atlas, sprite } => {
            let fallback = fallback.unwrap_or_else(|| {
                let sprite_name = sprite.strip_prefix("minecraft:").unwrap_or(sprite);
                let atlas_name = atlas.strip_prefix("minecraft:").unwrap_or(atlas);
                if atlas_name == "blocks" {
                    format!("[{sprite_name}]")
                } else {
                    format!("[{sprite_name}@{atlas_name}]")
                }
            });
            InlineObject::Atlas {
                atlas: atlas.into(),
                sprite: sprite.into(),
                fallback,
            }
        }
    })
}

/// `CustomName` as text: a string keeps its legacy formatting codes, anything
/// else is a component (`ComponentSerialization`).
pub fn name_text(tag: &Tag) -> Text {
    match tag {
        Tag::String(s) => Text::untrusted_legacy(&s.to_string_lossy()),
        t => crate::scoreboard::comp_text(&crate::scoreboard::tag_to_json(t)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn mixed_recursive_temporaries_are_released_on_256_kib() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                for hover_text in [false, true] {
                    for argument in [false, true] {
                        let mut child = serde_json::json!({"text":"leaf"});
                        for _ in 1..511 {
                            let mut event = serde_json::Map::new();
                            event.insert(
                                "action".into(),
                                if hover_text {
                                    "show_text"
                                } else {
                                    "show_entity"
                                }
                                .into(),
                            );
                            event.insert("id".into(), "minecraft:pig".into());
                            event.insert("uuid".into(), serde_json::json!([0, 0, 0, 1]));
                            event.insert(if hover_text { "value" } else { "name" }.into(), child);
                            let mut fields = serde_json::Map::new();
                            fields.insert("text".into(), "parent".into());
                            fields.insert("hover_event".into(), Value::Object(event));
                            child = Value::Object(fields);
                        }
                        let mut fields = serde_json::Map::new();
                        if argument {
                            fields.insert("translate".into(), "test:missing".into());
                            fields.insert("fallback".into(), "unused".into());
                            fields.insert("with".into(), Value::Array(vec![child]));
                        } else {
                            fields.insert("object".into(), "atlas".into());
                            fields.insert("sprite".into(), "test:icon".into());
                            fields.insert("fallback".into(), child);
                        }
                        let output = spans_of(Value::Object(fields));
                        assert_eq!(output.plain(), if argument { "unused" } else { "parent" });
                        std::mem::forget(output);
                    }
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn opaque_profile_and_click_payloads_build_on_256_kib() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let nested =
                    |depth| (0..depth).fold(serde_json::json!(1), |v, _| Value::Array(vec![v]));
                let profile = Value::Object(
                    [
                        ("name".into(), "Test".into()),
                        ("future".into(), nested(511)),
                    ]
                    .into_iter()
                    .collect(),
                );
                let mut tree = serde_json::json!({"object": "player"});
                tree["player"] = profile;
                for _ in 1..512 {
                    let mut parent = serde_json::json!({"translate": "test.opaque"});
                    parent["with"] = Value::Array(vec![tree]);
                    tree = parent;
                }
                let mut click = serde_json::json!({"action": "custom", "id": "test:click"});
                click["payload"] = nested(512);
                tree["click_event"] = click;
                let language =
                    crate::lang::Language::of("en_us", [("test.opaque".into(), "%s".into())]);
                crate::lang::scoped(language, || {
                    let text = Text::from(tree);
                    assert_eq!(text.plain(), "[Test head]");
                    assert!(matches!(
                        text.rendered.spans[0].style.inline_object,
                        Some(InlineObject::Player { .. })
                    ));
                    assert!(matches!(
                        text.rendered.spans[0].style.click,
                        Some(crate::text::ClickEvent::Custom { .. })
                    ));
                    crate::text::discard_text(text);
                });
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn iterative_builder_preserves_lazy_content_and_callback_order() {
        let value = serde_json::json!({"id":"parent", "text":"visible", "with":[{"id":"unused", "selector":"unused"}], "extra":[{"id":"extra", "selector":"@a"}]});
        let calls = std::cell::RefCell::new(Vec::new());
        let text = to_text_with(
            &value,
            &mut |selector| {
                calls.borrow_mut().push(selector.to_owned());
                Text::literal("resolved")
            },
            &mut |o| {
                calls
                    .borrow_mut()
                    .push(o["id"].as_str().unwrap().to_owned());
                None
            },
        );
        assert_eq!(text.plain(), "visibleresolved");
        assert_eq!(*calls.borrow(), ["parent", "extra", "@a"]);
        let value = serde_json::json!({"id":"override", "translate":"missing", "with":[{"selector":"unused"}], "fallback":{"selector":"unused"}});
        let text = to_text_with(&value, &mut |_| panic!("unused branch"), &mut |o| {
            assert_eq!(o.get("id").and_then(Value::as_str), Some("override"));
            Some(Text::literal("override"))
        });
        assert_eq!(text.plain(), "override");
    }

    #[test]
    fn iterative_builder_isolates_hover_resolution_and_empty_array_style() {
        let value = serde_json::json!({"text":"visible", "hover_event":{"action":"show_text", "value":{"selector":"@p"}}});
        let text = to_text_with(
            &value,
            &mut |_| panic!("hover resolver escaped"),
            &mut |o| {
                assert!(o.contains_key("text"));
                None
            },
        );
        assert_eq!(
            text.rendered.spans[0].style.hover.as_ref().unwrap().plain(),
            "@p"
        );
        let value = serde_json::json!({"text":"visible", "hover_event":{"action":"show_text", "value":{"translate":"test:hover"}}});
        let text = to_text(&value, &mut |_| panic!("hover resolver escaped"));
        assert_eq!(
            text.rendered.spans[0]
                .style
                .hover
                .as_ref()
                .unwrap()
                .component()["translate"],
            "test:hover"
        );
        let text = Text::from(serde_json::json!([[{"text":"", "color":"red"}], "tail"]));
        assert_eq!(text.plain(), "tail");
        assert_eq!(text.rendered.spans[0].style.color, None);
        let text = Text::from(serde_json::json!([{"text":"", "color":"red"}, "tail"]));
        assert_eq!(text.rendered.spans[0].style.color, Some(crate::text::RED));
    }

    #[test]
    fn translation_fallback_is_a_literal_template_not_a_second_key() {
        let language = crate::lang::Language::of(
            "en_us",
            [
                ("test.fallback".into(), "Wrong translation".into()),
                ("test.same".into(), "test.same".into()),
            ],
        );
        crate::lang::scoped(language, || {
            for (key, fallback, expected) in [
                ("test.missing", "test.fallback", "test.fallback"),
                ("test.same", "Wrong fallback", "test.same"),
            ] {
                let value = serde_json::json!({"translate":key, "fallback":fallback});
                assert_eq!(Text::from(value.clone()).plain(), expected);
                assert_eq!(to_text(&value, &mut |s| Text::literal(s)).plain(), expected);
            }
            let value = serde_json::json!({"translate":"test.missing", "fallback":"value: %s", "with":[{"text":"argument", "bold":true}]});
            let text = Text::from(value);
            assert_eq!(text.plain(), "value: argument");
            assert_eq!(text.rendered.spans.last().unwrap().style.bold, Some(true));
        });
    }

    #[test]
    fn nested_fallback_references_stay_bounded() {
        // 300 references to the argument, nested three deep: 27 million copies of "A" from a
        // 4 KB component, which ran a client out of memory.
        let bomb = |depth: usize, text: &str| {
            let pattern = "%1$s".repeat(300);
            let mut v = serde_json::json!({ "text": text });
            for _ in 0..depth {
                v = serde_json::json!({"translate": "x.unknown", "fallback": pattern, "with": [v]});
            }
            v
        };
        let started = std::time::Instant::now();
        let text = to_text(&bomb(3, "A"), &mut |s| Text::literal(s));
        assert!(
            text.rendered.spans.len() < 1 << 15,
            "{} spans",
            text.rendered.spans.len()
        );
        // One long argument referenced many times multiplies bytes, not spans.
        let long = "B".repeat(64 << 10);
        let text = to_text(&bomb(1, &long), &mut |s| Text::literal(s));
        let bytes: usize = text.rendered.spans.iter().map(|s| s.text.len()).sum();
        assert!(bytes <= 4 << 20, "{bytes} bytes");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // A real fallback still formats.
        let ok = serde_json::json!({
            "translate": "x.unknown", "fallback": "%1$s and %1$s", "with": [{"text": "A"}]
        });
        assert_eq!(to_text(&ok, &mut |s| Text::literal(s)).plain(), "A and A");
    }
    use crate::lang::{self, Language, ResourceSource};

    struct Mem(std::collections::HashMap<String, Vec<u8>>);
    impl ResourceSource for Mem {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            self.0.get(path).cloned()
        }
    }

    /// TranslatableContents.decompose: Language.getOrDefault(key, fallback). A missing key
    /// shows the fallback as written, even when the fallback is another key the language has.
    #[test]
    fn a_translatable_fallback_stays_literal() {
        let pack = Mem([(
            "assets/minecraft/lang/en_us.json".to_owned(),
            br#"{"real.key": "Translated", "greet": "Hello %s"}"#.to_vec(),
        )]
        .into());
        let ((), _) = lang::scoped(Language::load(&[&pack], "en_us"), || {
            let parse = |v: Value| to_text(&v, &mut |_| Text::empty()).plain();
            let missing = serde_json::json!({"translate": "missing.key", "fallback": "real.key"});
            assert_eq!(parse(missing), "real.key");
            // The fallback is a template: its arguments are filled in.
            let args = serde_json::json!({"translate": "missing.key", "fallback": "Hi %s", "with": ["Steve"]});
            assert_eq!(parse(args), "Hi Steve");
            // A key the language has wins over its fallback.
            let found =
                serde_json::json!({"translate": "greet", "fallback": "real.key", "with": ["Alex"]});
            assert_eq!(parse(found), "Hello Alex");
            // A component built with the constructor keeps its meaning through its JSON.
            let built = Text::translate_with_fallback("missing.key", "real.key", &[]);
            let again = to_text(&built.component(), &mut |_| Text::empty());
            assert_eq!(again.plain(), built.plain());
            assert_eq!(again.component(), built.component());
        });
    }

    /// SnbtGrammar: a compound or list may end with one comma; two in a row
    /// stay an error.
    #[test]
    fn a_component_takes_one_trailing_comma() {
        let parse = |s: &str| parse_snbt(s).map(|(v, _)| v);
        assert_eq!(
            parse("{text:\"a\",color:\"red\",}"),
            parse("{text:\"a\",color:\"red\"}")
        );
        assert_eq!(
            parse("[\"a\", {text:\"b\",} ,]"),
            parse("[\"a\",{text:\"b\"}]")
        );
        assert!(parse("{text:\"a\",,}").is_err());
        assert!(parse("[\"a\",,]").is_err());
    }

    #[test]
    fn a_text_received_whole_has_the_spans_its_parts_would() {
        let _guard = crate::key_display::TEST_BINDINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        use serde_json::json;
        for v in [
            json!(""),
            json!(7),
            json!(["a", {"text": "b", "color": "red"}, "c"]),
            json!({"text": "hello", "color": "red", "bold": true}),
            json!({"translate": "chat.type.text", "with": [{"text": "Steve", "color": "yellow"}, "hi"]}),
            json!({"translate": "no.such.key", "fallback": "Fell %s", "with": ["back"]}),
            json!({"text": "", "italic": true, "extra": ["a", {"text": "b", "underlined": true, "extra": ["d", 2]}]}),
            json!({"keybind": "key.jump", "color": "blue"}),
            json!({"selector": "@p"}),
            json!({"text": "x", "hover_event": {"action": "show_text", "value": "tip"}, "click_event": {"action": "open_url", "url": "https://example.com"}}),
        ] {
            let whole = spans_of(v.clone());
            let parts = to_text(&v, &mut |s| Text::literal(s));
            assert_eq!(whole.rendered.spans, parts.rendered.spans, "{v}");
            assert_eq!(whole.component(), v, "{v}");
        }
    }

    #[test]
    fn keybind_components_decode_assigned_key_and_reflect_rebinding() {
        let _guard = crate::key_display::TEST_BINDINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::key_display::replace_action_bindings([("key.jump", "key.keyboard.space")]);
        let ((), _) = crate::lang::scoped(
            crate::lang::Language::of(
                "en_us",
                [
                    ("key.jump".into(), "Jump".into()),
                    ("key.keyboard.space".into(), "Space".into()),
                    ("key.keyboard.up".into(), "Up Arrow".into()),
                    ("key.keyboard.unknown".into(), "None".into()),
                ],
            ),
            || {
                let v = serde_json::json!({"keybind": "key.jump"});
                assert_eq!(to_text(&v, &mut |_| Text::empty()).plain(), "Space");

                crate::key_display::set_action_binding("key.jump", "key.keyboard.up");
                assert_eq!(to_text(&v, &mut |_| Text::empty()).plain(), "Up Arrow");

                crate::key_display::set_action_binding("key.jump", "key.keyboard.unknown");
                assert_eq!(to_text(&v, &mut |_| Text::empty()).plain(), "None");

                crate::key_display::reset_action_bindings();
                assert_eq!(to_text(&v, &mut |_| Text::empty()).plain(), "Jump");
            },
        );
    }

    #[test]
    fn hex_color_follows_text_color_parse_color() {
        for bad in [
            "#1000000",
            "#FFFFFFFF",
            "#-FF",
            "#",
            "#zz",
            "FFFFFF",
            "#99999999999",
        ] {
            assert_eq!(hex_color(bad), None, "{bad}");
        }
        for (good, want) in [
            ("#FFFFFF", 0xFFFFFF),
            ("#000000", 0),
            ("#F", 15),
            ("#+FF", 255),
            ("#-0", 0),
        ] {
            assert_eq!(hex_color(good), Some(want), "{good}");
        }
        let out = serde_json::json!({"text": "A", "color": "#1000000"});
        let text = to_text(&out, &mut |_| Text::empty());
        assert_eq!(text.rendered.spans[0].style.color, None);
    }

    #[test]
    fn shadow_color_accepts_packed_and_rgba_styles() {
        let packed = serde_json::json!({"text": "A", "shadow_color": -16711936});
        let text = to_text(&packed, &mut |_| Text::empty());
        assert_eq!(text.rendered.spans[0].style.shadow_color, Some(0xff00ff00));
        assert_eq!(text.component()["shadow_color"], -16711936);
        let rgba = serde_json::json!({"text": "A", "shadow_color": [1, 0, 0.5, 1]});
        let text = to_text(&rgba, &mut |_| Text::empty());
        assert_eq!(text.rendered.spans[0].style.shadow_color, Some(0xffff007f));
        let translucent = serde_json::json!({"text": "A", "shadow_color": [1, 0, 0.5, 0.5]});
        let text = to_text(&translucent, &mut |_| Text::empty());
        assert_eq!(text.rendered.spans[0].style.shadow_color, Some(0x7fff007f));
        let no_shadow = serde_json::json!({"text": "A", "shadow_color": 0});
        let text = to_text(&no_shadow, &mut |_| Text::empty());
        assert_eq!(text.rendered.spans[0].style.shadow_color, Some(0));
    }

    #[test]
    fn player_object_survives_text_parsing_and_keeps_its_siblings() {
        let component = serde_json::json!({
            "text": "<",
            "extra": [
                {"object": "player", "player": {"name": "elyapluggin"}, "hat": false,
                 "fallback": "[head]"},
                {"text": " >", "color": "gold"}
            ]
        });
        let text = to_text(&component, &mut |_| Text::empty());
        assert_eq!(text.plain(), "<[head] >");
        assert!(matches!(
            &text.rendered.spans[1].style.inline_object,
            Some(InlineObject::Player { hat: false, .. })
        ));
        assert!(text.rendered.spans[2].style.inline_object.is_none());
        assert_eq!(text.component(), component);
        let sprite = serde_json::json!({"object": "atlas", "sprite": "minecraft:item/porkchop"});
        let text = to_text(&sprite, &mut |_| Text::empty());
        assert!(matches!(
            &text.rendered.spans[0].style.inline_object,
            Some(InlineObject::Atlas { sprite, .. }) if sprite == "minecraft:item/porkchop"
        ));
        // ObjectContents.visit without a fallback: the sprite's defaultFallback.
        assert_eq!(text.plain(), "[item/porkchop]");
        let head = serde_json::json!({"object": "player", "player": {"name": "Steve"}});
        assert_eq!(
            to_text(&head, &mut |_| Text::empty()).plain(),
            "[Steve head]"
        );
        let gui = serde_json::json!({"atlas": "gui", "sprite": "hud/heart/full"});
        assert_eq!(
            to_text(&gui, &mut |_| Text::empty()).plain(),
            "[hud/heart/full@gui]"
        );
    }

    #[test]
    fn snbt_and_json_components() {
        let input = "{text:'Hi ',color:red,extra:[{text:\"there\",bold:1b}]} rest";
        let (v, n) = parse_snbt(input).unwrap();
        assert_eq!(&input[n..], " rest");
        let t = to_text(&v, &mut |_| Text::empty());
        assert_eq!(t.plain(), "Hi there");
        assert_eq!(t.rendered.spans[1].style.color, named_color("red"));
        // SNBT `1b` is a number in JSON; style flags still take it as true.
        assert_eq!(t.rendered.spans[1].style.bold, Some(true));
        let (v, _) = parse_snbt("[\"a\",{\"text\":\"b\"}]").unwrap();
        assert_eq!(to_text(&v, &mut |_| Text::empty()).plain(), "ab");
    }

    #[test]
    fn legacy_click_event_open_url_uses_value_field() {
        use crate::text::ClickEvent;
        let modern = serde_json::json!({
            "text": "site",
            "click_event": {"action": "open_url", "url": "https://example.com"},
            "underlined": true
        });
        let legacy = serde_json::json!({
            "text": "site",
            "clickEvent": {"action": "open_url", "value": "https://example.com"},
            "underlined": 1
        });
        for component in [modern, legacy] {
            let text = to_text(&component, &mut |_| Text::empty());
            assert_eq!(
                text.rendered.spans[0].style.click,
                Some(ClickEvent::OpenUrl("https://example.com".into()))
            );
            assert_eq!(text.rendered.spans[0].style.underlined, Some(true));
        }
        let snbt = r#"{text:"site",underlined:1b,clickEvent:{action:open_url,value:"https://example.com"}}"#;
        let (v, _) = parse_snbt(snbt).unwrap();
        let text = to_text(&v, &mut |_| Text::empty());
        assert_eq!(
            text.rendered.spans[0].style.click,
            Some(ClickEvent::OpenUrl("https://example.com".into()))
        );
        assert_eq!(text.rendered.spans[0].style.underlined, Some(true));
    }

    #[test]
    fn custom_click_events_keep_payloads_and_inherit_to_inline_objects() {
        use crate::text::ClickEvent;
        use horizon_core::nbt::Tag;

        for (payload, expected) in [
            (None, None),
            (Some(serde_json::Value::Null), None),
            (Some(serde_json::json!(42)), Some(Tag::Byte(42))),
            (
                Some(serde_json::json!({"shop": "potions", "amount": 3})),
                Some(Tag::Compound(
                    [
                        ("amount".into(), Tag::Byte(3)),
                        ("shop".into(), Tag::String("potions".into())),
                    ]
                    .into_iter()
                    .collect(),
                )),
            ),
        ] {
            let mut event = serde_json::json!({"action": "custom", "id": "test:shop"});
            if let Some(payload) = payload {
                event["payload"] = payload;
            }
            let component = serde_json::json!({
                "text": "", "click_event": event,
                "extra": [{"object": "atlas", "sprite": "minecraft:item/potion"}, "Shop"]
            });
            let text = to_text(&component, &mut |_| Text::empty());
            for (_, style) in text.chars() {
                assert_eq!(
                    style.click,
                    Some(ClickEvent::Custom {
                        id: "test:shop".into(),
                        payload: expected.clone().map(Into::into),
                    })
                );
            }
            let encoded = text.component();
            let decoded = to_text(&encoded, &mut |_| Text::empty());
            assert_eq!(decoded, text);
            if expected.is_none() {
                assert!(!encoded.to_string().contains("\"payload\""));
            }
        }
        for id in ["Bad ID", "Uppercase:shop"] {
            let text = to_text(
                &serde_json::json!({
                    "text": "Shop", "click_event": {"action": "custom", "id": id}
                }),
                &mut |_| Text::empty(),
            );
            assert_eq!(text.rendered.spans[0].style.click, None);
        }
    }

    #[test]
    fn the_font_is_inherited_and_kept() {
        let (v, _) =
            parse_snbt(r#"{text:"a",font:"alt",extra:["b",{text:"c",font:"minecraft:uniform"}]}"#)
                .unwrap();
        let t = to_text(&v, &mut |_| Text::empty());
        let fonts: Vec<_> = t.chars().map(|(_, s)| s.font.clone()).collect();
        let (alt, uniform) = (
            Some("minecraft:alt".into()),
            Some("minecraft:uniform".into()),
        );
        assert_eq!(fonts, [alt.clone(), alt, uniform]);
        // Back to JSON with the field, which a re-read keeps.
        let again = to_text(&t.component(), &mut |_| Text::empty());
        let kept: Vec<_> = again.chars().map(|(_, s)| s.font.clone()).collect();
        assert_eq!(kept, fonts);
        // Not an identifier: no font.
        let (v, _) = parse_snbt(r#"{text:"a",font:"Bad Font"}"#).unwrap();
        let t = to_text(&v, &mut |_| Text::empty());
        assert_eq!(t.rendered.spans[0].style.font, None);
    }

    #[test]
    fn snbt_nesting_is_bounded() {
        let nest = |n: usize| format!("{}1{}", "[".repeat(n), "]".repeat(n));
        assert!(parse_snbt(&nest(512)).is_ok());
        assert!(parse_snbt(&nest(513)).is_err());
        assert!(parse_snbt(&nest(100_000)).is_err());
    }

    #[test]
    fn bare_words_that_look_numeric_stay_strings() {
        let (v, _) = parse_snbt("{text:nan}").unwrap();
        assert_eq!(v["text"], Value::String("nan".into()));
        let (v, _) = parse_snbt("{text:Infinity}").unwrap();
        assert_eq!(v["text"], Value::String("Infinity".into()));
        let (v, _) = parse_snbt("{n:1.5}").unwrap();
        assert_eq!(v["n"], Value::from(1.5));
    }

    #[test]
    fn name_text_preserves_legacy_and_decodes_components() {
        let legacy = Tag::String("§6Golden Name".into());
        let text = name_text(&legacy);
        assert_eq!(text.plain(), "Golden Name");

        let comp = Tag::compound([("text", Tag::String("Custom Name".into()))]);
        let text = name_text(&comp);
        assert_eq!(text.plain(), "Custom Name");
    }
}
