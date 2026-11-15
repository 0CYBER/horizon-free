//! `text_from_json`: a text component read as `/tellraw` takes it.
use super::Game;
use super::native_text_grammar::build;
use horizon_client::command::Registries;
use horizon_client::text::Text;
use mod_api::text::TextFormError;

impl Game<'_> {
    /// A component read as `/tellraw` takes it; None when it isn't one (an object with no
    /// content key, an empty list, a content key of the wrong kind).
    pub(super) fn text_from_json(&self, json: &str) -> Read {
        Read(component_from_json(json, &self.ui.registries))
    }
}

/// What `text_from_json` read: the component, none, or the nesting refusal, crossing to the
/// API as one `Result`.
pub(super) struct Read(Result<Option<Text>, TextFormError>);

impl crate::game::api_values::TryIntoApi for Read {
    type Api = Option<mod_api::Text>;
    type Error = TextFormError;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        self.0.clone().and_then(|text| text.try_into_api())
    }
}

/// One JSON or SNBT component (`ComponentSerialization.CODEC`'s string, list or object
/// forms), with nothing but whitespace after it.
fn component_from_json(json: &str, registries: &Registries) -> Result<Option<Text>, TextFormError> {
    // JSON first: it decodes escapes and has a real `null`. SNBT is the fallback (and takes
    // the nesting beyond serde_json's own limit to the parser's 512).
    let (value, snbt) = match serde_json::from_str::<serde_json::Value>(json) {
        Ok(value) => (value, false),
        Err(_) => {
            let (value, used) = match horizon_client::component::parse_snbt(json) {
                Ok(parsed) => parsed,
                // NBT's own limit of 512: the named refusal, not "not a component".
                Err(error) if error.starts_with("arguments.nbtpath.too_deep at ") => {
                    return Err(TextFormError::TooDeep);
                }
                Err(_) => return Ok(None),
            };
            if !json.as_bytes()[used..].iter().all(u8::is_ascii_whitespace) {
                return Ok(None);
            }
            (value, true)
        }
    };
    // Only the rebuilt value reaches the capture, so a failure anywhere leaves no partial text.
    Ok(build(&value, registries, snbt).map(Text::from))
}

#[cfg(test)]
mod tests {
    use super::{Registries, Text, TextFormError};
    use crate::game::api_values::TryIntoApi;
    use mod_api::text::{TextClick, TextContent, TextNbtSource};

    fn component_from_json(json: &str) -> Option<Text> {
        super::component_from_json(json, &horizon_client::command::Registries::default()).unwrap()
    }

    fn api(text: &Text) -> mod_api::Text {
        text.try_into_api().unwrap()
    }

