use super::*;
#[test]
fn axolotl_boolean_memory_preserves_values_and_saved_lifetimes() {
    let key = "minecraft:has_hunting_cooldown";
    for (value, expected) in [
        (Some(Tag::Byte(0)), Some(false)),
        (Some(Tag::Byte(1)), Some(true)),
        (Some(Tag::Int(256)), Some(true)),
        (Some(Tag::Float(0.5f32.to_bits())), Some(true)),
        (Some(Tag::Double((-0.5f64).to_bits())), Some(true)),
        (Some(string("true")), None),
        (Some(Tag::Compound(Compound::new())), None),
        (None, None),
    ] {
        for ttl in [None, Some(3), Some(0), Some(-1), Some(i64::MAX)] {
            let mut entry = Tag::Compound(Compound::new());
            if let Some(value) = &value {
                put(&mut entry, "value", value.clone());
            }
            if let Some(ttl) = ttl {
                put(&mut entry, "ttl", Tag::Long(ttl));
            }
            let raw = Tag::compound([(
                "Brain",
                Tag::compound([("memories", Tag::compound([(key, entry)]))]),
            )]);
            let mut kind = crate::mobs::sensor_scan_tests::kind();
            let ty = Arc::make_mut(&mut kind);
            ty.id = "minecraft:axolotl".into();
            ty.species = crate::mobs::Species::Axolotl;
            let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
            restore_axolotl_memories(&mut mob, &raw);
            let saved = mob_to_nbt(&mob, [0; 4], Some(&raw), None, |_| None);
            let entry = saved
                .get("Brain")
                .and_then(|b| b.get("memories"))
                .and_then(|m| m.get(key));
            match expected {
                Some(value) => {
                    let entry = entry.expect("false and nonpositive lifetimes remain present");
                    assert_eq!(entry.get("value"), Some(&Tag::Byte(i8::from(value))));
                    let expected_ttl = ttl.filter(|v| *v != i64::MAX).map(Tag::Long);
                    assert_eq!(entry.get("ttl"), expected_ttl.as_ref());
                }
                None => assert!(entry.is_none(), "invalid/missing boolean must be dropped"),
            }
        }
    }
}
#[test]
fn aquatic_from_bucket_saves_defaults_and_tadpole_constant() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let types: HashMap<_, _> = crate::mobs::MobTypes::load(&root)
        .unwrap()
        .types
        .into_iter()
        .map(|(id, kind)| (id, Arc::new(kind)))
        .collect();
    for kind in [
        "cod",
        "salmon",
        "pufferfish",
        "tropical_fish",
        "axolotl",
        "tadpole",
    ] {
        for value in [None, Some(false), Some(true)] {
            let mut input = Tag::compound([
                ("id", string(&format!("minecraft:{kind}"))),
                ("Pos", doubles(&[10.5, 220.0, 10.5])),
            ]);
            if let (Some(value), Tag::Compound(map)) = (value, &mut input) {
                map.insert("FromBucket".into(), Tag::Byte(i8::from(value)));
            }
            let m = mob_from_nbt(&input, &types, 1, None).unwrap();
            let expected = kind == "tadpole" || value.unwrap_or(false);
            for raw in [None, Some(&input)] {
                let saved = mob_to_nbt(&m, [1; 4], raw, None, |_| None);
                assert_eq!(
                    saved.get("FromBucket"),
                    Some(&Tag::Byte(i8::from(expected))),
                    "{kind}, input={value:?}, raw={}",
                    raw.is_some()
                );
            }
        }
    }
    let pig = Mob::new(1, types["minecraft:pig"].clone(), Vec3::ZERO, 0.0);
    assert!(
        mob_to_nbt(&pig, [1; 4], None, None, |_| None)
            .get("FromBucket")
            .is_none()
    );
}

/// `HappyGhast.addAdditionalSaveData` / `readAdditionalSaveData`: the still timeout
/// (`serverStillTimeout`) goes with the world. A ghast saved while it was held still
/// comes back still, which is what keeps the box a dropped item rests on
/// (`EntityGetter.getEntityCollisions`) and the control a rider has
/// (`getControllingPassenger` reads `isOnStillTimeout`).
#[test]
fn a_happy_ghast_keeps_its_still_timeout() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<_, _> = data
        .types
        .into_iter()
        .map(|(id, kind)| (id, Arc::new(kind)))
        .collect();
    let kind = types["minecraft:happy_ghast"].clone();
    let mut m = Mob::new(1, kind, Vec3::new(0.5, 80.0, 0.5), 0.0);
    // Past `HappyGhast.tick`'s 60-tick gate, so the timer counts down from here.
    m.extra.age = 100;
    for timeout in [0, 7, 10] {
        m.extra.counter = timeout;
        let saved = mob_to_nbt(&m, [1; 4], None, None, |_| None);
        assert_eq!(
            number(saved.get("still_timeout")),
            Some(i64::from(timeout)),
            "{timeout}"
        );
        let back = mob_from_nbt(&saved, &types, 2, None).unwrap();
        assert_eq!(back.extra.counter, timeout, "{timeout}");
        assert_eq!(
            crate::mobs::happy_ghast::on_still_timeout(&back),
            timeout > 0,
            "{timeout}"
        );
    }
}

