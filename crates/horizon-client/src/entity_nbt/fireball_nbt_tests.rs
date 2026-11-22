use super::*;

/// `WitherSkull`: a skull in flight is saved with its chunk, `dangerous`
/// is a byte (absent means a black skull), and it never saves a fire
/// because `WitherSkull.shouldBurn` is false.
#[test]
fn a_wither_skull_keeps_whether_it_is_dangerous() {
    let blue = Tag::compound([
        ("id", Tag::String("minecraft:wither_skull".into())),
        ("Pos", doubles(&[3.0, 64.0, 3.0])),
        ("Motion", doubles(&[0.0, 0.0, 0.0])),
        ("dangerous", Tag::Byte(1)),
    ]);
    let mut skull = fireball_from_nbt(&blue, None).unwrap();
    assert_eq!(skull.kind, ProjectileKind::WitherSkull { dangerous: true });
    assert!(saved_with_chunk(&skull));
    skull.age = 5;
    let back = fireball_to_nbt(&skull, [1, 2, 3, 4], None, None).unwrap();
    assert_eq!(kind(&back).as_deref(), Some("minecraft:wither_skull"));
    assert_eq!(number(back.get("dangerous")), Some(1));
    // Entity saves Fire every time; a skull's stays 0 (a fireball's is 20 once ticked).
    assert_eq!(number(back.get("Fire")), Some(0));

    let black = Tag::compound([
        ("id", Tag::String("minecraft:wither_skull".into())),
        ("Pos", doubles(&[0.0, 0.0, 0.0])),
    ]);
    let skull = fireball_from_nbt(&black, None).unwrap();
    assert_eq!(skull.kind, ProjectileKind::WitherSkull { dangerous: false });
    let back = fireball_to_nbt(&skull, [1, 2, 3, 4], None, None).unwrap();
    assert_eq!(number(back.get("dangerous")), Some(0));
}

#[test]
fn a_fireball_round_trips_its_flight() {
    let raw = Tag::compound([
        ("id", Tag::String("minecraft:fireball".into())),
        ("Pos", doubles(&[1.0, 70.0, -2.5])),
        ("Motion", doubles(&[0.0, -0.5, 1.0])),
        ("ExplosionPower", Tag::Byte(4)),
        ("acceleration_power", Tag::Double(0.0f64.to_bits())),
    ]);
    let p = fireball_from_nbt(&raw, None).unwrap();
    assert_eq!(p.kind, ProjectileKind::LargeFireball { power: 4 });
    assert_eq!((p.velocity.z, p.acceleration), (1.0, 0.0));
    let back = fireball_to_nbt(&p, [1, 2, 3, 4], None, None).unwrap();
    assert_eq!(number(back.get("ExplosionPower")), Some(4));
    assert_eq!(float(back.get("acceleration_power")), Some(0.0));
    // Projectile.addAdditionalSaveData: no LeftOwner unless set, HasBeenShot always.
    assert!(back.get("LeftOwner").is_none());
    assert_eq!(number(back.get("HasBeenShot")), Some(0));
    assert_eq!(
        read_doubles::<3>(back.get("Motion")),
        Some([0.0, -0.5, 1.0])
    );
    // A default fireball keeps vanilla's power and acceleration.
    let plain = Tag::compound([
        ("id", Tag::String("minecraft:fireball".into())),
        ("Pos", doubles(&[0.0, 0.0, 0.0])),
    ]);
    let p = fireball_from_nbt(&plain, None).unwrap();
    assert_eq!(
        (p.kind, p.acceleration),
        (ProjectileKind::LargeFireball { power: 1 }, 0.1)
    );
    // `/summon` and a saved world give back both wind charges.
    for (id, kind) in [
        ("minecraft:wind_charge", ProjectileKind::WindCharge),
        (
            "minecraft:breeze_wind_charge",
            ProjectileKind::BreezeWindCharge,
        ),
    ] {
        let tag = Tag::compound([
            ("id", Tag::String(id.into())),
            ("Pos", doubles(&[0.0, 0.0, 0.0])),
        ]);
        assert_eq!(fireball_from_nbt(&tag, None).unwrap().kind, kind);
    }
}

