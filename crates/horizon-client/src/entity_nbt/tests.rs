/// `ItemEntity.readAdditionalSaveData` reads `Age` and `PickupDelay` with
/// `getShortOr`: an int tag is narrowed (`IntTag.shortValue`,
/// `(short)(value & 65535)`), so 40000 is -25536 and 65536 is 0.
#[test]
fn an_item_narrows_its_age_and_pickup_delay_to_a_short() {
    let load = |fields: &str| {
        let snbt = format!("{{Pos:[0d,0d,0d],Item:{{id:\"minecraft:stone\",count:1}},{fields}}}");
        let tag = horizon_core::nbt::parse_snbt(&snbt).unwrap().0;
        let item = super::item_entity_from_nbt(&tag, 1, 0.0).unwrap();
        (item.age, item.pickup_delay)
    };
    assert_eq!(load("Age:40000,PickupDelay:65546"), (-25536, 10));
    assert_eq!(load("Age:65536,PickupDelay:40000"), (0, -25536));
    // What vanilla writes itself reads back as it was.
    assert_eq!(load("Age:-32768s,PickupDelay:32767s"), (-32768, 32767));
    assert_eq!(load("Age:5999s,PickupDelay:10s"), (5999, 10));
    // A float or double floors first (`Mth.floor`), as `Health` does.
    assert_eq!(load("Age:-0.5f,PickupDelay:9.9d"), (-1, 9));
    // The sentinels `ItemEntity.tick` tests for (`age != -32768`: never
    // ages; `pickupDelay != 32767`: never counts down), given as ints,
    // and the ints that narrow onto them.
    assert_eq!(load("Age:-32768,PickupDelay:32767"), (-32768, 32767));
    assert_eq!(load("Age:32768,PickupDelay:98303"), (-32768, 32767));
    // `Entity.load` reads the item's `Fire` with `getShortOr` as well.
    let fire = |fields: &str| {
        let snbt = format!("{{Pos:[0d,0d,0d],Item:{{id:\"minecraft:stone\",count:1}},{fields}}}");
        let tag = horizon_core::nbt::parse_snbt(&snbt).unwrap().0;
        super::item_entity_from_nbt(&tag, 1, 0.0)
            .unwrap()
            .fire_ticks
    };
    assert_eq!(
        [fire("Fire:40000"), fire("Fire:60s"), fire("Age:1s")],
        [-25536, 60, 0]
    );
    // They save as the shorts vanilla writes.
    let tag = horizon_core::nbt::parse_snbt(
        "{Pos:[0d,0d,0d],Item:{id:\"minecraft:stone\",count:1},Age:32768,PickupDelay:98303}",
    )
    .unwrap()
    .0;
    let item = super::item_entity_from_nbt(&tag, 1, 0.0).unwrap();
    let saved = super::item_entity_to_nbt(&item, [1; 4], None);
    assert_eq!(saved.get("Age"), Some(&super::Tag::Short(i16::MIN)));
    assert_eq!(saved.get("PickupDelay"), Some(&super::Tag::Short(i16::MAX)));
}

// Added after the actual old Game proof: live mutation and a fresh absent
// load cannot inherit a predicate from a contradictory saved compound.
#[test]
fn arrow_can_break_mutation_clone_and_absent_load_save_actual_state() {
    let parse = |text: &str| horizon_core::nbt::parse_snbt(text).unwrap().0;
    let raw = parse(
        "{id:\"minecraft:arrow\",Pos:[1d,200d,1d],NoGravity:1b,marker:37,can_break:{blocks:\"minecraft:chorus_flower\"}}",
    );
    let (mut arrow, _) = super::arrow_from_nbt(&raw, None, |_| None).unwrap();
    let original = raw.get("can_break").unwrap().clone();
    assert_eq!(
        arrow.arrow.as_ref().unwrap().can_break.as_ref(),
        Some(&original)
    );
    let replacement = parse("{blocks:\"minecraft:stone\"}");
    arrow.arrow.as_mut().unwrap().can_break = Some(replacement.clone());
    let cloned = arrow.clone();
    for source in [None, Some(&raw)] {
        let saved = super::arrow_to_nbt(&cloned, [197, 0, 0, 1], None, None, source).unwrap();
        assert_eq!(saved.get("can_break"), Some(&replacement));
        assert_eq!(saved.get("NoGravity"), Some(&super::Tag::Byte(1)));
    }
    arrow.arrow.as_mut().unwrap().can_break = None;
    let saved = super::arrow_to_nbt(&arrow, [197, 0, 0, 1], None, None, Some(&raw)).unwrap();
    assert!(saved.get("can_break").is_none());
    assert_eq!(saved.get("marker"), Some(&super::Tag::Int(37)));
    let absent = parse("{id:\"minecraft:arrow\",Pos:[1d,200d,1d]}");
    let (fresh, _) = super::arrow_from_nbt(&absent, None, |_| None).unwrap();
    assert!(fresh.arrow.as_ref().unwrap().can_break.is_none());
    assert!(
        super::arrow_to_nbt(&fresh, [197, 0, 0, 2], None, None, Some(&raw))
            .unwrap()
            .get("can_break")
            .is_none()
    );
}

/// `EvokerFangs` extends `Entity` and never moves: `Entity.saveWithoutId`
/// writes the `fall_distance` it loaded, and 0 for fangs a spell made.
#[test]
fn evoker_fangs_keep_their_loaded_fall_distance() {
    let raw = horizon_core::nbt::parse_snbt(
        "{id:\"minecraft:evoker_fangs\",Pos:[1d,200d,1d],fall_distance:5.0d,Warmup:100}",
    )
    .unwrap()
    .0;
    let at = super::Vec3::new(1.0, 200.0, 1.0);
    let fall = |raw: Option<&super::Tag>| match super::evoker_fangs_to_nbt(
        at, 0.0, 99, None, [1; 4], raw, None,
    )
    .get("fall_distance")
    {
        Some(super::Tag::Double(bits)) => f64::from_bits(*bits),
        other => panic!("no fall_distance: {other:?}"),
    };
    assert_eq!(fall(Some(&raw)), 5.0);
    assert_eq!(fall(None), 0.0);
}

/// `Shulker` reads `AttachFace` as a byte before
/// `Direction.from3DDataValue` (`abs(data % 6)`): an int is narrowed
/// first, so 258 is byte 2, not 258 % 6 = 0, and 250 is byte -6, face 0.
#[test]
fn a_shulkers_attach_face_is_narrowed_to_a_byte_first() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:shulker".into();
    ty.species = crate::mobs::Species::Shulker;
    let types = [("minecraft:shulker".into(), kind)].into();
    let face = |saved: Tag| {
        let raw = Tag::compound([
            ("id", string("minecraft:shulker")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("AttachFace", saved),
        ]);
        mob_from_nbt(&raw, &types, 1, None).unwrap().extra.variant3
    };
    assert_eq!(face(Tag::Byte(3)), 3);
    assert_eq!(face(Tag::Byte(-2)), 2);
    assert_eq!(face(Tag::Int(258)), 2);
    assert_eq!(face(Tag::Int(250)), 0);
}
use super::*;
use crate::items::ItemStack;

/// Older worlds name their single player by the offline UUID; the migration rewrites
/// every reference to it (owner, thrower, trusted list, anger, leash) and nothing else.
#[test]
fn a_players_offline_uuid_becomes_the_accounts_everywhere_in_an_entity() {
    let old = [1, 2, 3, 4];
    let new = [5, 6, 7, 8];
    let mut tag = Tag::compound([
        ("id", string("minecraft:wolf")),
        ("UUID", Tag::IntArray(vec![9, 9, 9, 9])),
        ("Owner", Tag::IntArray(old.to_vec())),
        ("Thrower", Tag::IntArray(old.to_vec())),
        ("angry_at", Tag::IntArray(old.to_vec())),
        (
            "Trusted",
            Tag::List(vec![
                Tag::IntArray(old.to_vec()),
                Tag::IntArray(vec![3, 3, 3, 3]),
            ]),
        ),
        (
            "leash",
            Tag::compound([("UUID", Tag::IntArray(old.to_vec()))]),
        ),
        ("Health", Tag::Float(20.0f32.to_bits())),
    ]);
    migrate_player_uuid(&mut tag, old, new);
    assert_eq!(tag.get("Owner"), Some(&Tag::IntArray(new.to_vec())));
    assert_eq!(tag.get("Thrower"), Some(&Tag::IntArray(new.to_vec())));
    assert_eq!(tag.get("angry_at"), Some(&Tag::IntArray(new.to_vec())));
    assert_eq!(
        tag.get("Trusted"),
        Some(&Tag::List(vec![
            Tag::IntArray(new.to_vec()),
            Tag::IntArray(vec![3, 3, 3, 3]),
        ]))
    );
    assert_eq!(
        tag.get("leash").and_then(|leash| leash.get("UUID")),
        Some(&Tag::IntArray(new.to_vec()))
    );
    // The entity's own UUID is not the player's.
    assert_eq!(tag.get("UUID"), Some(&Tag::IntArray(vec![9, 9, 9, 9])));
}

/// An entity that never moves (an `AreaEffectCloud`) saves the
/// `fall_distance` `Entity.load` read, and 0 when it was made in play.
#[test]
fn a_base_tag_keeps_the_loaded_fall_distance() {
    let raw = horizon_core::nbt::parse_snbt(
        "{id:\"minecraft:area_effect_cloud\",Pos:[1d,200d,1d],fall_distance:5.0d}",
    )
    .unwrap()
    .0;
    let fall = |raw: Option<&super::Tag>| {
        let tag = super::entity_base_tag(
            "minecraft:area_effect_cloud",
            super::Vec3::new(1.0, 200.0, 1.0),
            super::Vec3::ZERO,
            [0.0, 0.0],
            [1; 4],
            raw,
        );
        match tag.get("fall_distance") {
            Some(super::Tag::Double(bits)) => f64::from_bits(*bits),
            other => panic!("no fall_distance: {other:?}"),
        }
    };
    assert_eq!(fall(Some(&raw)), 5.0);
    assert_eq!(fall(None), 0.0);
}

