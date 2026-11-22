use super::*;

/// `PrimedTnt.readAdditionalSaveData`: `explosion_power` defaults to 4 and
/// is clamped to 0..128 both ways, so a summoned `explosion_power:200` is
/// 128 and one that is absent is ordinary TNT.
#[test]
fn explosion_power_is_read_clamped_and_defaulted() {
    let with = |power: Option<f32>| {
        let mut map = std::collections::BTreeMap::new();
        if let Some(p) = power {
            map.insert("explosion_power".into(), Tag::Float(p.to_bits()));
        }
        Tag::Compound(map)
    };
    assert_eq!(tnt_power(&with(None)), 4.0);
    assert_eq!(tnt_power(&with(Some(10.0))), 10.0);
    assert_eq!(tnt_power(&with(Some(200.0))), 128.0);
    assert_eq!(tnt_power(&with(Some(-3.0))), 0.0);
    // Any numeric tag type counts, as with `getFloatOr`.
    let numeric = |tag: Tag| {
        let mut map = std::collections::BTreeMap::new();
        map.insert("explosion_power".into(), tag);
        Tag::Compound(map)
    };
    assert_eq!(tnt_power(&numeric(Tag::Int(50))), 50.0);
    assert_eq!(tnt_power(&numeric(Tag::Byte(7))), 7.0);
    assert_eq!(tnt_power(&numeric(Tag::Short(9))), 9.0);
    assert_eq!(tnt_power(&numeric(Tag::Long(500))), 128.0);
    assert_eq!(tnt_power(&numeric(Tag::Double(12.5f64.to_bits()))), 12.5);
    // And it round-trips through the save, which writes it only when it is
    // not the default.
    let saved = tnt_to_nbt(
        Vec3::ZERO,
        Vec3::ZERO,
        false,
        0.0,
        20,
        10.0,
        string("minecraft:tnt"),
        [0.0, 0.0],
        [1, 2, 3, 4],
        None,
        None,
    );
    assert_eq!(tnt_power(&saved), 10.0);
    let plain = tnt_to_nbt(
        Vec3::ZERO,
        Vec3::ZERO,
        false,
        0.0,
        20,
        4.0,
        string("minecraft:tnt"),
        [0.0, 0.0],
        [1, 2, 3, 4],
        None,
        None,
    );
    assert!(plain.get("explosion_power").is_none());
}

#[test]
fn saved_power_uses_the_model_and_omits_stale_defaults() {
    let block = string("minecraft:stone");
    let save = |power, raw| {
        tnt_to_nbt(
            Vec3::new(1.5, 200.0, 4.5),
            Vec3::ZERO,
            false,
            0.0,
            80,
            power,
            block.clone(),
            [0.0; 2],
            [91, 0, 0, 1],
            Some([91, 1, 0, 90]),
            raw,
        )
    };
    let raw_absent = Tag::compound([("Silent", Tag::Byte(1))]);
    let raw_default = Tag::compound([
        ("Silent", Tag::Byte(1)),
        ("explosion_power", Tag::Float(4.0_f32.to_bits())),
    ]);
    let raw_stale = Tag::compound([
        ("Silent", Tag::Byte(1)),
        ("explosion_power", Tag::Float(2.0_f32.to_bits())),
    ]);
    for raw in [
        None,
        Some(&raw_absent),
        Some(&raw_default),
        Some(&raw_stale),
    ] {
        let saved = save(4.0, raw);
        assert!(saved.get("explosion_power").is_none(), "stale raw {raw:?}");
        assert_eq!(saved.get("fuse"), Some(&Tag::Short(80)));
        assert_eq!(saved.get("block_state"), Some(&block));
        assert_eq!(saved.get("owner"), Some(&Tag::IntArray(vec![91, 1, 0, 90])));
        assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(vec![91, 0, 0, 1])));
        if raw.is_some() {
            assert_eq!(saved.get("Silent"), Some(&Tag::Byte(1)));
        }
    }
    for power in [0.0, 2.0, 5.0, 128.0] {
        for raw in [None, Some(&raw_default), Some(&raw_stale)] {
            assert_eq!(
                save(power, raw).get("explosion_power"),
                Some(&Tag::Float(power.to_bits()))
            );
        }
    }
}

/// `EntityReference.store(owner, output, "owner")`: the igniter's UUID as
/// an int array, read back by `EntityReference.read`; an owner the save
/// could not name keeps the loaded one from the raw compound.
/// A dragon fireball saved with no raw compound: its own id, UUID,
/// motion, power and owner, no fireball item or explosion power, and no
/// fire (`DragonFireball.shouldBurn` is false); it loads back as itself.
#[test]
fn a_dragon_fireball_saves_and_loads_as_itself_without_raw_data() {
    let mut p = Projectile::new(
        ProjectileKind::DragonFireball,
        Vec3::new(1.5, 70.0, -2.5),
        None,
    );
    p.velocity = Vec3::new(0.1, 0.0, 0.0);
    p.acceleration = 0.1;
    p.age = 3;
    let tag = fireball_to_nbt(&p, [1, 2, 3, 4], Some([5, 6, 7, 8]), None).unwrap();
    assert_eq!(kind(&tag).as_deref(), Some("minecraft:dragon_fireball"));
    assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
    assert_eq!(uuid_from_tag(tag.get("Owner")), Some([5, 6, 7, 8]));
    assert_eq!(float(tag.get("acceleration_power")), Some(0.1));
    for absent in ["ExplosionPower", "Item"] {
        assert!(tag.get(absent).is_none(), "{absent}");
    }
    // Entity.saveWithoutId always writes Fire; a fireball that does not
    // burn is never ignited (an item fireball saves at least 20).
    assert!(
        number(tag.get("Fire")).unwrap_or(0) <= 0,
        "{:?}",
        tag.get("Fire")
    );
    let back = fireball_from_nbt(&tag, None).unwrap();
    assert_eq!(back.kind, ProjectileKind::DragonFireball);
    assert_eq!(back.velocity, p.velocity);
    assert_eq!(back.owner_uuid, Some([5, 6, 7, 8]));
    assert!(saved_with_chunk(&back));
}
#[test]
fn the_owner_round_trips_as_a_uuid() {
    let save = |owner, raw: Option<&Tag>| {
        let t = string("minecraft:tnt");
        tnt_to_nbt(
            Vec3::ZERO,
            Vec3::ZERO,
            false,
            0.0,
            80,
            4.0,
            t,
            [0.0; 2],
            [1; 4],
            owner,
            raw,
        )
    };
    let saved = save(Some([5, 6, 7, 8]), None);
    assert_eq!(saved.get("owner"), Some(&Tag::IntArray(vec![5, 6, 7, 8])));
    assert_eq!(tnt_owner(&saved), Some([5, 6, 7, 8]));
    assert_eq!(tnt_owner(&save(None, None)), None);
    assert_eq!(tnt_owner(&save(None, Some(&saved))), Some([5, 6, 7, 8]));
    assert_eq!(tnt_owner(&save(Some([9; 4]), Some(&saved))), Some([9; 4]));
}
