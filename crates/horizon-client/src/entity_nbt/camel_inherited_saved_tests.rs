use super::*;
use crate::mobs::Species;

fn types(species: Species, id: &str) -> HashMap<String, Arc<MobType>> {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = id.into();
    ty.species = species;
    HashMap::from([(id.to_owned(), kind)])
}

fn input(id: &str, temper: Option<i32>) -> Tag {
    let mut raw = Tag::compound([
        ("id", string(id)),
        ("Pos", doubles(&[1.5, 200.0, 4.5])),
        ("Health", Tag::Float(10.0_f32.to_bits())),
        ("NoAI", Tag::Byte(1)),
        ("NoGravity", Tag::Byte(1)),
        ("Silent", Tag::Byte(1)),
        ("Invulnerable", Tag::Byte(1)),
        ("LastPoseTick", Tag::Long(1)),
        ("CustomName", string("kept")),
        ("Tags", Tag::List(vec![string("kept")])),
        ("audit:kept", Tag::Int(17)),
    ]);
    if let Some(temper) = temper {
        put(&mut raw, "Tame", Tag::Byte(0));
        put(&mut raw, "Temper", Tag::Int(temper));
        put(&mut raw, "Bred", Tag::Byte(1));
        put(&mut raw, "EatingHaystack", Tag::Byte(1));
    }
    raw
}

#[test]
fn camel_inherited_fields_decode_and_save_without_raw_or_with_stale_raw() {
    for (species, id) in [
        (Species::Camel, "minecraft:camel"),
        (Species::CamelHusk, "minecraft:camel_husk"),
    ] {
        let types = types(species, id);
        for temper in [None, Some(0), Some(50), Some(-1), Some(256), Some(i32::MAX)] {
            let raw = input(id, temper);
            let mob = mob_from_nbt(&raw, &types, 92, None).unwrap();
            assert!(
                mob.extra.tame,
                "Camel.isTamed remains true even with Tame:0b"
            );
            assert_eq!(mob.extra.bred, temper.is_some());
            assert_eq!(
                mob.extra.flag,
                temper.is_some(),
                "actual loaded eating state"
            );
            assert!(mob.no_ai && mob.no_gravity && mob.silent && mob.invulnerable_flag);
            assert!(!mob.extra.camel_sitting && !mob.extra.flag2);
            assert_eq!(mob.extra.pose_change_tick, 1);
            let mut stale = raw.clone();
            for (field, value) in [
                ("Tame", Tag::Byte(0)),
                ("Temper", Tag::Long(9999)),
                ("Bred", Tag::Byte(i8::from(temper.is_none()))),
                ("EatingHaystack", Tag::Byte(i8::from(temper.is_none()))),
            ] {
                put(&mut stale, field, value);
            }
            for source in [None, Some(&raw), Some(&stale)] {
                let saved = mob_to_nbt(&mob, [92, 0, 0, 1], source, None, |_| None);
                assert_eq!(saved.get("Tame"), Some(&Tag::Byte(1)), "{id}, {temper:?}");
                assert_eq!(saved.get("Temper"), Some(&Tag::Int(temper.unwrap_or(0))));
                for field in ["Bred", "EatingHaystack"] {
                    assert_eq!(
                        saved.get(field),
                        Some(&Tag::Byte(i8::from(temper.is_some())))
                    );
                }
                for field in ["NoAI", "NoGravity", "Silent", "Invulnerable"] {
                    assert_eq!(saved.get(field), Some(&Tag::Byte(1)));
                }
                assert_eq!(saved.get("UUID"), Some(&Tag::IntArray(vec![92, 0, 0, 1])));
                if source.is_some() {
                    for field in ["CustomName", "Tags", "audit:kept"] {
                        assert_eq!(saved.get(field), raw.get(field));
                    }
                }
            }
        }
    }
}
#[test]
fn camel_signed_state_and_mutations_override_contradictory_raw() {
    for (species, id) in [
        (Species::Camel, "minecraft:camel"),
        (Species::CamelHusk, "minecraft:camel_husk"),
    ] {
        let types = types(species, id);
        for (tag, value) in [
            (Tag::Int(-1), -1),
            (Tag::Int(256), 256),
            (Tag::Int(i32::MAX), i32::MAX),
            (Tag::Float((-1.5_f32).to_bits()), -2),
            (Tag::Long(4_294_967_297), 1),
            (string("bad"), 0),
        ] {
            let mut raw = input(id, Some(50));
            put(&mut raw, "Temper", tag);
            let mob = mob_from_nbt(&raw, &types, 92, None).unwrap();
            assert_eq!(mob.extra.equine_temper, value, "actual signed model state");
            assert!(mob.extra.tame && mob.extra.bred && mob.extra.flag);
            assert_eq!(
                mob_to_nbt(&mob, [92, 0, 0, 1], None, None, |_| None).get("Temper"),
                Some(&Tag::Int(value))
            );
        }
        let raw = input(id, Some(50));
        let mut mob = mob_from_nbt(&raw, &types, 92, None).unwrap();
        crate::mobs::camel::finish_load(&mut mob, 1024);
        let unchanged = (
            mob.id,
            mob.position,
            mob.velocity,
            mob.health,
            mob.extra.variant3,
            mob.extra.pose_change_tick,
            mob.extra.egg_time,
            mob.extra.camel_sitting,
            mob.extra.flag2,
        );
        mob.extra.equine_temper = -77;
        mob.extra.bred = false;
        mob.extra.flag = false;
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [92, 0, 0, 1], source, None, |_| None);
            assert_eq!(saved.get("Temper"), Some(&Tag::Int(-77)));
            assert_eq!(saved.get("Tame"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("Bred"), Some(&Tag::Byte(0)));
            assert_eq!(saved.get("EatingHaystack"), Some(&Tag::Byte(0)));
        }
        assert_eq!(
            (
                mob.id,
                mob.position,
                mob.velocity,
                mob.health,
                mob.extra.variant3,
                mob.extra.pose_change_tick,
                mob.extra.egg_time,
                mob.extra.camel_sitting,
                mob.extra.flag2
            ),
            unchanged
        );
    }
}

