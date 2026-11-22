#[test]
fn pet_variant_holders_validate_command_assignment_and_equality_predicates() {
    use super::{Reader, Registries, parse_predicate, parse_stack};
    use crate::args::RegistryIds;
    let mut reg = Registries {
        items: vec!["minecraft:stone".into()],
        ..Default::default()
    };
    for (component, registry) in [
        ("minecraft:cat/sound_variant", "minecraft:cat_sound_variant"),
        ("minecraft:cat/variant", "minecraft:cat_variant"),
        (
            "minecraft:wolf/sound_variant",
            "minecraft:wolf_sound_variant",
        ),
        ("minecraft:wolf/variant", "minecraft:wolf_variant"),
    ] {
        reg.persistent_components.push(component.into());
        reg.by_registry.insert(
            registry.into(),
            RegistryIds {
                ids: vec!["example:custom/pet".into()],
                tags: vec!["example:pets".into()],
            },
        );
    }
    reg.by_registry.insert(
        "minecraft:data_component_type".into(),
        RegistryIds {
            ids: reg.persistent_components.clone(),
            tags: vec![],
        },
    );
    for component in &reg.persistent_components {
        for parser in [parse_stack, parse_predicate] {
            let input = format!("stone[{component}=\"example:custom/pet\"]");
            let mut reader = Reader::new(&input);
            assert!(parser(&mut reader, &reg).is_ok(), "{input}");
            assert_eq!(reader.cursor, input.len());
            for value in [
                "\"example:missing\"",
                "\"#example:pets\"",
                "{id:\"example:custom/pet\"}",
                "1",
            ] {
                let input = format!("stone[{component}={value}]");
                assert!(parser(&mut Reader::new(&input), &reg).is_err(), "{input}");
            }
            let input = format!("stone[!{component}]");
            assert!(parser(&mut Reader::new(&input), &reg).is_ok(), "{input}");
        }
    }
}

#[test]
fn pet_variant_holders_use_the_supplied_registry_and_normalize_identifiers() {
    use super::{Registries, Tag, decoded_component_value};
    use crate::args::RegistryIds;
    use horizon_core::nbt::JavaString;

    // DataComponents -> each variant CODEC -> RegistryFixedCodec, not DIRECT_CODEC.
    for (component, registry, builtin) in [
        (
            "minecraft:cat/sound_variant",
            "minecraft:cat_sound_variant",
            "classic",
        ),
        ("minecraft:cat/variant", "minecraft:cat_variant", "tabby"),
        (
            "minecraft:wolf/sound_variant",
            "minecraft:wolf_sound_variant",
            "classic",
        ),
        ("minecraft:wolf/variant", "minecraft:wolf_variant", "pale"),
    ] {
        let mut reg = Registries::default();
        assert!(decoded_component_value(component, &Tag::String(builtin.into()), &reg).is_err());
        reg.by_registry.insert(
            registry.into(),
            RegistryIds {
                ids: vec![
                    format!("minecraft:{builtin}"),
                    "example:custom/pet".into(),
                    "minecraft:".into(),
                ],
                tags: vec!["example:known_tag".into()],
            },
        );
        for raw in [
            builtin.to_owned(),
            format!(":{builtin}"),
            format!("minecraft:{builtin}"),
        ] {
            assert_eq!(
                decoded_component_value(component, &Tag::String(raw.as_str().into()), &reg)
                    .unwrap(),
                Tag::String(format!("minecraft:{builtin}").as_str().into()),
            );
        }
        assert_eq!(
            decoded_component_value(component, &Tag::String("example:custom/pet".into()), &reg)
                .unwrap(),
            Tag::String("example:custom/pet".into()),
        );
        // Identifier accepts an empty path, but it must still resolve in the snapshot.
        assert_eq!(
            decoded_component_value(component, &Tag::String("".into()), &reg).unwrap(),
            Tag::String("minecraft:".into()),
        );
        for value in [
            Tag::String("example:missing".into()),
            Tag::String("example:known_tag".into()),
            Tag::String("#example:known_tag".into()),
            Tag::String("EXAMPLE:custom/pet".into()),
            Tag::String("example:Custom/pet".into()),
            Tag::String("..:custom/pet".into()),
            Tag::String("example:custom:pet".into()),
            Tag::String(JavaString(vec![0xd800])),
            Tag::String("é".into()),
            Tag::Int(1),
            Tag::Float(f32::NAN.to_bits()),
            Tag::List(vec![Tag::String(builtin.into())]),
            Tag::compound([("id", Tag::String(builtin.into()))]),
            Tag::compound([("asset_id", Tag::String("entity/cat/cat_tabby".into()))]),
        ] {
            assert!(
                decoded_component_value(component, &value, &reg).is_err(),
                "{component}: {value:?}"
            );
            assert!(decode_component(component, &value, &reg).is_err());
        }
        reg.by_registry.get_mut(registry).unwrap().ids.clear();
        assert!(decode_component(component, &Tag::String(builtin.into()), &reg).is_err());
    }
}