#[test]
fn empty_saved_tags_are_omitted_without_changing_raw_or_nonempty_tags() {
    let base = Base {
        id: "minecraft:pig",
        uuid: [328, 0, 0, 1],
        position: Vec3::new(4.5, 200.0, 4.5),
        motion: Vec3::ZERO,
        rotation: [0.0; 2],
        on_ground: false,
        fall_distance: 0.0,
    };
    let raw = |tags| {
        Tag::compound([
            ("Tags", Tag::List(tags)),
            ("PortalCooldown", Tag::Int(17)),
            ("Invulnerable", Tag::Byte(1)),
        ])
    };
    let fresh = base.tag(None);
    assert!(fresh.get("Tags").is_none());
    assert_eq!(fresh.get("PortalCooldown"), Some(&Tag::Int(0)));
    for tags in [vec![string("keep")], vec![]] {
        let raw = raw(tags.clone());
        let saved = base.tag(Some(&raw));
        assert_eq!(saved.get("PortalCooldown"), Some(&Tag::Int(17)));
        assert_eq!(saved.get("Invulnerable"), Some(&Tag::Byte(1)));
        assert_eq!(raw.get("Tags"), Some(&Tag::List(tags.clone())));
        assert_eq!(
            saved.get("Tags"),
            if tags.is_empty() {
                None
            } else {
                Some(Tag::List(tags))
            }
            .as_ref()
        );
    }
}

#[test]
fn conversion_keeps_only_the_old_mobs_command_tags() {
    let tags = Tag::List(vec![
        Tag::String("subject".into()),
        Tag::String("quest".into()),
    ]);
    let raw = Tag::compound([
        ("id", Tag::String("minecraft:husk".into())),
        ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
        ("HuskConversionTime", Tag::Int(1)),
        ("Tags", tags.clone()),
    ]);
    let original = raw.clone();
    assert_eq!(
        conversion_tags(Some(&raw)),
        Some(Tag::compound([("Tags", tags)]))
    );
    assert_eq!(raw, original);
    assert_eq!(conversion_tags(None), None);
    assert_eq!(
        conversion_tags(Some(&Tag::compound([("Tags", Tag::List(Vec::new()))]))),
        None
    );
    assert_eq!(
        conversion_tags(Some(&Tag::Compound(Default::default()))),
        None
    );
}

#[test]
fn an_ageable_mob_that_cannot_be_a_baby_saves_no_age() {
    use crate::mobs::Species;
    // AgeableMob.addAdditionalSaveData writes Age only when canBeABaby is
    // true, so every ageable class that overrides it to false writes none
    // of it: a frog and a camel husk keep no raw `Age` either.
    for (species, id) in [
        (Species::Slime, "minecraft:slime"),
        (Species::MagmaCube, "minecraft:magma_cube"),
        (Species::Frog, "minecraft:frog"),
        (Species::CamelHusk, "minecraft:camel_husk"),
        (Species::Pig, "minecraft:pig"),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.species = species;
        ty.id = id.into();
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.age = -37;
        let expected = (species == Species::Pig).then_some(Tag::Int(-37));
        for age in [None, Some(0), Some(-24000), Some(37)] {
            let mut raw = Tag::compound([("h228_control", Tag::Int(42))]);
            if let Some(age) = age
                && let Tag::Compound(map) = &mut raw
            {
                map.insert("Age".into(), Tag::Int(age));
            }
            let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
            assert_eq!(saved.get("Age"), expected.as_ref(), "{species:?}, {age:?}");
            assert_eq!(saved.get("h228_control"), Some(&Tag::Int(42)));
        }
        let saved = mob_to_nbt(&mob, [0; 4], None, None, |_| None);
        assert_eq!(
            saved.get("Age"),
            expected.as_ref(),
            "{species:?}, no raw tag"
        );
    }
}

#[test]
fn zombie_nautilus_save_omits_ignored_age_fields() {
    use crate::mobs::Species;
    for (id, species) in [
        ("minecraft:zombie_nautilus", Species::ZombieNautilus),
        ("minecraft:nautilus", Species::Nautilus),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = id.into();
        def.species = species;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.age = -123;
        mob.extra.forced_age = 19;
        mob.extra.age_locked = true;
        let raw = Tag::compound([
            ("Age", Tag::Int(37)),
            ("ForcedAge", Tag::Int(99)),
            ("AgeLocked", Tag::Byte(0)),
            ("audit:unrelated", Tag::Int(7)),
        ]);
        for base in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [0; 4], base, None, |_| None);
            for (key, expected) in [
                ("Age", Tag::Int(-123)),
                ("ForcedAge", Tag::Int(19)),
                ("AgeLocked", Tag::Byte(1)),
            ] {
                if species == Species::ZombieNautilus {
                    assert!(saved.get(key).is_none(), "{id}: {key}");
                } else {
                    assert_eq!(saved.get(key), Some(&expected), "{id}: {key}");
                }
            }
            if base.is_some() {
                assert_eq!(saved.get("audit:unrelated"), Some(&Tag::Int(7)));
            }
        }
    }
}

/// `Horse.getVariant` is `Variant.byId(typeVariant & 255)` and
/// `getMarkings` `Markings.byId((typeVariant & 0xFF00) >> 8)`; both maps
/// are `OutOfBoundsStrategy.WRAP` (7 coats, 5 markings), so 9 is coat 2
/// and markings 6 is 1.
#[test]
fn a_horses_unknown_coat_and_markings_wrap() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:horse".into();
    ty.species = crate::mobs::Species::Horse;
    let types = [("minecraft:horse".into(), kind)].into();
    let look = |saved: i32| {
        let raw = Tag::compound([
            ("id", string("minecraft:horse")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("Variant", Tag::Int(saved)),
        ]);
        let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
        (m.extra.variant, m.extra.variant2)
    };
    assert_eq!(look(6 | 4 << 8), (6, 4));
    assert_eq!(look(9), (2, 0));
    assert_eq!(look(6 << 8), (0, 1));
    assert_eq!(look(255 | 255 << 8), (3, 0));
    // `Horse.addAdditionalSaveData` writes the loaded int back as it was;
    // a horse whose look changed since saves the two ids.
    let saved = |loaded: i32, change: Option<u8>| {
        let raw = Tag::compound([
            ("id", string("minecraft:horse")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("Variant", Tag::Int(loaded)),
        ]);
        let mut m = mob_from_nbt(&raw, &types, 1, None).unwrap();
        if let Some(coat) = change {
            m.extra.variant = coat;
        }
        number(mob_to_nbt(&m, [0; 4], Some(&raw), None, |_| None).get("Variant"))
    };
    assert_eq!(saved(9, None), Some(9));
    assert_eq!(saved(6 << 8 | 0x7_0000, None), Some(6 << 8 | 0x7_0000));
    assert_eq!(saved(9, Some(5)), Some(5));
}

/// `LivingEntity.readAdditionalSaveData` reads `HurtTime` and `DeathTime`
/// with `getShortOr`: an int tag is narrowed (`(short)(value & 65535)`),
/// so 40000 is -25536 and never counts down.
#[test]
fn a_mob_narrows_its_hurt_and_death_time_to_a_short() {
    let kind = crate::mobs::sensor_scan_tests::kind();
    let types = [(kind.id.to_string(), kind.clone())].into();
    let load = |hurt: Tag, death: Tag| {
        let raw = Tag::compound([
            ("id", string(&kind.id)),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("HurtTime", hurt),
            ("DeathTime", death),
        ]);
        let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
        (m.hurt_time, m.death_time)
    };
    assert_eq!(load(Tag::Int(40000), Tag::Int(65546)), (-25536, 10));
    assert_eq!(load(Tag::Short(7), Tag::Short(19)), (7, 19));
}

/// `Axolotl.Variant.BY_ID` is `OutOfBoundsStrategy.ZERO`: an id outside
/// 0..=4 is lucy (0). `Parrot.Variant.BY_ID` is `CLAMP`: it is the nearest
/// end.
#[test]
fn an_unknown_axolotl_variant_is_lucy_and_a_parrots_is_clamped() {
    let variant = |species, id: &str, saved: i32| {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = id.into();
        ty.species = species;
        let types = [(id.into(), kind)].into();
        let raw = Tag::compound([
            ("id", string(id)),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("Variant", Tag::Int(saved)),
        ]);
        mob_from_nbt(&raw, &types, 1, None).unwrap().extra.variant
    };
    let axolotl = |saved| variant(crate::mobs::Species::Axolotl, "minecraft:axolotl", saved);
    let parrot = |saved| variant(crate::mobs::Species::Parrot, "minecraft:parrot", saved);
    assert_eq!([0, 3, 4, 5, 7, -1].map(axolotl), [0, 3, 4, 0, 0, 0]);
    assert_eq!([0, 3, 4, 5, 7, -1].map(parrot), [0, 3, 4, 4, 4, 0]);
}

/// A mount keeps its body armour and a helmet (say
/// `/item replace entity <horse> armor.head with minecraft:iron_helmet`)
/// in their own slots through a save: vanilla holds both (`equipment.body`
/// and `equipment.head`), one in each.
#[test]
fn a_horses_save_keeps_a_helmet_and_its_body_armour_apart() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:horse".into();
    ty.species = crate::mobs::Species::Horse;
    let types = [("minecraft:horse".into(), kind)].into();
    let raw = Tag::compound([
        ("id", string("minecraft:horse")),
        ("Pos", doubles(&[4.5, 220.0, 4.5])),
        ("Tame", Tag::Byte(1)),
        (
            "equipment",
            Tag::compound([
                (
                    "body",
                    Tag::compound([
                        ("id", string("minecraft:diamond_horse_armor")),
                        ("count", Tag::Int(1)),
                    ]),
                ),
                (
                    "head",
                    Tag::compound([
                        ("id", string("minecraft:iron_helmet")),
                        ("count", Tag::Int(1)),
                    ]),
                ),
            ]),
        ),
    ]);
    let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
    assert_eq!(m.extra.body_id(), Some("minecraft:diamond_horse_armor"));
    assert_eq!(m.extra.head_id(), Some("minecraft:iron_helmet"));
    let saved = mob_to_nbt(&m, [0; 4], Some(&raw), None, |_| None);
    let eq = saved.get("equipment").unwrap();
    let id = |key: &str| {
        eq.get(key)
            .and_then(|i| text(i.get("id")))
            .unwrap_or_default()
    };
    assert_eq!(id("body"), "minecraft:diamond_horse_armor");
    assert_eq!(id("head"), "minecraft:iron_helmet");
}

