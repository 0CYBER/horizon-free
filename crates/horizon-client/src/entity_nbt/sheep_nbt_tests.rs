use super::*;
#[test]
fn a_sheep_keeps_its_dyed_and_sheared_wool() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let mut m = Mob::new(
        1,
        Arc::new(data.types["minecraft:sheep"].clone()),
        Vec3::new(0.0, 4.0, 0.0),
        0.0,
    );
    // Summoned white, then dyed red and sheared: the save has it now.
    let raw = Tag::compound([("Color", Tag::Byte(0)), ("Sheared", Tag::Byte(0))]);
    (m.extra.color, m.extra.sheared) = (14, true);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], Some(&raw), None, |_| None);
    assert_eq!(
        (number(tag.get("Color")), number(tag.get("Sheared"))),
        (Some(14), Some(1))
    );
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!((back.extra.color, back.extra.sheared), (14, true));
}

#[test]
fn a_snow_golem_keeps_its_pumpkin_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let saved = |pumpkin: Option<i8>| {
        let mut tag = Tag::compound([
            ("id", string("minecraft:snow_golem")),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ]);
        if let Some(value) = pumpkin {
            put(&mut tag, "Pumpkin", Tag::Byte(value));
        }
        tag
    };
    // SnowGolem.readAdditionalSaveData: getBooleanOr("Pumpkin", true).
    for (pumpkin, sheared) in [(None, false), (Some(1), false), (Some(0), true)] {
        let m = mob_from_nbt(&saved(pumpkin), &types, 2, None).unwrap();
        assert_eq!(m.extra.sheared, sheared, "{pumpkin:?}");
        // addAdditionalSaveData always writes it, over the loaded value.
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], Some(&saved(pumpkin)), None, |_| None);
        assert_eq!(number(tag.get("Pumpkin")), Some(i64::from(!sheared)));
    }
    // Sheared in game: the save and the next load keep it off.
    let mut m = mob_from_nbt(&saved(None), &types, 3, None).unwrap();
    m.extra.sheared = true;
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert!(mob_from_nbt(&tag, &types, 4, None).unwrap().extra.sheared);
}

/// `Hoglin.readAdditionalSaveData` and `addAdditionalSaveData`:
/// `CannotBeHunted` (a bastion's hoglins) stays with the hoglin and is always
/// written, and such a hoglin is no piglin's prey (`Hoglin.canBeHunted`).
#[test]
fn a_hoglin_keeps_cannot_be_hunted_and_is_no_prey() {
    use crate::mobs::Species;
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.species = Species::Hoglin;
    ty.id = "minecraft:hoglin".into();
    let types = HashMap::from([("minecraft:hoglin".to_owned(), kind)]);
    let hoglin = |cannot: Option<i8>| {
        let mut tag = Tag::compound([
            ("id", string("minecraft:hoglin")),
            ("Pos", doubles(&[0.0, 64.0, 0.0])),
        ]);
        if let (Some(b), Tag::Compound(map)) = (cannot, &mut tag) {
            map.insert("CannotBeHunted".into(), Tag::Byte(b));
        }
        tag
    };
    let raw = hoglin(Some(1));
    let protected = mob_from_nbt(&raw, &types, 1, None).unwrap();
    assert!(protected.extra.cannot_be_hunted);
    assert!(!protected.seen(None, &crate::items::Items::empty()).huntable);
    let saved = mob_to_nbt(&protected, [0; 4], Some(&raw), None, |_| None);
    assert_eq!(saved.get("CannotBeHunted"), Some(&Tag::Byte(1)));
    for raw in [hoglin(None), hoglin(Some(0))] {
        let prey = mob_from_nbt(&raw, &types, 2, None).unwrap();
        assert!(!prey.extra.cannot_be_hunted);
        assert!(prey.seen(None, &crate::items::Items::empty()).huntable);
        let saved = mob_to_nbt(&prey, [0; 4], Some(&raw), None, |_| None);
        assert_eq!(saved.get("CannotBeHunted"), Some(&Tag::Byte(0)));
    }
}

