use super::*;

fn types() -> HashMap<String, Arc<MobType>> {
    let kind = crate::mobs::sensor_scan_tests::kind();
    HashMap::from([(kind.id.clone(), kind)])
}

fn input(amplifier: Option<Tag>) -> Tag {
    let mut effect = Tag::compound([
        ("id", string("minecraft:night_vision")),
        ("duration", Tag::Int(200)),
        ("ambient", Tag::Byte(1)),
        ("show_particles", Tag::Byte(0)),
        ("show_icon", Tag::Byte(0)),
    ]);
    if let Some(value) = amplifier {
        put(&mut effect, "amplifier", value);
    }
    Tag::compound([
        ("id", string("minecraft:cow")),
        ("Pos", doubles(&[6.5, 200.0, 6.5])),
        ("Health", Tag::Float(10.0f32.to_bits())),
        ("NoAI", Tag::Byte(1)),
        ("NoGravity", Tag::Byte(1)),
        ("Tags", Tag::List(vec![string("amplifier-control")])),
        ("active_effects", Tag::List(vec![effect])),
    ])
}

fn saved_effect(tag: &Tag) -> &Tag {
    let Some(Tag::List(effects)) = tag.get("active_effects") else {
        panic!("actual effect missing: {tag:?}");
    };
    assert_eq!(effects.len(), 1);
    &effects[0]
}

#[test]
fn decoded_numeric_amplifiers_roundtrip_without_raw_input() {
    let cases = [
        (None, 0),
        (Some(Tag::Byte(0)), 0),
        (Some(Tag::Int(1)), 1),
        (Some(Tag::Byte(127)), 127),
        (Some(Tag::Byte(-128)), 128),
        (Some(Tag::Byte(-1)), 255),
        (Some(Tag::Int(256)), 0),
        (Some(Tag::Int(-1)), 255),
        (Some(Tag::Long(9007199254740993)), 1),
        (Some(Tag::Long(-4294967297)), 255),
        (Some(Tag::Float(1.75f32.to_bits())), 1),
        (Some(Tag::Float((-0.5f32).to_bits())), 0),
        (Some(Tag::Double((-1.75f64).to_bits())), 255),
        (Some(Tag::Double(256.75f64.to_bits())), 0),
        (Some(Tag::Double(1e20f64.to_bits())), 255),
        (Some(Tag::Double((-1e20f64).to_bits())), 0),
    ];
    let types = types();
    for (value, expected) in cases {
        let raw = input(value.clone());
        let mob = mob_from_nbt(&raw, &types, 7, None).unwrap();
        let mut loaded = crate::mobs::StoredEffect::new("minecraft:night_vision", 200, expected);
        loaded.show_icon = false;
        assert_eq!(mob.extra.effects, [loaded], "{value:?}");
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [158, 0, 0, 1], source, None, |_| None);
            let effect = saved_effect(&saved);
            let expected_tag = (expected != 0).then_some(Tag::Byte(expected as u8 as i8));
            assert_eq!(effect.get("amplifier"), expected_tag.as_ref(), "{value:?}");
            assert_eq!(effect.get("duration"), Some(&Tag::Int(200)));
            assert_eq!(effect.get("ambient"), Some(&Tag::Byte(1)));
            assert_eq!(effect.get("show_particles"), Some(&Tag::Byte(0)));
            assert_eq!(effect.get("show_icon"), Some(&Tag::Byte(0)));
            let back = mob_from_nbt(&saved, &types, 8, None).unwrap();
            assert_eq!(back.extra.effects, mob.extra.effects);
        }
    }
}

