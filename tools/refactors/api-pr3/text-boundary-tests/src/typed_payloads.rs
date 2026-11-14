use super::{TryIntoApi, TryIntoEngine};
use horizon_client_core::text::{ClickEvent, InlineObject, Span, Style, Text as Engine};
use mod_api::{
    Tag,
    text::{
        Text, TextClick, TextContent, TextFormError, TextHover, TextObject, TextProfile,
        TextProfileProperty,
    },
};
use serde_json::{Value, json};
fn opaque(depth: usize) -> Value {
    let mut v = Value::from(1);
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}
#[test]
fn late_shallow_failure_releases_all_owned_metadata() {
    std::thread::Builder::new().stack_size(256*1024).spawn(|| {
 for where_null in 0..3 {
 let mut source=json!({"object":"player","player":{"name":"Test","properties":[{"name":"x","value":"y"}]},"click_event":{"action":"custom","id":"test:x"}});
 source["test:root"]=opaque(511);
 source["player"]["test:profile"]=opaque(511);
 source["player"]["properties"][0]["test:property"]=opaque(511);
 match where_null {
 0=>source["click_event"]["payload"]=json!({"bad":null}),
 1=>{source["hover_event"]=json!({"action":"show_item","id":"minecraft:stone","components":{"bad":null}});},
 _=>{source["hover_event"]=json!({"action":"show_text","value":"hover","test:bad":null});},
 }
 let engine=Engine::empty().with_component(source);
 assert!(matches!(engine.try_into_api(),Err(TextFormError::NestedNull)));
 std::mem::forget(engine);
 }
 }).unwrap().join().unwrap();
}
#[test]
fn proof_refuses_late_payload_in_every_tag_slot() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            for slot in 0..8 {
                for invalid_string in [false, true] {
                    let bad = if invalid_string {
                        Tag::String(horizon_core::nbt::JavaString(vec![0xd800]))
                    } else {
                        Tag::Double(f64::INFINITY.to_bits())
                    };
                    let mut text = Text::literal("root");
                    let mut deep = Tag::Byte(1);
                    for _ in 0..511 {
                        deep = Tag::List(vec![deep]);
                    }
                    text.metadata = Tag::compound([("test:deep", deep)]);
                    let wrapped =
                        || Tag::compound([("test:bad", horizon_core::nbt::clone_tag(&bad))]);
                    match slot {
                        0 => text.metadata = wrapped(),
                        1 => text.style.metadata = wrapped(),
                        2 | 3 => {
                            let mut profile = TextProfile::default();
                            profile.name = Some("Test".into());
                            if slot == 2 {
                                profile.metadata = wrapped();
                            } else {
                                let mut p = TextProfileProperty::default();
                                p.name = "x".into();
                                p.value = "y".into();
                                p.metadata = wrapped();
                                profile.properties.push(p);
                            }
                            text.content = TextContent::Object(TextObject::Player {
                                profile,
                                hat: true,
                                fallback: None,
                            });
                        }
                        4 => {
                            text.style.click = Some(TextClick::Custom {
                                id: "test:x".into(),
                                payload: Some(horizon_core::nbt::clone_tag(&bad)),
                            })
                        }
                        5 => {
                            text.style.click =
                                Some(TextClick::ShowDialog(horizon_core::nbt::clone_tag(&bad)))
                        }
                        6 => {
                            text.style.hover = Some(TextHover::Item {
                                id: "minecraft:stone".into(),
                                count: 1,
                                components: Some(horizon_core::nbt::clone_tag(&bad)),
                            })
                        }
                        _ => {
                            let mut child = Text::literal("late");
                            child.metadata = wrapped();
                            text.children.push(child);
                        }
                    }
                    text.validate().unwrap();
                    let result = text.try_into_engine(&());
                    assert!(
                        matches!(result, Err(TextFormError::NonFinite)) && !invalid_string
                            || matches!(result, Err(TextFormError::InvalidString))
                                && invalid_string,
                        "slot {slot}"
                    );
                    std::mem::forget(text);
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn source_less_native_mixed_tags_obey_semantic_depth() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            for depth in [512, 513] {
                let mut tag = Tag::List(vec![Tag::Byte(1), Tag::Short(128)]);
                for _ in 1..depth {
                    tag = Tag::List(vec![tag]);
                }
                let engine = Engine::from_spans(vec![Span::new(
                    "x",
                    Style {
                        click: Some(ClickEvent::Custom {
                            id: "test:x".into(),
                            payload: Some(tag.into()),
                        }),
                        ..Style::default()
                    },
                )]);
                let result = engine.try_into_api();
                if depth == 513 {
                    assert!(matches!(result, Err(TextFormError::TooDeep)));
                } else {
                    let api = result.as_ref().unwrap();
                    api.validate().unwrap();
                    let returned = api.try_into_engine(&()).unwrap();
                    std::mem::forget(returned);
                }
                std::mem::forget((engine, result));
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn source_less_profile_4096_refuses_without_clone() {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut p = json!({"name":"Test"});
            p["test:deep"] = opaque(4096);
            let engine = Engine::from_spans(vec![Span::new(
                "x",
                Style {
                    inline_object: Some(InlineObject::Player {
                        profile: p.into(),
                        hat: true,
                        fallback: "x".into(),
                    }),
                    ..Style::default()
                },
            )]);
            assert!(matches!(engine.try_into_api(), Err(TextFormError::TooDeep)));
            std::mem::forget(engine);
        })
        .unwrap()
        .join()
        .unwrap();
}
#[test]
fn event_metadata_is_scoped_to_the_actual_discriminant() {
    for event in [
        json!({"click_event":{"action":"custom","id":"test:x","name":"opaque-name","count":42}}),
        json!({"hover_event":{"action":"show_text","value":{"text":"hover"},"payload":{"name":"opaque"}}}),
        json!({"hover_event":{"action":"show_item","id":"minecraft:stone","count":1,"dialog":"opaque"}}),
        json!({"hover_event":{"action":"show_entity","id":"minecraft:pig","uuid":[0,0,0,1],"contents":"opaque"}}),
    ] {
        let mut source = json!({"text":"root"});
        source
            .as_object_mut()
            .unwrap()
            .extend(event.as_object().unwrap().clone());
        let engine = Engine::empty().with_component(source.clone());
        let api = engine.try_into_api().unwrap();
        assert_eq!(api.try_into_engine(&()).unwrap().component(), source);
    }
}
