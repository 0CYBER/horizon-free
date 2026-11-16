use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::mobs::{
    Mob,
    anim::{self, Poses},
};
use horizon_client::{items::ItemStack, motion::PlayerMotion};
use horizon_core::geometry::Vec3;
use horizon_items::equipment::MAINHAND;
use mod_api::{ArmPose, ClientHost, CustomArmPose, Hooks, ModEntry};
use std::{collections::HashMap, sync::OnceLock};

struct Skillet(OnceLock<CustomArmPose>);
#[mod_api::__subscriptions]
impl Hooks for Skillet {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        self.0
            .set(registrar.register_arm_pose("skillet:flip", false, false))
            .unwrap();
    }
    fn get_arm_pose(
        &self,
        _: &dyn ClientHost,
        _: &mut mod_api::GetArmPoseEvent<'_>,
    ) -> Option<ArmPose> {
        Some(ArmPose::Custom(*self.0.get().unwrap()))
    }
    fn arm_pose_transform(
        &self,
        _: &dyn ClientHost,
        event: &mut mod_api::ArmPoseTransformEvent<'_>,
    ) {
        assert_eq!(event.pose, *self.0.get().unwrap());
        assert!(event.humanoid.main_hand.is_some());
        if event.right_arm {
            event.angles.right = [-0.71, 0.42, 0.63];
        } else {
            event.angles.left = [-0.71, 0.42, 0.63];
        }
    }
}
fn arms(mob: &Mob) -> [[f32; 9]; 2] {
    let mut poses = Poses::new(&mob.kind.model);
    anim::setup(mob, &anim::state(mob, 0.0), &mut poses, &HashMap::new());
    [*poses.get("right_arm"), *poses.get("left_arm")]
}

#[test]
#[ignore = "daily"]
fn registered_angles_reach_local_remote_and_humanoid_before_species_animation() {
    with_client_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
        game.tick_player_model();
        for (id, kind) in [
            (60, game.res.player_type().clone().unwrap()),
            (
                61,
                game.res
                    .mob_types()
                    .get("minecraft:skeleton")
                    .unwrap()
                    .clone(),
            ),
            (
                62,
                game.res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone(),
            ),
        ] {
            let mut mob = Mob::new(id, kind, Vec3::ZERO, 0.0);
            mob.ai.aggressive = false;
            mob.extra.main_hand_left = false;
            mob.extra.left_handed = false;
            mob.extra
                .set_slot_stack(MAINHAND, Some(ItemStack::new("minecraft:stick", 1)));
            game.mobs.push(mob);
        }
        let baseline: Vec<_> = game.mobs.iter().map(arms).collect();
        mod_platform::testing::with_mods(
            vec![ModEntry::new("skillet", Box::new(Skillet(OnceLock::new())))].into(),
            || {
                game.tick_remote_arm_poses();
                game.tick_player_model();
                for mob in [
                    game.player_model.as_ref().unwrap(),
                    &game.mobs[0],
                    &game.mobs[1],
                ] {
                    let posed = arms(mob);
                    let age = anim::state(mob, 0.0).age;
                    // Vanilla's arm bobbing follows the custom pose at the model's age.
                    // Mth's 65536-entry trig table differs from sin/cos by at most
                    // one sample; after the 0.05 scale that is below 0.000005.
                    let bob_x = (age * 0.067).sin() * 0.05;
                    let bob_z = (age * 0.09).cos() * 0.05 + 0.05;
                    assert!(
                        (posed[0][3] - (-0.71 + bob_x)).abs() < 0.000005,
                        "id {} main left {} age {age} poses {:?} arms {:?}",
                        mob.id,
                        mob.main_arm_left(),
                        mob.extra.held_arm_pose,
                        posed
                    );
                    assert!((posed[0][4] - 0.42).abs() < 1e-6);
                    assert!((posed[0][5] - (0.63 + bob_z)).abs() < 0.000005);
                }
                // Zombie animation is later than the base pose and overrides its arms.
                assert_eq!(arms(&game.mobs[2]), baseline[2]);
            },
        );
        game.tick_remote_arm_poses();
        assert_eq!(game.mobs.iter().map(arms).collect::<Vec<_>>(), baseline);
    });
}

#[test]
#[ignore = "daily"]
fn suppressed_custom_offhand_never_calls_its_transformer() {
    use std::sync::{Arc, Mutex};
    struct PosesMod {
        main: OnceLock<CustomArmPose>,
        off: OnceLock<CustomArmPose>,
        calls: Arc<Mutex<Vec<bool>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for PosesMod {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            self.main
                .set(registrar.register_arm_pose("skillet:main", false, true))
                .unwrap();
            self.off
                .set(registrar.register_arm_pose("skillet:off", false, false))
                .unwrap();
        }
        fn get_arm_pose(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::GetArmPoseEvent<'_>,
        ) -> Option<ArmPose> {
            Some(ArmPose::Custom(
                *if event.hand == mod_api::Hand::Main {
                    self.main.get()
                } else {
                    self.off.get()
                }
                .unwrap(),
            ))
        }
        fn arm_pose_transform(
            &self,
            _: &dyn ClientHost,
            event: &mut mod_api::ArmPoseTransformEvent<'_>,
        ) {
            assert_eq!(event.pose, *self.main.get().unwrap());
            assert_eq!(event.humanoid.main_arm_pose, ArmPose::Custom(event.pose));
            assert_eq!(event.humanoid.off_arm_pose, ArmPose::Empty);
            self.calls.lock().unwrap().push(event.right_arm);
        }
    }
    with_client_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
        game.offhand = Some(ItemStack::new("minecraft:stick", 1));
        let calls = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "skillet",
                Box::new(PosesMod {
                    main: OnceLock::new(),
                    off: OnceLock::new(),
                    calls: calls.clone(),
                }),
            )]
            .into(),
            || {
                game.tick_player_model();
                assert_eq!(*calls.lock().unwrap(), [true]);
            },
        );
    });
}
