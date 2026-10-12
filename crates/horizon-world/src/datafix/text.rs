//! The text component fixes of 1.21.5 (keys 4290 to 4292): JSON strings become
//! NBT components, legacy hover events are replaced, stringified style flags
//! become booleans and hover/click events take their new field names.
use super::FixFn;
use super::dynamic::{
    as_number, as_str, as_string, boolean, empty_map, remove, rename, rename_and_fix, set, string,
};
use super::json;
use super::schema::{Key, Ty, key};
use crate::nbt::Tag;

pub(super) fn register(out: &mut Vec<(Key, FixFn)>) {
    out.push((key(4290, 0), |cx| cx.each(Ty::TextComponent, unflatten)));
    out.push((key(4291, 0), |cx| {
        cx.each(Ty::TextComponent, legacy_hover_event)
    }));
    out.push((key(4291, 0), |cx| {
        cx.each(Ty::TextComponent, stringified_flags)
    }));
    out.push((key(4292, 0), |cx| {
        cx.each(Ty::TextComponent, hover_and_click_event)
    }));
}

/// `UnflattenTextComponentFix`: the JSON of a string component as NBT.
fn unflatten(v: &mut Tag) {
    let Tag::String(s) = v else {
        return;
    };
    if let Some(parsed) = json::parse_lenient(&s.0)
        && let Some(tag) = json::to_nbt(&parsed)
    {
        *v = tag;
    }
}

fn is_full(v: &Tag) -> bool {
    matches!(v, Tag::Compound(_))
}

/// The hover event a schema of 4290/4291 reads as typed (a known action).
fn typed_hover(v: &Tag) -> Option<String> {
    let h = v.get("hoverEvent")?;
    let action = as_str(h.get("action"))?;
    matches!(action.as_str(), "show_text" | "show_item" | "show_entity").then_some(action)
}

/// `LegacyHoverEventFix`.
fn legacy_hover_event(v: &mut Tag) {
    if !is_full(v) {
        return;
    }
    let Some(hover) = v.get("hoverEvent").cloned() else {
        return;
    };
    if hover.get("value").is_none() {
        return;
    }
    let action = typed_hover(v).unwrap_or_default();
    let new = if action == "show_text" {
        let mut h = hover;
        rename(&mut h, "value", "contents");
        h
    } else {
        let mut text = "Legacy hoverEvent: ".encode_utf16().collect::<Vec<u16>>();
        text.extend(json::to_stable_string(&json::from_nbt(&hover)));
        Tag::compound([
            ("action", string("show_text")),
            (
                "contents",
                Tag::compound([("text", Tag::String(crate::nbt::JavaString(text)))]),
            ),
        ])
    };
    set(v, "hoverEvent", new);
}

/// `TextComponentStringifiedFlagsFix`: `Boolean.parseBoolean` of string flags.
fn stringified_flags(v: &mut Tag) {
    if !is_full(v) {
        return;
    }
    for flag in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        if let Some(s) = as_str(v.get(flag)) {
            set(v, flag, boolean(s.eq_ignore_ascii_case("true")));
        }
    }
}

/// `TextComponentHoverAndClickEventFix`.
fn hover_and_click_event(v: &mut Tag) {
    if !is_full(v) {
        return;
    }
    if typed_hover(v).is_none() && v.get("clickEvent").is_none() {
        return;
    }
    rename_and_fix(v, "hoverEvent", "hover_event", |h| Some(fix_hover_event(h)));
    rename_and_fix(v, "clickEvent", "click_event", fix_click_event);
}

fn copy_fields(target: &mut Tag, source: &Tag, fields: &[&str]) {
    for f in fields {
        if let Some(value) = source.get(f) {
            set(target, f, value.clone());
        }
    }
}

