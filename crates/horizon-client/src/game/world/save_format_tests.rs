//! The ordinary-world save contract is independent of object residency bookkeeping.
#![cfg(test)]

use horizon_core::nbt::parse_snbt;

#[test]
#[ignore = "daily"]
fn ordinary_nested_entities_keep_the_complete_save_format() {
    // Fully specified ordinary-world contract, checked independently on main f8f517a8e4.
    // Do not build this expectation with the serializer being tested: extra engine
    // bookkeeping fields, missing fields, numeric tag-type changes and tree loss all fail.
    let expected = parse_snbt(
        r#"{
        id:"minecraft:pig",UUID:[I;701,0,0,1],Pos:[1.0d,70.0d,2.0d],
        Motion:[0.0d,0.0d,0.0d],Rotation:[0.0f,0.0f],NoAI:1b,
        Air:300s,Fire:0s,Health:10.0f,AbsorptionAmount:0.0f,HurtTime:0s,DeathTime:0s,
        OnGround:0b,Invulnerable:0b,FallFlying:0b,LeftHanded:0b,
        PersistenceRequired:0b,CanPickUpLoot:0b,PortalCooldown:0,fall_distance:0.0d,
        current_impulse_context_reset_grace_time:0,attributes:[],Brain:{memories:{}},
        Age:0,ForcedAge:0,AgeLocked:0b,InLove:0,
        sound_variant:"minecraft:classic",variant:"minecraft:temperate",
        Passengers:[{
            id:"minecraft:cow",UUID:[I;701,0,0,2],Pos:[1.0d,71.0d,2.0d],
            Motion:[0.0d,0.0d,0.0d],Rotation:[0.0f,0.0f],NoAI:1b,
            Air:300s,Fire:0s,Health:10.0f,AbsorptionAmount:0.0f,HurtTime:0s,DeathTime:0s,
            OnGround:0b,Invulnerable:0b,FallFlying:0b,LeftHanded:0b,
            PersistenceRequired:0b,CanPickUpLoot:0b,PortalCooldown:0,fall_distance:0.0d,
            current_impulse_context_reset_grace_time:0,attributes:[],Brain:{memories:{}},
            Age:0,ForcedAge:0,AgeLocked:0b,InLove:0,
            sound_variant:"minecraft:classic",variant:"minecraft:temperate"
        }]
    }"#,
    )
    .unwrap()
    .0;
    let mut executed = false;
    crate::game::native_entity_refs::tests::with_game(|game| {
        executed = true;
        let source = parse_snbt(
            r#"{
            id:"minecraft:pig",UUID:[I;701,0,0,1],Pos:[1.0d,70.0d,2.0d],NoAI:1b,
            Passengers:[{id:"minecraft:cow",UUID:[I;701,0,0,2],Pos:[1.0d,71.0d,2.0d],NoAI:1b}]
        }"#,
        )
        .unwrap()
        .0;
        let root = game.load_mob(source, None).unwrap();
        assert_eq!(game.vehicle_tree_nbt(root).unwrap(), expected);
    });
    assert!(
        executed,
        "fixture did not execute; prepared data is missing"
    );
}