/// A body item taken off (`/item replace entity <cube> armor.body with minecraft:air`)
/// is not saved again from the entity's earlier save: vanilla writes `equipment.body` only
/// while the slot holds something (#12038).
#[test]
fn an_emptied_body_slot_is_not_saved_from_the_earlier_nbt() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:sulfur_cube".into();
    ty.species = crate::mobs::Species::SulfurCube;
    let types = [("minecraft:sulfur_cube".into(), kind)].into();
    let raw = Tag::compound([
        ("id", string("minecraft:sulfur_cube")),
        ("Pos", doubles(&[4.5, 220.0, 4.5])),
        (
            "equipment",
            Tag::compound([(
                "body",
                Tag::compound([
                    ("id", string("minecraft:oak_planks")),
                    ("count", Tag::Int(1)),
                ]),
            )]),
        ),
    ]);
    let mut m = mob_from_nbt(&raw, &types, 1, None).unwrap();
    assert_eq!(m.extra.body_id(), Some("minecraft:oak_planks"));
    m.extra.set_body_stack(None);
    let saved = mob_to_nbt(&m, [0; 4], Some(&raw), None, |_| None);
    assert!(saved.get("equipment").is_none(), "{saved:?}");
}

/// A wolf loads its armor (`equipment.body`) into the body slot with the
/// durability it lost, and a helmet next to it into the head slot.
#[test]
fn a_wolfs_save_keeps_its_armour_and_a_helmet_apart() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:wolf".into();
    ty.species = crate::mobs::Species::Wolf;
    let types = [("minecraft:wolf".into(), kind)].into();
    let armor = Tag::compound([
        ("id", string("minecraft:wolf_armor")),
        ("count", Tag::Int(1)),
        (
            "components",
            Tag::compound([("minecraft:damage", Tag::Int(9))]),
        ),
    ]);
    let raw = Tag::compound([
        ("id", string("minecraft:wolf")),
        ("Pos", doubles(&[4.5, 220.0, 4.5])),
        ("Owner", Tag::IntArray([0; 4].to_vec())),
        (
            "equipment",
            Tag::compound([
                ("body", armor),
                (
                    "head",
                    Tag::compound([
                        ("id", string("minecraft:iron_helmet")),
                        ("count", Tag::Int(1)),
                    ]),
                ),
            ]),
        ),
    ]);
    let m = mob_from_nbt(&raw, &types, 1, None).unwrap();
    assert_eq!(m.extra.body_id(), Some("minecraft:wolf_armor"));
    assert_eq!(m.extra.body_damage(), 9);
    assert_eq!(m.extra.head_id(), Some("minecraft:iron_helmet"));
}

/// `Wolf` and `Cat` read `CollarColor` with `DyeColor.LEGACY_ID_CODEC`: a
/// byte through `DyeColor.byId`, whose out-of-range answer is the first
/// colour, white (`OutOfBoundsStrategy.ZERO`). Nothing wraps.
#[test]
fn a_collar_colour_outside_the_sixteen_reads_white() {
    for (species, id) in [
        (crate::mobs::Species::Wolf, "minecraft:wolf"),
        (crate::mobs::Species::Cat, "minecraft:cat"),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = id.into();
        ty.species = species;
        let types = [(id.into(), kind)].into();
        let colour = |saved: Option<Tag>| {
            let mut fields = vec![("id", string(id)), ("Pos", doubles(&[4.5, 220.0, 4.5]))];
            fields.extend(saved.map(|c| ("CollarColor", c)));
            let m = mob_from_nbt(&Tag::compound(fields), &types, 1, None).unwrap();
            m.extra.color
        };
        assert_eq!(colour(Some(Tag::Byte(11))), 11, "{id}");
        assert_eq!(colour(Some(Tag::Byte(17))), 0, "{id}: not 17 & 15");
        assert_eq!(colour(Some(Tag::Byte(-1))), 0, "{id}: not 15");
        // Codec.BYTE narrows an int first: 259 is byte 3.
        assert_eq!(colour(Some(Tag::Int(259))), 3, "{id}");
        assert_eq!(colour(None), crate::mobs::tame::RED, "{id}");
    }
}

fn fused_creeper(fuse: Option<i16>) -> (Mob, Tag) {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:creeper".into();
    ty.species = crate::mobs::Species::Creeper;
    let types = [("minecraft:creeper".into(), kind)].into();
    let mut tag = Tag::compound([
        ("id", string("minecraft:creeper")),
        ("Pos", doubles(&[4.5, 220.0, 4.5])),
        ("ignited", Tag::Byte(1)),
        ("NoAI", Tag::Byte(1)),
        ("NoGravity", Tag::Byte(1)),
    ]);
    if let (Some(fuse), Tag::Compound(c)) = (fuse, &mut tag) {
        c.insert("Fuse".into(), Tag::Short(fuse));
    }
    (mob_from_nbt(&tag, &types, 1, None).unwrap(), tag)
}

#[test]
fn creeper_fuse_nbt_controls_explosion_tick_and_saves_runtime_value() {
    for (fuse, boundary) in [
        (Some(1), 1),
        (None, 30),
        (Some(40), 40),
        (Some(0), 1),
        (Some(-1), 1),
    ] {
        let (mut mob, raw) = fused_creeper(fuse);
        let mut events = Vec::new();
        for tick in 1..=boundary {
            crate::mobs::hostile::creeper_tick(&mut mob, &mut events);
            assert_eq!(mob.removed, tick == boundary, "Fuse={fuse:?}, tick={tick}");
        }
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, crate::mobs::MobEvent::Explode(..)))
                .count(),
            1
        );
        mob.max_swell = 7;
        let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
        assert_eq!(saved.get("Fuse"), Some(&Tag::Short(7)));
    }
}

/// #11332: `Creeper.readAdditionalSaveData` reads `ExplosionRadius` as a
/// byte (default 3), `explodeCreeper` blows with it, twice it when powered,
/// and `addAdditionalSaveData` writes the value back as a byte.
#[test]
fn creeper_explosion_radius_nbt_sizes_the_blast_and_saves() {
    let blast = |mob: &mut Mob| -> f32 {
        let mut events = Vec::new();
        mob.max_swell = 1;
        crate::mobs::hostile::creeper_tick(mob, &mut events);
        events
            .iter()
            .find_map(|e| match e {
                crate::mobs::MobEvent::Explode(_, radius, ..) => Some(*radius),
                _ => None,
            })
            .expect("the creeper exploded")
    };
    for (radius, powered, expected) in [
        (None, false, 3.0),
        (None, true, 6.0),
        (Some(Tag::Byte(12)), false, 12.0),
        (Some(Tag::Byte(12)), true, 24.0),
        (Some(Tag::Byte(0)), false, 0.0),
        // getByteOr takes any numeric tag, narrowed to a byte.
        (Some(Tag::Long(300)), false, 44.0),
    ] {
        let (mut mob, raw) = fused_creeper(None);
        let mut tag = raw.clone();
        if let Tag::Compound(c) = &mut tag {
            if let Some(radius) = radius.clone() {
                c.insert("ExplosionRadius".into(), radius);
            }
            c.insert("powered".into(), Tag::Byte(i8::from(powered)));
        }
        let types = [("minecraft:creeper".into(), mob.kind.clone())].into();
        mob = mob_from_nbt(&tag, &types, 1, None).unwrap();
        let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
        assert_eq!(
            saved.get("ExplosionRadius"),
            Some(&Tag::Byte(expected as i8 / if powered { 2 } else { 1 })),
            "{radius:?} powered={powered}"
        );
        assert_eq!(blast(&mut mob), expected, "{radius:?} powered={powered}");
    }
}

#[test]
fn creeper_fall_swell_uses_custom_fuse_without_a_zero_floor() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    for (fuse, expected) in [(Some(1), -4), (None, 25), (Some(40), 35)] {
        let (mut mob, _) = fused_creeper(fuse);
        mob.cause_fall_damage_from(
            crate::test_resources::server_loop_context(),
            level,
            100.0,
            0.0,
            &mut Vec::new(),
        );
        assert_eq!(mob.swell, expected, "Fuse={fuse:?}");
    }
}

#[test]
fn villager_level_round_trips_above_byte_range() {
    use crate::mobs::Species;
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:villager".into();
    def.species = Species::Villager;
    let types = [("minecraft:villager".into(), kind)].into();
    let data = Tag::compound([
        ("profession", string("minecraft:farmer")),
        ("type", string("minecraft:plains")),
        ("level", Tag::Int(123456)),
    ]);
    let tag = Tag::compound([
        ("id", string("minecraft:villager")),
        ("Pos", doubles(&[0.0, 100.0, 0.0])),
        ("VillagerData", data.clone()),
    ]);
    let mob = mob_from_nbt(&tag, &types, 7, None).unwrap();
    assert_eq!(villager_data(&mob), data);
}

#[test]
fn unresolved_villager_profession_round_trips_through_entity_nbt() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:villager".into();
    def.species = crate::mobs::Species::Villager;
    let types = [("minecraft:villager".into(), kind)].into();
    let data = Tag::compound([
        ("profession", string("example:nested/engineer")),
        ("type", string("minecraft:plains")),
        ("level", Tag::Int(1)),
    ]);
    let tag = Tag::compound([
        ("id", string("minecraft:villager")),
        ("Pos", doubles(&[0.0, 100.0, 0.0])),
        ("VillagerData", data.clone()),
    ]);
    let mob = mob_from_nbt(&tag, &types, 7, None).unwrap();
    assert_eq!(villager_data(&mob), data);
    assert_eq!(mob.extra.profession.index(), None);
    let saved = mob_to_nbt(&mob, [0; 4], None, None, |_| None);
    let reloaded = mob_from_nbt(&saved, &types, 8, None).unwrap();
    assert_eq!(villager_data(&reloaded), data);
}

