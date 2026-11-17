use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{
    EnderPearlTeleportEvent, EntityTeleportEvent, Flow, Hooks, ModEntry, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

struct PearlProbe {
    mode: Arc<AtomicU8>,
    owners: Arc<Mutex<Vec<u32>>>,
    command_teleports: Arc<Mutex<Vec<u32>>>,
}
#[mod_api::__subscriptions]
impl Hooks for PearlProbe {
    fn entity_teleport(&self, _: &mut dyn ServerHost, event: &mut EntityTeleportEvent<'_>) {
        self.command_teleports.lock().unwrap().push(event.entity);
    }

    fn ender_pearl_teleport(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EnderPearlTeleportEvent<'_>,
    ) -> Flow {
        self.owners.lock().unwrap().push(event.owner.id());
        assert_eq!(event.owner.position(host), Some(event.from));
        assert_eq!(
            event.damage, 0.0,
            "non-player owners take no vanilla fall damage"
        );
        match self.mode.load(Ordering::SeqCst) {
            0 => Flow::Cancel,
            1 => {
                *event.target = [12.0, 80.0, -4.0];
                Flow::Continue
            }
            2 => {
                assert!(event.owner.teleport_to(host, [3.0, 70.0, 3.0], None));
                *event.target = [12.0, 80.0, -4.0];
                Flow::Continue
            }
            3 => {
                assert!(event.owner.discard(host));
                Flow::Continue
            }
            _ => unreachable!(),
        }
    }
}

struct PlayerPearlProbe(Arc<AtomicU8>);
#[mod_api::__subscriptions]
impl Hooks for PlayerPearlProbe {
    fn ender_pearl_teleport(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EnderPearlTeleportEvent<'_>,
    ) -> Flow {
        assert_eq!(
            host.players().acting_player().unwrap().entity(),
            event.owner
        );
        assert_eq!(event.damage, 5.0);
        // What the pearl hit: the block and its face, no entity.
        assert_eq!(event.hit_entity, None);
        assert_eq!(
            (event.hit_block, event.hit_face, event.hit_at),
            ([6, 75, 6], [0, 1, 0], [6.0, 75.0, 6.0])
        );
        match self.0.load(Ordering::SeqCst) {
            0 => return Flow::Cancel,
            2 => assert!(event.owner.teleport_to(host, [3.0, 70.0, 3.0], None)),
            _ => {}
        }
        *event.target = [12.0, 80.0, -4.0];
        event.damage = 0.0;
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn player_pearl_keeps_acting_context_cancellation_and_mutable_damage() {
    use horizon_client::motion::PlayerMotion;
    let mode = Arc::new(AtomicU8::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "player-pearl-proof",
        Box::new(PlayerPearlProbe(Arc::clone(&mode))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let from = Vec3::new(0.0, 65.0, 0.0);
            game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
            game.lan_acting = Some(12);
            game.lan_acting_uuid = Some(7);
            crate::game::native_entity_refs::tests::bind_local_player(game, 2, 7);
            game.lan_acting_name = Some("PearlTest".into());
            game.lan_acting_permission = Some(4);
            game.lan_acting_entity = Some(2);
            crate::game::native_entity_refs::tests::bind_acting_guest(game);
            game.ui.difficulty = 0;
            let pearl = Projectile::new(
                ProjectileKind::EnderPearl,
                Vec3::new(6.0, 75.0, 6.0),
                Some(Who::Player),
            );
            let hit = || Hit::Block {
                pos: [6, 75, 6],
                at: pearl.position,
                face: [0, 1, 0],
            };
            let mut expected_random = game.world.random.clone();
            let health = game.player.as_ref().unwrap().health.health();
            game.projectile_hit(&pearl, hit()).unwrap();
            assert_eq!(game.player.as_ref().unwrap().position, from);
            assert_eq!(game.player.as_ref().unwrap().health.health(), health);
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
            mode.store(1, Ordering::SeqCst);
            expected_random = game.world.random.clone();
            expected_random.next_float();
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.position, Vec3::new(12.0, 80.0, -4.0));
            assert_eq!(
                player.health.health(),
                health,
                "listener suppressed player fall damage"
            );
            assert_eq!(player.fall_distance, 0.0);
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
            mode.store(2, Ordering::SeqCst);
            expected_random = game.world.random.clone();
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(
                player.position,
                Vec3::new(3.0, 70.0, 3.0),
                "a listener's direct move is not overwritten by the pearl target"
            );
            assert_eq!(player.health.health(), health);
            assert_eq!(
                game.world.random.next_long(),
                expected_random.next_long(),
                "a stale player move makes no endermite draw"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn mob_pearls_use_the_hook_and_recheck_listener_writes_without_player_side_effects() {
    let mode = Arc::new(AtomicU8::new(0));
    let owners = Arc::new(Mutex::new(Vec::new()));
    let command_teleports = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-pearl-proof",
        Box::new(PearlProbe {
            mode: Arc::clone(&mode),
            owners: Arc::clone(&owners),
            command_teleports: Arc::clone(&command_teleports),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for action in 0..4 {
                mode.store(action, Ordering::SeqCst);
                let id = 40 + u32::from(action);
                let from = Vec3::new(0.0, 65.0, 0.0);
                let ty = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let mut mob = Mob::new(id, ty, from, 37.0);
                mob.head_pitch = -12.0;
                mob.previous_yaw = -10.0;
                mob.previous_head_pitch = 20.0;
                mob.velocity = Vec3::new(0.5, -0.25, 0.75);
                mob.fall_distance = 9.0;
                let health = mob.health;
                game.mobs.push(mob);
                let mut pearl = Projectile::new(
                    ProjectileKind::EnderPearl,
                    Vec3::new(6.0, 75.0, 6.0),
                    Some(Who::Mob(id)),
                );
                pearl.id = 100 + id;
                let mut expected_random = game.world.random.clone();
                game.projectile_hit(
                    &pearl,
                    Hit::Block {
                        pos: [6, 75, 6],
                        at: pearl.position,
                        face: [0; 3],
                    },
                )
                .unwrap();
                assert_eq!(
                    game.world.random.next_long(),
                    expected_random.next_long(),
                    "mob pearl handling makes no player endermite draw"
                );
                let mob = game.mobs.iter().find(|m| m.id == id && !m.removed);
                match action {
                    0 => {
                        let m = mob.unwrap();
                        assert_eq!(m.position, from);
                        assert_eq!(m.fall_distance, 9.0);
                    }
                    1 => {
                        let m = mob.unwrap();
                        assert_eq!(m.position, Vec3::new(12.0, 80.0, -4.0));
                        assert_eq!(m.velocity, Vec3::ZERO);
                        assert_eq!(m.fall_distance, 0.0);
                        assert_eq!((m.yaw, m.head_pitch), (37.0, -12.0));
                        assert_eq!((m.previous_yaw, m.previous_head_pitch), (37.0, -12.0));
                        assert_eq!(m.health, health);
                    }
                    2 => assert_eq!(mob.unwrap().position, Vec3::new(3.0, 70.0, 3.0)),
                    3 => assert!(mob.is_none()),
                    _ => unreachable!(),
                }
                assert!(!game.mobs.iter().any(|m| m.species() == Species::Endermite));
                assert_eq!(
                    *command_teleports.lock().unwrap(),
                    if action < 2 { vec![] } else { vec![42] },
                    "only the listener's explicit teleport emits the command notification"
                );
            }
            assert_eq!(*owners.lock().unwrap(), [40, 41, 42, 43]);
        })
    });
}