#[test]
fn bucket_cooldown_and_brain_memories_survive_and_expired_cooldown_is_removed() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let kind = Arc::new(data.types.get("minecraft:axolotl").unwrap().clone());
    let mut m = Mob::new(1, kind.clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
    crate::mobs::animal_brain::set_hunting_cooldown(&mut m, 123);
    m.extra.variant = 4;
    m.extra.age = -24000;
    m.extra.age_locked = true;
    m.no_ai = true;
    m.silent = true;
    m.no_gravity = true;
    m.glowing = true;
    m.invulnerable_flag = true;
    m.persistent = true;
    m.custom_name = Some("Bucket friend".into());
    m.health = 5.0;

    {
        let mm = &mut m.extra.animal_brain.as_mut().unwrap().mem;
        mm.play_dead_ticks.set(78);
        mm.temptation_cooldown_ticks.set(42);
    }
    let raw = Tag::compound([(
        "Brain",
        Tag::compound([(
            "memories",
            Tag::compound([("test:unrelated", Tag::compound([("value", Tag::Int(9))]))]),
        )]),
    )]);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], Some(&raw), None, |_| None);
    let memories = tag.get("Brain").unwrap().get("memories").unwrap();
    assert!(memories.get("test:unrelated").is_some());
    assert_eq!(
        number(
            memories
                .get("minecraft:play_dead_ticks")
                .unwrap()
                .get("value")
        ),
        Some(78)
    );
    assert_eq!(
        number(
            memories
                .get("minecraft:temptation_cooldown_ticks")
                .unwrap()
                .get("value")
        ),
        Some(42)
    );
    let mut stack = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
    stack.extra = Some(Box::new(bucket_components(&m)));
    assert_eq!(
        text(stack.data().unwrap().other("minecraft:axolotl/variant")),
        Some("blue".into())
    );
    assert!(bucket_data(&m).get("Variant").is_none());
    let stack = item_from_nbt(&item_to_nbt(&stack)).unwrap();
    let mut restored = Mob::new(2, kind, Vec3::new(0.0, 4.0, 0.0), 0.0);
    apply_item_components(&mut restored, &stack);
    assert_eq!(
        crate::mobs::animal_brain::hunting_cooldown(&restored),
        Some(123)
    );
    assert_eq!(
        (
            restored.extra.variant,
            restored.extra.age,
            restored.extra.age_locked
        ),
        (4, -24000, true)
    );
    assert!(
        restored.no_ai
            && restored.silent
            && restored.no_gravity
            && restored.glowing
            && restored.invulnerable_flag
            && restored.persistent
    );
    assert_eq!(restored.custom_name.as_deref(), Some("Bucket friend"));
    assert_eq!(restored.health, 5.0);
    m.glowing = false;
    m.add_effect("glowing", 200, 0);
    assert!(m.has_effect("minecraft:glowing"));
    assert!(bucket_data(&m).get("Glowing").is_none());
    m.glowing = true;
    assert_eq!(number(bucket_data(&m).get("Glowing")), Some(1));
    for (persistent, expected) in [(None, false), (Some(0), false), (Some(1), true)] {
        let mut data = Tag::Compound(Compound::new());
        if let Some(value) = persistent {
            put(&mut data, "PersistenceRequired", Tag::Byte(value));
        }
        let mut bucket = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
        bucket
            .data_mut()
            .set_other("minecraft:bucket_entity_data", Some(data));
        restored.persistent = false;
        apply_item_components(&mut restored, &bucket);
        assert_eq!(restored.persistent, expected);
        restored.persistent = true;
        apply_item_components(&mut restored, &bucket);
        assert!(restored.persistent, "bucket data cannot clear persistence");
    }
    restored.ticks_frozen = 140;
    restored.ignite_for_ticks(160);
    assert_eq!(restored.ticks_frozen, 0);
    restored.ticks_frozen = 140;
    restored.ignite_for_ticks(1);
    assert_eq!(
        restored.ticks_frozen, 0,
        "even an ignition that does not extend burning clears freeze"
    );
    for (id, name) in AXOLOTL_VARIANTS.iter().enumerate() {
        let mut vanilla = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
        vanilla
            .data_mut()
            .set_other("minecraft:axolotl/variant", Some(string(name)));
        apply_item_components(&mut restored, &vanilla);
        assert_eq!(
            usize::from(restored.extra.variant),
            id,
            "component-only vanilla buckets work"
        );
    }
    let mut legacy = crate::items::ItemStack::new("minecraft:axolotl_bucket", 1);
    legacy.data_mut().set_other(
        "minecraft:bucket_entity_data",
        Some(Tag::compound([("Variant", Tag::Int(3))])),
    );
    apply_item_components(&mut restored, &legacy);
    assert_eq!(restored.extra.variant, 3);
    legacy
        .data_mut()
        .set_other("minecraft:axolotl/variant", Some(string("gold")));
    apply_item_components(&mut restored, &legacy);
    assert_eq!(
        restored.extra.variant, 2,
        "component wins over old Horizon data"
    );

    crate::mobs::animal_brain::set_hunting_cooldown(&mut m, 0);
    assert!(bucket_data(&m).get("HuntingCooldown").is_none());
    let saved = mob_to_nbt(&m, [1, 2, 3, 4], Some(&tag), None, |_| None);
    assert!(
        saved
            .get("Brain")
            .unwrap()
            .get("memories")
            .unwrap()
            .get("minecraft:has_hunting_cooldown")
            .is_none()
    );
    restore_axolotl_memories(&mut restored, &saved);
    let mm = &restored.extra.animal_brain.as_ref().unwrap().mem;
    assert_eq!(mm.play_dead_ticks.value(), Some(78));
    assert_eq!(mm.temptation_cooldown_ticks.value(), Some(42));
}