#[test]
fn loaded_villager_command_names_follow_profession_and_custom_name() {
    use crate::mobs::Species;
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:villager".into();
    def.species = Species::Villager;
    let types = [("minecraft:villager".into(), kind)].into();
    for (profession, key) in [
        ("none", "entity.minecraft.villager.none"),
        ("nitwit", "entity.minecraft.villager.nitwit"),
        ("cleric", "entity.minecraft.villager.cleric"),
        ("farmer", "entity.minecraft.villager.farmer"),
        ("librarian", "entity.minecraft.villager.librarian"),
    ] {
        let data = Tag::compound([
            ("profession", string(&format!("minecraft:{profession}"))),
            ("type", string("minecraft:plains")),
            ("level", Tag::Int(1)),
        ]);
        let mut tag = Tag::compound([
            ("id", string("minecraft:villager")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
            ("NoAI", Tag::Byte(1)),
            ("VillagerData", data.clone()),
        ]);
        let mob = mob_from_nbt(&tag, &types, 7, None).unwrap();
        assert_eq!(villager_data(&mob), data);
        assert_eq!(mob.command_name().translation_key(), Some(key));
        put(&mut tag, "CustomName", string("Ada"));
        let named = mob_from_nbt(&tag, &types, 8, None).unwrap();
        assert_eq!(
            named.command_name().component(),
            crate::text::Text::literal("Ada").component()
        );
    }
}

/// `Entity.getDisplayName` and `saveWithoutId`: a styled custom name is
/// the command name and is saved whole, over an older saved name.
#[test]
fn a_styled_mob_name_is_its_command_name_and_saved_whole() {
    let kind = crate::mobs::sensor_scan_tests::kind();
    let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
    let styled = Tag::compound([("text", string("Tagged")), ("color", string("aqua"))]);
    mob.custom_name = Some("Tagged".into());
    mob.custom_name_text = Some(name_text(&styled));
    assert_eq!(mob.command_name(), name_text(&styled));
    let raw = Tag::compound([("CustomName", string("Old"))]);
    let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
    assert_eq!(
        saved.get("CustomName").map(name_text),
        Some(name_text(&styled))
    );
    assert!(matches!(saved.get("CustomName"), Some(Tag::Compound(_))));
    // A plain name keeps being saved as a string.
    mob.custom_name_text = None;
    let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
    assert_eq!(text(saved.get("CustomName")).as_deref(), Some("Tagged"));
}

/// `Component.getString`: a compound or list name is flattened whole, so
/// `jeb_` written as `extra` still counts; a bucket keeps the styled name.
#[test]
fn a_compound_mob_name_flattens_and_a_bucket_keeps_its_style() {
    let mut sheep = crate::mobs::sensor_scan_tests::kind();
    Arc::make_mut(&mut sheep).id = "minecraft:sheep".into();
    let types = [("minecraft:sheep".into(), sheep)].into();
    let tag = Tag::compound([
        ("id", string("minecraft:sheep")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        (
            "CustomName",
            Tag::compound([
                ("text", string("")),
                ("extra", Tag::List(vec![string("jeb"), string("_")])),
            ]),
        ),
    ]);
    let m = mob_from_nbt(&tag, &types, 1, None).unwrap();
    assert_eq!(m.custom_name.as_deref(), Some("jeb_"));
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    Arc::make_mut(&mut kind).id = "minecraft:axolotl".into();
    let mut fish = Mob::new(2, kind, Vec3::ZERO, 0.0);
    let styled = Tag::compound([("text", string("Axo")), ("color", string("gold"))]);
    fish.custom_name = Some("Axo".into());
    fish.custom_name_text = Some(name_text(&styled));
    let saved = bucket_components(&fish).custom_name.map(|t| name_text(&t));
    assert_eq!(saved, Some(name_text(&styled)));
}

#[test]
fn other_mob_command_names_keep_their_type_and_custom_name() {
    use crate::mobs::Species;
    for (id, species, key) in [
        (
            "minecraft:zombie_villager",
            Species::ZombieVillager,
            "entity.minecraft.zombie_villager",
        ),
        (
            "minecraft:wandering_trader",
            Species::WanderingTrader,
            "entity.minecraft.wandering_trader",
        ),
        ("audit:helper", Species::Pig, "entity.audit.helper"),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let def = Arc::make_mut(&mut kind);
        def.id = id.into();
        def.species = species;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.variant2 = 4;
        assert_eq!(mob.command_name().translation_key(), Some(key));
        mob.custom_name = Some("Ada".into());
        assert_eq!(mob.command_name().plain(), "Ada");
    }
}

#[test]
fn mob_loading_accepts_default_namespace_and_preserves_custom_namespaces() {
    let cow = crate::mobs::sensor_scan_tests::kind();
    let types = [
        ("minecraft:cow".into(), cow.clone()),
        ("test:cow".into(), cow),
    ]
    .into();
    let pos = || {
        Tag::List(
            [4.5_f64, 220.0, 6.5]
                .map(|v| Tag::Double(v.to_bits()))
                .to_vec(),
        )
    };
    for id in ["cow", ":cow", "minecraft:cow", "test:cow"] {
        let tag = Tag::compound([("id", string(id)), ("Pos", pos()), ("NoAI", Tag::Byte(1))]);
        let mob = mob_from_nbt(&tag, &types, 7, None).expect(id);
        assert_eq!(mob.position, Vec3::new(4.5, 220.0, 6.5));
        assert!(mob.no_ai);
    }
    let unknown = Tag::compound([("id", string("other:cow")), ("Pos", pos())]);
    assert!(mob_from_nbt(&unknown, &types, 8, None).is_none());
}

/// A saved rocket and trident come back with their own Life, LifeTime,
/// flags and position (the keys vanilla's addAdditionalSaveData writes).
#[test]
fn rocket_and_trident_round_trip() {
    let item = ItemStack::new("minecraft:firework_rocket", 1);
    let tag = firework_rocket_to_nbt(
        &RocketSave {
            rotation: [90.0, 10.0],
            position: Vec3::new(1.5, 70.0, -3.25),
            motion: Vec3::new(0.0, 0.5, 0.0),
            life: 7,
            lifetime: 31,
            item: &item,
            angled: true,
            owner: None,
            left_owner: false,
            on_ground: true,
            has_been_shot: false,
            fall_distance: 2.5,
        },
        [1, 2, 3, 4],
        None,
    );
    let r = firework_rocket_from_nbt(&tag, None).unwrap();
    assert_eq!(
        (r.life, r.lifetime, r.angled, r.on_ground, r.has_been_shot),
        (7, 31, true, true, false)
    );
    assert_eq!(r.position, Vec3::new(1.5, 70.0, -3.25));
    assert_eq!(r.rotation, [90.0, 10.0]);
    // Entity.saveWithoutId/load keep the fall.
    assert_eq!(r.fall_distance, 2.5);
    let trident = ItemStack::new("minecraft:trident", 1);
    let tag = trident_to_nbt(
        &TridentSave {
            position: Vec3::new(2.0, 64.0, 2.0),
            motion: Vec3::new(0.0, -0.25, 0.0),
            yaw: 10.0,
            pitch: 20.0,
            in_ground: true,
            life: 9,
            shake: 3,
            dealt_damage: true,
            pickup: Pickup::CreativeOnly,
            left_owner: true,
            has_been_shot: true,
            owner: Some([5, 6, 7, 8]),
            no_gravity: true,
            on_ground: true,
            fire_ticks: 0,
            fall_distance: 0.0,
            sound_event: TRIDENT_HIT_GROUND,
            item: &trident,
        },
        [1, 2, 3, 4],
        None,
    );
    let t = trident_from_nbt(&tag).unwrap();
    // `Entity.saveWithoutId` writes NoGravity only when set.
    assert!(t.no_gravity);
    assert!(matches!(tag.get("NoGravity"), Some(Tag::Byte(1))));
    // Entity.load/saveWithoutId: OnGround as it was.
    assert!(t.on_ground);
    assert_eq!(tag.get("OnGround"), Some(&Tag::Byte(1)));
    assert_eq!((t.in_ground, t.life, t.shake), (true, 9, 3));
    assert_eq!((t.dealt_damage, t.pickup), (true, Pickup::CreativeOnly));
    assert!(t.left_owner && t.has_been_shot);
    assert_eq!(t.owner, Some([5, 6, 7, 8]));
    assert_eq!((t.yaw, t.pitch), (10.0, 20.0));
}

/// The item, the spawn delay and the fields Horizon doesn't model come
/// back; a compound without an item is no spawner.
#[test]
fn ominous_item_spawner_round_trip() {
    let item = ItemStack {
        item: Arc::from("minecraft:ominous_trial_key"),
        count: 1,
        damage: 0,
        extra: None,
    };
    let raw = Tag::compound([("CustomName", string("kept"))]);
    let at = Vec3::new(10.5, 64.25, -3.5);
    let tag = ominous_item_spawner_to_nbt(&item, at, 97, [1, 2, 3, 4], Some(&raw));
    assert_eq!(
        kind(&tag).as_deref(),
        Some("minecraft:ominous_item_spawner")
    );
    assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
    assert!(matches!(
        tag.get("spawn_item_after_ticks"),
        Some(Tag::Long(97))
    ));
    assert!(tag.get("CustomName").is_some());
    let (back, pos, after) = ominous_item_spawner_from_nbt(&tag).unwrap();
    assert_eq!(
        (back.item.as_ref(), back.count),
        ("minecraft:ominous_trial_key", 1)
    );
    assert_eq!((pos, after), (at, 97));
    assert!(ominous_item_spawner_from_nbt(&raw).is_none());
    // No item is `ItemStack.EMPTY`, which writes no key back.
    let empty = ominous_item_spawner_to_nbt(
        &crate::items::ItemStack::new("minecraft:air", 1),
        at,
        0,
        [1, 2, 3, 4],
        Some(&tag),
    );
    assert!(empty.get("item").is_none());
    let (back, pos, after) = ominous_item_spawner_from_nbt(&empty).unwrap();
    assert_eq!((back.item.as_ref(), pos, after), ("minecraft:air", at, 0));
}

#[test]
fn armor_stand_names_keep_their_formatting_across_a_save() {
    let tag = Tag::compound([
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(64.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
            ]),
        ),
        ("CustomName", string("§6Hello")),
        ("CustomNameVisible", Tag::Byte(1)),
    ]);
    let a = armor_stand_from_nbt(&tag, 7).unwrap();
    let name = a.custom_name.as_ref().unwrap();
    assert_eq!(name.plain(), "Hello");
    assert!(name.spans[0].style.color.is_some());
    assert!(a.name_visible);
    let saved = armor_stand_to_nbt(&a, [0; 4], Some(&tag));
    let restored = armor_stand_from_nbt(&saved, 7).unwrap();
    assert_eq!(restored.custom_name, a.custom_name);
    assert!(restored.name_visible);
}

#[test]
fn armor_stand_keeps_no_gravity_through_a_save() {
    let tag = Tag::compound([
        ("id", string("minecraft:armor_stand")),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(64.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
            ]),
        ),
        ("NoGravity", Tag::Byte(1)),
    ]);
    let mut a = armor_stand_from_nbt(&tag, 7).unwrap();
    assert!(a.no_gravity, "summon NBT sets the flag");
    let saved = armor_stand_to_nbt(&a, [0; 4], Some(&tag));
    assert_eq!(saved.get("NoGravity"), Some(&Tag::Byte(1)));
    assert!(armor_stand_from_nbt(&saved, 7).unwrap().no_gravity);
    // `data merge entity @s {NoGravity:0b}` clears it again.
    a.no_gravity = false;
    let cleared = armor_stand_to_nbt(&a, [0; 4], Some(&saved));
    assert!(cleared.get("NoGravity").is_none());
    assert!(!armor_stand_from_nbt(&cleared, 7).unwrap().no_gravity);
}