#[test]
fn cushion_color_codec_rejects_non_string_and_noncanonical_names() {
    use super::{Registries, Tag, decoded_component_value};
    use horizon_core::nbt::JavaString;
    let registry = Registries::default();
    assert!(super::DECODED_COMPONENTS.contains(&"minecraft:cushion/color"));
    for value in [
        Tag::End,
        Tag::String("RED".into()),
        Tag::String("silver".into()),
        Tag::String("minecraft:red".into()),
        Tag::String(" red".into()),
        Tag::String("".into()),
        Tag::String(JavaString(vec![0xd800])),
        Tag::String(JavaString(vec![114, 101, 100, 0xd800])),
        Tag::Byte(14),
        Tag::Short(14),
        Tag::Long(14),
        Tag::Float(14.0f32.to_bits()),
        Tag::Double(14.0f64.to_bits()),
        Tag::List(vec![]),
        Tag::Compound(Default::default()),
        Tag::ByteArray(vec![]),
        Tag::IntArray(vec![]),
        Tag::LongArray(vec![]),
    ] {
        assert!(decoded_component_value("minecraft:cushion/color", &value, &registry).is_err());
    }
    for ordinal in -1..=16 {
        assert!(
            decoded_component_value("minecraft:cushion/color", &Tag::Int(ordinal), &registry)
                .is_err()
        );
    }
    let value = Tag::String("light_gray".into());
    assert_eq!(
        decoded_component_value("minecraft:cushion/color", &value, &registry).unwrap(),
        value
    );
}