/// `Fireball.addAdditionalSaveData` / `readAdditionalSaveData`: both fireballs save
/// `Item`, a fire charge by default, and a loaded one stays, as one item.
#[test]
fn a_fireball_saves_its_item() {
    let item_of = |tag: &Tag| {
        let item = tag.get("Item").and_then(item_from_nbt).unwrap();
        (item.item.to_string(), item.count)
    };
    for id in ["minecraft:fireball", "minecraft:small_fireball"] {
        let plain = Tag::compound([
            ("id", Tag::String(id.into())),
            ("Pos", doubles(&[0.0, 0.0, 0.0])),
        ]);
        let p = fireball_from_nbt(&plain, None).unwrap();
        let back = fireball_to_nbt(&p, [1, 2, 3, 4], None, Some(&plain)).unwrap();
        assert_eq!(
            item_of(&back),
            ("minecraft:fire_charge".to_owned(), 1),
            "{id}"
        );
        // A loaded item stays, as a single one, on the entity itself.
        let mut loaded = plain.clone();
        put(
            &mut loaded,
            "Item",
            item_to_nbt(&crate::items::ItemStack::new("minecraft:diamond", 5)),
        );
        let p = fireball_from_nbt(&loaded, None).unwrap();
        let held = p.item.as_ref().unwrap();
        assert_eq!((&*held.item, held.count), ("minecraft:diamond", 1), "{id}");
        // `addAdditionalSaveData` writes `getItem()`: the item in the
        // compound it was loaded from does not replace the one it holds,
        // which is what `/data get` and the chunk save then report.
        let mut stale = plain.clone();
        put(
            &mut stale,
            "Item",
            item_to_nbt(&crate::items::ItemStack::new("minecraft:stone", 8)),
        );
        let back = fireball_to_nbt(&p, [1, 2, 3, 4], None, Some(&stale)).unwrap();
        assert_eq!(item_of(&back), ("minecraft:diamond".to_owned(), 1), "{id}");
        // An empty one falls back to the fire charge.
        let mut empty = plain.clone();
        put(
            &mut empty,
            "Item",
            item_to_nbt(&crate::items::ItemStack::new("minecraft:air", 1)),
        );
        let p = fireball_from_nbt(&empty, None).unwrap();
        let back = fireball_to_nbt(&p, [1, 2, 3, 4], None, Some(&empty)).unwrap();
        assert_eq!(
            item_of(&back),
            ("minecraft:fire_charge".to_owned(), 1),
            "{id}"
        );
    }
    // The wind charges are not `Fireball`s: they save no item.
    let wind = Tag::compound([
        ("id", Tag::String("minecraft:wind_charge".into())),
        ("Pos", doubles(&[0.0, 0.0, 0.0])),
    ]);
    let p = fireball_from_nbt(&wind, None).unwrap();
    let back = fireball_to_nbt(&p, [1, 2, 3, 4], None, Some(&wind)).unwrap();
    assert!(back.get("Item").is_none());
}