/// `Entity.load`: a Motion component past 10 blocks a tick loads as 0,
/// the others as saved.
#[test]
fn armor_stand_drops_a_motion_component_past_ten() {
    let doubles = |v: [f64; 3]| Tag::List(v.map(|d| Tag::Double(d.to_bits())).to_vec());
    let tag = Tag::compound([
        ("id", string("minecraft:armor_stand")),
        ("Pos", doubles([0.0, 64.0, 0.0])),
        ("Motion", doubles([1.0e10, -0.5, -10.5])),
    ]);
    let a = armor_stand_from_nbt(&tag, 7).unwrap();
    assert_eq!(a.velocity, Vec3::new(0.0, -0.5, 0.0));
    let tag = Tag::compound([
        ("id", string("minecraft:armor_stand")),
        ("Pos", doubles([0.0, 64.0, 0.0])),
        ("Motion", doubles([10.0, 0.0, -10.0])),
    ]);
    let a = armor_stand_from_nbt(&tag, 7).unwrap();
    assert_eq!(a.velocity, Vec3::new(10.0, 0.0, -10.0), "10 itself stays");
}

fn falling_decode_registry() -> BlockRegistry {
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        (
            "states",
            Tag::List(
                ["minecraft:air", "minecraft:sand", "minecraft:gravel"]
                    .into_iter()
                    .map(string)
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:sand", Tag::Int(1)),
                ("minecraft:gravel", Tag::Int(2)),
            ]),
        ),
    ]))
    .unwrap()
}

fn falling_decode_input() -> Tag {
    horizon_core::nbt::parse_snbt(
        "{Pos:[3.5d,200d,4.5d],Motion:[0d,0d,0d],Time:10,DropItem:0b,NoGravity:1b}",
    )
    .unwrap()
    .0
}

fn falling_decode_save(registry: &BlockRegistry, input: &Tag) -> Tag {
    let (state, pos, motion, time, distance) =
        falling_block_from_nbt(input).expect("absent BlockState is valid input");
    let actual = falling_state_from_nbt(registry, &state).unwrap();
    assert_eq!(falling_block_flags(input), (true, false));
    // Save only the decoded concrete model; no original source compound.
    let saved = falling_block_to_nbt(
        state_to_nbt(registry, actual),
        pos,
        motion,
        time,
        false,
        crate::falling_block::Damage::default(),
        distance,
        false,
        [0.0, 0.0],
        [96, 0, 0, 1],
    );
    assert_eq!(saved.get("Pos"), input.get("Pos"));
    assert_eq!(saved.get("Motion"), input.get("Motion"));
    assert_eq!(saved.get("Time"), Some(&Tag::Int(10)));
    assert_eq!(saved.get("DropItem"), Some(&Tag::Byte(0)));
    saved
}

#[test]
fn falling_decode_absent_state_saves_actual_sand_without_raw() {
    let registry = falling_decode_registry();
    let saved = falling_decode_save(&registry, &falling_decode_input());
    assert_eq!(saved.get("BlockState"), Some(&string("minecraft:sand")));
}

#[test]
fn falling_decode_modern_boundary_keeps_shared_legacy_reader() {
    let registry = falling_decode_registry();
    let legacy = Tag::compound([("Name", string("minecraft:gravel"))]);
    assert_eq!(
        state_from_nbt(&registry, &legacy),
        registry.default_state("minecraft:gravel")
    );
    for (state, expected) in [
        (string("minecraft:sand"), "minecraft:sand"),
        (
            Tag::compound([("id", string("minecraft:gravel"))]),
            "minecraft:gravel",
        ),
        (string("minecraft:air"), "minecraft:air"),
        (legacy, "minecraft:sand"),
        (Tag::Compound(Default::default()), "minecraft:sand"),
    ] {
        let mut input = falling_decode_input();
        put(&mut input, "BlockState", state);
        let saved = falling_decode_save(&registry, &input);
        assert_eq!(saved.get("BlockState"), Some(&string(expected)));
    }
}

#[test]
fn falling_block_round_trip_keeps_impact_distance_and_settings() {
    let state = Tag::compound([("Name", string("minecraft:damaged_anvil"))]);
    let position = Vec3::new(3.5, 100.0, -7.5);
    let velocity = Vec3::new(0.0, -1.25, 0.0);
    let damage = crate::falling_block::Damage {
        cancel_drop: true,
        ..crate::falling_block::Damage::anvil()
    };
    let tag = falling_block_to_nbt(
        state.clone(),
        position,
        velocity,
        30,
        false,
        damage,
        19.75,
        true,
        [0.0, 0.0],
        [1, 2, 3, 4],
    );
    let (loaded_state, pos, motion, age, distance) = falling_block_from_nbt(&tag).unwrap();
    assert_eq!(loaded_state, state);
    assert_eq!(
        (pos, motion, age, distance),
        (position, velocity, 30, 19.75)
    );
    assert_eq!(falling_block_flags(&tag), (false, false));
    assert_eq!(crate::falling_block::Damage::from_nbt(&tag, true), damage);
    assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
}

#[test]
fn loaded_item_lifetime_counters_tick_with_signed_sentinels() {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("movement-properties.nbt").exists() {
        return;
    }
    let assets = crate::assets::Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = crate::blocks::BlockFacts::load(&root, registry).unwrap();
    let collision = crate::collisions::load(&root, registry).unwrap();
    let fluids = crate::motion::FluidResources::new(&assets, &facts).unwrap();
    let mut world = crate::world::World::new(registry.default_state("minecraft:air").unwrap(), 0);
    world
        .chunks
        .insert((0, 0), Arc::new(crate::chunk::Chunk::empty(0, 0)));
    let tick = |item: &mut ItemEntity| {
        item.tick(
            &world,
            registry,
            &facts,
            &collision,
            false,
            &fluids,
            &[],
            None,
            None,
        )
    };
    let fresh = Tag::compound([
        ("id", string("minecraft:item")),
        ("Pos", doubles(&[4.5, 210.0, 4.5])),
        ("NoGravity", Tag::Byte(1)),
        (
            "Item",
            Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(1))]),
        ),
    ]);
    for delay in [32767_i16, -1, 0, 1, 2] {
        for age in [-32768_i16, -1, 0, 5998, 5999, 6000, 32767] {
            let mut item = item_entity_from_nbt(&fresh, 1, 0.0).unwrap();
            for _ in 0..3 {
                assert!(tick(&mut item));
            }
            assert_eq!(item.age, 3);
            let mut data = item_entity_to_nbt(&item, [0, 0, 0, 1], None);
            put(&mut data, "Age", Tag::Short(age));
            put(&mut data, "PickupDelay", Tag::Short(delay));
            let mut item = item_entity_from_nbt(&data, 1, 0.0).unwrap();
            let mut alive = true;
            for step in 0..=3 {
                if step > 0 && alive {
                    alive = tick(&mut item);
                }
                let expected_age = if age == i16::MIN {
                    i32::from(age)
                } else {
                    i32::from(age) + step
                };
                let expected_alive = step == 0 || expected_age < 6000;
                assert_eq!(
                    alive, expected_alive,
                    "age={age}, delay={delay}, tick={step}"
                );
                if !alive {
                    continue;
                }
                let data = item_entity_to_nbt(&item, [0, 0, 0, 1], None);
                let expected_delay = if delay > 0 && delay != i16::MAX {
                    (i32::from(delay) - step).max(0)
                } else {
                    i32::from(delay)
                };
                assert_eq!(
                    number(data.get("Age")),
                    Some(i64::from(expected_age)),
                    "age={age}, tick={step}"
                );
                assert_eq!(
                    number(data.get("PickupDelay")),
                    Some(i64::from(expected_delay)),
                    "age={age}, delay={delay}, tick={step}"
                );
            }
        }
    }
    let mut legacy = item_entity_from_nbt(&fresh, 1, 0.0).unwrap();
    legacy.pickup_delay = i64::from(u32::MAX);
    assert!(tick(&mut legacy));
    assert_eq!(legacy.pickup_delay, i64::from(u32::MAX));
}

#[test]
fn item_health_uses_floor_then_short_narrowing() {
    let cases = [
        (None, 5),
        (Some(Tag::Short(5)), 5),
        (Some(Tag::Int(5)), 5),
        (Some(Tag::Long(5)), 5),
        (Some(Tag::Byte(5)), 5),
        (Some(Tag::Float((-1.5_f32).to_bits())), -2),
        (Some(Tag::Double((-1.5_f64).to_bits())), -2),
        (Some(Tag::Float(1.5_f32.to_bits())), 1),
        (Some(Tag::Double(1.5_f64.to_bits())), 1),
        (Some(Tag::Double((-40000.5_f64).to_bits())), 25535),
        (Some(Tag::Int(32767)), 32767),
        (Some(Tag::Int(32768)), -32768),
        (Some(Tag::Int(-32768)), -32768),
        (Some(Tag::Int(-32769)), 32767),
        (Some(Tag::Long(65535)), -1),
        (Some(Tag::Long(65536)), 0),
        (Some(Tag::Long(-65537)), -1),
        (Some(Tag::Long(i64::MAX)), -1),
        (Some(string("not_numeric")), 5),
        (Some(Tag::Compound(Compound::new())), 5),
    ];
    for (input, expected) in cases {
        let mut tag = Tag::compound([
            ("id", string("minecraft:item")),
            ("Pos", doubles(&[4.5, 200.0, 4.5])),
            ("NoGravity", Tag::Byte(1)),
            (
                "Item",
                Tag::compound([("id", string("minecraft:diamond")), ("count", Tag::Int(1))]),
            ),
        ]);
        if let Some(input) = input {
            put(&mut tag, "Health", input);
        }
        let item = item_entity_from_nbt(&tag, 9, 0.0).unwrap();
        assert_eq!(item.health, expected, "input {tag:?}");
        let saved = item_entity_to_nbt(&item, [1, 2, 3, 4], None);
        assert_eq!(saved.get("Health"), Some(&Tag::Short(expected as i16)));
    }
}

