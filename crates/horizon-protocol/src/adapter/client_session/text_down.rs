//! A text component's click and hover events in the form 1.21.4 and older read, from 26.3's.
//!
//! Until 1.21.4 a style's keys are `clickEvent` (`action`, `value`) and `hoverEvent` (`action`, `contents`; checked on
//! the 1.21.4 client jar: `ClickEvent`, `HoverEvent`, `Style$Serializer`). 1.21.5 renamed them `click_event` and
//! `hover_event` and gave each action its own fields (26.3 jar: `ClickEvent$OpenUrl` has `url`, `$RunCommand` and
//! `$SuggestCommand` `command`, `$ChangePage` the int `page`, `$CopyToClipboard` `value`; `HoverEvent$ShowText` has
//! `value`, `$ShowItem` the stack's `id`/`count`/`components`, `$ShowEntity` `id` (the type), `uuid` and `name`).
//! An event 1.21.4 has no action for, or does not accept from a server (`open_file`), is left out.
//!
//! The item components of a `show_item` are not converted.
use horizon_core::nbt::{JavaString, Tag};

/// Rewrites the events of `tag` (a text component in NBT form) and of its `extra`, `with` and `separator`
/// components, and of the `show_text` and `show_entity` components of its hover events.
pub(super) fn component_down(tag: &mut Tag) {
    match tag {
        Tag::List(items) => items.iter_mut().for_each(component_down),
        Tag::Compound(c) => {
            if let Some(click) = c.remove(&JavaString::from("click_event"))
                && let Some(click) = click_down(&click)
            {
                c.insert(JavaString::from("clickEvent"), click);
            }
            if let Some(hover) = c.remove(&JavaString::from("hover_event"))
                && let Some(hover) = hover_down(hover)
            {
                c.insert(JavaString::from("hoverEvent"), hover);
            }
            // `""` wraps an element of a list that mixes plain and styled text.
            for key in ["extra", "with", "separator", ""] {
                if let Some(child) = c.get_mut(&JavaString::from(key)) {
                    component_down(child);
                }
            }
        }
        _ => {}
    }
}

fn string(value: &str) -> Tag {
    Tag::String(JavaString::from(value))
}

fn text<'t>(tag: &'t Tag, key: &str) -> Option<&'t JavaString> {
    match tag.get(key)? {
        Tag::String(s) => Some(s),
        _ => None,
    }
}

fn click_down(click: &Tag) -> Option<Tag> {
    let action = text(click, "action")?.to_string_lossy();
    let value = match action.as_str() {
        "open_url" => Tag::String(text(click, "url")?.clone()),
        "run_command" | "suggest_command" => Tag::String(text(click, "command")?.clone()),
        "copy_to_clipboard" => Tag::String(text(click, "value")?.clone()),
        // 1.21.4 reads the page from a string.
        "change_page" => match click.get("page")? {
            Tag::Int(page) => string(&page.to_string()),
            _ => return None,
        },
        _ => return None,
    };
    Some(Tag::compound([
        ("action", string(&action)),
        ("value", value),
    ]))
}