    /// JSON and SNBT components keep their structure: styles, children, translations
    /// and click events; what is not one component is refused.
    #[test]
    fn text_from_json_reads_components_as_tellraw_does() {
        let text = api(&component_from_json(
            r#"{"text":"Hi ","color":"gold","extra":[{"text":"there","bold":true}]}"#,
        )
        .unwrap());
        assert_eq!(text.content, TextContent::Literal("Hi ".into()));
        assert_eq!(text.style.color.as_deref(), Some("gold"));
        assert_eq!(text.children.len(), 1);
        assert_eq!(text.children[0].style.bold, Some(true));
        let text = api(
            &component_from_json(r#"{"translate":"chat.type.text","with":["a","b"]}"#).unwrap(),
        );
        assert!(
            matches!(&text.content, TextContent::Translatable { key, arguments, .. } if key == "chat.type.text" && arguments.len() == 2)
        );
        let text = api(&component_from_json(
            "{text:'run',click_event:{action:run_command,command:'/spawn'}}",
        )
        .unwrap());
        assert_eq!(
            text.style.click,
            Some(TextClick::RunCommand("/spawn".into()))
        );
        assert_eq!(
            api(&component_from_json("  \"plain\"\n").unwrap()).content,
            TextContent::Literal("plain".into())
        );
        for refused in [
            "",
            "{\"text\":\"a\"} trailing",
            "{\"text\":\"open",
            "12",
            "true",
            "[]",
        ] {
            assert!(component_from_json(refused).is_none(), "{refused:?}");
        }
    }

    /// Objects and lists that are not one component are refused whole, however deep.
    const NOT_COMPONENTS: [&str; 43] = [
        "{}",
        r#"{"color":"red"}"#,
        r#"{"unrelated":"x","extra":[]}"#,
        r#"{"bold":true,"italic":false}"#,
        r#"{"text":{"a":1}}"#,
        r#"{"text":["a"]}"#,
        r#"{"translate":5}"#,
        r#"{"translate":"k","with":"a"}"#,
        r#"{"keybind":true}"#,
        r#"{"selector":["@a"]}"#,
        r#"{"score":{"name":"a"}}"#,
        r#"{"score":"a"}"#,
        r#"{"nbt":"a.b"}"#,
        r#"{"nbt":"a.b","source":"entity","block":"0 0 0"}"#,
        r#"{"type":"bogus","text":"a"}"#,
        r#"{"type":"keybind","text":"a"}"#,
        r#"{"text":"a","extra":"b"}"#,
        r#"{"text":"a","extra":[{}]}"#,
        r#"{"text":"a","extra":[{"text":"b","extra":[{"color":"red"}]}]}"#,
        r#"[{}]"#,
        r#"["a",{"color":"red"}]"#,
        r#"["a",[]]"#,
        r#"[1]"#,
        r#"{"text":"a","extra":[{"text":"b","extra":5}]}"#,
        // A number is not a string: `text` is not coerced.
        r#"{"text":1}"#,
        // `extra` is a non-empty list.
        r#"{"text":"a","extra":[]}"#,
        // A style field that is there and wrong refuses the whole component.
        r##"{"text":"a","color":"#GGGGGG"}"##,
        r#"{"text":"a","color":5}"#,
        r#"{"text":"a","bold":"yes"}"#,
        r#"{"text":"a","shadow_color":"red"}"#,
        r#"{"text":"a","insertion":1}"#,
        r#"{"text":"a","font":5}"#,
        r#"{"text":"a","click_event":{"action":"bogus"}}"#,
        r#"{"text":"a","click_event":"run_command"}"#,
        r#"{"text":"a","hover_event":{"action":"show_text"}}"#,
        r#"{"text":"a","hover_event":{"action":"show_text","value":{}}}"#,
        // The partial result is never used: a good entry before a bad one.
        r#"["a",{"text":"b","extra":["ok",{"bad":1}]}]"#,
        r#"{"text":"a","bold":true,"extra":["ok",{"text":"b","italic":"no"}]}"#,
        // A selector and an NBT path must parse; a selector's separator stays strict.
        r#"{"selector":"@x"}"#,
        r#"{"selector":"@a","separator":5}"#,
        r#"{"nbt":"[","entity":"@s"}"#,
        r#"{"nbt":"a","entity":"@x"}"#,
        r#"{"nbt":"a","entity":"@s","interpret":true,"plain":true}"#,
    ];

    const NUMERIC_FLAGS: [&str; 10] = [
        r#"{"text":"a","bold":0}"#,
        r#"{"text":"a","bold":1}"#,
        r#"{"text":"a","italic":0}"#,
        r#"{"text":"a","italic":1}"#,
        r#"{"text":"a","underlined":0}"#,
        r#"{"text":"a","underlined":1}"#,
        r#"{"text":"a","strikethrough":0}"#,
        r#"{"text":"a","strikethrough":1}"#,
        r#"{"text":"a","obfuscated":0}"#,
        r#"{"text":"a","obfuscated":1}"#,
    ];

    #[test]
    fn text_from_json_distinguishes_json_flags_from_snbt_bytes() {
        for input in NUMERIC_FLAGS {
            assert!(component_from_json(input).is_none(), "{input}");
        }
        for flag in [
            "bold",
            "italic",
            "underlined",
            "strikethrough",
            "obfuscated",
        ] {
            for value in [false, true] {
                let json = format!(r#"{{"text":"a","{flag}":{value}}}"#);
                assert!(component_from_json(&json).is_some(), "{json}");
                let snbt = format!("{{text:a,{flag}:{}b}}", u8::from(value));
                let built = api(&component_from_json(&snbt).unwrap());
                let actual = match flag {
                    "bold" => built.style.bold,
                    "italic" => built.style.italic,
                    "underlined" => built.style.underlined,
                    "strikethrough" => built.style.strikethrough,
                    _ => built.style.obfuscated,
                };
                assert_eq!(actual, Some(value), "{snbt}");
            }
        }
    }

    #[test]
    fn text_from_json_rebuilds_hover_entity_name() {
        let input = r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","keybind":"key.jump","text":"wrong"}}}"#;
        let text = api(&component_from_json(input).unwrap());
        assert!(
            matches!(text.style.hover, Some(mod_api::text::TextHover::Entity { name: Some(name), .. }) if name.content == TextContent::Keybind("key.jump".into()))
        );
        assert!(component_from_json(r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","text":"wrong"}}}"#).is_none());
    }

    #[test]
    fn text_from_json_refuses_objects_that_are_not_components() {
        for refused in NOT_COMPONENTS {
            assert!(component_from_json(refused).is_none(), "{refused}");
        }
        // The same objects in SNBT.
        assert!(component_from_json("{color:red}").is_none());
        assert!(component_from_json("{}").is_none());
    }

    /// An explicit `type` is built as that type, whatever other content keys sit beside it.
    #[test]
    fn text_from_json_builds_an_explicit_type_alone() {
        let content = |json: &str| api(&component_from_json(json).unwrap()).content;
        assert_eq!(
            content(r#"{"type":"keybind","keybind":"key.jump","text":"wrong"}"#),
            TextContent::Keybind("key.jump".into())
        );
        assert_eq!(
            content(r#"{"type":"text","text":"right","keybind":"key.jump"}"#),
            TextContent::Literal("right".into())
        );
        let text = api(&component_from_json(
            r#"{"text":"a","extra":[{"type":"keybind","keybind":"key.use","text":"x"}]}"#,
        )
        .unwrap());
        assert_eq!(
            text.children[0].content,
            TextContent::Keybind("key.use".into())
        );
        assert!(matches!(
            content(
                r#"{"type":"nbt","source":"entity","nbt":"Health","block":"0 64 0","entity":"@s"}"#
            ),
            TextContent::Nbt { path, source: TextNbtSource::Entity(selector), .. }
                if path == "Health" && selector == "@s"
        ));
        assert!(matches!(
            content(r#"{"type":"nbt","source":"block","nbt":"Items","block":"0 64 0","entity":"@s"}"#),
            TextContent::Nbt { source: TextNbtSource::Block(pos), .. } if pos == "0 64 0"
        ));
    }

    /// Without a `type` the first kind the keys satisfy decides, in the codec's order
    /// (`text`, `translatable`, `keybind`, `score`, `selector`, `nbt`, `object`); a lenient
    /// field of the wrong kind is dropped from the built value.
    #[test]
    fn text_from_json_builds_what_the_codec_decodes() {
        let content = |json: &str| api(&component_from_json(json).unwrap()).content;
        assert_eq!(
            content(r#"{"text":"a","keybind":"key.jump"}"#),
            TextContent::Literal("a".into())
        );
        assert_eq!(
            content(r#"{"selector":"@a","keybind":"key.jump"}"#),
            TextContent::Keybind("key.jump".into())
        );
        assert!(matches!(
            content(r#"{"selector":"@a","score":{"name":"x","objective":"o"}}"#),
            TextContent::Score { .. }
        ));
        assert!(matches!(
            content(r#"{"translate":"k","fallback":5}"#),
            TextContent::Translatable { key, fallback: None, .. } if key == "k"
        ));
        // `{"text":1}` fails the text kind, so the next kind that decodes is used.
        assert_eq!(
            content(r#"{"text":1,"keybind":"key.use"}"#),
            TextContent::Keybind("key.use".into())
        );
    }

    /// JSON escapes decode to the character they name.
    #[test]
    fn text_from_json_decodes_json_escapes() {
        for (json, expected) in [
            (r#"{"text":"\u0041"}"#, "A"),
            // SNBT takes the same escapes.
            (r#"'\u0041\ud83d\ude00'"#, "A\u{1F600}"),
            (r#"{"text":"\""}"#, "\""),
            (r#"{"text":"\\"}"#, "\\"),
            (r#"{"text":"\/"}"#, "/"),
            (r#"{"text":"a\nb\tc\r\b\f"}"#, "a\nb\tc\r\u{8}\u{c}"),
            (r#""\u00e9\ud83d\ude00""#, "\u{e9}\u{1F600}"),
        ] {
            assert_eq!(
                api(&component_from_json(json).unwrap()).content,
                TextContent::Literal(expected.into()),
                "{json}"
            );
        }
    }

    /// A component 513 `extra` levels deep is refused with the named nesting error, not read
    /// as "not a component".
    #[test]
    fn text_from_json_names_the_nesting_refusal() {
        let deep = |levels: usize| {
            let mut json = String::from(r#"{"text":"leaf"}"#);
            for _ in 0..levels {
                json = format!(r#"{{"text":"n","extra":[{json}]}}"#);
            }
            json
        };
        assert!(matches!(
            super::component_from_json(&deep(513), &Registries::default()),
            Err(TextFormError::TooDeep)
        ));
        assert!(matches!(
            super::component_from_json(&deep(3), &Registries::default()),
            Ok(Some(_))
        ));
    }

    /// Each content kind of `ComponentSerialization`, with `extra` and arrays, is a component.
    #[test]
    fn text_from_json_accepts_each_content_kind() {
        for accepted in [
            r#"{"text":"a"}"#,
            r#"{"text":"","extra":null}"#,
            r#"{"type":null,"text":"a","bold":null,"unknown":[1]}"#,
            r#"{"text":"a","shadow_color":[1,0,0.5,1],"font":"alt","insertion":"i"}"#,
            r##"{"text":"a","color":"#FF00FF","bold":true,"click_event":{"action":"open_url","url":"https://example.com"}}"##,
            r#"{"text":"a","hover_event":{"action":"show_text","value":["tip",{"text":"x"}]}}"#,
            r#"{"translate":"k","fallback":5}"#,
            r#"{"nbt":"a","entity":"@s","interpret":"x","plain":5,"separator":5}"#,
            r#"{"translate":"k"}"#,
            r#"{"translate":"k","fallback":"f","with":["a",1,true,{"text":"b"}]}"#,
            r#"{"keybind":"key.jump"}"#,
            r#"{"selector":"@a","separator":{"text":","}}"#,
            r#"{"score":{"name":"@s","objective":"o"}}"#,
            r#"{"nbt":"a.b","entity":"@s"}"#,
            r#"{"nbt":"a.b","block":"0 64 0","interpret":true}"#,
            r#"{"nbt":"a.b","storage":"ns:s"}"#,
            r#"{"type":"nbt","source":"storage","nbt":"a","storage":"ns:s"}"#,
            r#"{"type":"text","text":"a"}"#,
            r#"{"type":"translatable","translate":"k"}"#,
            r#"{"object":"atlas","sprite":"block/stone"}"#,
            r#"{"player":{"name":"Steve"}}"#,
            r#"{"text":"a","extra":["b",{"text":"c","extra":[{"keybind":"key.use"}]}]}"#,
            r#"["a",{"text":"b"},["c"]]"#,
            "{text:a}",
            "[{text:'a'},'b']",
        ] {
            assert!(component_from_json(accepted).is_some(), "{accepted}");
        }
    }

    /// Through the public host: literal valid components read, literal invalid ones are
    /// `Ok(None)`, and a text over the nesting limit is the named refusal.
    #[test]
    #[ignore = "daily"]
    fn the_host_reads_components_and_returns_none_for_the_rest() {
        use super::super::native_entity_refs::tests::with_game;
        with_game(|game| {
            let host: &dyn mod_api::Host = game;
            let read = |json: &str| host.text_from_json(json).unwrap().map(|t| t.content);
            assert_eq!(
                read(r#"{"text":"Hi"}"#),
                Some(TextContent::Literal("Hi".into()))
            );
            assert_eq!(
                read("\"plain\""),
                Some(TextContent::Literal("plain".into()))
            );
            assert_eq!(
                read(r#"["a",{"text":"b"}]"#),
                Some(TextContent::Literal("a".into()))
            );
            assert!(matches!(
                read(r#"{"translate":"chat.type.text"}"#),
                Some(TextContent::Translatable { .. })
            ));
            // An explicit `type` decides alone: the stray `text` and the other sources are ignored.
            assert_eq!(
                read(r#"{"type":"keybind","keybind":"key.jump","text":"wrong"}"#),
                Some(TextContent::Keybind("key.jump".into()))
            );
            assert!(matches!(
                read(r#"{"type":"nbt","source":"entity","nbt":"Health","block":"0 64 0","entity":"@s"}"#),
                Some(TextContent::Nbt { path, source: TextNbtSource::Entity(selector), .. })
                    if path == "Health" && selector == "@s"
            ));
            let hover = host.text_from_json(r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","keybind":"key.jump","text":"wrong"}}}"#).unwrap().unwrap();
            assert!(
                matches!(hover.style.hover, Some(mod_api::text::TextHover::Entity { name: Some(name), .. }) if name.content == TextContent::Keybind("key.jump".into()))
            );
            assert_eq!(
                read(
                    r#"{"text":"a","hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"name":{"type":"keybind","text":"wrong"}}}"#
                ),
                None
            );
            for valid in [
                r#"{"text":"a","bold":true}"#,
                r#"{"text":"a","italic":false}"#,
                r#"{"text":"a","underlined":true}"#,
                r#"{"text":"a","strikethrough":false}"#,
                r#"{"text":"a","obfuscated":true}"#,
            ] {
                assert!(read(valid).is_some(), "{valid}");
            }
            for invalid in NOT_COMPONENTS.into_iter().chain(NUMERIC_FLAGS) {
                assert_eq!(read(invalid), None, "{invalid}");
            }
            assert_eq!(
                read(r#"{"text":"\u0041"}"#),
                Some(TextContent::Literal("A".into()))
            );
            let mut deep = String::from(r#""leaf""#);
            for _ in 0..513 {
                deep = format!(r#"{{"text":"n","extra":[{deep}]}}"#);
            }
            assert_eq!(
                host.text_from_json(&deep)
                    .map(|text| text.map(|t| t.content)),
                Err(mod_api::text::TextFormError::TooDeep)
            );
            assert_eq!(read("{} trailing"), None);
            assert_eq!(read("12"), None);
        });
    }
}
