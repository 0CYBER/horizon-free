//! Production residency regressions for projectile ownership.
#![cfg(test)]
use crate::game::Game;
use horizon_client::{
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::geometry::Vec3;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use std::sync::{Arc, Mutex};

fn arrow(id: u32, at: [f64; 3]) -> Projectile {
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::new(at[0], at[1], at[2]),
        None,
    );
    p.id = id;
    p
}

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "projectile_restore",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
#[ignore = "daily"]
fn projectile_restore_entries_keep_the_bound_arrow() {
    for path in 0..3 {
        let exercise = |game: &mut Game<'_>| {
            let at = [3.0, 64.0, 5.0];
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            game.creatures.projectiles.push(arrow(71, at));
            game.creatures
                .projectiles
                .update(0, |p| p.velocity = Vec3::new(0.3, 0.0, 0.0));
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
            let original = game.creatures.projectiles.update(0, |p| {
                p.attachment_lifetime().bind(&mut values, &index, holder);
                p.attachment_lifetime().registration().unwrap()
            });
            let key = ("test".into(), "projectile_restore".into());
            let before = index.lock().unwrap().snapshot(&key);
            assert_eq!(before.len(), 1);
            if path == 0 {
                game.tick_remote_projectiles(|_, at| at);
                assert!(game.creatures.projectiles[0].position.x > at[0]);
            } else {
                // One takeable unbound arrow exercises the pickup branch while
                // the bound, disallowed arrow must survive the same retain.
                let mut picked = arrow(72, at);
                let state = picked.arrow.as_mut().unwrap();
                state.in_ground = true;
                state.pickup = horizon_client::arrows::Pickup::Allowed;
                for id in [73, 74] {
                    game.creatures.projectiles.push(arrow(id, at));
                }
                game.creatures.projectiles.push(picked);
                if path == 1 {
                    crate::game::entity::taken_pass_tests::assert_projectile_spawn_survives(
                        game,
                        |game| {
                            game.guest_arrows(0);
                        },
                    );
                } else {
                    game.player = Some(
                        PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2])).unwrap(),
                    );
                    crate::game::entity::taken_pass_tests::assert_projectile_spawn_survives(
                        game,
                        |game| {
                            game.touch_arrows();
                        },
                    );
                }
                assert_eq!(
                    game.creatures
                        .projectiles
                        .iter()
                        .map(|p| p.id)
                        .collect::<Vec<_>>(),
                    [71, 73, 74]
                );
            }
            let current = game
                .creatures
                .projectiles
                .update(0, |p| p.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current), "restore path {path}");
            assert_eq!(
                index.lock().unwrap().snapshot(&key).len(),
                1,
                "restore path {path}"
            );
            game.creatures.projectiles.clear();
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        };
        if path == 0 {
            crate::game::native_entity_refs::tests::with_client_game(exercise);
        } else {
            crate::game::native_guest_turn_tests::with_guests(exercise);
        }
    }
}