#[test]
fn a_summoned_potion_flies_as_the_item_its_compound_names() {
    // `ThrowableItemProjectile.getDefaultItem`: the item its entity type is
    // named after, so a bare summon is a splash potion with no effects and
    // the chunk saves it (ThrowableProjectile is not transient).
    for id in ["minecraft:splash_potion", "minecraft:lingering_potion"] {
        let bare = Tag::compound([
            ("id", Tag::String(id.into())),
            ("Pos", doubles(&[24.5, 216.0, 24.5])),
        ]);
        let p = throwable_from_nbt(&bare, None).unwrap();
        assert_eq!(p.kind.id(), id);
        assert_eq!(
            p.kind,
            ProjectileKind::Potion(
                Box::new(crate::items::ItemStack::new(id, 1)),
                id == "minecraft:lingering_potion"
            )
        );
        assert!(saved_with_chunk(&p));
        let tag = throwable_to_nbt(&p, [1; 4], None, None).unwrap();
        let item = tag.get("Item").and_then(item_from_nbt).unwrap();
        assert_eq!((&*item.item, item.count), (id, 1));
        assert_eq!(item.potion(), None);
    }
    // `readAdditionalSaveData`: the `Item` it flies as, `potion_contents`
    // and all, so what it splashes is the potion's.
    let harming = Tag::compound([
        ("id", Tag::String("minecraft:splash_potion".into())),
        ("Pos", doubles(&[24.5, 216.0, 24.5])),
        (
            "Item",
            item_to_nbt(
                &crate::items::ItemStack::new("minecraft:splash_potion", 1)
                    .with_potion("minecraft:harming"),
            ),
        ),
    ]);
    let p = throwable_from_nbt(&harming, None).unwrap();
    let ProjectileKind::Potion(stack, _) = &p.kind else {
        panic!("a potion");
    };
    assert_eq!(stack.potion(), Some("minecraft:harming"));
    // The stack stays in the tag it was loaded from.
    let tag = throwable_to_nbt(&p, [1; 4], None, Some(&harming)).unwrap();
    let item = tag.get("Item").and_then(item_from_nbt).unwrap();
    assert_eq!(item.potion(), Some("minecraft:harming"));
    // A lingering potion is its own entity type.
    let lingering = Tag::compound([
        ("id", Tag::String("minecraft:lingering_potion".into())),
        ("Pos", doubles(&[0.0, 64.0, 0.0])),
        (
            "Item",
            item_to_nbt(
                &crate::items::ItemStack::new("minecraft:lingering_potion", 1)
                    .with_potion("minecraft:poison"),
            ),
        ),
    ]);
    let p = throwable_from_nbt(&lingering, None).unwrap();
    assert_eq!(p.kind.id(), "minecraft:lingering_potion");
}

#[test]
fn wind_charges_in_flight_are_saved_and_read_back() {
    // vanilla 26.3 saves them (`Entity.shouldBeSaved` is true; the round
    // trip of a vanilla world lost all four before this).
    for id in ["minecraft:wind_charge", "minecraft:breeze_wind_charge"] {
        let saved = Tag::compound([
            ("id", Tag::String(id.into())),
            ("Pos", doubles(&[4.5, 80.0, 10.5])),
            ("NoGravity", Tag::Byte(1)),
            ("acceleration_power", Tag::Double(0.1_f64.to_bits())),
        ]);
        let p = fireball_from_nbt(&saved, None).unwrap();
        assert_eq!(p.kind.id(), id);
        assert!(saved_with_chunk(&p), "{id}");
        let tag = fireball_to_nbt(&p, [1; 4], None, Some(&saved)).unwrap();
        assert_eq!(kind(&tag).as_deref(), Some(id));
        assert_eq!(tag.get("NoGravity"), Some(&Tag::Byte(1)));
    }
}

#[test]
fn a_lingering_potion_keeps_its_type_whatever_item_it_flies_as() {
    // `/summon minecraft:lingering_potion ~ ~ ~ {Item:{id:"minecraft:splash_potion"}}`
    // is a `ThrownLingeringPotion` in vanilla and stays one when saved.
    let tag = Tag::compound([
        ("id", Tag::String("minecraft:lingering_potion".into())),
        ("Pos", doubles(&[24.5, 80.0, 8.5])),
        (
            "Item",
            item_to_nbt(&crate::items::ItemStack::new("minecraft:splash_potion", 1)),
        ),
    ]);
    let p = throwable_from_nbt(&tag, None).unwrap();
    assert_eq!(p.kind.id(), "minecraft:lingering_potion");
    let back = throwable_to_nbt(&p, [1; 4], None, Some(&tag)).unwrap();
    assert_eq!(kind(&back).as_deref(), Some("minecraft:lingering_potion"));
    let item = back.get("Item").and_then(item_from_nbt).unwrap();
    assert_eq!(&*item.item, "minecraft:splash_potion");
}

