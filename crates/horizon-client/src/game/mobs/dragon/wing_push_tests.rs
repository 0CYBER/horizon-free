//! Wing contacts exercise the real Game damage/equipment and velocity continuation.
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{items::ItemStack, motion::PlayerMotion};
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn wing_contacts_restore_motion_and_sync_only_when_hurt_is_refused() {
    with_game(|g| {
        g.creative = false;
        let initial = Vec3::new(0.25, -0.125, 0.75);
        let push = Vec3::new(0.5, f64::from(0.2_f32), -0.25);
        g.player = Some(PlayerMotion::new(g.motion, Vec3::new(30.0, 64.0, 0.5)).unwrap());
        g.player.as_mut().unwrap().velocity = initial;
        g.armor[1] = Some(ItemStack::new("minecraft:leather_chestplate", 1));
        g.dragon_wing_knock_back(push, false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), 15.0);
        assert_eq!(p.velocity, initial.add_vector(push));
        assert!(p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 1);

        // A same-strength hit in the cooldown cannot push or wear armor again.
        g.player.as_mut().unwrap().health.sync_velocity = false;
        g.dragon_wing_knock_back(push, false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), 15.0);
        assert_eq!(p.velocity, initial.add_vector(push));
        assert!(!p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 1);

        // Sitting contacts must restore even a pre-existing sync marker.
        g.player.as_mut().unwrap().health.sync_velocity = true;
        g.dragon_wing_knock_back(push, true);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), 15.0);
        assert_eq!(p.velocity, initial.add_vector(push));
        assert!(p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 1);

        // Outside the cooldown an accepted contact carries another push.
        for _ in 0..10 {
            g.player.as_mut().unwrap().health.advance_clock();
            g.world.game_time += 1;
        }
        g.player.as_mut().unwrap().health.sync_velocity = false;
        g.dragon_wing_knock_back(push, false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), 10.0);
        assert_eq!(p.velocity, initial.add_vector(push).add_vector(push));
        assert!(p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 2);

        // Creative refusal must stand alone, not be masked by hit cooldown.
        for _ in 0..10 {
            g.player.as_mut().unwrap().health.advance_clock();
            g.world.game_time += 1;
        }
        g.creative = true;
        g.player.as_mut().unwrap().health.sync_velocity = false;
        g.dragon_wing_knock_back(push, false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), 10.0);
        assert_eq!(p.velocity, initial.add_vector(push).add_vector(push));
        assert!(!p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 2);
    });
}

#[test]
#[ignore = "daily"]
fn a_shield_faces_the_dragons_origin_not_the_wing_push() {
    with_game(|g| {
        g.creative = false;
        g.player = Some(PlayerMotion::new(g.motion, Vec3::ZERO).unwrap());
        g.armor[1] = Some(ItemStack::new("minecraft:leather_chestplate", 1));
        g.inventory[0] = Some(ItemStack::new("minecraft:shield", 1));
        g.selected = 0;
        g.using = Some((0, 20));
        g.yaw = 0.0;
        let mut dragon = super::Dragon::new(Vec3::new(0.0, 0.0, 4.0), 0.0, 1);
        dragon.id = 900;
        g.dragon.replace(dragon);
        assert!(
            g.player_blocking(Some([0.0, 0.0, 4.0]), "minecraft:mob_attack", false)
                .is_some()
        );
        // The source is ahead, although the deliberately independent push points sideways.
        g.dragon_wing_knock_back(Vec3::new(0.5, 0.2, 0.0), false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), 20.0);
        assert_eq!(p.velocity, Vec3::ZERO);
        assert!(!p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 0);
        // Items.SHIELD uses ItemDamageFunction(3, 1, 1): floor(1 + 5).
        assert_eq!(g.inventory[0].as_ref().unwrap().damage, 6);
    });
}