#[test]
fn item_numeric_uuid_codec_adopts_numeric_streams_before_save() {
    let cases = [
        (Tag::IntArray(vec![54, 1, -2, 3]), [54, 1, -2, 3]),
        (
            Tag::LongArray(vec![
                54,
                4_294_967_297,
                -4_294_967_298,
                9_007_199_254_740_993,
            ]),
            [54, 1, -2, 1],
        ),
        (
            Tag::List(vec![
                Tag::Float(54.9_f32.to_bits()),
                Tag::Float(1.75_f32.to_bits()),
                Tag::Float((-2.75_f32).to_bits()),
                Tag::Float((-0.5_f32).to_bits()),
            ]),
            [54, 1, -2, 0],
        ),
        (
            Tag::List(vec![
                Tag::Double(54.9_f64.to_bits()),
                Tag::Double(1.75_f64.to_bits()),
                Tag::Double((-2.75_f64).to_bits()),
                Tag::Double((-0.5_f64).to_bits()),
            ]),
            [54, 1, -2, 0],
        ),
        (
            Tag::List(vec![
                Tag::Long(54),
                Tag::Long(4_294_967_297),
                Tag::Long(-4_294_967_298),
                Tag::Long(9_007_199_254_740_993),
            ]),
            [54, 1, -2, 1],
        ),
        (
            Tag::ByteArray(vec![54, 1, (-2_i8) as u8, 3]),
            [54, 1, -2, 3],
        ),
        // Util.fixedSize returns a partial first-four result for overlong streams.
        (Tag::IntArray(vec![54, 1, -2, 3, 4]), [54, 1, -2, 3]),
    ];
    for field in ["Owner", "Thrower"] {
        for (value, expected) in &cases {
            let raw = Tag::compound([
                ("Pos", doubles(&[2.5, 200.0, 2.5])),
                (
                    "Item",
                    Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(1))]),
                ),
                ("CustomName", string("kept")),
                (field, value.clone()),
            ]);
            let mut item = item_entity_from_nbt(&raw, 7, 0.0).unwrap();
            assert_eq!(
                if field == "Owner" {
                    item.target
                } else {
                    item.thrower
                },
                Some(*expected),
                "{field}: {value:?}"
            );
            assert_eq!(
                if field == "Owner" {
                    item.thrower
                } else {
                    item.target
                },
                None
            );
            for source in [None, Some(&raw)] {
                let saved = item_entity_to_nbt(&item, [54, 0, 0, 1], source);
                assert_eq!(saved.get(field), Some(&Tag::IntArray(expected.to_vec())));
                assert_eq!(saved.get("Item").unwrap().get("count"), Some(&Tag::Int(1)));
            }
            let saved = item_entity_to_nbt(&item, [54, 0, 0, 1], Some(&raw));
            assert_eq!(saved.get("CustomName"), raw.get("CustomName"));
            item.target = None;
            item.thrower = None;
            let cleared = item_entity_to_nbt(&item, [54, 0, 0, 1], Some(&saved));
            assert!(cleared.get("Owner").is_none());
            assert!(cleared.get("Thrower").is_none());
        }
    }
}

#[test]
fn item_numeric_uuid_codec_rejects_nonstreams_and_short_inputs() {
    let cases = [
        None,
        Some(string("00000036-0000-0001-ffff-fffe00000003")),
        Some(Tag::IntArray(vec![])),
        Some(Tag::IntArray(vec![54, 1, -2])),
        Some(Tag::Int(7)),
        Some(Tag::compound([("value", Tag::Int(7))])),
        Some(Tag::List(vec![
            Tag::Int(54),
            Tag::Int(1),
            Tag::Int(-2),
            string("3"),
        ])),
    ];
    for field in ["Owner", "Thrower"] {
        for value in &cases {
            let mut raw = Tag::compound([
                ("Pos", doubles(&[2.5, 200.0, 2.5])),
                (
                    "Item",
                    Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(1))]),
                ),
            ]);
            if let Some(value) = value {
                put(&mut raw, field, value.clone());
            }
            let mut item = item_entity_from_nbt(&raw, 7, 0.0).unwrap();
            assert_eq!(
                (item.target, item.thrower),
                (None, None),
                "{field}: {value:?}"
            );
            for source in [None, Some(&raw)] {
                let saved = item_entity_to_nbt(&item, [54, 0, 0, 1], source);
                assert!(saved.get("Owner").is_none());
                assert!(saved.get("Thrower").is_none());
            }
            item.target = Some([54, 1, -2, 3]);
            item.thrower = Some([54, 4, 5, 6]);
            let saved = item_entity_to_nbt(&item, [54, 0, 0, 1], Some(&raw));
            assert_eq!(saved.get("Owner"), Some(&Tag::IntArray(vec![54, 1, -2, 3])));
            assert_eq!(
                saved.get("Thrower"),
                Some(&Tag::IntArray(vec![54, 4, 5, 6]))
            );
        }
    }
}

#[test]
fn item_entities_round_trip() {
    let e = ItemEntity {
        attachment_lifetime: Default::default(),
        fall_distance: 2.5,
        rotation: [0.0, 0.0],
        join: Default::default(),
        no_gravity: false,
        health: 2,
        fire_ticks: 73,
        thrower: Some([5, 6, 7, 8]),
        previous: Vec3::ZERO,
        on_ground: true,
        age: 120,
        pickup_delay: 10,
        ..ItemEntity::new(
            7,
            ItemStack {
                item: Arc::from("minecraft:diamond_pickaxe"),
                count: 1,
                damage: 5,
                extra: None,
            },
            Vec3::new(100.5, 64.0, -31.5),
            Vec3::new(0.0, -0.04, 0.0),
            0.0,
        )
    };
    let tag = item_entity_to_nbt(&e, [1, 2, 3, 4], None);
    assert_eq!(kind(&tag).as_deref(), Some("minecraft:item"));
    assert_eq!(uuid(&tag), Some([1, 2, 3, 4]));
    let back = item_entity_from_nbt(&tag, 9, 0.0).unwrap();
    assert_eq!(back.stack.damage, 5);
    assert_eq!((back.health, back.fire_ticks), (2, 73));
    assert_eq!(
        (back.age, back.pickup_delay, back.on_ground),
        (120, 10, true)
    );
    assert_eq!(back.position.x, 100.5);
    assert_eq!(back.fall_distance, 2.5);
    assert_eq!(back.thrower, e.thrower);
    let mut cleared = back;
    cleared.thrower = None;
    let cleared = item_entity_to_nbt(&cleared, [1, 2, 3, 4], Some(&tag));
    assert!(cleared.get("Thrower").is_none());
}
#[test]
fn nbt_numeric_flags_decode_and_save_modeled_values_without_raw() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:pig".into();
    def.species = crate::mobs::Species::Pig;
    let types = [("minecraft:pig".into(), kind)].into();
    for (input, expected) in [
        (None, false),
        (Some(Tag::Byte(0)), false),
        (Some(Tag::Byte(1)), true),
        (Some(Tag::Byte(-1)), true),
        (Some(Tag::Short(256)), false),
        (Some(Tag::Int(256)), false),
        (Some(Tag::Int(257)), true),
        (Some(Tag::Int(-256)), false),
        (Some(Tag::Long(256)), false),
        (Some(Tag::Long(257)), true),
        (Some(Tag::Long(4294967296)), false),
        (Some(Tag::Long(9007199254740993)), true),
        (Some(Tag::Float(0.5f32.to_bits())), false),
        (Some(Tag::Float((-0.5f32).to_bits())), true),
        (Some(Tag::Float(256.5f32.to_bits())), false),
        (Some(Tag::Float((-255.5f32).to_bits())), false),
        (Some(Tag::Double(0.5f64.to_bits())), false),
        (Some(Tag::Double((-0.5f64).to_bits())), true),
        (Some(Tag::Double(256.5f64.to_bits())), false),
        (Some(Tag::Double((-255.5f64).to_bits())), false),
        (Some(Tag::Float((-1e20f32).to_bits())), false),
        (Some(Tag::Double((-1e20f64).to_bits())), false),
        (Some(Tag::Double(1e20f64.to_bits())), true),
        (Some(string("1")), false),
        (Some(string("true")), false),
        (Some(Tag::compound([("a", Tag::Int(1))])), false),
    ] {
        let mut raw = Tag::compound([
            ("id", string("minecraft:pig")),
            ("Pos", doubles(&[4.5, 200.0, 4.5])),
            ("Health", Tag::Float(10f32.to_bits())),
            ("NoGravity", Tag::Byte(1)),
            ("Unrelated", string("keep")),
        ]);
        if let Some(value) = input.as_ref() {
            for key in ["NoAI", "CanPickUpLoot"] {
                put(&mut raw, key, value.clone());
            }
        }
        let loaded = mob_from_nbt(&raw, &types, 7, None).unwrap();
        assert_eq!(loaded.no_ai, expected, "NoAI {input:?}");
        assert_eq!(loaded.can_pick_up_loot(), expected, "pickup {input:?}");
        assert!(loaded.no_gravity);
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&loaded, [45, 0, 0, 1], source, None, |_| None);
            assert_eq!(
                saved.get("NoAI"),
                expected.then_some(&Tag::Byte(1)),
                "NoAI {input:?}"
            );
            assert_eq!(
                saved.get("CanPickUpLoot"),
                Some(&Tag::Byte(i8::from(expected))),
                "pickup {input:?}"
            );
            if source.is_some() {
                assert_eq!(saved.get("Unrelated"), Some(&string("keep")));
            }
        }
    }
}

/// `Goat` reads `HasLeftHorn` and `HasRightHorn` with `getBooleanOr(key,
/// true)`: a number is true when its byte value is not 0, so an int 256
/// (byte 0) is a lost horn, as `0b` is.
#[test]
fn a_goats_horn_flags_are_read_as_bytes() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:goat".into();
    ty.species = crate::mobs::Species::Goat;
    let types = [("minecraft:goat".into(), kind)].into();
    let lost = |left: Option<Tag>| {
        let mut fields = vec![
            ("id", string("minecraft:goat")),
            ("Pos", doubles(&[4.5, 220.0, 4.5])),
        ];
        fields.extend(left.map(|v| ("HasLeftHorn", v)));
        mob_from_nbt(&Tag::compound(fields), &types, 1, None)
            .unwrap()
            .extra
            .lost_horns
    };
    assert_eq!(lost(None), [false, false]);
    assert_eq!(lost(Some(Tag::Byte(1))), [false, false]);
    assert_eq!(lost(Some(Tag::Byte(0))), [true, false]);
    assert_eq!(lost(Some(Tag::Int(256))), [true, false]);
    assert_eq!(lost(Some(Tag::Int(257))), [false, false]);
}

