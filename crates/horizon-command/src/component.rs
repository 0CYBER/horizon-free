//! ComponentSerialization.CODEC and Style.Serializer.MAP_CODEC decoding of
//! SNBT text components, with the data-fixer-upper error messages
//! (ComponentArgument, StyleArgument and component-valued item data).
use super::codec::{get, no_key, not_list, not_map};
use horizon_world::nbt::{Compound, Tag};

/// A DataResult error: its message and whether it carries a partial result.
struct E {
    msg: String,
    partial: bool,
}
type R = Result<(), E>;
fn err(msg: impl Into<String>) -> R {
    Err(E {
        msg: msg.into(),
        partial: false,
    })
}
fn partial(msg: impl Into<String>) -> R {
    Err(E {
        msg: msg.into(),
        partial: true,
    })
}
/// Codec.either.
fn either(first: R, second: impl FnOnce() -> R) -> R {
    let a = match first {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    let b = match second() {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if a.partial {
        return Err(a);
    }
    if b.partial {
        return Err(b);
    }
    err(format!(
        "Failed to parse either. First: {}; Second: {}",
        a.msg, b.msg
    ))
}
fn is_string(t: &Tag) -> bool {
    matches!(t, Tag::String(_))
}
fn string(t: &Tag) -> R {
    if is_string(t) {
        Ok(())
    } else {
        err("Not a string")
    }
}
fn string_of(t: &Tag) -> Option<String> {
    match t {
        Tag::String(s) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// ListCodec: element errors (newest first) with the decoded rest as partial.
fn list(t: &Tag, element: &dyn Fn(&Tag) -> R) -> Result<usize, E> {
    let Tag::List(items) = t else {
        return Err(E {
            msg: not_list(t),
            partial: false,
        });
    };
    let mut errors: Vec<String> = Vec::new();
    let mut ok = 0;
    for i in items {
        match element(i) {
            Ok(()) => ok += 1,
            Err(e) => {
                if e.partial {
                    ok += 1;
                }
                errors.push(e.msg);
            }
        }
    }
    if errors.is_empty() {
        return Ok(ok);
    }
    errors.reverse();
    Err(E {
        msg: errors.join("; "),
        partial: true,
    })
}
/// ExtraCodecs.nonEmptyList(listOf(element)).
fn non_empty_list(t: &Tag, element: &dyn Fn(&Tag) -> R) -> R {
    match list(t, element) {
        Ok(0) => err("List must have contents"),
        Ok(_) => Ok(()),
        Err(e) if !e.partial => Err(e),
        Err(e) => {
            // The partial result is validated too.
            let decoded = match t {
                Tag::List(items) => items.iter().filter(|i| element(i).is_ok()).count(),
                _ => 0,
            };
            if decoded == 0 {
                err(format!("{}; List must have contents", e.msg))
            } else {
                Err(e)
            }
        }
    }
}
fn bool_value(t: &Tag) -> R {
    if super::codec::is_number(t) {
        Ok(())
    } else {
        err("Not a number")
    }
}
/// Identifier.CODEC.
pub fn identifier(t: &Tag) -> Result<(), String> {
    let Some(s) = string_of(t) else {
        return Err("Not a string".into());
    };
    let (ns, path) = match s.split_once(':') {
        Some(("", p)) => ("minecraft".to_owned(), p.to_owned()),
        Some((n, p)) => (n.to_owned(), p.to_owned()),
        None => ("minecraft".to_owned(), s.clone()),
    };
    let ns_ok = ns != ".."
        && ns
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-'));
    if !ns_ok {
        return Err(format!(
            "Not a valid resource location: {s} Non [a-z0-9_.-] character in namespace of location: {ns}:{path}"
        ));
    }
    let path_ok = path.chars().all(|c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | '-' | '/')
    });
    if !path_ok {
        return Err(format!(
            "Not a valid resource location: {s} Non [a-z0-9/._-] character in path of location: {ns}:{path}"
        ));
    }
    Ok(())
}
/// TextColor.CODEC.
fn color(t: &Tag) -> R {
    let Some(s) = string_of(t) else {
        return partial("Not a string");
    };
    if let Some(hex) = s.strip_prefix('#') {
        let digits = hex.strip_prefix(['+', '-']).unwrap_or(hex);
        let valid = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_hexdigit());
        return match i64::from_str_radix(hex, 16) {
            Ok(v) if valid && i32::try_from(v).is_ok() => {
                if (0..=0xFF_FFFF).contains(&v) {
                    Ok(())
                } else {
                    partial(format!("Color value out of range: {s}"))
                }
            }
            _ => partial(format!("Invalid color value: {s}")),
        };
    }
    if horizon_client_core::text::named_color(&s).is_some() {
        Ok(())
    } else {
        partial(format!("Invalid color name: {s}"))
    }
}
/// A KeyDispatch over an action enum (`Unknown element name:`), with
/// required fields per action.
fn action_event(t: &Tag, actions: &[(&str, &[&str])]) -> R {
    let Tag::Compound(map) = t else {
        return partial(not_map(t));
    };
    let Some(action) = get(map, "action") else {
        return partial(no_key("action", t));
    };
    let Some(name) = string_of(action) else {
        return partial("Not a string");
    };
    let Some((_, fields)) = actions.iter().find(|(a, _)| *a == name) else {
        return partial(format!("Unknown element name:{name}"));
    };
    for f in *fields {
        if get(map, f).is_none() {
            return partial(no_key(f, t));
        }
    }
    Ok(())
}
const CLICK_ACTIONS: &[(&str, &[&str])] = &[
    ("open_url", &["url"]),
    ("open_file", &["path"]),
    ("run_command", &["command"]),
    ("suggest_command", &["command"]),
    ("show_dialog", &["dialog"]),
    ("change_page", &["page"]),
    ("copy_to_clipboard", &["value"]),
    ("custom", &["id"]),
];
const HOVER_ACTIONS: &[(&str, &[&str])] = &[
    ("show_text", &["value"]),
    ("show_item", &["id"]),
    ("show_entity", &["id", "uuid"]),
];
/// Style.Serializer.MAP_CODEC (every field optional; errors keep a partial style).
fn style_fields(map: &Compound) -> R {
    if let Some(c) = get(map, "color") {
        color(c)?;
    }
    for key in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        if let Some(v) = get(map, key)
            && bool_value(v).is_err()
        {
            return partial("Not a number");
        }
    }
    if let Some(v) = get(map, "click_event") {
        action_event(v, CLICK_ACTIONS)?;
    }
    if let Some(v) = get(map, "hover_event") {
        action_event(v, HOVER_ACTIONS)?;
    }
    if let Some(v) = get(map, "insertion")
        && !is_string(v)
    {
        return partial("Not a string");
    }
    if let Some(v) = get(map, "font")
        && let Err(m) = identifier(v)
    {
        return partial(m);
    }
    Ok(())
}
fn selector_string(t: &Tag) -> bool {
    let Some(s) = string_of(t) else { return false };
    let reg = super::args::Registries {
        permission: 4,
        ..Default::default()
    };
    let mut r = super::Reader::new(&s);
    super::selector::parse(&mut r, &reg, true).is_ok() && !r.can_read()
}
fn optional_component(map: &Compound, key: &str) -> bool {
    get(map, key).is_none_or(|v| component(v).is_ok())
}
/// The content codecs (PlainText, Translatable, Keybind, Score, Selector, Nbt, Object).
fn content(kind: &str, map: &Compound) -> bool {
    match kind {
        "text" => get(map, "text").is_some_and(is_string),
        "translatable" => {
            get(map, "translate").is_some_and(is_string)
                && get(map, "fallback").is_none_or(is_string)
                && get(map, "with").is_none_or(|w| matches!(w, Tag::List(_)))
        }
        "keybind" => get(map, "keybind").is_some_and(is_string),
        "score" => match get(map, "score") {
            Some(Tag::Compound(s)) => {
                get(s, "name").is_some_and(is_string) && get(s, "objective").is_some_and(is_string)
            }
            _ => false,
        },
        "selector" => {
            get(map, "selector").is_some_and(selector_string)
                && optional_component(map, "separator")
        }
        "nbt" => {
            let path_ok = get(map, "nbt")
                .and_then(string_of)
                .is_some_and(|p| horizon_client_core::nbt_path::NbtPath::parse(&p).is_ok());
            let source = if let Some(e) = get(map, "entity") {
                selector_string(e)
            } else if let Some(b) = get(map, "block") {
                string_of(b).is_some_and(|s| {
                    let reg = super::args::Registries::default();
                    let mut r = super::Reader::new(&s);
                    super::ArgType::BlockPos.parse(&mut r, &reg).is_ok() && !r.can_read()
                })
            } else if let Some(s) = get(map, "storage") {
                identifier(s).is_ok()
            } else {
                false
            };
            path_ok && source && optional_component(map, "separator")
        }
        "object" => {
            let kind = get(map, "object").and_then(string_of);
            let atlas = || {
                get(map, "sprite").is_some_and(|s| identifier(s).is_ok())
                    && get(map, "atlas").is_none_or(|a| identifier(a).is_ok())
            };
            let player = || get(map, "player").is_some();
            match kind.as_deref() {
                Some("atlas") => atlas(),
                Some("player") => player(),
                Some(_) => false,
                None => atlas() || player(),
            }
        }
        _ => false,
    }
}
const CONTENT_TYPES: [&str; 7] = [
    "text",
    "translatable",
    "keybind",
    "score",
    "selector",
    "nbt",
    "object",
];
fn full(t: &Tag) -> R {
    let Tag::Compound(map) = t else {
        return err(not_map(t));
    };
    let contents_ok = if let Some(ty) = get(map, "type") {
        let Some(name) = string_of(ty) else {
            return err("Not a string");
        };
        if !CONTENT_TYPES.contains(&name.as_str()) {
            return err(format!("Unknown element id: {name}"));
        }
        content(&name, map)
    } else {
        CONTENT_TYPES.iter().any(|k| content(k, map))
    };
    if !contents_ok {
        return err("No matching codec found");
    }
    if let Some(extra) = get(map, "extra") {
        match non_empty_list(extra, &component) {
            Ok(()) => {}
            Err(e) => return partial(e.msg),
        }
    }
    style_fields(map)
}
fn component(t: &Tag) -> R {
    either(either(string(t), || non_empty_list(t, &component)), || {
        full(t)
    })
}
/// ComponentSerialization.CODEC.parse: the error message on failure.
pub fn decode(t: &Tag) -> Result<(), String> {
    component(t).map_err(|e| e.msg)
}
/// Style.Serializer.CODEC.parse.
pub fn decode_style(t: &Tag) -> Result<(), String> {
    match t {
        Tag::Compound(map) => style_fields(map).map_err(|e| e.msg),
        _ => Err(not_map(t)),
    }
}
