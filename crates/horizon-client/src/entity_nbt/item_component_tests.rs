use super::*;
#[test]
fn a_spawn_egg_beats_the_biome_variant() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let spawn = |kind: &str, id: u32| {
        let mut m = Mob::new(
            id,
            Arc::new(data.types[kind].clone()),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        // What `finalizeSpawn` leaves in a temperate biome.
        m.extra.variant = 0;
        m
    };
    let mut egg = crate::items::ItemStack::new("minecraft:chicken_spawn_egg", 1);
    egg.data_mut()
        .set_other("minecraft:chicken/variant", Some(string("minecraft:warm")));
    let mut hen = spawn("minecraft:chicken", 1);
    assert_eq!(hen.extra.variant, 0);
    apply_item_components(&mut hen, &egg);
    assert_eq!(hen.extra.variant, 1, "minecraft:warm, not the biome's");
    // MushroomCow.applyImplicitComponents: the egg's variant, and the
    // component the mob reports back.
    let mut egg = crate::items::ItemStack::new("minecraft:mooshroom_spawn_egg", 1);
    egg.data_mut().set_other(
        "minecraft:mooshroom/variant",
        Some(string("minecraft:brown")),
    );
    let mut mooshroom = spawn("minecraft:mooshroom", 3);
    apply_item_components(&mut mooshroom, &egg);
    let (id, variant) = crate::mobs::finalize::variant_component(&mooshroom).unwrap();
    assert_eq!(
        (id, variant.as_str()),
        ("minecraft:mooshroom/variant", "minecraft:brown")
    );
    // Sheep.applyImplicitComponents: the egg's colour over the mob's.
    let mut egg = crate::items::ItemStack::new("minecraft:sheep_spawn_egg", 1);
    egg.data_mut()
        .set_other("minecraft:sheep/color", Some(string("lime")));
    let mut sheep = spawn("minecraft:sheep", 2);
    apply_item_components(&mut sheep, &egg);
    assert_eq!(sheep.extra.color, 5, "lime, not the white it started as");
    // ZombieNautilus.applyImplicitComponents: the egg carries the coral
    // variant over the temperate one the biome picked.
    let mut egg = crate::items::ItemStack::new("minecraft:zombie_nautilus_spawn_egg", 1);
    egg.data_mut().set_other(
        "minecraft:zombie_nautilus/variant",
        Some(string("minecraft:warm")),
    );
    let mut nautilus = spawn("minecraft:zombie_nautilus", 4);
    apply_item_components(&mut nautilus, &egg);
    assert_eq!(nautilus.extra.variant, 1, "minecraft:warm, not temperate");
}

/// `Fox.applyImplicitComponent` and `Frog.applyImplicitComponent`: the
/// variant component alone decides the mob, `setVariant` from the egg.
#[test]
fn a_fox_or_frog_spawn_egg_picks_its_variant_component() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let spawn = |kind: &str, id: u32| {
        Mob::new(
            id,
            Arc::new(data.types[kind].clone()),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        )
    };
    // A red fox, as a spawn in a forest leaves it.
    let mut fox = spawn("minecraft:fox", 1);
    assert_eq!(fox.extra.variant, 0);
    let mut egg = crate::items::ItemStack::new("minecraft:fox_spawn_egg", 1);
    egg.data_mut()
        .set_other("minecraft:fox/variant", Some(string("minecraft:snow")));
    apply_item_components(&mut fox, &egg);
    assert_eq!(fox.extra.variant, 1, "Fox.Variant.SNOW, not red");
    // A temperate frog, as a swamp spawn leaves it.
    let mut frog = spawn("minecraft:frog", 2);
    assert_eq!(frog.extra.variant, 0);
    let mut egg = crate::items::ItemStack::new("minecraft:frog_spawn_egg", 1);
    egg.data_mut()
        .set_other("minecraft:frog/variant", Some(string("minecraft:warm")));
    apply_item_components(&mut frog, &egg);
    assert_eq!(frog.extra.variant, 1, "minecraft:warm, not temperate");
    // An unknown name falls back to the codec's default, red.
    let mut egg = crate::items::ItemStack::new("minecraft:fox_spawn_egg", 1);
    egg.data_mut()
        .set_other("minecraft:fox/variant", Some(string("minecraft:temperate")));
    apply_item_components(&mut fox, &egg);
    assert_eq!(fox.extra.variant, 0, "unknown names read as red");
}