#[test]
fn non_mob_saves_drop_mob_flags_without_losing_entity_fields() {
    let uuid = [104, 0, 0, 1];
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:pig".into();
    def.species = crate::mobs::Species::Pig;
    let types = [("minecraft:pig".into(), kind)].into();
    for input in [None, Some(0), Some(1)] {
        for id in [
            "minecraft:oak_boat",
            "minecraft:oak_chest_boat",
            "minecraft:armor_stand",
            "minecraft:item",
            "minecraft:pig",
        ] {
            let mut raw = Tag::compound([
                ("id", string(id)),
                ("Pos", doubles(&[4.5, 200.0, 4.5])),
                ("Motion", doubles(&[0.0, 0.0, 0.0])),
                ("Silent", Tag::Byte(1)),
                ("NoGravity", Tag::Byte(1)),
                ("Invulnerable", Tag::Byte(1)),
                ("PortalCooldown", Tag::Int(17)),
                ("Tags", Tag::List(vec![string("audit")])),
                ("Unrelated", string("keep")),
                (
                    "Item",
                    item_to_nbt(&crate::items::ItemStack::new("minecraft:stone", 1)),
                ),
            ]);
            if let Some(value) = input {
                put(&mut raw, "NoAI", Tag::Byte(value));
                put(&mut raw, "PersistenceRequired", Tag::Byte(value));
            }
            let stand = armor_stand_from_nbt(&raw, 1).unwrap();
            let item = item_entity_from_nbt(&raw, 1, 0.0).unwrap();
            let boat = boat_from_nbt(&raw).unwrap();
            for source in [None, Some(&raw)] {
                let saved = match id {
                    "minecraft:armor_stand" => armor_stand_to_nbt(&stand, uuid, source),
                    "minecraft:item" => item_entity_to_nbt(&item, uuid, source),
                    "minecraft:pig" => {
                        let mut mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
                        // The writer must use live state, even opposite stale raw values.
                        mob.no_ai = input != Some(1);
                        mob.persistent = input != Some(1);
                        mob_to_nbt(&mob, uuid, source, None, |_| None)
                    }
                    _ => boat_to_nbt(
                        id,
                        boat.0,
                        boat.1,
                        [boat.2, 0.0],
                        (false, 0.0),
                        uuid,
                        source,
                        (id == "minecraft:oak_chest_boat").then_some(&[]),
                    ),
                };
                if id == "minecraft:pig" {
                    let live = input != Some(1);
                    assert_eq!(saved.get("NoAI"), live.then_some(&Tag::Byte(1)));
                    assert_eq!(
                        saved.get("PersistenceRequired"),
                        Some(&Tag::Byte(i8::from(live)))
                    );
                } else {
                    assert!(saved.get("NoAI").is_none(), "{id} {input:?}");
                    assert!(saved.get("PersistenceRequired").is_none(), "{id} {input:?}");
                }
                assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(uuid.to_vec())));
                assert_eq!(saved.get("Pos"), raw.get("Pos"));
                if source.is_some() {
                    for key in [
                        "Silent",
                        "NoGravity",
                        "Invulnerable",
                        "PortalCooldown",
                        "Tags",
                        "Unrelated",
                    ] {
                        assert_eq!(saved.get(key), raw.get(key), "{id} {key}");
                    }
                }
            }
        }
    }
}

#[test]
fn animal_integer_loads_save_modeled_values_without_raw_source() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:pig".into();
    def.species = crate::mobs::Species::Pig;
    let types = [("minecraft:pig".into(), kind)].into();
    for (input, expected) in [
        (None, 0),
        (Some(Tag::Int(0)), 0),
        (Some(Tag::Byte(-7)), -7),
        (Some(Tag::Short(13)), 13),
        (Some(Tag::Int(i32::MIN)), i32::MIN),
        (Some(Tag::Int(i32::MAX)), i32::MAX),
        (Some(Tag::Long(2147483648)), i32::MIN),
        (Some(Tag::Long(-2147483649)), i32::MAX),
        (Some(Tag::Long(4294967296)), 0),
        (Some(Tag::Long(i64::MAX)), -1),
        (Some(Tag::Long(i64::MIN)), 0),
        (Some(Tag::Float(2.5f32.to_bits())), 2),
        (Some(Tag::Float((-2.5f32).to_bits())), -3),
        (Some(Tag::Double(2.5f64.to_bits())), 2),
        (Some(Tag::Double((-2.5f64).to_bits())), -3),
        (Some(Tag::Float(2147483520f32.to_bits())), 2147483520),
        (Some(Tag::Float(2147483648f32.to_bits())), i32::MAX),
        (Some(Tag::Float((-2147483648f32).to_bits())), i32::MIN),
        (Some(Tag::Float((-2147483904f32).to_bits())), i32::MIN),
        (Some(Tag::Double(2147483647.75f64.to_bits())), i32::MAX),
        (Some(Tag::Double(2147483648f64.to_bits())), i32::MAX),
        (Some(Tag::Double((-2147483647.5f64).to_bits())), i32::MIN),
        (Some(Tag::Double((-2147483648.5f64).to_bits())), i32::MIN),
        (Some(Tag::Double(1e20f64.to_bits())), i32::MAX),
        (Some(Tag::Double((-1e20f64).to_bits())), i32::MIN),
        (Some(Tag::Float(1e20f32.to_bits())), i32::MAX),
        (Some(Tag::Float((-1e20f32).to_bits())), i32::MIN),
    ] {
        let mut raw = Tag::compound([
            ("id", string("minecraft:pig")),
            ("Pos", doubles(&[4.5, 200.0, 4.5])),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
            ("AgeLocked", Tag::Byte(1)),
            ("Unrelated", string("keep")),
        ]);
        if let Some(value) = input.as_ref() {
            for key in ["Age", "ForcedAge", "InLove"] {
                put(&mut raw, key, value.clone());
            }
        }
        let mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
        assert_eq!(
            (mob.extra.age, mob.extra.forced_age, mob.extra.in_love),
            (expected, expected, expected),
            "{input:?}"
        );
        assert!(mob.no_ai && mob.no_gravity && mob.extra.age_locked);
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [0, 0, 0, 55], source, None, |_| None);
            for key in ["Age", "ForcedAge", "InLove"] {
                assert_eq!(
                    saved.get(key),
                    Some(&Tag::Int(expected)),
                    "{key}: {input:?}"
                );
            }
            assert_eq!(saved.get("NoAI"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("AgeLocked"), Some(&Tag::Byte(1)));
            assert_eq!(
                saved.get("Unrelated"),
                source.and_then(|tag| tag.get("Unrelated"))
            );
        }
    }
}

#[test]
fn animal_nbt_preserves_signed_integer_in_love_and_defaults_after_removal() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let def = Arc::make_mut(&mut kind);
    def.id = "minecraft:pig".into();
    def.species = crate::mobs::Species::Pig;
    let types = [("minecraft:pig".into(), kind)].into();
    for (input, expected) in [
        (Tag::Int(-32769), -32769),
        (Tag::Int(-32768), -32768),
        (Tag::Int(-129), -129),
        (Tag::Int(-128), -128),
        (Tag::Int(-1), -1),
        (Tag::Byte(-1), -1),
        (Tag::Byte(-128), -128),
        (Tag::Int(0), 0),
        (Tag::Int(1), 1),
        (Tag::Int(37), 37),
    ] {
        let raw = Tag::compound([
            ("id", string("minecraft:pig")),
            ("Pos", doubles(&[4.0, 200.0, 4.0])),
            ("NoAI", Tag::Byte(1)),
            ("NoGravity", Tag::Byte(1)),
            ("InLove", input),
        ]);
        let mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
        assert_eq!(mob.extra.in_love, expected);
        // Query snapshots and saves both write the live field as an int,
        // regardless of whether there is an old compound to preserve.
        for old in [None, Some(&raw)] {
            let mut saved = mob_to_nbt(&mob, [0, 0, 0, 1], old, None, |_| None);
            assert_eq!(saved.get("InLove"), Some(&Tag::Int(expected)));
            if let Tag::Compound(map) = &mut saved {
                map.remove(&"InLove".into());
            }
            let cleared = mob_from_nbt(&saved, &types, 2, None).unwrap();
            assert_eq!(cleared.extra.in_love, 0);
            let saved = mob_to_nbt(&cleared, [0, 0, 0, 1], Some(&saved), None, |_| None);
            assert_eq!(saved.get("InLove"), Some(&Tag::Int(0)));
        }
    }
}

#[test]
fn unknown_fields_survive() {
    let raw = Tag::compound([
        ("id", string("minecraft:item")),
        ("CustomName", string("x")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        (
            "Item",
            Tag::compound([("id", string("minecraft:stone")), ("count", Tag::Int(3))]),
        ),
    ]);
    let e = item_entity_from_nbt(&raw, 1, 0.0).unwrap();
    let tag = item_entity_to_nbt(&e, [0; 4], Some(&raw));
    assert_eq!(text(tag.get("CustomName")).as_deref(), Some("x"));
    assert_eq!(number(tag.get("Health")), Some(5));
}
#[test]
fn armor_stand_loads_and_saves_pitch() {
    let tag = Tag::compound([
        ("id", string("minecraft:armor_stand")),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(4103.5f64.to_bits()),
                Tag::Double(300.0f64.to_bits()),
                Tag::Double(4103.5f64.to_bits()),
            ]),
        ),
        (
            "Rotation",
            Tag::List(vec![
                Tag::Float((-170.5f32).to_bits()),
                Tag::Float((-90.0f32).to_bits()),
            ]),
        ),
    ]);
    let stand = armor_stand_from_nbt(&tag, 1).unwrap();
    assert_eq!(stand.yaw, -170.5);
    assert_eq!(stand.pitch, -90.0);

    let saved = armor_stand_to_nbt(&stand, [1, 2, 3, 4], None);
    let rot = rotation(&saved);
    assert_eq!(rot[0], -170.5);
    assert_eq!(rot[1], -90.0);

    // Out-of-range pitch (e.g. 450.0f) normalizes to 90.0f matching Entity.setXRot.
    let out_of_range = Tag::compound([
        ("id", string("minecraft:armor_stand")),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
            ]),
        ),
        (
            "Rotation",
            Tag::List(vec![
                Tag::Float(0.0f32.to_bits()),
                Tag::Float(450.0f32.to_bits()),
            ]),
        ),
    ]);
    let normalized = armor_stand_from_nbt(&out_of_range, 2).unwrap();
    assert_eq!(normalized.pitch, 90.0);

    // Nonfinite pitch resets to 0.0f.
    let nonfinite = Tag::compound([
        ("id", string("minecraft:armor_stand")),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
            ]),
        ),
        (
            "Rotation",
            Tag::List(vec![
                Tag::Float(0.0f32.to_bits()),
                Tag::Float(f32::NAN.to_bits()),
            ]),
        ),
    ]);
    let nan_stand = armor_stand_from_nbt(&nonfinite, 3).unwrap();
    assert_eq!(nan_stand.pitch, 0.0);
}