#[test]
fn tooltip_display_decodes_complete_persistent_nbt_codec() {
    use super::{Registries, Tag, decoded_component_value};
    let mut registry = Registries::default();
    registry
        .by_registry
        .entry("minecraft:data_component_type".into())
        .or_default()
        .ids = vec!["minecraft:damage".into(), "fixture:transient".into()];
    registry
        .persistent_components
        .push("fixture:declared".into());
    let decode = |tag: &Tag| decoded_component_value("minecraft:tooltip_display", tag, &registry);
    for (number, expected) in [
        (Tag::Byte(-1), 1),
        (Tag::Short(0), 0),
        (Tag::Int(2), 1),
        (Tag::Long(1_i64 << 32), 1),
        (Tag::Float(0.5_f32.to_bits()), 1),
        (Tag::Float((-0.0_f32).to_bits()), 0),
        (Tag::Float(f32::NAN.to_bits()), 1),
        (Tag::Double(f64::MIN_POSITIVE.to_bits()), 1),
        (Tag::Double((-0.0_f64).to_bits()), 0),
        (Tag::Double(f64::NAN.to_bits()), 1),
    ] {
        assert_eq!(
            decode(&Tag::compound([("hide_tooltip", number)])).unwrap(),
            Tag::compound([
                ("hide_tooltip", Tag::Byte(expected)),
                ("hidden_components", Tag::List(vec![]))
            ])
        );
    }
    let expected = Tag::compound([
        ("hide_tooltip", Tag::Byte(0)),
        (
            "hidden_components",
            Tag::List(vec![
                Tag::String("minecraft:damage".into()),
                Tag::String("fixture:declared".into()),
                Tag::String("fixture:transient".into()),
            ]),
        ),
    ]);
    assert_eq!(
        decode(&Tag::compound([
            (
                "hidden_components",
                Tag::List(vec![
                    Tag::String("damage".into()),
                    Tag::String(":damage".into()),
                    Tag::String("fixture:declared".into()),
                    Tag::String("fixture:transient".into()),
                ])
            ),
            (
                "ignored",
                Tag::String("record codecs ignore unknown fields".into())
            ),
        ]))
        .unwrap(),
        expected
    );
    for empty in [
        Tag::List(vec![]),
        Tag::ByteArray(vec![]),
        Tag::IntArray(vec![]),
        Tag::LongArray(vec![]),
    ] {
        assert_eq!(
            decode(&Tag::compound([("hidden_components", empty)])).unwrap(),
            Tag::compound([
                ("hide_tooltip", Tag::Byte(0)),
                ("hidden_components", Tag::List(vec![]))
            ])
        );
    }
    for invalid in [
        Tag::Byte(0),
        Tag::compound([("hide_tooltip", Tag::String("true".into()))]),
        Tag::compound([("hide_tooltip", Tag::Compound(Default::default()))]),
        Tag::compound([("hidden_components", Tag::String("damage".into()))]),
        Tag::compound([("hidden_components", Tag::List(vec![Tag::Int(1)]))]),
        Tag::compound([("hidden_components", Tag::ByteArray(vec![1]))]),
        Tag::compound([("hidden_components", Tag::IntArray(vec![1]))]),
        Tag::compound([("hidden_components", Tag::LongArray(vec![1]))]),
        Tag::compound([(
            "hidden_components",
            Tag::List(vec![Tag::String("fixture:unknown".into())]),
        )]),
        Tag::compound([(
            "hidden_components",
            Tag::List(vec![Tag::String("Bad:identifier".into())]),
        )]),
    ] {
        assert!(decode(&invalid).is_err(), "{invalid:?}");
        assert!(decode_component("minecraft:tooltip_display", &invalid, &registry).is_err());
    }
}

#[test]
fn dye_codec_accepts_only_exact_names_under_ordinary_nbt_ops() {
    use super::{Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    // Literal oracle names, independent of the palette used by the decoder.
    for name in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let value = Tag::String(name.into());
        assert_eq!(
            decoded_component_value("minecraft:dye", &value, &registry).unwrap(),
            value
        );
        assert!(
            decode_component(
                "minecraft:dye",
                &Tag::String(name.to_uppercase().as_str().into()),
                &registry
            )
            .is_err()
        );
    }
    for value in [
        Tag::String("silver".into()),
        Tag::String("minecraft:red".into()),
        Tag::String(" red".into()),
        Tag::String("".into()),
        Tag::Byte(0),
        Tag::Byte(14),
        Tag::Short(14),
        Tag::Int(14),
        Tag::Long(14),
        Tag::Float(14.0f32.to_bits()),
        Tag::Double(14.0f64.to_bits()),
        Tag::List(vec![]),
        Tag::Compound(Default::default()),
    ] {
        assert!(decoded_component_value("minecraft:dye", &value, &registry).is_err());
    }
    // File compression does not enable DynamicOps.compressMaps(): even valid
    // dye ordinals are not persistent NbtOps names.
    for ordinal in -1..=16 {
        assert!(decode_component("minecraft:dye", &Tag::Int(ordinal), &registry).is_err());
    }
}

#[test]
fn dye_codec_rejects_non_names_without_lossy_utf16_conversion() {
    use super::{Registries, Tag, decoded_component_value};
    use horizon_core::nbt::JavaString;
    let registry = Registries::default();
    for value in [
        Tag::End,
        Tag::ByteArray(vec![]),
        Tag::IntArray(vec![]),
        Tag::LongArray(vec![]),
        Tag::String(JavaString(vec![0xd800])),
        Tag::String(JavaString(vec![114, 101, 100, 0xd800])),
    ] {
        assert!(decoded_component_value("minecraft:dye", &value, &registry).is_err());
    }
}