fn hover_down(mut hover: Tag) -> Option<Tag> {
    let action = text(&hover, "action")?.to_string_lossy();
    let Tag::Compound(fields) = &mut hover else {
        return None;
    };
    fields.remove(&JavaString::from("action"));
    let contents = match action.as_str() {
        "show_text" => {
            let mut value = fields.remove(&JavaString::from("value"))?;
            component_down(&mut value);
            value
        }
        "show_item" => Tag::Compound(std::mem::take(fields)),
        "show_entity" => {
            let mut entity = std::mem::take(fields);
            if let Some(kind) = entity.remove(&JavaString::from("id")) {
                entity.insert(JavaString::from("type"), kind);
            }
            if let Some(uuid) = entity.remove(&JavaString::from("uuid")) {
                entity.insert(JavaString::from("id"), uuid);
            }
            if let Some(name) = entity.get_mut(&JavaString::from("name")) {
                component_down(name);
            }
            Tag::Compound(entity)
        }
        _ => return None,
    };
    Some(Tag::compound([
        ("action", string(&action)),
        ("contents", contents),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(action: &str, field: &str, value: Tag) -> Tag {
        Tag::compound([("action", string(action)), (field, value)])
    }

    #[test]
    fn every_click_action_takes_the_old_value() {
        for (new, old) in [
            (
                click("open_url", "url", string("https://a.b/c")),
                "https://a.b/c",
            ),
            (
                click("run_command", "command", string("/say hi")),
                "/say hi",
            ),
            (click("suggest_command", "command", string("/me")), "/me"),
            (click("copy_to_clipboard", "value", string("text")), "text"),
            (click("change_page", "page", Tag::Int(12)), "12"),
        ] {
            let action = text(&new, "action").unwrap().to_string_lossy();
            let mut tag = Tag::compound([("text", string("x")), ("click_event", new)]);
            component_down(&mut tag);
            assert_eq!(
                tag,
                Tag::compound([
                    ("text", string("x")),
                    ("clickEvent", click(&action, "value", string(old))),
                ])
            );
        }
    }

    #[test]
    fn a_click_the_old_client_does_not_accept_is_left_out() {
        for action in ["open_file", "show_dialog", "custom"] {
            let mut tag = Tag::compound([
                ("text", string("x")),
                ("click_event", click(action, "path", string("a"))),
            ]);
            component_down(&mut tag);
            assert_eq!(tag, Tag::compound([("text", string("x"))]));
        }
    }

    #[test]
    fn hover_text_item_and_entity_take_the_old_contents() {
        let mut tag = Tag::compound([
            ("text", string("x")),
            (
                "hover_event",
                Tag::compound([
                    ("action", string("show_text")),
                    (
                        "value",
                        Tag::compound([
                            ("text", string("inner")),
                            ("click_event", click("run_command", "command", string("/a"))),
                        ]),
                    ),
                ]),
            ),
        ]);
        component_down(&mut tag);
        assert_eq!(
            tag,
            Tag::compound([
                ("text", string("x")),
                (
                    "hoverEvent",
                    Tag::compound([
                        ("action", string("show_text")),
                        (
                            "contents",
                            Tag::compound([
                                ("text", string("inner")),
                                ("clickEvent", click("run_command", "value", string("/a"))),
                            ])
                        ),
                    ])
                ),
            ])
        );
        let item = || [("id", string("minecraft:stone")), ("count", Tag::Int(3))];
        let mut tag = Tag::compound([(
            "hover_event",
            Tag::compound([("action", string("show_item"))].into_iter().chain(item())),
        )]);
        component_down(&mut tag);
        assert_eq!(
            tag,
            Tag::compound([(
                "hoverEvent",
                Tag::compound([
                    ("action", string("show_item")),
                    ("contents", Tag::compound(item())),
                ])
            )])
        );
        let mut tag = Tag::compound([(
            "hover_event",
            Tag::compound([
                ("action", string("show_entity")),
                ("id", string("minecraft:pig")),
                ("uuid", Tag::IntArray(vec![1, 2, 3, 4])),
                ("name", Tag::compound([("text", string("Pig"))])),
            ]),
        )]);
        component_down(&mut tag);
        assert_eq!(
            tag,
            Tag::compound([(
                "hoverEvent",
                Tag::compound([
                    ("action", string("show_entity")),
                    (
                        "contents",
                        Tag::compound([
                            ("type", string("minecraft:pig")),
                            ("id", Tag::IntArray(vec![1, 2, 3, 4])),
                            ("name", Tag::compound([("text", string("Pig"))])),
                        ])
                    ),
                ])
            )])
        );
    }

    #[test]
    fn events_are_found_in_extra_with_and_list_elements() {
        let ev = || click("open_url", "url", string("u"));
        let old = || click("open_url", "value", string("u"));
        let mut tag = Tag::List(vec![
            Tag::compound([("", string("plain"))]),
            Tag::compound([
                ("text", string("a")),
                (
                    "extra",
                    Tag::List(vec![Tag::compound([("click_event", ev())])]),
                ),
                (
                    "with",
                    Tag::List(vec![Tag::compound([("click_event", ev())]), string("s")]),
                ),
            ]),
        ]);
        component_down(&mut tag);
        assert_eq!(
            tag,
            Tag::List(vec![
                Tag::compound([("", string("plain"))]),
                Tag::compound([
                    ("text", string("a")),
                    (
                        "extra",
                        Tag::List(vec![Tag::compound([("clickEvent", old())])]),
                    ),
                    (
                        "with",
                        Tag::List(vec![Tag::compound([("clickEvent", old())]), string("s")]),
                    ),
                ]),
            ])
        );
    }
}