/// `Mob.addAdditionalSaveData` and `readAdditionalSaveData`:
/// `DeathLootTable` and a set `DeathLootTableSeed` stay with the mob.
#[test]
fn a_death_loot_table_round_trips() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let summoned = Tag::compound([
        ("id", string("minecraft:zombie")),
        ("Pos", doubles(&[0.0, 64.0, 0.0])),
        ("DeathLootTable", string("mymod:entities/boss")),
        ("DeathLootTableSeed", Tag::Long(42)),
    ]);
    let m = mob_from_nbt(&summoned, &types, 2, None).unwrap();
    assert_eq!(
        m.extra.death_loot_table.as_deref(),
        Some("mymod:entities/boss")
    );
    assert_eq!(m.extra.death_table_seed, 42);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(
        text(tag.get("DeathLootTable")).as_deref(),
        Some("mymod:entities/boss")
    );
    assert_eq!(number(tag.get("DeathLootTableSeed")), Some(42));
    // None set: neither is written.
    let plain = Tag::compound([
        ("id", string("minecraft:zombie")),
        ("Pos", doubles(&[0.0, 64.0, 0.0])),
    ]);
    let plain = mob_from_nbt(&plain, &types, 3, None).unwrap();
    let tag = mob_to_nbt(&plain, [1, 2, 3, 4], None, None, |_| None);
    assert!(tag.get("DeathLootTable").is_none() && tag.get("DeathLootTableSeed").is_none());
}

/// `Rabbit.addAdditionalSaveData` and `readAdditionalSaveData`: `MoreCarrotTicks`.
#[test]
fn a_rabbit_keeps_how_full_of_carrots_it_is() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut m = Mob::new(
        1,
        Arc::new(data.types["minecraft:rabbit"].clone()),
        Vec3::new(0.0, 4.0, 0.0),
        0.0,
    );
    m.extra.more_carrot_ticks = 33;
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("MoreCarrotTicks")), Some(33));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(back.extra.more_carrot_ticks, 33);
}

/// `Ocelot.addAdditionalSaveData` and `readAdditionalSaveData`: `Trusting`.
#[test]
fn an_ocelot_keeps_its_trust() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut m = Mob::new(
        1,
        Arc::new(data.types["minecraft:ocelot"].clone()),
        Vec3::new(0.0, 4.0, 0.0),
        0.0,
    );
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("Trusting")), Some(0));
    m.extra.trusting = true;
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("Trusting")), Some(1));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert!(back.extra.trusting);
}

/// `Mannequin.addAdditionalSaveData` and `readAdditionalSaveData`: `immovable`, and the
/// description (the default is not written, `hide_description` hides it, a component stays).
#[test]
fn a_mannequin_keeps_its_immovable_flag_and_description() {
    use crate::mobs::MannequinDescription as D;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut m = Mob::new(
        1,
        Arc::new(data.types["minecraft:mannequin"].clone()),
        Vec3::new(0.0, 4.0, 0.0),
        0.0,
    );
    let save = |m: &Mob| mob_to_nbt(m, [1, 2, 3, 4], None, None, |_| None);
    // The default description is not written; it comes back.
    let tag = save(&m);
    assert!(tag.get("description").is_none() && tag.get("hide_description").is_none());
    assert_eq!(number(tag.get("immovable")), Some(0));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(
        (back.extra.immovable, back.extra.description),
        (false, D::Default)
    );
    // Hidden, and immovable.
    m.extra.immovable = true;
    m.extra.description = D::Hidden;
    let tag = save(&m);
    assert_eq!(number(tag.get("hide_description")), Some(1));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(
        (back.extra.immovable, back.extra.description),
        (true, D::Hidden)
    );
    // A component of the world's own.
    let own = Tag::compound([("text", string("Guard")), ("color", string("red"))]);
    m.extra.description = D::Custom(own.clone());
    let tag = save(&m);
    assert_eq!(tag.get("description"), Some(&own));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(back.extra.description, D::Custom(own));
}