fn fix_hover_event(mut h: Tag) -> Tag {
    match as_string(h.get("action"), "").as_str() {
        "show_text" => rename(&mut h, "contents", "value"),
        "show_item" => {
            let contents = h.get("contents").cloned().unwrap_or_else(empty_map);
            remove(&mut h, "contents");
            copy_fields(&mut h, &contents, &["id", "count", "components"]);
        }
        "show_entity" => {
            let contents = h.get("contents").cloned().unwrap_or_else(empty_map);
            remove(&mut h, "contents");
            copy_fields(&mut h, &contents, &["id", "type", "name"]);
            rename(&mut h, "id", "uuid");
            rename(&mut h, "type", "id");
        }
        _ => {}
    }
    h
}

fn fix_click_event(mut c: Tag) -> Option<Tag> {
    let action = as_string(c.get("action"), "");
    let value = as_string(c.get("value"), "");
    match action.as_str() {
        "open_url" => {
            if !valid_uri(&value) {
                return None;
            }
            rename(&mut c, "value", "url");
        }
        "open_file" => rename(&mut c, "value", "path"),
        "run_command" | "suggest_command" => {
            if !valid_chat(&value) {
                return None;
            }
            rename(&mut c, "value", "command");
        }
        "change_page" => {
            let old = c.get("value").and_then(|v| match as_number(v) {
                Some(n) => Some(n.int_value()),
                None => parse_int(&as_string(Some(v), "")),
            })?;
            remove(&mut c, "value");
            set(&mut c, "page", Tag::Int(old.max(1)));
        }
        _ => {}
    }
    Some(c)
}

/// `Integer.parseInt`.
fn parse_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `new URI(uri)` succeeds with an http or https scheme.
fn valid_uri(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let scheme_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !scheme_ok || rest.is_empty() {
        return false;
    }
    // Characters java.net.URI refuses anywhere, and escapes it checks.
    let bytes: Vec<char> = rest.chars().collect();
    let mut hashes = 0;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '%' {
            if !bytes.get(i + 1).is_some_and(char::is_ascii_hexdigit)
                || !bytes.get(i + 2).is_some_and(char::is_ascii_hexdigit)
            {
                return false;
            }
            i += 3;
            continue;
        }
        if c == '#' {
            hashes += 1;
        }
        if c.is_control()
            || c.is_whitespace()
            || matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}')
        {
            return false;
        }
        i += 1;
    }
    hashes <= 1 && matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
}

/// `StringUtil.isAllowedChatCharacter` for every character.
fn valid_chat(s: &str) -> bool {
    s.encode_utf16().all(|c| c != 167 && c >= 32 && c != 127)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(s: &str) -> Tag {
        json::to_nbt(&json::parse_lenient(&s.encode_utf16().collect::<Vec<_>>()).unwrap()).unwrap()
    }

    #[test]
    fn click_events_take_their_new_fields() {
        let mut c = j(r#"{"text":"a","clickEvent":{"action":"change_page","value":"0"}}"#);
        hover_and_click_event(&mut c);
        // The page is written as an int (`createInt`), not as JSON's narrowest number.
        let mut expected = j(r#"{"text":"a","click_event":{"action":"change_page"}}"#);
        if let Some(click) = expected.get_mut("click_event") {
            super::super::dynamic::set(click, "page", Tag::Int(1));
        }
        assert_eq!(c, expected);
        let mut c = j(r#"{"text":"a","clickEvent":{"action":"open_url","value":"ftp://x"}}"#);
        hover_and_click_event(&mut c);
        assert_eq!(c, j(r#"{"text":"a"}"#));
    }

    #[test]
    fn legacy_hover_events_become_text() {
        let mut c = j(r#"{"text":"a","hoverEvent":{"action":"show_item","value":"{id:stone}"}}"#);
        legacy_hover_event(&mut c);
        let expected = j(
            r#"{"text":"a","hoverEvent":{"action":"show_text","contents":{"text":"Legacy hoverEvent: {\"action\":\"show_item\",\"value\":\"{id:stone}\"}"}}}"#,
        );
        assert_eq!(c, expected);
    }
}