/// `Horse.applyImplicitComponent` and `Llama.applyImplicitComponent`: the
/// `.../variant` component sets the coat colour. A horse's markings are
/// the other half of the packed variant its spawn picked and stay.
#[test]
fn a_horse_or_llama_spawn_egg_picks_its_variant_component() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let spawn = |kind: &str, id: u32| {
        Mob::new(
            id,
            Arc::new(data.types[kind].clone()),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        )
    };
    // A black horse (Horse.Variant.BLACK, 4) with a white field (markings
    // 2), as a spawn in a plains leaves it.
    let mut horse = spawn("minecraft:horse", 1);
    (horse.extra.variant, horse.extra.variant2) = (4, 2);
    let mut egg = crate::items::ItemStack::new("minecraft:horse_spawn_egg", 1);
    egg.data_mut().set_other(
        "minecraft:horse/variant",
        Some(string("minecraft:dark_brown")),
    );
    apply_item_components(&mut horse, &egg);
    assert_eq!(horse.extra.variant, 6, "Horse.Variant.DARK_BROWN");
    assert_eq!(
        horse.extra.variant2, 2,
        "the markings the spawn picked stay"
    );
    // `Horse.get(DataComponents.HORSE_VARIANT)` reports it back.
    let (id, variant) = crate::mobs::finalize::variant_component(&horse).unwrap();
    assert_eq!(
        (id, variant.as_str()),
        ("minecraft:horse/variant", "minecraft:dark_brown")
    );
    // The save keeps the packed `Variant` int and gains no `variant` name,
    // which only `VariantUtils.writeVariant` species store.
    let tag = mob_to_nbt(&horse, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(tag.get("variant"), None);
    assert!(
        matches!(tag.get("Variant"), Some(Tag::Int(v)) if *v == 6 | 2 << 8),
        "colour 6 and markings 2 packed"
    );
    // A llama, as a spawn in a plains leaves it: creamy.
    let mut egg = crate::items::ItemStack::new("minecraft:llama_spawn_egg", 1);
    egg.data_mut()
        .set_other("minecraft:llama/variant", Some(string("minecraft:gray")));
    let mut llama = spawn("minecraft:llama", 2);
    assert_eq!(llama.extra.variant, 0);
    apply_item_components(&mut llama, &egg);
    assert_eq!(llama.extra.variant, 3, "Llama.Variant.GRAY");
    // TraderLlama inherits the llama's arm.
    let mut trader = spawn("minecraft:trader_llama", 3);
    apply_item_components(&mut trader, &egg);
    assert_eq!(trader.extra.variant, 3, "the trader llama reads it too");
}

/// `EntityType.updateCustomEntityTag`: the egg's `entity_data` on the mob it
/// spawned, when the type is its own — the mob's compound, the data merged
/// over it and loaded back, as the spawn egg site does it.
#[test]
fn a_spawn_eggs_entity_data_reaches_the_mob_it_spawned() {
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
    let egg = |entity_data: Option<Tag>| {
        let mut egg = crate::items::ItemStack::new("minecraft:zombie_spawn_egg", 1);
        if let Some(data) = entity_data {
            egg.data_mut()
                .set_other("minecraft:entity_data", Some(data));
        }
        egg
    };
    let spawn = |stack: &crate::items::ItemStack| {
        let mut m = Mob::new(
            2,
            types["minecraft:zombie"].clone(),
            Vec3::new(0.0, 4.0, 0.0),
            0.0,
        );
        apply_item_components(&mut m, stack);
        let Some(data) = crate::block_entity_item::spawn_egg_entity_data(stack, &m.kind.id, false)
        else {
            return m;
        };
        let tag =
            crate::data_cmd::merged(&mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None), data);
        mob_from_nbt(&tag, &types, 2, None).unwrap()
    };
    // `entity_data={NoAI:1b}`: no id, so the egg's own type holds the data.
    assert!(spawn(&egg(Some(Tag::compound([("NoAI", Tag::Byte(1))])))).no_ai);
    // `entity_data.id` names another type: the egg's own type, data skipped.
    let other = spawn(&egg(Some(Tag::compound([
        ("id", string("minecraft:pig")),
        ("NoAI", Tag::Byte(1)),
    ]))));
    assert_eq!(other.kind.id.as_str(), "minecraft:zombie");
    assert!(!other.no_ai);
    // A plain egg is unchanged.
    assert!(!spawn(&egg(None)).no_ai);
}