#[test]
fn negative_facing_wraps_like_vanilla_direction_codecs() {
    // from3DDataValue: abs(-1 % 6) = 1 (UP); 7 -> 1; -6 -> 0.
    for (raw, want) in [(-1i8, 1u8), (7, 1), (-6, 0), (5, 5), (0, 0)] {
        let tag = Tag::compound([
            ("Facing", Tag::Byte(raw)),
            ("block_pos", Tag::IntArray(vec![0, 0, 0])),
        ]);
        assert_eq!(item_frame_from_nbt(&tag, 1).unwrap().facing, want);
    }
    // from2DDataValue: abs(-1 % 4) = 1.
    let tag = Tag::compound([
        ("facing", Tag::Byte(-1)),
        ("block_pos", Tag::IntArray(vec![0, 0, 0])),
    ]);
    assert_eq!(painting_from_nbt(&tag).unwrap().1, 1);
}

#[test]
fn saved_rotation_is_the_entitys_own() {
    // `/tp <entity> x y z 90 10` sets the entity's yaw and pitch; the save
    // (`/data get ... Rotation`) writes them, not [0, 0].
    let tnt = tnt_to_nbt(
        Vec3::ZERO,
        Vec3::ZERO,
        false,
        0.0,
        20,
        4.0,
        string("minecraft:tnt"),
        [90.0, 10.0],
        [1; 4],
        None,
        None,
    );
    assert_eq!(rotation(&tnt), [90.0, 10.0]);
    let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
    stand.yaw = 90.0;
    stand.pitch = 10.0;
    assert_eq!(
        rotation(&armor_stand_to_nbt(&stand, [1; 4], None)),
        [90.0, 10.0]
    );
    let cart = vehicle_to_nbt(
        "minecraft:minecart",
        Vec3::ZERO,
        Vec3::ZERO,
        [90.0, 10.0],
        (false, 0.0),
        [1; 4],
        None,
    );
    assert_eq!(rotation(&cart), [90.0, 10.0]);
}

#[test]
fn entities_without_fire_save_zero_and_a_mob_saves_its_speed_instance() {
    // Entity.remainingFireTicks starts at 0; LivingEntity.removeFrost makes
    // the movement_speed instance AttributeMap.pack then saves.
    let orb = orbs_from_nbt(&Tag::compound([("Pos", doubles(&[0.0; 3]))])).remove(0);
    let tag = orb_to_nbt(&orb, [0; 4]);
    assert_eq!(tag.get("Fire"), Some(&Tag::Short(0)));
    // A stand summoned this tick has no instance yet; its first tick makes
    // it, and a save that lists it (`/data merge`, a reload) keeps it.
    let attributes =
        |stand: &ArmorStand| match armor_stand_to_nbt(stand, [0; 4], None).get("attributes") {
            Some(Tag::List(attrs)) => attrs
                .iter()
                .map(|a| (text(a.get("id")).unwrap(), a.get("base").cloned()))
                .collect::<Vec<_>>(),
            other => panic!("attributes: {other:?}"),
        };
    let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
    assert_eq!(attributes(&stand), []);
    stand.speed_instance = true;
    let speed = vec![(
        "minecraft:movement_speed".to_owned(),
        Some(Tag::Double(0.7f64.to_bits())),
    )];
    assert_eq!(attributes(&stand), speed);
    let saved = armor_stand_to_nbt(&stand, [0; 4], None);
    assert_eq!(attributes(&armor_stand_from_nbt(&saved, 1).unwrap()), speed);
}
#[test]
fn a_loaded_stand_gains_the_speed_instance_its_first_tick_makes() {
    // Vanilla 26.3: `/summon armor_stand ~ ~ ~ {attributes:[]}` saves `[]` until the
    // first tick, whose removeFrost makes movement_speed (base 0.7d); a saved
    // instance of another attribute stays in front of it.
    let ids = |stand: &ArmorStand, raw: &Tag| match armor_stand_to_nbt(stand, [0; 4], Some(raw))
        .get("attributes")
    {
        Some(Tag::List(attrs)) => attrs
            .iter()
            .map(|a| (text(a.get("id")).unwrap(), a.get("base").cloned()))
            .collect::<Vec<_>>(),
        other => panic!("attributes: {other:?}"),
    };
    let health = (
        "minecraft:max_health".to_owned(),
        Some(Tag::Double(20f64.to_bits())),
    );
    let speed = (
        "minecraft:movement_speed".to_owned(),
        Some(Tag::Double(0.7f64.to_bits())),
    );
    let empty = Tag::compound([("attributes", Tag::List(Vec::new()))]);
    let saved = Tag::compound([(
        "attributes",
        Tag::List(vec![Tag::compound([
            ("id", string("minecraft:max_health")),
            ("base", Tag::Double(20f64.to_bits())),
        ])]),
    )]);
    let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
    assert_eq!(ids(&stand, &empty), []);
    assert_eq!(ids(&stand, &saved), std::slice::from_ref(&health));
    stand.speed_instance = true;
    assert_eq!(ids(&stand, &empty), std::slice::from_ref(&speed));
    assert_eq!(ids(&stand, &saved), [health, speed]);
}
#[test]
fn summoned_trident_defaults_are_not_pickable_or_already_shot() {
    let raw = Tag::compound([
        ("Pos", doubles(&[0.0, 0.0, 0.0])),
        ("LeftOwner", Tag::Byte(0)),
    ]);
    let loaded = trident_from_nbt(&raw).unwrap();
    assert_eq!(loaded.pickup, Pickup::Disallowed);
    assert!(!loaded.left_owner && !loaded.has_been_shot);
    let trident = ItemStack::new("minecraft:trident", 1);
    let tag = trident_to_nbt(
        &TridentSave {
            position: Vec3::ZERO,
            motion: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            in_ground: false,
            life: 0,
            shake: 0,
            dealt_damage: false,
            pickup: loaded.pickup,
            left_owner: loaded.left_owner,
            has_been_shot: loaded.has_been_shot,
            owner: loaded.owner,
            no_gravity: false,
            on_ground: false,
            fire_ticks: 0,
            fall_distance: 0.0,
            sound_event: TRIDENT_HIT_GROUND,
            item: &trident,
        },
        [0; 4],
        Some(&raw),
    );
    assert!(tag.get("LeftOwner").is_none());
    assert_eq!(tag.get("HasBeenShot"), Some(&Tag::Byte(0)));
    assert_eq!(tag.get("pickup"), Some(&Tag::Byte(0)));
}
#[test]
fn orbs_keep_their_count() {
    let at = ("Pos", doubles(&[1.0, 2.0, 3.0]));
    let loaded = [at, ("Value", Tag::Short(7)), ("Age", Tag::Short(3))];
    let o = orbs_from_nbt(&Tag::compound(
        loaded
            .into_iter()
            .chain([("Count", Tag::Int(3)), ("Air", Tag::Short(32767))]),
    ))
    .remove(0);
    let tag = orb_to_nbt(&o, [0; 4]);
    let back = orbs_from_nbt(&tag);
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].value, 7);
    assert_eq!(back[0].count, 3);
    // Entity.load keeps the saved Air; the orb writes it back unchanged.
    assert_eq!(back[0].air, 32767);
    assert_eq!(tag.get("Air"), Some(&Tag::Short(32767)));
    assert_eq!(
        orb_to_nbt(&back[0], [0; 4]).get("Air"),
        Some(&Tag::Short(32767))
    );
}
#[test]
fn every_orb_set_experience_orb_accepts_reloads_unchanged() {
    let at = Tag::compound([("Pos", doubles(&[1.0, 2.0, 3.0]))]);
    let mut o = orbs_from_nbt(&at).remove(0);
    for (value, count) in [(11, 1), (32767, 2147483647), (-32768, 1)] {
        (o.value, o.count) = (value, count);
        let back = orbs_from_nbt(&orb_to_nbt(&o, [0; 4])).remove(0);
        assert_eq!((back.value, back.count), (value, count));
    }
}
/// `Entity.saveWithoutId` writes `Fire` for every entity and `Entity.load`
/// reads it back with `getShortOr("Fire", 0)`: a summon's `Fire:40s` and the
/// `-1s` an entity carries after leaving water both survive, an absent key
/// is 0, and the short wraps like `(short) remainingFireTicks`.
#[test]
fn orbs_save_and_load_their_fire() {
    let mut o = ExperienceOrb {
        attachment_lifetime: Default::default(),
        rotation: [0.0, 0.0],
        join: Default::default(),
        section_order: 0,
        no_gravity: false,
        id: 0,
        position: Vec3::new(1.0, 2.0, 3.0),
        previous: Vec3::ZERO,
        velocity: Vec3::ZERO,
        value: 7,
        age: 3,
        on_ground: false,
        count: 1,
        ticks: 0,
        health: 5,
        air: 300,
        following: None,
        fall_distance: 0.0,
        fire_ticks: 0,
    };
    for (ticks, saved) in [(0, 0), (40, 40), (-1, -1), (300, 300), (40_000, -25_536)] {
        o.fire_ticks = ticks;
        let tag = orb_to_nbt(&o, [0; 4]);
        assert_eq!(tag.get("Fire"), Some(&Tag::Short(saved)), "saved {ticks}");
        assert_eq!(orbs_from_nbt(&tag)[0].fire_ticks, i32::from(saved));
    }
    let mut tag = orb_to_nbt(&o, [0; 4]);
    let Tag::Compound(map) = &mut tag else {
        unreachable!()
    };
    map.remove(&"Fire".into());
    assert_eq!(orbs_from_nbt(&tag)[0].fire_ticks, 0);
}
/// `Entity.baseTick` takes one tick off a burning `Fire` (`Fire:300s` reads
/// 299 after one tick); 0 and -1 stay. A fire-immune cloud clears only a
/// positive `Fire` on its tick.
#[test]
fn a_loaded_fire_counts_down_and_a_cloud_clears_it() {
    let fire = |snbt: &str| {
        let mut tag = horizon_core::nbt::parse_snbt(snbt).unwrap().0;
        super::count_down_fire(&mut tag);
        number(tag.get("Fire"))
    };
    assert_eq!(fire("{Fire:300s}"), Some(299));
    assert_eq!(fire("{Fire:1s}"), Some(0));
    assert_eq!(fire("{Fire:0s}"), Some(0));
    assert_eq!(fire("{Fire:-1s}"), Some(-1));
    let cleared = |snbt: &str| {
        let mut tag = horizon_core::nbt::parse_snbt(snbt).unwrap().0;
        super::clear_positive_fire(&mut tag);
        number(tag.get("Fire"))
    };
    assert_eq!(cleared("{Fire:300s}"), Some(0));
    assert_eq!(cleared("{Fire:-5s}"), Some(-5));
    assert_eq!(cleared("{Fire:0s}"), Some(0));
    assert_eq!(cleared("{Age:5}"), None);
}