/// The only lib test that registers mod effects (once per process).
#[test]
fn a_mob_keeps_a_mod_effect() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let types: HashMap<String, Arc<MobType>> = crate::mobs::MobTypes::load(&root)
        .unwrap()
        .types
        .into_iter()
        .map(|(k, v)| (k, Arc::new(v)))
        .collect();
    crate::brewing::register_mod_effects([(
        "testfx:glow".to_owned(),
        1,
        crate::brewing::Category::Harmful,
        None,
    )]);
    let mut m = Mob::new(1, types["minecraft:pig"].clone(), Vec3::ZERO, 0.0);
    m.add_effect(crate::brewing::effect("testfx:glow").unwrap().id, 200, 2);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    let Some(Tag::List(saved)) = tag.get("active_effects") else {
        panic!("no active_effects");
    };
    assert_eq!(text(saved[0].get("id")).as_deref(), Some("testfx:glow"));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(
        back.extra.effects,
        [crate::mobs::StoredEffect::new("testfx:glow", 200, 2)]
    );
}

#[test]
fn an_amplifier_of_zero_is_left_out_of_active_effects() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let types: HashMap<String, Arc<MobType>> = crate::mobs::MobTypes::load(&root)
        .unwrap()
        .types
        .into_iter()
        .map(|(k, v)| (k, Arc::new(v)))
        .collect();
    let mut m = Mob::new(1, types["minecraft:pig"].clone(), Vec3::ZERO, 0.0);
    let speed = crate::brewing::effect("minecraft:speed").unwrap().id;
    m.add_effect(speed, 200, 0);
    m.add_effect(
        crate::brewing::effect("minecraft:slowness").unwrap().id,
        200,
        2,
    );
    m.add_effect("minecraft:slowness", 90, 3);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    let Some(Tag::List(saved)) = tag.get("active_effects") else {
        panic!("no active_effects");
    };
    assert_eq!(
        number(saved[1].get("hidden_effect").unwrap().get("duration")),
        Some(200)
    );
    assert!(saved[0].get("amplifier").is_none());
    assert!(saved[1].get("amplifier").is_some());
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(back.extra.effects[0].amplifier, 0);
    assert_eq!(back.extra.effects[1].amplifier, 3);
    assert_eq!(back.extra.effect_chains[0].amplifier, 2);
    assert_eq!(back.extra.effect_chains[0].duration, 200);
}

#[test]
fn farm_animals_keep_their_variant_and_sound_variant() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut m = Mob::new(
        1,
        types["minecraft:pig"].clone(),
        Vec3::new(0.0, 4.0, 0.0),
        0.0,
    );
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(
        text(tag.get("variant")).as_deref(),
        Some("minecraft:temperate")
    );
    assert_eq!(
        text(tag.get("sound_variant")).as_deref(),
        Some("minecraft:classic")
    );
    // Spawned in a warm biome with the mini sounds.
    (m.extra.variant, m.extra.sound_variant) = (1, Some(2));
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(text(tag.get("variant")).as_deref(), Some("minecraft:warm"));
    assert_eq!(
        text(tag.get("sound_variant")).as_deref(),
        Some("minecraft:mini")
    );
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!((back.extra.variant, back.extra.sound_variant), (1, Some(2)));
    let mut hen = Mob::new(
        3,
        types["minecraft:chicken"].clone(),
        Vec3::new(0.0, 4.0, 0.0),
        0.0,
    );
    (hen.extra.variant, hen.extra.egg_time) = (2, 7795);
    let tag = mob_to_nbt(&hen, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(text(tag.get("variant")).as_deref(), Some("minecraft:cold"));
    let back = mob_from_nbt(&tag, &types, 4, None).unwrap();
    assert_eq!((back.extra.variant, back.extra.egg_time), (2, 7795));
    // The sound variants are indices into the registries finalizeSpawn draws from.
    use crate::mobs::{Species, finalize};
    let registries = finalize::SpawnRegistries::load(&root);
    for (species, registry) in [
        (Species::Pig, &registries.pig_sound),
        (Species::Cow, &registries.cow_sound),
        (Species::Chicken, &registries.chicken_sound),
        (Species::Cat, &registries.cat_sound),
        (Species::Wolf, &registries.wolf_sound),
    ] {
        assert_eq!(
            finalize::sound_variants(species),
            registry.names,
            "{species:?}"
        );
    }
}