#[test]
fn weapon_codec_defaults_numeric_conversions_and_ranges() {
    use super::{Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    let decode = |tag: &Tag| decoded_component_value("minecraft:weapon", tag, &registry);
    let canonical = |damage, seconds: f32| {
        Tag::compound([
            ("item_damage_per_attack", Tag::Int(damage)),
            (
                "disable_blocking_for_seconds",
                Tag::Float(seconds.to_bits()),
            ),
        ])
    };
    assert_eq!(
        decode(&Tag::Compound(Default::default())).unwrap(),
        canonical(1, 0.0)
    );
    for (number, expected) in [
        (Tag::Byte(2), 2),
        (Tag::Short(2), 2),
        (Tag::Long(4_294_967_298), 2),
        (Tag::Double(2.75f64.to_bits()), 2),
        (Tag::Float(f32::NAN.to_bits()), 0),
        (Tag::Double(f64::INFINITY.to_bits()), i32::MAX),
    ] {
        assert_eq!(
            decode(&Tag::compound([
                ("item_damage_per_attack", number),
                (
                    "disable_blocking_for_seconds",
                    Tag::Double(1.25f64.to_bits())
                ),
                ("ignored", Tag::String("anything".into())),
            ]))
            .unwrap(),
            canonical(expected, 1.25)
        );
    }
    for number in [
        Tag::Byte(-1),
        Tag::Long(2_147_483_648),
        Tag::String("1".into()),
    ] {
        assert!(decode(&Tag::compound([("item_damage_per_attack", number)])).is_err());
    }
    for number in [
        Tag::Float((-0.0f32).to_bits()),
        Tag::Float((-1.0f32).to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Double(f64::MAX.to_bits()),
        Tag::String("0".into()),
    ] {
        assert!(decode(&Tag::compound([("disable_blocking_for_seconds", number)])).is_err());
    }
    assert_eq!(
        decode(&Tag::compound([(
            "disable_blocking_for_seconds",
            Tag::Float(f32::MAX.to_bits()),
        )]))
        .unwrap(),
        canonical(1, f32::MAX)
    );
    assert!(decode(&Tag::List(Vec::new())).is_err());
}

use super::decode_component;

#[test]
fn fox_variant_codec_accepts_only_exact_persistent_names() {
    use super::{Registries, Tag, decoded_component_value};
    use horizon_core::nbt::JavaString;
    let registry = Registries::default();
    for name in ["red", "snow"] {
        let value = Tag::String(name.into());
        assert_eq!(
            decoded_component_value("minecraft:fox/variant", &value, &registry),
            Ok(value)
        );
    }
    for value in [
        Tag::Byte(0),
        Tag::Int(1),
        Tag::Double(1.0f64.to_bits()),
        Tag::List(vec![]),
        Tag::compound([] as [(&str, Tag); 0]),
    ] {
        assert_eq!(
            decoded_component_value("minecraft:fox/variant", &value, &registry),
            Err("Not a string".into())
        );
    }
    for name in ["", "RED", "Snow", "minecraft:red", "arctic", "red "] {
        assert_eq!(
            decoded_component_value(
                "minecraft:fox/variant",
                &Tag::String(name.into()),
                &registry
            ),
            Err(format!("Unknown element name:{name}"))
        );
    }
    for units in [vec![0xd800], vec![114, 101, 100, 0xdc00]] {
        assert!(
            decoded_component_value(
                "minecraft:fox/variant",
                &Tag::String(JavaString(units)),
                &registry
            )
            .is_err()
        );
    }
}

#[test]
fn decoded_component_values_use_codec_numbers_and_retain_stored_values() {
    use super::{Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    assert_eq!(
        decoded_component_value(
            "minecraft:damage",
            &Tag::Double(5.0f64.to_bits()),
            &registry
        )
        .unwrap(),
        Tag::Int(5)
    );
    assert_eq!(
        decoded_component_value(
            "minecraft:repair_cost",
            &Tag::Float(2.0f32.to_bits()),
            &registry
        )
        .unwrap(),
        Tag::Int(2)
    );
    assert!(
        decoded_component_value(
            "minecraft:damage",
            &Tag::String("not a number".into()),
            &registry
        )
        .is_err()
    );
    assert_eq!(
        decoded_component_value(
            "minecraft:unbreakable",
            &Tag::compound([] as [(&str, Tag); 0]),
            &registry
        )
        .unwrap(),
        Tag::compound([] as [(&str, Tag); 0])
    );
    assert!(
        decoded_component_value(
            "minecraft:block_entity_data",
            &Tag::String("invalid".into()),
            &registry
        )
        .is_err()
    );
}

#[test]
fn enchantment_maps_validate_entries_against_the_supplied_registry() {
    use super::{Reader, Registries, parse_predicate, parse_stack};
    use crate::args::RegistryIds;
    let mut reg = Registries {
        items: vec!["minecraft:diamond_sword".into()],
        persistent_components: vec![
            "minecraft:enchantments".into(),
            "minecraft:stored_enchantments".into(),
        ],
        ..Default::default()
    };
    reg.by_registry.insert(
        "minecraft:data_component_type".into(),
        RegistryIds {
            ids: reg.persistent_components.clone(),
            tags: vec![],
        },
    );
    reg.by_registry.insert(
        "minecraft:enchantment".into(),
        RegistryIds {
            ids: vec!["minecraft:sharpness".into()],
            tags: vec![],
        },
    );
    for component in ["enchantments", "stored_enchantments"] {
        for value in [
            "{}",
            "{sharpness:1}",
            "{sharpness:255}",
            "{sharpness:1.5f}",
            "{sharpness:1b}",
            "{sharpness:4294967297L}",
            "{sharpness:9007199254740993L}",
            "{\"minecraft:sharpness\":1}",
        ] {
            let input = format!("diamond_sword[{component}={value}]");
            for parser in [parse_stack, parse_predicate] {
                let mut reader = Reader::new(&input);
                assert!(parser(&mut reader, &reg).is_ok(), "{input}");
                assert_eq!(reader.cursor, input.len());
            }
        }
        for value in [
            "{sharpness:-1}",
            "{sharpness:0}",
            "{sharpness:256}",
            "{sharpness:100000}",
            "{sharpness:-0.5d}",
            "{sharpness:\"1\"}",
            "{levels:{sharpness:1}}",
            "{\"audit:missing\":1}",
        ] {
            let input = format!("diamond_sword[{component}={value}]");
            for parser in [parse_stack, parse_predicate] {
                assert!(
                    parser(&mut Reader::new(&input), &reg).is_err(),
                    "{input} crossed command parsing"
                );
            }
        }
    }
}

fn check(snbt: &str) -> Result<(), String> {
    let tag = horizon_core::nbt::parse_snbt(snbt).expect("snbt").0;
    decode_component(
        "minecraft:block_entity_data",
        &tag,
        &super::Registries::default(),
    )
}

#[test]
fn block_entity_data_needs_a_known_type() {
    assert_eq!(check("{id:\"chest\",Lock:1}"), Ok(()));
    assert_eq!(
        check("{Command:\"say\"}"),
        Err("Expected 'id' field in {Command:\"say\"}".into())
    );
    assert_eq!(
        check("{id:\"minecraft:stone\"}"),
        Err("Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: minecraft:stone".into())
    );
}

#[test]
#[ignore = "daily"]
fn block_entity_data_takes_a_type_a_mod_declared() {
    // The declared types are frozen once per process: install them in a fresh one.
    const CHILD: &str = "HORIZON_TEST_DECLARED_BLOCK_ENTITY_DATA";
    if std::env::var_os(CHILD).is_none() {
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "item::tests::block_entity_data_takes_a_type_a_mod_declared",
                    "--nocapture"
                ])
                .env(CHILD, "1")
                .status()
                .unwrap()
                .success()
        );
        return;
    }
    use horizon_client_core::block_entity_types::{self, BlockEntityTypes};
    block_entity_types::install(BlockEntityTypes::new(
        [("example:crate".into(), "example:crate".into())].into(),
        vec!["example:crate".into()],
    ))
    .unwrap();
    assert_eq!(check("{id:\"example:crate\",Tank:1}"), Ok(()));
    assert_eq!(
        check("{id:\"example:nope\"}"),
        Err("Unknown registry key in ResourceKey[minecraft:root / minecraft:block_entity_type]: example:nope".into())
    );
}

