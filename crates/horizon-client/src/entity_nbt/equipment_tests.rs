//! The mob `equipment` compound is saved as `ar/main` saved it. The `.nbt`
//! files in `tests/data/mob_equipment` were written by that codec (set
//! `HORIZON_WRITE_FIXTURES=1` and run the ignored test to regenerate them);
//! each must load and save back to the same bytes. An armor stand keeps all
//! eight slots in the same compound, body and saddle too.
use super::*;
use crate::mobs::Species;
use horizon_core::nbt::{Limits, NamedTag, parse_snbt, read_named, write_named};

const UUID: [i32; 4] = [1, 2, 3, 4];

/// (file, mob id, species, the SNBT a save of that mob holds).
const CASES: [(&str, &str, Species, &str); 4] = [
    (
        "zombie_armor",
        "minecraft:zombie",
        Species::Zombie,
        r#"{id:"minecraft:zombie",Pos:[0.5d,64.0d,0.5d],equipment:{mainhand:{id:"minecraft:iron_sword",count:1},head:{id:"minecraft:iron_helmet",count:1,components:{"minecraft:enchantments":{"minecraft:protection":2}}},chest:{id:"minecraft:leather_chestplate",count:1,components:{"minecraft:dyed_color":16711680}},legs:{id:"minecraft:iron_leggings",count:1,components:{"minecraft:damage":7}},feet:{id:"minecraft:golden_boots",count:1}}}"#,
    ),
    (
        "horse_body_saddle",
        "minecraft:horse",
        Species::Horse,
        r#"{id:"minecraft:horse",Pos:[0.5d,64.0d,0.5d],Tame:1b,Variant:259,Temper:20,equipment:{body:{id:"minecraft:iron_horse_armor",count:1,components:{"minecraft:damage":5}},saddle:{id:"minecraft:saddle",count:1}}}"#,
    ),
    (
        "llama_carpet",
        "minecraft:llama",
        Species::Llama,
        r#"{id:"minecraft:llama",Pos:[0.5d,64.0d,0.5d],Tame:1b,Strength:3,Variant:1,equipment:{body:{id:"minecraft:red_carpet",count:1}}}"#,
    ),
    (
        "zombie_hands_enchanted_armor",
        "minecraft:zombie",
        Species::Zombie,
        r#"{id:"minecraft:zombie",Pos:[0.5d,64.0d,0.5d],equipment:{mainhand:{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":40,"minecraft:custom_name":"Biter","minecraft:enchantments":{"minecraft:sharpness":3,"minecraft:fire_aspect":1}}},offhand:{id:"minecraft:shield",count:1,components:{"minecraft:damage":12}},head:{id:"minecraft:diamond_helmet",count:1,components:{"minecraft:damage":9,"minecraft:enchantments":{"minecraft:protection":4,"minecraft:unbreaking":2}}},chest:{id:"minecraft:iron_chestplate",count:1,components:{"minecraft:damage":30,"minecraft:enchantments":{"minecraft:thorns":1}}},legs:{id:"minecraft:chainmail_leggings",count:1,components:{"minecraft:damage":3}},feet:{id:"minecraft:leather_boots",count:1,components:{"minecraft:dyed_color":255,"minecraft:damage":5,"minecraft:enchantments":{"minecraft:feather_falling":3}}}}}"#,
    ),
];

/// A vanilla armor stand holding body and saddle beside its armor.
const STAND: (&str, &str) = (
    "armor_stand_body_saddle",
    r#"{id:"minecraft:armor_stand",Pos:[0.5d,64.0d,0.5d],ShowArms:1b,equipment:{mainhand:{id:"minecraft:iron_sword",count:1,components:{"minecraft:damage":4}},head:{id:"minecraft:iron_helmet",count:1},body:{id:"minecraft:diamond_horse_armor",count:1,components:{"minecraft:damage":6,"minecraft:custom_name":"Plate"}},saddle:{id:"minecraft:saddle",count:1}}}"#,
);

/// Loads `tag` as an armor stand and saves it back over itself.
fn stand_round_trip(tag: &Tag) -> Tag {
    let a = armor_stand_from_nbt(tag, 1).unwrap();
    armor_stand_to_nbt(&a, UUID, Some(tag))
}

fn types(id: &str, species: Species) -> HashMap<String, Arc<MobType>> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = id.into();
    ty.species = species;
    [(id.to_string(), kind)].into()
}

/// Loads `tag` as a mob and saves it back over itself, as a chunk save does.
fn round_trip(tag: &Tag, id: &str, species: Species) -> Tag {
    let m = mob_from_nbt(tag, &types(id, species), 1, None).unwrap();
    mob_to_nbt(&m, UUID, Some(tag), None, |_| None)
}

fn bytes(tag: &Tag) -> Vec<u8> {
    let root = NamedTag {
        name: "".into(),
        tag: tag.clone(),
    };
    write_named(&root, Limits::default()).unwrap()
}

fn path(name: &str) -> std::path::PathBuf {
    // This module is compiled into horizon-client-lib, beside horizon-client.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../horizon-client/tests/data/mob_equipment/{name}.nbt"
    ))
}

#[test]
#[ignore = "writes the fixtures; run with HORIZON_WRITE_FIXTURES=1 on the codec to record"]
fn write_fixtures() {
    assert!(std::env::var_os("HORIZON_WRITE_FIXTURES").is_some());
    for (name, id, species, snbt) in CASES {
        let input = parse_snbt(snbt).unwrap().0;
        std::fs::write(path(name), bytes(&round_trip(&input, id, species))).unwrap();
    }
    let input = parse_snbt(STAND.1).unwrap().0;
    std::fs::write(path(STAND.0), bytes(&stand_round_trip(&input))).unwrap();
}

#[test]
fn a_saved_armor_stand_keeps_its_body_and_saddle_byte_identically() {
    let file = std::fs::read(path(STAND.0)).unwrap();
    let fixture = read_named(&file, Limits::default()).unwrap().0.tag;
    let equipment = fixture.get("equipment").unwrap();
    assert!(equipment.get("body").is_some() && equipment.get("saddle").is_some());
    let stand = armor_stand_from_nbt(&fixture, 1).unwrap();
    assert!(stand.equipment[horizon_items::equipment::BODY].is_some());
    assert!(stand.equipment[horizon_items::equipment::SADDLE].is_some());
    assert_eq!(bytes(&stand_round_trip(&fixture)), file);
}

#[test]
fn saved_mob_equipment_loads_and_saves_byte_identically() {
    for (name, id, species, _) in CASES {
        let file = std::fs::read(path(name)).unwrap();
        let fixture = read_named(&file, Limits::default()).unwrap().0.tag;
        assert!(fixture.get("equipment").is_some(), "{name} has equipment");
        let saved = round_trip(&fixture, id, species);
        assert_eq!(bytes(&saved), file, "{name} saves back unchanged");
    }
}
