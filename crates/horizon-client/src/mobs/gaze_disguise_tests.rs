use super::*;
use crate::mobs::world::{test_player, with_resource_test_world};

fn gaze_cases(replace_tag: bool) {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources");
        assert!(with_resource_test_world(
            std::path::Path::new(&root),
            &[],
            true,
            true,
            |_, _| {},
            |w, types| {
                let mut items = w.items.unwrap().clone();
                if replace_tag {
                    items.item_tags.insert(
                        "minecraft:gaze_disguise_equipment".into(),
                        ["minecraft:skeleton_skull".into()].into(),
                    );
                }
                for (head, disguised) in if replace_tag {
                    [
                        (Some("minecraft:skeleton_skull"), true),
                        (Some("minecraft:carved_pumpkin"), false),
                        (None, false),
                    ]
                } else {
                    [
                        (Some("minecraft:carved_pumpkin"), true),
                        (Some("minecraft:skeleton_skull"), false),
                        (None, false),
                    ]
                } {
                    for id in ["minecraft:enderman", "minecraft:creaking"] {
                        let mut m = Mob::new(
                            1,
                            std::sync::Arc::new(types.types[id].clone()),
                            Vec3::new(0.5, 64.0, 0.5),
                            0.0,
                        );
                        let mut p = test_player(Vec3::new(2.5, 64.0, 0.5));
                        p.head = head.map(Into::into);
                        let dy = m.eye_y() - p.eye[1];
                        let len = (4.0 + dy * dy).sqrt();
                        p.look = [-2.0 / len, dy / len, 0.0];
                        let mut world = w.clone();
                        world.items = Some(&items);
                        world.player = Some(p);
                        if id == "minecraft:enderman" {
                            assert_eq!(
                                crate::mobs::enderman::stared_at(&m, &world),
                                !disguised,
                                "{head:?}"
                            );
                        } else {
                            monster_brain::init(&mut m, 0);
                            monster_brain::mem(&mut m)
                                .nearest_players
                                .set_list(vec![Who::Player]);
                            m.extra.flag = true;
                            assert_eq!(
                                check_can_move(&mut m, &world, &mut Vec::new()),
                                disguised,
                                "{head:?}"
                            );
                        }
                    }
                }
            }
        ));
    });
}

#[test]
#[ignore = "daily"]
fn gaze_disguise_shipped_tag() {
    gaze_cases(false);
}

#[test]
#[ignore = "daily"]
fn gaze_disguise_replaced_tag() {
    gaze_cases(true);
}
