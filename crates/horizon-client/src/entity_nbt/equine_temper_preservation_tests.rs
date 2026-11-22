use super::*;
use crate::mobs::Species;

#[test]
fn temper_preservation_decodes_and_saves_without_raw_input() {
    for (name, species) in [
        ("llama", Species::Llama),
        ("trader_llama", Species::TraderLlama),
        ("horse", Species::Horse),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = format!("minecraft:{name}");
        def.species = species;
        let types = [(def.id.clone(), kind.clone())].into();
        for value in [50, 0, 30, 100, -7, 1000, i32::MIN, i32::MAX] {
            let raw = Tag::compound([
                ("id", string(&format!("minecraft:{name}"))),
                ("Pos", doubles(&[1.5, 200.0, 1.5])),
                ("Temper", Tag::Int(value)),
                ("Tame", Tag::Byte(1)),
                ("NoAI", Tag::Byte(1)),
                ("Strength", Tag::Int(3)),
                ("ChestedHorse", Tag::Byte(1)),
            ]);
            let m = mob_from_nbt(&raw, &types, 7, None).unwrap();
            assert_eq!(m.species(), species);
            assert!(m.extra.tame && m.no_ai);
            let stale = Tag::compound([("Temper", Tag::Int(17)), ("audit:unrelated", Tag::Int(9))]);
            for base in [None, Some(&raw), Some(&stale)] {
                let saved = mob_to_nbt(&m, [93, 0, 0, 1], base, None, |_| None);
                assert_eq!(
                    saved.get("Temper"),
                    Some(&Tag::Int(value)),
                    "{name}: loaded temper is not a gameplay limit"
                );
                assert_eq!(saved.get("Tame"), Some(&Tag::Byte(1)));
                if species != Species::Horse {
                    assert_eq!(saved.get("Strength"), Some(&Tag::Int(3)));
                    assert_eq!(saved.get("ChestedHorse"), Some(&Tag::Byte(1)));
                }
                if base == Some(&stale) {
                    assert_eq!(saved.get("audit:unrelated"), Some(&Tag::Int(9)));
                }
            }
        }
        let fresh = Mob::new(7, kind, Vec3::ZERO, 0.0);
        let saved = mob_to_nbt(&fresh, [93, 0, 0, 1], None, None, |_| None);
        assert_eq!(saved.get("Temper"), Some(&Tag::Int(0)));
        let absent = Tag::compound([
            ("id", string(&format!("minecraft:{name}"))),
            ("Pos", doubles(&[1.5, 200.0, 1.5])),
        ]);
        let decoded = mob_from_nbt(&absent, &types, 7, None).unwrap();
        assert_eq!(
            mob_to_nbt(&decoded, [0; 4], None, None, |_| None).get("Temper"),
            Some(&Tag::Int(0))
        );
    }
}

#[test]
fn equine_and_ravager_attributes_parity() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    Arc::make_mut(&mut kind).species = Species::Donkey;
    let types = [("minecraft:donkey".into(), kind.clone())].into();
    let donkey = mob_from_nbt(
        &Tag::compound([
            ("id", string("minecraft:donkey")),
            ("Pos", doubles(&[0.0, 0.0, 0.0])),
        ]),
        &types,
        1,
        None,
    )
    .unwrap();
    assert_eq!(donkey.extra.vector[0], 0.5);
    assert_eq!(
        mob_to_nbt(&donkey, [0; 4], None, None, |_| None).get("attributes"),
        Some(&Tag::List(vec![]))
    );
    Arc::make_mut(&mut kind).species = Species::Ravager;
    let types = [("minecraft:ravager".into(), kind)].into();
    // Before its first aiStep a roaring ravager has no movement_speed
    // instance; a saved base (mid-ramp after a roar) loads back as is.
    let speed = Tag::List(vec![Tag::compound([
        ("id", string("minecraft:movement_speed")),
        ("base", Tag::Double(0.10317f64.to_bits())),
    ])]);
    for (key, value, saved) in [
        ("RoarTick", Tag::Int(10), Tag::List(vec![])),
        ("attributes", speed.clone(), speed),
    ] {
        let pos = ("Pos", doubles(&[0.0, 0.0, 0.0]));
        let tag = Tag::compound([("id", string("minecraft:ravager")), pos, (key, value)]);
        let ravager = mob_from_nbt(&tag, &types, 2, None).unwrap();
        let nbt = mob_to_nbt(&ravager, [0; 4], None, None, |_| None);
        assert_eq!(nbt.get("attributes"), Some(&saved));
    }
}
