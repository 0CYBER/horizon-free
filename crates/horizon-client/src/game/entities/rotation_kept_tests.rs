//! `Entity.load` keeps a saved `Rotation` on falling blocks, shulkers and area
//! effect clouds, and `/data get` reads it back (#16909).
use super::commands::Target;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::nbt::Tag;

#[test]
#[ignore = "daily"]
fn a_summoned_falling_block_shulker_and_cloud_keep_their_rotation() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.summon_with_nbt(
            "minecraft:falling_block",
            [0.5, 70.0, 0.5],
            r#"{Rotation:[75f,20f],BlockState:{Name:"minecraft:sand"},Time:1}"#,
        )
        .unwrap();
        game.summon_with_nbt(
            "minecraft:shulker",
            [4.5, 70.0, 0.5],
            "{Rotation:[75f,20f]}",
        )
        .unwrap();
        game.summon_with_nbt(
            "minecraft:area_effect_cloud",
            [8.5, 70.0, 0.5],
            "{Rotation:[75f,20f]}",
        )
        .unwrap();
        let targets = [
            ("falling block", Target::Falling(game.falling[0].id)),
            ("shulker", Target::Mob(game.mobs[0].id)),
            ("cloud", Target::Cloud(game.breath_clouds[0].id)),
        ];
        for (name, target) in targets {
            let tag = game.command_target_nbt(target).unwrap();
            let Some(Tag::List(rotation)) = tag.get("Rotation") else {
                panic!("{name}: no Rotation in {tag:?}");
            };
            assert_eq!(
                rotation.as_slice(),
                [
                    Tag::Float(75.0_f32.to_bits()),
                    Tag::Float(20.0_f32.to_bits())
                ],
                "{name}"
            );
        }
    });
}

/// `/tp` with a rotation on an area effect cloud writes it to the cloud's saved `Rotation` (what `/data get`
/// reads).
#[test]
#[ignore = "daily"]
fn a_teleported_cloud_reads_back_the_rotation_the_teleport_set() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.summon_with_nbt(
            "minecraft:area_effect_cloud",
            [8.5, 70.0, 0.5],
            "{Rotation:[75f,20f]}",
        )
        .unwrap();
        let cloud = Target::Cloud(game.breath_clouds[0].id);
        assert!(game.run_command("tp @e[type=area_effect_cloud,limit=1] 9.5 70 0.5 30 -10"));
        let tag = game.command_target_nbt(cloud).unwrap();
        let Some(Tag::List(rotation)) = tag.get("Rotation") else {
            panic!("no Rotation in {tag:?}");
        };
        assert_eq!(
            rotation.as_slice(),
            [
                Tag::Float(30.0_f32.to_bits()),
                Tag::Float((-10.0_f32).to_bits())
            ]
        );
    });
}

/// `/tp <victim> <cloud>` takes the cloud's rotation (its saved one, as loaded: yaw `% 360`).
#[test]
#[ignore = "daily"]
fn a_teleport_to_a_cloud_takes_the_clouds_rotation() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.summon_with_nbt(
            "minecraft:area_effect_cloud",
            [8.5, 70.0, 0.5],
            "{Rotation:[400f,20f]}",
        )
        .unwrap();
        game.summon_with_nbt("minecraft:pig", [2.5, 70.0, 0.5], "{}")
            .unwrap();
        assert!(game.run_command("tp @e[type=pig,limit=1] @e[type=area_effect_cloud,limit=1]"));
        let pig = game
            .mobs
            .iter()
            .find(|m| m.kind.id == "minecraft:pig")
            .unwrap();
        assert_eq!([pig.yaw, pig.head_pitch], [40.0, 20.0]);
    });
}

/// A teleport with a rotation sets a cushion's yaw and its saved pitch: the save writes `[30, -10]`, not `[30, 20]`.
#[test]
#[ignore = "daily"]
fn a_teleported_cushion_keeps_the_pitch_the_teleport_set() {
    with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        game.summon_with_nbt(
            "minecraft:cushion",
            [8.5, 70.0, 0.5],
            "{Rotation:[75f,20f]}",
        )
        .unwrap();
        let cushion = Target::Cushion(game.cushions[0].id);
        assert!(game.run_command("tp @e[type=cushion,limit=1] 9.5 70 0.5 30 -10"));
        let tag = game.command_target_nbt(cushion).unwrap();
        let Some(Tag::List(rotation)) = tag.get("Rotation") else {
            panic!("no Rotation in {tag:?}");
        };
        assert_eq!(
            rotation.as_slice(),
            [
                Tag::Float(30.0_f32.to_bits()),
                Tag::Float((-10.0_f32).to_bits())
            ]
        );
    });
}