#[test]
fn parrots_omit_raw_age_and_pigs_keep_modeled_age() {
    use crate::mobs::Species;
    for species in [Species::Parrot, Species::Pig] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.species = species;
        ty.id = if species == Species::Parrot {
            "minecraft:parrot"
        } else {
            "minecraft:pig"
        }
        .into();
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        let expected = if species == Species::Pig {
            mob.extra.age = -37;
            Some(Tag::Int(-37))
        } else {
            None
        };
        for raw in [
            None,
            Some(Tag::Compound(Default::default())),
            Some(Tag::compound([("Age", Tag::Int(0))])),
            Some(Tag::compound([("Age", Tag::Int(-100))])),
            Some(Tag::compound([("Age", Tag::Int(100))])),
        ] {
            let saved = mob_to_nbt(&mob, [0; 4], raw.as_ref(), None, |_| None);
            assert_eq!(
                saved.get("Age"),
                expected.as_ref(),
                "{species:?}, raw={raw:?}"
            );
        }
    }
}

#[test]
fn a_baby_villager_keeps_its_age() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let saved = Tag::compound([
        ("id", string("minecraft:villager")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("Age", Tag::Int(-21359)),
    ]);
    let m = mob_from_nbt(&saved, &types, 1, None).unwrap();
    assert_eq!(m.extra.age, -21359);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("Age")), Some(-21359));
    assert_eq!(number(tag.get("AgeLocked")), Some(0));
    assert_eq!(number(tag.get("VillagerDataFinalized")), Some(0));

    let with_data = Tag::compound([
        ("id", string("minecraft:villager")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("VillagerData", Tag::Compound(Default::default())),
    ]);
    let m_data = mob_from_nbt(&with_data, &types, 2, None).unwrap();
    let tag_data = mob_to_nbt(&m_data, [1, 2, 3, 5], None, None, |_| None);
    assert_eq!(number(tag_data.get("VillagerDataFinalized")), Some(1));

    let with_flag = Tag::compound([
        ("id", string("minecraft:villager")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("VillagerDataFinalized", Tag::Byte(1)),
    ]);
    let m_flag = mob_from_nbt(&with_flag, &types, 3, None).unwrap();
    let tag_flag = mob_to_nbt(&m_flag, [1, 2, 3, 6], None, None, |_| None);
    assert_eq!(number(tag_flag.get("VillagerDataFinalized")), Some(1));
}

/// `Dolphin.addAdditionalSaveData`: a fed dolphin stays fed and a wet one
/// stays wet across a reload; a save with neither field reads as an unfed
/// dolphin at full moistness.
#[test]
fn a_dolphins_moistness_and_fish_survive_a_save() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    // A fresh dolphin starts full, as `Dolphin`'s field does.
    let mut m = Mob::new(
        1,
        types["minecraft:dolphin"].clone(),
        Vec3::new(0.5, 64.0, 0.5),
        0.0,
    );
    assert_eq!(m.extra.counter, 2400);
    m.extra.flag = true;
    m.extra.counter = 123;
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("Moistness")), Some(123));
    assert_eq!(number(tag.get("GotFish")), Some(1));
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert!(back.extra.flag, "fed");
    assert_eq!(back.extra.counter, 123);
    // A save with neither field: unfed, full.
    let bare = Tag::compound([
        ("id", string("minecraft:dolphin")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
    ]);
    let back = mob_from_nbt(&bare, &types, 3, None).unwrap();
    assert!(!back.extra.flag);
    assert_eq!(back.extra.counter, 2400);
}

/// `WanderingTrader.addAdditionalSaveData`: the visit's countdown and the
/// place it is walking to (a `BlockPos` codec, so an int array), which the
/// despawn and stroll goals read back after a reload.
#[test]
fn a_wandering_traders_countdown_and_target_survive_a_save() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut m = Mob::new(
        1,
        types["minecraft:wandering_trader"].clone(),
        Vec3::new(0.5, 64.0, 0.5),
        0.0,
    );
    m.extra.ints[5] = 48000;
    m.extra.point = Some([12.0, 65.0, -30.0]);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(number(tag.get("DespawnDelay")), Some(48000));
    let Some(Tag::IntArray(target)) = tag.get("wander_target") else {
        panic!("wander_target missing");
    };
    assert_eq!(target, &vec![12, 65, -30]);
    let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
    assert_eq!(back.extra.ints[5], 48000);
    assert_eq!(back.extra.point, Some([12.0, 65.0, -30.0]));
    // A save with neither field: idle, and nothing to walk to.
    let bare = Tag::compound([
        ("id", string("minecraft:wandering_trader")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
    ]);
    let back = mob_from_nbt(&bare, &types, 3, None).unwrap();
    assert_eq!((back.extra.ints[5], back.extra.point), (0, None));
}