#[test]
fn a_painting_keeps_its_saved_pitch_and_takes_its_yaw_from_the_facing() {
    // Entity.load sets both angles; Painting.readAdditionalSaveData's
    // setDirection only rewrites the yaw. `/summon painting ...
    // {Rotation:[45.0f,10.0f]}` reads back `Rotation: [0.0f, 10.0f]`.
    let painting_at = |rotation: Option<Tag>| {
        let mut tag = Tag::compound([
            ("id", Tag::String("minecraft:painting".into())),
            ("Pos", doubles(&[28.5, 80.0, 6.5])),
            ("variant", Tag::String("minecraft:kebab".into())),
            ("facing", Tag::Byte(3)),
        ]);
        if let (Tag::Compound(map), Some(rotation)) = (&mut tag, rotation) {
            map.insert("Rotation".into(), rotation);
        }
        tag
    };
    let saved = |raw: &Tag| {
        let (pos, facing, variant) = painting_from_nbt(raw).unwrap();
        let painting = crate::entities::Painting {
            attachment_lifetime: Default::default(),
            join: Default::default(),
            id: 1,
            pos,
            facing,
            variant: variant.unwrap().into(),
            width: 1,
            height: 1,
            check_interval: 0,
        };
        rotation(&painting_to_nbt(&painting, [1; 4], Some(raw)))
    };
    assert_eq!(
        saved(&painting_at(Some(floats(&[45.0, 10.0])))),
        [270.0, 10.0]
    );
    // setXRot: `xRot % 360` clamped to +-90.
    assert_eq!(
        saved(&painting_at(Some(floats(&[0.0, -400.0])))),
        [270.0, -40.0]
    );
    assert_eq!(
        saved(&painting_at(Some(floats(&[0.0, 135.0])))),
        [270.0, 90.0]
    );
    // A non-finite pitch is ignored, as is no Rotation at all.
    assert_eq!(
        saved(&painting_at(Some(floats(&[0.0, f32::NAN])))),
        [270.0, 0.0]
    );
    assert_eq!(saved(&painting_at(None)), [270.0, 0.0]);
}

#[test]
fn a_summoned_painting_or_frame_has_no_block_pos_and_sits_at_its_position() {
    // BlockAttachedEntity.setPos: the block holding the summon position.
    let painting = Tag::compound([
        ("id", Tag::String("minecraft:painting".into())),
        ("Pos", doubles(&[28.5, 80.0, 6.5])),
        ("variant", Tag::String("minecraft:kebab".into())),
        ("facing", Tag::Byte(3)),
    ]);
    assert_eq!(
        painting_from_nbt(&painting),
        Some(([28, 80, 6], 3, Some("minecraft:kebab".to_string())))
    );
    // No variant saved (a bare summon): the position and facing still load.
    let mut bare = painting.clone();
    if let Tag::Compound(m) = &mut bare {
        m.remove(&"variant".into());
    }
    assert_eq!(painting_from_nbt(&bare), Some(([28, 80, 6], 3, None)));
    let frame = Tag::compound([
        ("id", Tag::String("minecraft:item_frame".into())),
        ("Pos", doubles(&[-0.5, 80.0, 6.5])),
        ("Facing", Tag::Byte(1)),
    ]);
    assert_eq!(item_frame_from_nbt(&frame, 1).unwrap().pos, [-1, 80, 6]);
    // BlockAttachedEntity.readAdditionalSaveData: a block_pos 16 blocks or
    // more from the Pos block is refused for that block.
    let mut far = painting.clone();
    put(&mut far, "block_pos", Tag::IntArray(vec![28, 80, 22]));
    assert_eq!(painting_from_nbt(&far).map(|p| p.0), Some([28, 80, 6]));
    let mut near = painting.clone();
    put(&mut near, "block_pos", Tag::IntArray(vec![28, 80, 21]));
    assert_eq!(painting_from_nbt(&near).map(|p| p.0), Some([28, 80, 21]));
}