#[test]
fn unbreakable_unit_codec_requires_maps_and_discards_unknown_fields() {
    use super::{Compound, Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    let empty = Tag::Compound(Compound::new());
    for value in [
        empty.clone(),
        Tag::compound([
            ("show_in_tooltip", Tag::Byte(0)),
            ("unknown", Tag::List(vec![Tag::Int(7)])),
        ]),
    ] {
        assert_eq!(
            decoded_component_value("minecraft:unbreakable", &value, &registry).unwrap(),
            empty
        );
    }
    for value in [
        Tag::Byte(1),
        Tag::Int(0),
        Tag::String("unit".into()),
        Tag::List(vec![]),
        Tag::IntArray(vec![]),
    ] {
        assert!(decode_component("minecraft:unbreakable", &value, &registry).is_err());
        assert!(decoded_component_value("minecraft:unbreakable", &value, &registry).is_err());
    }
}

#[test]
fn base_color_codec_accepts_only_exact_dye_names() {
    use super::{Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    for name in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let value = Tag::String(name.into());
        assert_eq!(
            decoded_component_value("minecraft:base_color", &value, &registry).unwrap(),
            value
        );
        assert!(decode_component("minecraft:base_color", &value, &registry).is_ok());
        assert!(
            decoded_component_value(
                "minecraft:base_color",
                &Tag::String(name.to_uppercase().as_str().into()),
                &registry
            )
            .is_err()
        );
    }
    for value in [
        "",
        "silver",
        "RED",
        "Red",
        " red",
        "red ",
        "minecraft:red",
        "16",
    ] {
        assert!(
            decoded_component_value(
                "minecraft:base_color",
                &Tag::String(value.into()),
                &registry
            )
            .is_err(),
            "{value}"
        );
    }
    let surrogate = Tag::String(horizon_core::nbt::JavaString(vec![0xD800]));
    assert!(decoded_component_value("minecraft:base_color", &surrogate, &registry).is_err());
    for value in [
        Tag::Byte(0),
        Tag::Short(0),
        Tag::Int(14),
        Tag::Long(14),
        Tag::Float(14.0f32.to_bits()),
        Tag::Double(14.0f64.to_bits()),
        Tag::List(vec![]),
        Tag::Compound(Default::default()),
        Tag::ByteArray(vec![]),
        Tag::IntArray(vec![]),
        Tag::LongArray(vec![]),
    ] {
        assert_eq!(
            decoded_component_value("minecraft:base_color", &value, &registry),
            Err("Not a string".into())
        );
        assert_eq!(
            decode_component("minecraft:base_color", &value, &registry),
            Err("Not a string".into())
        );
    }
}

#[test]
fn note_block_sound_identifier_codec_normalizes_and_validates() {
    use super::{Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    for (input, canonical) in [
        ("unregistered/sound", "minecraft:unregistered/sound"),
        (":unregistered/sound", "minecraft:unregistered/sound"),
        (
            "minecraft:unregistered/sound",
            "minecraft:unregistered/sound",
        ),
        ("fixture:unregistered/sound", "fixture:unregistered/sound"),
        ("", "minecraft:"),
        (":", "minecraft:"),
        ("minecraft:", "minecraft:"),
        ("fixture:", "fixture:"),
        (".:..", ".:.."),
        ("a-z_0.9:/a.._-0", "a-z_0.9:/a.._-0"),
    ] {
        let value = Tag::String(input.into());
        assert_eq!(
            decoded_component_value("minecraft:note_block_sound", &value, &registry),
            Ok(Tag::String(canonical.into())),
            "{input:?}",
        );
        assert!(decode_component("minecraft:note_block_sound", &value, &registry).is_ok());
    }
    for input in [
        "Minecraft:sound",
        "minecraft:Sound",
        "bad namespace:sound",
        "bad/namespace:sound",
        "..:sound",
        "a:b:c",
        "sound with spaces",
        "café",
        "sound\u{0}",
    ] {
        let value = Tag::String(input.into());
        assert!(
            decoded_component_value("minecraft:note_block_sound", &value, &registry).is_err(),
            "{input:?}",
        );
        assert!(decode_component("minecraft:note_block_sound", &value, &registry).is_err());
    }
    for value in [
        Tag::Int(1),
        Tag::compound([] as [(&str, Tag); 0]),
        Tag::List(vec![]),
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        Tag::String(horizon_core::nbt::JavaString(vec![0xdc00])),
        Tag::String(horizon_core::nbt::JavaString(vec![0xd83d, 0xde00])),
    ] {
        assert!(
            decoded_component_value("minecraft:note_block_sound", &value, &registry).is_err(),
            "{value:?}",
        );
    }
}

#[test]
fn tooltip_style_identifier_codec_normalizes_and_rejects_malformed_utf16() {
    use super::{Registries, Tag, decoded_component_value};
    use horizon_core::nbt::JavaString;
    let registry = Registries::default();
    for (raw, expected) in [
        ("", "minecraft:"),
        (":", "minecraft:"),
        (":style", "minecraft:style"),
        ("style", "minecraft:style"),
        ("fixture:", "fixture:"),
        ("fixture:../style", "fixture:../style"),
    ] {
        assert_eq!(
            decoded_component_value(
                "minecraft:tooltip_style",
                &Tag::String(raw.into()),
                &registry
            ),
            Ok(Tag::String(expected.into()))
        );
    }
    for invalid in [
        Tag::Int(1),
        Tag::String("Upper".into()),
        Tag::String("..:style".into()),
        Tag::String("a:b:c".into()),
        Tag::String("a b".into()),
        Tag::String("é".into()),
        Tag::String(JavaString(vec![0xd800])),
        Tag::String(JavaString(vec![0xdc00])),
        Tag::String(JavaString(vec![0xd800, 0xdc00])),
    ] {
        assert!(decoded_component_value("minecraft:tooltip_style", &invalid, &registry).is_err());
        assert!(decode_component("minecraft:tooltip_style", &invalid, &registry).is_err());
    }
}

#[test]
fn waxed_unit_codec_requires_maps_and_discards_unknown_fields() {
    use super::{Compound, Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    for value in [
        Tag::Compound(Compound::new()),
        Tag::compound([("ignored", Tag::String("anything".into()))]),
    ] {
        assert_eq!(
            decoded_component_value("minecraft:waxed", &value, &registry).unwrap(),
            Tag::Compound(Compound::new())
        );
    }
    for value in [
        Tag::Byte(1),
        Tag::Int(0),
        Tag::String("unit".into()),
        Tag::List(vec![]),
    ] {
        assert!(decode_component("minecraft:waxed", &value, &registry).is_err());
        assert!(decoded_component_value("minecraft:waxed", &value, &registry).is_err());
    }
}

#[test]
fn intangible_projectile_unit_codec_requires_map_and_ignores_fields() {
    use super::{Compound, Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    let id = "minecraft:intangible_projectile";
    for value in [
        Tag::Compound(Compound::new()),
        Tag::compound([("unknown", Tag::List(vec![Tag::Int(7)]))]),
    ] {
        assert_eq!(
            decoded_component_value(id, &value, &registry).unwrap(),
            Tag::Compound(Compound::new())
        );
    }
    for value in [
        Tag::Byte(1),
        Tag::Int(0),
        Tag::String("unit".into()),
        Tag::List(vec![]),
    ] {
        assert!(decode_component(id, &value, &registry).is_err());
        assert!(decoded_component_value(id, &value, &registry).is_err());
    }
}

#[test]
fn glider_unit_codec_requires_maps_and_discards_unknown_fields() {
    use super::{Registries, Tag, decoded_component_value};
    let registry = Registries::default();
    let empty = Tag::Compound(Default::default());
    for value in [
        empty.clone(),
        Tag::compound([("ignored", Tag::List(vec![Tag::Int(7)]))]),
    ] {
        assert_eq!(
            decoded_component_value("minecraft:glider", &value, &registry).unwrap(),
            empty
        );
    }
    for value in [
        Tag::Byte(1),
        Tag::Int(0),
        Tag::String("unit".into()),
        Tag::List(vec![]),
        Tag::ByteArray(vec![]),
    ] {
        assert!(decode_component("minecraft:glider", &value, &registry).is_err());
        assert!(decoded_component_value("minecraft:glider", &value, &registry).is_err());
    }
}