#[test]
fn the_seven_equine_paths_load_and_save_their_inherited_fields() {
    for (species, id) in [
        (Species::Horse, "minecraft:horse"),
        (Species::Donkey, "minecraft:donkey"),
        (Species::Mule, "minecraft:mule"),
        (Species::SkeletonHorse, "minecraft:skeleton_horse"),
        (Species::ZombieHorse, "minecraft:zombie_horse"),
        (Species::Llama, "minecraft:llama"),
        (Species::TraderLlama, "minecraft:trader_llama"),
    ] {
        let mut raw = input(id, Some(12));
        put(&mut raw, "Tame", Tag::Byte(1));
        let mob = mob_from_nbt(&raw, &types(species, id), 92, None).unwrap();
        assert_eq!(mob.extra.equine_temper, 12);
        // `EatingHaystack:1b` loads as eating for every equine (#18677).
        assert!(mob.extra.flag, "{id} loads its eating state");
        for source in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [92, 0, 0, 1], source, None, |_| None);
            assert_eq!(saved.get("Temper"), Some(&Tag::Int(12)));
            assert_eq!(saved.get("Tame"), Some(&Tag::Byte(1)));
            assert_eq!(saved.get("Bred"), Some(&Tag::Byte(1)));
            // The live eating state, with or without the loaded compound.
            assert_eq!(saved.get("EatingHaystack"), Some(&Tag::Byte(1)));
        }
    }
}

/// `AbstractHorse.readAdditionalSaveData` reads `EatingHaystack` for every
/// equine as `getBooleanOr`, and `addAdditionalSaveData` writes the live
/// eating state, whatever stale value the loaded compound held (#18677).
#[test]
fn every_equine_loads_and_saves_its_eating_state() {
    for (species, id) in [
        (Species::Horse, "minecraft:horse"),
        (Species::Donkey, "minecraft:donkey"),
        (Species::Mule, "minecraft:mule"),
        (Species::SkeletonHorse, "minecraft:skeleton_horse"),
        (Species::ZombieHorse, "minecraft:zombie_horse"),
        (Species::Llama, "minecraft:llama"),
        (Species::TraderLlama, "minecraft:trader_llama"),
    ] {
        let types = types(species, id);
        for (loaded, eating) in [
            (None, false),
            (Some(0), false),
            (Some(-1), true),
            (Some(1), true),
        ] {
            let mut raw = input(id, None);
            if let Some(v) = loaded {
                put(&mut raw, "EatingHaystack", Tag::Byte(v));
            }
            let mut mob = mob_from_nbt(&raw, &types, 92, None).unwrap();
            assert_eq!(mob.extra.flag, eating, "{id} loads {loaded:?}");
            let saved = mob_to_nbt(&mob, [92, 0, 0, 1], Some(&raw), None, |_| None);
            let want = Some(&Tag::Byte(i8::from(eating)));
            assert_eq!(saved.get("EatingHaystack"), want, "{id} saves {loaded:?}");
            // A grazing that ended since the load saves as not eating.
            mob.extra.flag = false;
            let saved = mob_to_nbt(&mob, [92, 0, 0, 1], Some(&raw), None, |_| None);
            assert_eq!(saved.get("EatingHaystack"), Some(&Tag::Byte(0)), "{id}");
        }
    }
}
