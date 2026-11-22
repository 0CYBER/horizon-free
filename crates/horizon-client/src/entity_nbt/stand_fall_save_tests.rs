use super::*;

/// `Entity.load` reads `Fire` with `getShortOr`: an int tag is narrowed
/// (`(short)(value & 65535)`), so 40000 is -25536 and the stand or mob
/// does not burn.
#[test]
fn a_stand_and_a_mob_narrow_a_loaded_fire_to_a_short() {
    let kind = crate::mobs::sensor_scan_tests::kind();
    let types = [(kind.id.to_string(), kind.clone())].into();
    for (fire, expected) in [
        (Tag::Int(40000), -25536),
        (Tag::Int(65546), 10),
        (Tag::Short(60), 60),
    ] {
        let input = Tag::compound([
            ("id", string(&kind.id)),
            ("Pos", doubles(&[4.5, 205.0, 4.5])),
            ("Fire", fire),
        ]);
        assert_eq!(
            armor_stand_from_nbt(&input, 1).unwrap().fire_ticks,
            expected
        );
        assert_eq!(
            mob_from_nbt(&input, &types, 1, None).unwrap().fire_ticks,
            expected
        );
    }
}

/// `LargeFireball.readAdditionalSaveData` reads `ExplosionPower` with
/// `getByteOr`: an int tag is narrowed (`(byte)(value & 255)`), so 300 is
/// 44 and 200 is -56.
#[test]
fn a_fireball_narrows_its_explosion_power_to_a_byte() {
    for (saved, expected) in [(Tag::Int(300), 44), (Tag::Int(200), -56), (Tag::Byte(4), 4)] {
        let input = Tag::compound([
            ("id", string("minecraft:fireball")),
            ("Pos", doubles(&[4.5, 205.0, 4.5])),
            ("ExplosionPower", saved),
        ]);
        let fireball = fireball_from_nbt(&input, None).unwrap();
        assert!(
            matches!(fireball.kind, ProjectileKind::LargeFireball { power } if power == expected),
            "{expected}"
        );
    }
}

#[test]
fn decoded_stand_fall_distance_survives_save_without_raw_or_with_stale_raw() {
    let stale = Tag::compound([("fall_distance", Tag::Double(99.0_f64.to_bits()))]);
    for no_gravity in [false, true] {
        for distance in [0.0_f64, 1.25, 17.5] {
            let input = Tag::compound([
                ("Pos", doubles(&[4.5, 205.0, 4.5])),
                ("NoGravity", Tag::Byte(i8::from(no_gravity))),
                ("fall_distance", Tag::Double(distance.to_bits())),
            ]);
            let stand = armor_stand_from_nbt(&input, 1).unwrap();
            for raw in [None, Some(&stale)] {
                let saved = armor_stand_to_nbt(&stand, [28, 0, 0, 1], raw);
                assert_eq!(
                    saved.get("fall_distance"),
                    Some(&Tag::Double(distance.to_bits())),
                    "distance={distance} no_gravity={no_gravity} raw={}",
                    raw.is_some()
                );
                assert_eq!(saved.get("Pos"), input.get("Pos"));
                assert_eq!(stand.no_gravity, no_gravity);
            }
        }
    }
}

/// `Entity.saveWithoutId`'s common fields are filled in when a raw compound
/// (a marker's) lacks them, and explicit values win.
#[test]
fn marker_common_fields_default_and_keep_explicit_values() {
    let mut tag = Tag::compound([("Air", Tag::Short(10)), ("OnGround", Tag::Byte(1))]);
    fill_common_defaults(&mut tag);
    assert_eq!(tag.get("Air"), Some(&Tag::Short(10)));
    assert_eq!(tag.get("OnGround"), Some(&Tag::Byte(1)));
    assert_eq!(tag.get("Invulnerable"), Some(&Tag::Byte(0)));
    assert_eq!(tag.get("PortalCooldown"), Some(&Tag::Int(0)));
    assert_eq!(
        tag.get("fall_distance"),
        Some(&Tag::Double(0.0f64.to_bits()))
    );
    assert_eq!(tag.get("Motion"), Some(&doubles(&[0.0, 0.0, 0.0])));
    // Anything but a compound is left untouched.
    let mut plain = Tag::String("x".into());
    fill_common_defaults(&mut plain);
    assert_eq!(plain, Tag::String("x".into()));
}