#[test]
fn frame_missing_facing_loads_down_and_valid_directions_round_trip() {
    // ItemFrame.readAdditionalSaveData: LEGACY_ID_CODEC, orElse(DOWN).
    for kind in ["minecraft:item_frame", "minecraft:glow_item_frame"] {
        // Explicit controls run before the missing-field assertion.
        for facing in [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), None] {
            let mut tag = Tag::compound([
                ("id", string(kind)),
                ("block_pos", Tag::IntArray(vec![4, 102, 6])),
                ("Fixed", Tag::Byte(1)),
                ("ItemRotation", Tag::Byte(5)),
                ("ItemDropChance", Tag::Float(0.25_f32.to_bits())),
                (
                    "Item",
                    item_to_nbt(&crate::items::ItemStack::new("minecraft:diamond", 1)),
                ),
            ]);
            if let Some(direction) = facing {
                put(&mut tag, "Facing", Tag::Byte(direction));
            }
            let frame = item_frame_from_nbt(&tag, 17).unwrap();
            assert_eq!(frame.kind(), kind);
            assert_eq!(frame.id, 17);
            assert!(frame.fixed);
            assert_eq!(
                frame.item.as_ref().unwrap().item.as_ref(),
                "minecraft:diamond"
            );
            for raw in [None, Some(&tag)] {
                let saved = item_frame_to_nbt(&frame, [1, 2, 3, 4], raw);
                assert_eq!(
                    saved.get("Facing"),
                    Some(&Tag::Byte(facing.unwrap_or(0))),
                    "{kind}"
                );
                assert_eq!(saved.get("Item"), tag.get("Item"));
                assert_eq!(saved.get("ItemRotation"), Some(&Tag::Byte(5)));
                assert_eq!(saved.get("ItemDropChance"), tag.get("ItemDropChance"));
                assert_eq!(saved.get("Fixed"), Some(&Tag::Byte(1)));
                assert_eq!(saved.get("block_pos"), tag.get("block_pos"));
                if facing.is_none() {
                    assert_eq!(saved.get("Pos"), Some(&doubles(&[4.5, 102.96875, 6.5])));
                }
            }
        }
    }
}

#[test]
fn frame_rotation_uses_signed_byte_value_and_round_trips() {
    // `ItemFrame.readAdditionalSaveData`: getByteOr, then setRotation's `% 8`.
    for kind in ["minecraft:item_frame", "minecraft:glow_item_frame"] {
        for (rotation, expected) in [
            (Some(Tag::Byte(-1)), -1),
            (Some(Tag::Byte(-9)), -1),
            (Some(Tag::Byte(-128)), 0),
            (Some(Tag::Byte(0)), 0),
            (Some(Tag::Byte(7)), 7),
            (Some(Tag::Byte(8)), 0),
            (Some(Tag::Byte(127)), 7),
            (Some(Tag::Short(255)), -1),
            (Some(Tag::Int(129)), -7),
            (Some(Tag::Int(-129)), 7),
            (Some(Tag::Long(255)), -1),
            (Some(Tag::Float((-0.5_f32).to_bits())), -1),
            (Some(Tag::Double((-9.5_f64).to_bits())), -2),
            (Some(string("-1")), 0),
            (None, 0),
        ] {
            let mut tag = Tag::compound([
                ("id", string(kind)),
                ("block_pos", Tag::IntArray(vec![4, 102, 6])),
            ]);
            if let Some(rotation) = rotation {
                put(&mut tag, "ItemRotation", rotation);
            }
            let frame = item_frame_from_nbt(&tag, 17).unwrap();
            assert_eq!(frame.rotation, expected, "{kind}: {tag:?}");
            for raw in [None, Some(&tag)] {
                let saved = item_frame_to_nbt(&frame, [1, 2, 3, 4], raw);
                assert_eq!(saved.get("ItemRotation"), Some(&Tag::Byte(expected)));
                assert_eq!(item_frame_from_nbt(&saved, 17).unwrap().rotation, expected);
            }
        }
    }
}