/// `MobEffectInstance.Details.CODEC`: `show_icon` is its own field, and a saved effect
/// without one takes `show_particles`.
#[test]
fn a_mobs_show_icon_roundtrips_apart_from_show_particles() {
    let types = types();
    let with = |fields: &[(&str, i8)]| {
        let mut raw = input(None);
        let mut effect = Tag::compound([
            ("id", string("minecraft:night_vision")),
            ("duration", Tag::Int(200)),
        ]);
        for (key, value) in fields {
            put(&mut effect, key, Tag::Byte(*value));
        }
        put(&mut raw, "active_effects", Tag::List(vec![effect]));
        raw
    };
    let cases: [(&[(&str, i8)], bool, Option<Tag>); 4] = [
        (&[("show_icon", 0)], false, None),
        (
            &[("show_particles", 0), ("show_icon", 1)],
            true,
            Some(Tag::Byte(0)),
        ),
        (&[("show_particles", 0)], false, Some(Tag::Byte(0))),
        (&[], true, None),
    ];
    for (fields, icon, particles) in cases {
        let mob = mob_from_nbt(&with(fields), &types, 7, None).unwrap();
        assert_eq!(mob.extra.effects[0].show_icon, icon, "{fields:?}");
        let saved = mob_to_nbt(&mob, [158, 0, 0, 1], None, None, |_| None);
        let effect = saved_effect(&saved);
        assert_eq!(effect.get("show_icon"), Some(&Tag::Byte(i8::from(icon))));
        assert_eq!(effect.get("show_particles"), particles.as_ref());
    }
}

#[test]
fn modeled_amplifier_mutation_overrides_stale_raw_and_omits_zero() {
    let raw = input(Some(Tag::Byte(12)));
    let mut mob = mob_from_nbt(&raw, &types(), 7, None).unwrap();
    for amplifier in [0, 128, 255, 1] {
        mob.extra.effects[0].amplifier = amplifier;
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [158, 0, 0, 2], source, None, |_| None);
            let expected = (amplifier != 0).then_some(Tag::Byte(amplifier as u8 as i8));
            assert_eq!(saved_effect(&saved).get("amplifier"), expected.as_ref());
            assert_eq!(saved.get("Health"), Some(&Tag::Float(10.0f32.to_bits())));
            assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
            if source.is_some() {
                assert_eq!(saved.get("Tags"), raw.get("Tags"));
            }
        }
    }
}

/// `MobEffectInstance.Details.CODEC`: a `hidden_effect` is another `Details`, so the weaker
/// instance's amplifier is the same unsigned byte as the active one's, and 200 (`-56b`)
/// loads back as 200, not as 0 (level I).
#[test]
fn a_hidden_effects_amplifier_roundtrips_as_an_unsigned_byte() {
    let types = types();
    let cases = [
        (Tag::Byte(3), 3),
        (Tag::Byte(-128), 128),
        (Tag::Byte(-56), 200),
        (Tag::Byte(-1), 255),
        (Tag::Int(256), 0),
    ];
    for (value, expected) in cases {
        let mut raw = input(None);
        let effect = Tag::compound([
            ("id", string("minecraft:night_vision")),
            ("amplifier", Tag::Byte(-1)),
            ("duration", Tag::Int(200)),
            (
                "hidden_effect",
                Tag::compound([("duration", Tag::Int(400)), ("amplifier", value.clone())]),
            ),
        ]);
        put(&mut raw, "active_effects", Tag::List(vec![effect]));
        let mob = mob_from_nbt(&raw, &types, 7, None).unwrap();
        assert_eq!(mob.extra.effect_chains.len(), 1, "{value:?}");
        assert_eq!(mob.extra.effect_chains[0].amplifier, expected, "{value:?}");
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [158, 0, 0, 1], source, None, |_| None);
            let hidden = saved_effect(&saved).get("hidden_effect").unwrap();
            let expected_tag = (expected != 0).then_some(Tag::Byte(expected as u8 as i8));
            assert_eq!(hidden.get("amplifier"), expected_tag.as_ref(), "{value:?}");
            let back = mob_from_nbt(&saved, &types, 8, None).unwrap();
            assert_eq!(
                back.extra.effect_chains, mob.extra.effect_chains,
                "{value:?}"
            );
        }
    }
}
