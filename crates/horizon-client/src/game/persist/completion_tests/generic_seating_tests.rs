use super::{id, with_game};
use crate::game::commands::Target;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "needs prepared resources; physical server passenger positioning"]
fn generic_vehicle_mob_seats_follow_movement_and_detach() {
    with_game(|g| {
        for (case, kind, height, extra) in [
            (0, "item", 0.25, r#",Item:{id:"minecraft:stone",count:1}"#),
            (1, "item_display", 0.0, ""),
            (2, "block_display", 0.0, ""),
        ] {
            let base = 1000 + case * 10;
            assert!(g.run_command(&format!(
                r#"summon minecraft:{kind} 4.5 200 4.5 {{UUID:[I;161,26,3,{base}],NoGravity:1b{extra},Passengers:[{{id:"minecraft:skeleton",UUID:[I;161,26,3,{}],NoAI:1b,NoGravity:1b,Passengers:[{{id:"minecraft:armor_stand",UUID:[I;161,26,3,{}],NoGravity:1b,Passengers:[{{id:"minecraft:zombie",UUID:[I;161,26,3,{}],IsBaby:1b,NoAI:1b,NoGravity:1b}}]}}]}}]}}"#,
                base + 1, base + 2, base + 3
            )));
            let root = id(g, base).unwrap();
            let skeleton = id(g, base + 1).unwrap();
            let stand = id(g, base + 2).unwrap();
            let baby = id(g, base + 3).unwrap();
            assert!(g.remote.is_none());
            assert_eq!(g.passenger_vehicle(skeleton), Some(root));
            assert_eq!(g.passenger_vehicle(stand), Some(skeleton));
            assert_eq!(g.passenger_vehicle(baby), Some(stand));
            // Pinned Entity.positionRider subtracts VEHICLE from PASSENGER.
            // Skeleton VEHICLE=0.7F, PASSENGER=1.99F; baby zombie=0.1875F.
            for [x, y, z] in [[4.5, 200.0, 4.5], [8.5, 205.0, 7.5]] {
                let target = g.level_entity(root as i32).unwrap();
                g.place_saved_passenger(target, Vec3::new(x, y, z));
                g.sync_riders();
                let seat = y + height - f64::from(0.7_f32);
                assert_eq!(
                    g.target_position(Target::Mob(skeleton)),
                    Some([x, seat, z]),
                    "{kind}: actual skeleton feet"
                );
                let stand_y = seat + f64::from(1.99_f32);
                assert_eq!(
                    g.target_position(Target::Stand(stand)),
                    Some([x, stand_y, z]),
                    "{kind}: ordered mixed tree"
                );
                assert_eq!(
                    g.target_position(Target::Mob(baby)),
                    Some([x, stand_y + f64::from(1.975_f32) - f64::from(0.1875_f32), z]),
                    "{kind}: attachment on moved stand"
                );
                for rider in [skeleton, baby] {
                    let m = g.mobs.iter().find(|m| m.id == rider).unwrap();
                    assert_eq!(m.velocity, Vec3::ZERO);
                    assert!(m.extra.passenger);
                }
            }
            assert!(g.run_command(&format!(
                "ride @e[nbt={{UUID:[I;161,26,3,{}]}},limit=1] dismount",
                base + 1
            )));
            let before = g.target_position(Target::Mob(skeleton));
            g.remove_vehicle_tree(root);
            g.sync_riders();
            assert!(!g.passenger_present(root));
            assert!(g.passenger_present(skeleton));
            assert_eq!(g.passenger_vehicle(skeleton), None);
            assert_eq!(g.target_position(Target::Mob(skeleton)), before);
            g.remove_vehicle_tree(skeleton);
            for rider in [skeleton, stand, baby] {
                assert!(!g.passenger_present(rider));
            }
        }
    });
}

#[test]
#[ignore = "needs prepared resources; specialized seat after generic vehicle moves"]
fn generic_root_moves_specialized_vehicle_before_its_mob() {
    with_game(|g| {
        assert!(g.run_command(r#"summon minecraft:item 4.5 200 4.5 {UUID:[I;161,26,3,1100],NoGravity:1b,Item:{id:"minecraft:stone",count:1},Passengers:[{id:"minecraft:minecart",UUID:[I;161,26,3,1101],NoGravity:1b,Passengers:[{id:"minecraft:skeleton",UUID:[I;161,26,3,1102],NoAI:1b,NoGravity:1b}]}]}"#));
        let root = id(g, 1100).unwrap();
        let cart = id(g, 1101).unwrap();
        let skeleton = id(g, 1102).unwrap();
        assert_eq!(g.passenger_vehicle(cart), Some(root));
        assert_eq!(g.passenger_vehicle(skeleton), Some(cart));
        for [x, y, z] in [[4.5, 200.0, 4.5], [8.5, 205.0, 7.5]] {
            g.place_saved_passenger(g.level_entity(root as i32).unwrap(), Vec3::new(x, y, z));
            g.sync_riders();
            assert_eq!(
                g.target_position(g.level_entity(cart as i32).unwrap()),
                Some([x, y + 0.25, z])
            );
            assert_eq!(
                g.target_position(Target::Mob(skeleton)),
                Some([x, y + 0.25 + 0.1875 - f64::from(0.7_f32), z])
            );
        }
        g.remove_vehicle_tree(root);
        for rider in [root, cart, skeleton] {
            assert!(!g.passenger_present(rider));
        }
    });
}