#[test]
fn a_named_frame_drops_with_its_custom_name() {
    // `ItemFrame.getFrameItemStackWithData`: the frame item, then
    // `DataComponents.CUSTOM_NAME` from `getCustomName`. Glow frames
    // share it through `GlowItemFrame.getFrameItemStack`.
    for id in ["minecraft:item_frame", "minecraft:glow_item_frame"] {
        let tag = Tag::compound([
            ("id", string(id)),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
            ("Facing", Tag::Byte(2)),
            ("CustomName", string("Gallery")),
        ]);
        let frame = item_frame_from_nbt(&tag, 1).unwrap();
        let stack = frame.item_stack_with_data();
        assert_eq!(&*stack.item, id);
        assert_eq!(stack.count, 1);
        assert_eq!(stack.custom_name().as_deref(), Some("Gallery"));
        let saved = item_frame_to_nbt(&frame, [1, 2, 3, 4], None);
        assert_eq!(text(saved.get("CustomName")).as_deref(), Some("Gallery"));
    }
    let bare = Tag::compound([
        ("id", string("minecraft:item_frame")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("Facing", Tag::Byte(3)),
    ]);
    let frame = item_frame_from_nbt(&bare, 2).unwrap();
    assert!(frame.custom_name.is_none());
    assert!(frame.item_stack_with_data().custom_name().is_none());
    assert!(
        item_frame_to_nbt(&frame, [1, 2, 3, 4], Some(&bare))
            .get("CustomName")
            .is_none()
    );
}

#[test]
fn projectile_shot_flag_round_trips_independently_of_age() {
    for id in [
        "minecraft:snowball",
        "minecraft:fireball",
        "minecraft:llama_spit",
        "minecraft:shulker_bullet",
    ] {
        for shot in [0, 1] {
            let raw = Tag::compound([
                ("id", Tag::String(id.into())),
                ("Pos", doubles(&[1.0, 70.0, 2.0])),
                ("HasBeenShot", Tag::Byte(shot)),
            ]);
            let mut p = throwable_from_nbt(&raw, None)
                .or_else(|| fireball_from_nbt(&raw, None))
                .or_else(|| spit_or_bullet_from_nbt(&raw, None, None))
                .unwrap();
            assert_eq!(p.has_been_shot, shot != 0);
            for age in [0, 12] {
                p.age = age;
                let tag = throwable_to_nbt(&p, [1, 2, 3, 4], None, Some(&raw))
                    .or_else(|| fireball_to_nbt(&p, [1, 2, 3, 4], None, Some(&raw)))
                    .or_else(|| spit_or_bullet_to_nbt(&p, [1, 2, 3, 4], None, None, Some(&raw)))
                    .unwrap();
                assert_eq!(number(tag.get("HasBeenShot")), Some(i64::from(shot)));
            }
        }
    }
}

#[test]
fn thrown_item_projectiles_round_trip_their_flight_item_and_owner() {
    let player = Some([5, 6, 7, 8]);
    for (kind, id) in [
        (ProjectileKind::Snowball, "minecraft:snowball"),
        (ProjectileKind::Egg, "minecraft:egg"),
        (ProjectileKind::EnderPearl, "minecraft:ender_pearl"),
    ] {
        let mut p = Projectile::new(kind.clone(), Vec3::new(1.5, 70.0, -2.5), Some(Who::Player));
        (p.velocity, p.left_owner, p.age) = (Vec3::new(0.3, -0.1, 1.2), true, 4);
        p.has_been_shot = true;
        [p.yaw, p.pitch] = [30.0, -10.0];
        let tag = throwable_to_nbt(&p, [1, 2, 3, 4], player, None).unwrap();
        assert_eq!(super::kind(&tag).as_deref(), Some(id));
        // ThrowableItemProjectile: the item it flies as, by its default.
        let item = tag.get("Item").and_then(item_from_nbt).unwrap();
        assert_eq!((&*item.item, item.count), (id, 1));
        assert_eq!(number(tag.get("HasBeenShot")), Some(1));
        assert_eq!(uuid_from_tag(tag.get("Owner")), player);
        let back = throwable_from_nbt(&tag, player).unwrap();
        assert_eq!((back.kind, back.owner), (kind, Some(Who::Player)));
        assert_eq!((back.position, back.velocity), (p.position, p.velocity));
        assert_eq!(
            ([back.yaw, back.pitch], back.left_owner),
            ([30.0, -10.0], true)
        );
        // an owner who is not here keeps their UUID through a moved resave.
        let mut away = throwable_from_nbt(&tag, Some([9; 4])).unwrap();
        assert_eq!((away.owner, away.owner_uuid), (None, player));
        away.position.y += 3.0;
        let resaved = throwable_to_nbt(&away, [1; 4], away.owner_uuid, None).unwrap();
        assert_eq!(uuid_from_tag(resaved.get("Owner")), player);
    }
    // the chunk saves arrows, fireballs, wind charges and these, and so takes them with it.
    let kinds = [
        (ProjectileKind::SmallFireball, true),
        (ProjectileKind::Snowball, true),
        (ProjectileKind::WindCharge, true),
        (ProjectileKind::BreezeWindCharge, true),
        (ProjectileKind::Trident, false),
    ];
    for (kind, saved) in kinds {
        let p = Projectile::new(kind, Vec3::ZERO, None);
        assert_eq!(saved_with_chunk(&p), saved, "{:?}", p.kind);
    }
    // a loaded item stays, and a new projectile leaves LeftOwner out.
    let raw = Tag::compound([
        ("id", Tag::String("minecraft:egg".into())),
        ("Pos", doubles(&[0.0, 64.0, 0.0])),
        (
            "Item",
            item_to_nbt(&crate::items::ItemStack::new("minecraft:blue_egg", 1)),
        ),
    ]);
    let egg = throwable_from_nbt(&raw, None).unwrap();
    // With or without the tag it was loaded from, which is lost once it moves.
    for raw in [Some(&raw), None] {
        let tag = throwable_to_nbt(&egg, [1; 4], None, raw).unwrap();
        let item = tag.get("Item").and_then(item_from_nbt).unwrap();
        assert_eq!(&*item.item, "minecraft:blue_egg");
    }
    let tag = throwable_to_nbt(&egg, [1; 4], None, Some(&raw)).unwrap();
    assert!(tag.get("LeftOwner").is_none() && tag.get("Owner").is_none());
    assert_eq!(number(tag.get("HasBeenShot")), Some(0));
    let wind = Projectile::new(ProjectileKind::WindCharge, Vec3::ZERO, None);
    assert!(throwable_to_nbt(&wind, [1; 4], None, None).is_none());
    // NoGravity is loaded and saved again (a summoned pearl that floats).
    let floating = Tag::compound([
        ("id", Tag::String("minecraft:ender_pearl".into())),
        ("Pos", doubles(&[0.0, 64.0, 0.0])),
        ("NoGravity", Tag::Byte(1)),
    ]);
    let pearl = throwable_from_nbt(&floating, None).unwrap();
    assert!(pearl.no_gravity);
    let tag = throwable_to_nbt(&pearl, [1; 4], None, None).unwrap();
    assert_eq!(number(tag.get("NoGravity")), Some(1));
    assert!(
        throwable_to_nbt(&egg, [1; 4], None, None)
            .unwrap()
            .get("NoGravity")
            .is_none()
    );
}