#[test]
fn mobs_keep_their_variant_puff_state_carried_block_and_conversion_times() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<String, Arc<MobType>> = data
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let registry = horizon_world::registry::BlockRegistry::from_facts_json(
        std::fs::File::open(root.join("facts.json")).unwrap(),
    )
    .unwrap();
    let new = |kind: &str| Mob::new(1, types[kind].clone(), Vec3::new(0.0, 4.0, 0.0), 0.0);
    let round_trip = |m: &Mob| {
        let tag = mob_to_nbt(m, [1, 2, 3, 4], None, Some(&registry), |_| None);
        (mob_from_nbt(&tag, &types, 2, Some(&registry)).unwrap(), tag)
    };
    // A save with nothing but where the mob stands.
    let empty =
        |kind: &str| Tag::compound([("id", string(kind)), ("Pos", doubles(&[0.5, 64.0, 0.5]))]);
    let mut fox = new("minecraft:fox");
    fox.extra.variant = 1;
    let (back, tag) = round_trip(&fox);
    assert_eq!(text(tag.get("Type")).as_deref(), Some("snow"));
    assert_eq!(number(tag.get("Sleeping")), Some(0));
    assert_eq!(back.extra.variant, 1);
    // MushroomCow: a brown mooshroom stays brown, and a save without (or
    // with an unknown) Type reads as red.
    let mut mooshroom = new("minecraft:mooshroom");
    mooshroom.extra.variant = 1;
    let (back, tag) = round_trip(&mooshroom);
    assert_eq!(text(tag.get("Type")).as_deref(), Some("brown"));
    assert_eq!(back.extra.variant, 1);
    mooshroom.extra.variant = 0;
    let (back, tag) = round_trip(&mooshroom);
    assert_eq!(text(tag.get("Type")).as_deref(), Some("red"));
    assert_eq!(back.extra.variant, 0);
    // Its stew effects round-trip as the component's bare {id, duration}
    // entries, and a cleared list is not written at all.
    mooshroom.extra.variant = 1;
    mooshroom.extra.stew_effects = Some(vec![(Arc::from("minecraft:night_vision"), 100)]);
    let (back, tag) = round_trip(&mooshroom);
    assert_eq!(back.extra.stew_effects, mooshroom.extra.stew_effects);
    let entry = match tag.get("stew_effects") {
        Some(Tag::List(list)) => list.first().expect("one effect"),
        other => panic!("stew_effects: {other:?}"),
    };
    assert_eq!(
        (text(entry.get("id")), number(entry.get("duration"))),
        (Some("minecraft:night_vision".into()), Some(100))
    );
    mooshroom.extra.stew_effects = None;
    let (back, tag) = round_trip(&mooshroom);
    assert_eq!(back.extra.stew_effects, None);
    assert!(tag.get("stew_effects").is_none());
    let mut rabbit = new("minecraft:rabbit");
    rabbit.extra.variant = 4;
    let (back, tag) = round_trip(&rabbit);
    assert_eq!(number(tag.get("RabbitType")), Some(4));
    assert_eq!(back.extra.variant, 4);
    let mut goat = new("minecraft:goat");
    (goat.extra.screaming, goat.extra.lost_horns) = (true, [false, true]);
    let (back, tag) = round_trip(&goat);
    assert_eq!(number(tag.get("HasRightHorn")), Some(0));
    assert_eq!(
        (back.extra.screaming, back.extra.lost_horns),
        (true, [false, true])
    );
    // Pufferfish.addAdditionalSaveData: the puff state, 0 when the save
    // has none and at most 2 when it has more.
    let mut pufferfish = new("minecraft:pufferfish");
    pufferfish.extra.size = 2;
    let (back, tag) = round_trip(&pufferfish);
    assert_eq!(number(tag.get("PuffState")), Some(2));
    assert_eq!(back.extra.size, 2);
    assert_eq!(
        mob_from_nbt(&empty("minecraft:pufferfish"), &types, 3, Some(&registry))
            .unwrap()
            .extra
            .size,
        0
    );
    let puffed = Tag::compound([
        ("id", string("minecraft:pufferfish")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("PuffState", Tag::Int(5)),
    ]);
    assert_eq!(
        mob_from_nbt(&puffed, &types, 3, Some(&registry))
            .unwrap()
            .extra
            .size,
        2
    );
    // Enderman.addAdditionalSaveData: the block it carries, a block name
    // for the block's default state and `{id, properties}` otherwise.
    let mut enderman = new("minecraft:enderman");
    let grass = registry.default_state("minecraft:grass_block");
    enderman.extra.carried = grass;
    let (back, tag) = round_trip(&enderman);
    assert_eq!(
        text(tag.get("carriedBlockState")).as_deref(),
        Some("minecraft:grass_block")
    );
    assert_eq!(back.extra.carried, grass);
    let log = registry
        .id(&horizon_world::chunk::BlockState {
            name: "minecraft:oak_log".into(),
            properties: [("axis".to_owned(), "x".to_owned())].into(),
        })
        .unwrap();
    enderman.extra.carried = Some(log);
    let (back, tag) = round_trip(&enderman);
    assert!(matches!(
        tag.get("carriedBlockState"),
        Some(Tag::Compound(_))
    ));
    assert_eq!(back.extra.carried, Some(log));
    // Nothing carried: no key, and none read back; an air state reads as
    // no block at all.
    enderman.extra.carried = None;
    let (back, tag) = round_trip(&enderman);
    assert_eq!(tag.get("carriedBlockState"), None);
    assert_eq!(back.extra.carried, None);
    let air = Tag::compound([
        ("id", string("minecraft:enderman")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("carriedBlockState", string("minecraft:air")),
    ]);
    assert_eq!(
        mob_from_nbt(&air, &types, 3, Some(&registry))
            .unwrap()
            .extra
            .carried,
        None
    );
    // A goat saved without horn flags has both, as vanilla reads it.
    assert_eq!(
        mob_from_nbt(&empty("minecraft:goat"), &types, 3, None)
            .unwrap()
            .extra
            .lost_horns,
        [false; 2]
    );
    // Zombie and Skeleton.addAdditionalSaveData: the `ConversionTracker`
    // tags, -1 for a counter that is not running, and both come back into
    // the counters the tick uses.
    let mut zombie = new("minecraft:zombie");
    // Positive affliction counters represent a zombie with immersed eyes.
    zombie.fluids.eye_in_water = true;
    zombie.extra.counter = 300;
    let (back, tag) = round_trip(&zombie);
    assert_eq!(number(tag.get("InWaterTime")), Some(300));
    assert_eq!(number(tag.get("DrownedConversionTime")), Some(-1));
    assert_eq!((back.extra.counter, back.extra.counter2), (300, 0));
    (zombie.extra.counter, zombie.extra.counter2) = (600, 301);
    let (back, tag) = round_trip(&zombie);
    assert_eq!(number(tag.get("InWaterTime")), Some(600));
    assert_eq!(number(tag.get("DrownedConversionTime")), Some(300));
    assert_eq!((back.extra.counter, back.extra.counter2), (600, 301));
    // A skeleton outside powder snow is not afflicted: -1 whatever the
    // counter holds, as with a fresh or NoAI one.
    let (_, tag) = round_trip(&new("minecraft:skeleton"));
    assert_eq!(number(tag.get("FreezingTime")), Some(-1));
    assert_eq!(number(tag.get("StrayConversionTime")), Some(-1));
    let mut skeleton = new("minecraft:skeleton");
    skeleton.in_powder_snow = true;
    skeleton.extra.counter = 70;
    let (back, tag) = round_trip(&skeleton);
    assert_eq!(number(tag.get("FreezingTime")), Some(70));
    assert_eq!(number(tag.get("StrayConversionTime")), Some(-1));
    assert_eq!((back.extra.counter, back.extra.counter2), (70, 0));
    (skeleton.extra.counter, skeleton.extra.counter2) = (140, 301);
    let (back, tag) = round_trip(&skeleton);
    assert_eq!(number(tag.get("FreezingTime")), Some(140));
    assert_eq!(number(tag.get("StrayConversionTime")), Some(300));
    assert_eq!((back.extra.counter, back.extra.counter2), (140, 301));
    // Not converting: -1 in both tags, and nothing starts on load.
    for (kind, affliction, conversion) in [
        ("minecraft:zombie", "InWaterTime", "DrownedConversionTime"),
        ("minecraft:skeleton", "FreezingTime", "StrayConversionTime"),
    ] {
        let mut m = new(kind);
        (m.extra.counter, m.extra.counter2) = (-1, 0);
        let (back, tag) = round_trip(&m);
        assert_eq!(number(tag.get(affliction)), Some(-1));
        assert_eq!(number(tag.get(conversion)), Some(-1));
        assert_eq!((back.extra.counter, back.extra.counter2), (-1, 0));
    }
    // Zombie villagers and zombified piglins keep Zombie's tracker tags.
    for kind in ["minecraft:zombie_villager", "minecraft:zombified_piglin"] {
        let (_, tag) = round_trip(&new(kind));
        assert_eq!(number(tag.get("DrownedConversionTime")), Some(-1), "{kind}");
        assert_eq!(number(tag.get("InWaterTime")), Some(-1), "{kind}");
    }
    // A ticking one that left the snow or the water saves -1 over the times
    // it was loaded with (ConversionTracker.tick: setConverting(false)).
    for (kind, conversion, affliction) in [
        ("minecraft:skeleton", "StrayConversionTime", "FreezingTime"),
        ("minecraft:zombie", "DrownedConversionTime", "InWaterTime"),
    ] {
        let loaded = Tag::compound([
            ("id", string(kind)),
            ("Pos", doubles(&[0.5, 64.0, 0.5])),
            (conversion, Tag::Int(200)),
            (affliction, Tag::Int(5)),
        ]);
        let mut m = new(kind);
        (m.extra.counter, m.extra.counter2) = (-1, 0);
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], Some(&loaded), Some(&registry), |_| None);
        assert_eq!(number(tag.get(conversion)), Some(-1), "{kind}");
        assert_eq!(number(tag.get(affliction)), Some(-1), "{kind}");
    }
    // AbstractPiglin: a piglin or brute saved without CanPickUpLoot picks up
    // loot. Piglin always writes CannotHunt, the brute never does.
    for (kind, hunt) in [
        ("minecraft:piglin", Some(0)),
        ("minecraft:piglin_brute", None),
    ] {
        let back = mob_from_nbt(&empty(kind), &types, 5, None).unwrap();
        assert!(back.can_pick_up_loot(), "{kind}");
        let tag = mob_to_nbt(&back, [1, 2, 3, 4], None, Some(&registry), |_| None);
        assert_eq!(number(tag.get("CannotHunt")), hunt, "{kind}");
    }
    let mut saved = empty("minecraft:piglin");
    put(&mut saved, "CanPickUpLoot", Tag::Byte(0));
    let back = mob_from_nbt(&saved, &types, 6, None).unwrap();
    assert!(!back.can_pick_up_loot());
    // Bogged: whether it was sheared comes back, and a fresh one saves false.
    let (_, tag) = round_trip(&new("minecraft:bogged"));
    assert_eq!(number(tag.get("sheared")), Some(0));
    let mut bogged = new("minecraft:bogged");
    bogged.extra.sheared = true;
    let (back, tag) = round_trip(&bogged);
    assert_eq!(number(tag.get("sheared")), Some(1));
    assert!(back.extra.sheared);
    // A saved conversion time starts one: `startConversion`.
    let saved = Tag::compound([
        ("id", string("minecraft:zombie")),
        ("Pos", doubles(&[0.5, 64.0, 0.5])),
        ("InWaterTime", Tag::Int(600)),
        ("DrownedConversionTime", Tag::Int(300)),
    ]);
    let back = mob_from_nbt(&saved, &types, 4, None).unwrap();
    assert_eq!((back.extra.counter, back.extra.counter2), (600, 301));
}
