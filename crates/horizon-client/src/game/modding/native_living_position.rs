//! `LivingRef` line of sight, random teleport and the box of a pose (arismoko/horizon#944):
//! the engine's own clip and `randomTeleport` paths, for players and mobs
//! alike.
use super::{Game, commands::Target};
use horizon_core::geometry::Vec3;
use mod_api::{EntityRef, LivingRef, Pose};

impl Game<'_> {
    /// `LivingEntity.hasLineOfSight(Entity)`: whether nothing that collides
    /// (`ClipContext.Block.COLLIDER`, fluids ignored) lies between the
    /// living entity's eyes and `target`'s eye height, within 128 blocks.
    /// False for a target in another level. None for a stale or unavailable
    /// living entity or target, one outside the level in place, or on a
    /// client.
    pub(super) fn living_has_line_of_sight(
        &self,
        living: LivingRef,
        target: EntityRef,
    ) -> Option<bool> {
        let viewer = self.native_living_target(living)?;
        let seen = self.native_entity_target(target)?;
        if self.target_level(viewer) != self.dim {
            return None;
        }
        if self.target_level(seen) != self.dim {
            return Some(false);
        }
        let feet = self.entity_position(living.entity())?;
        let from = self.entity_eye_position(living.entity())?;
        let to = self.entity_eye_position(target)?;
        let w = self.mob_world_base(&[]);
        Some(w.line_of_sight_for(
            Some(feet[1]),
            Vec3::new(from[0], from[1], from[2]),
            Vec3::new(to[0], to[1], to[2]),
        ))
    }

    /// `LivingEntity.getDimensions(Pose)`: the box the living entity has in
    /// `pose` — its width, height and eye height, `minecraft:scale`
    /// included, the fixed 0.2 box of a sleeping pose included. The species'
    /// own state still answers where its `getDefaultDimensions` reads it: a
    /// digging sniffer or an emerging warden is low in every pose. None for
    /// a stale or unavailable handle.
    pub(super) fn living_dimensions_for_pose(
        &self,
        living: LivingRef,
        pose: Pose,
    ) -> Option<[f64; 3]> {
        let target = self.native_living_read_target(living)?;
        // LivingEntity.getDimensions is final: sleeping wins over every
        // species' default dimensions, but never makes a stale handle valid.
        if pose == Pose::Sleeping {
            return Some([f64::from(0.2_f32); 3]);
        }
        match target {
            t @ (Target::Player | Target::Guest(_)) => {
                // `Avatar.getDefaultDimensions(pose).scale(getScale())`.
                let scale = self.native_target_attribute(
                    t,
                    "minecraft:scale",
                    horizon_world::attribute_map::ValueKind::Computed,
                )? as f32;
                let d = crate::pose::Dimensions::avatar(pose, scale);
                Some([
                    f64::from(d.width),
                    f64::from(d.height),
                    f64::from(d.eye_height),
                ])
            }
            Target::Mob(id) => {
                let (w, h, e) = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)?
                    .dims_in_pose(pose);
                Some([w, h, e])
            }
            // ArmorStand.getDefaultDimensions ignores the pose: a marker's
            // box is empty, a small stand's BABY_DIMENSIONS, else the type's.
            Target::Stand(id) => {
                let a = self.armor_stands.iter().find(|a| a.id == id)?;
                let dims = if a.marker {
                    (0.0, 0.0, 0.0)
                } else {
                    a.probe_dimensions()
                };
                let scale = self.native_target_attribute(
                    target,
                    "minecraft:scale",
                    horizon_world::attribute_map::ValueKind::Computed,
                )? as f32;
                Some([dims.0, dims.1, dims.2].map(|v| f64::from(v as f32 * scale)))
            }
            Target::Dragon(id) => {
                self.dragon.as_ref().filter(|d| d.id == id)?;
                // EntityTypes.ENDER_DRAGON: scalable(16, 8), default eye
                // height 8 * 0.85. Dragons are not in the ordinary mob table.
                let scale = self.native_target_attribute(
                    target,
                    "minecraft:scale",
                    horizon_world::attribute_map::ValueKind::Computed,
                )? as f32;
                Some([16.0_f32, 8.0, 6.8].map(|v| f64::from(v * scale)))
            }
            _ => None,
        }
    }

    /// `LivingEntity.randomTeleport(x, y, z, true, avoid)`: the point `at`
    /// is clamped into the world border and drops to the first block in
    /// `minecraft:entities_can_teleport_to` below it, losing a block of height
    /// per step down. The entity lands there, keeping its motion, unless that
    /// ground or a block in its box at the landing is in the `avoid` block
    /// tag, the box collides or holds a liquid, or the species refuses the
    /// spot (an enderman never lands on ground holding water). A landing
    /// stops a mob's navigation and broadcasts entity event 46 (the portal
    /// particles). Nothing else is played: a chorus fruit's sound and an
    /// enderman's own teleport effects belong to their callers.
    ///
    /// True when the entity moved. False when no spot was found, for a
    /// non-finite point, a fake player, a player in another level, an armor
    /// stand or the dragon, a stale handle, or on a client.
    pub(super) fn living_random_teleport(
        &mut self,
        living: LivingRef,
        at: [f64; 3],
        avoid: Option<&str>,
    ) -> bool {
        if !at.iter().all(|v| v.is_finite()) {
            return false;
        }
        let at = Vec3::new(at[0], at[1], at[2]);
        match self.native_living_target(living) {
            Some(target @ (Target::Player | Target::Guest(_))) => {
                if self.target_level(target) != self.dim {
                    return false;
                }
                self.native_player_sleep_write(target, |game| {
                    game.player.is_some() && game.random_teleport(at, avoid)
                })
            }
            Some(Target::Mob(id)) => {
                if !self.mobs.iter().any(|m| m.id == id && !m.removed) {
                    return false;
                }
                let mut mobs = self.mobs.lift();
                #[cfg(test)]
                super::entity::taken_pass_tests::after_take(self);
                let w = self.mob_world_base(&[]);
                let mut events = Vec::new();
                let mut teleported = None;
                let moved = mobs.iter_mut().find(|m| m.id == id).is_some_and(|m| {
                    let from = m.position;
                    let moved = horizon_client::mobs::living::random_teleport(
                        m,
                        &w,
                        at,
                        avoid,
                        &mut events,
                    );
                    teleported = moved.then_some((from, m.position));
                    moved
                });
                self.mobs
                    .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
                let _ = self.handle_mob_events(events);
                // `teleportTo` is one position update, told at once.
                if let Some((from, to)) = teleported {
                    let (from, to) = ([from.x, from.y, from.z], [to.x, to.y, to.z]);
                    self.entity_section_moved(id, Some(from), to);
                }
                moved
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use horizon_client::mobs::Mob;
    use mod_api::PathType;

    #[test]
    #[ignore = "daily"]
    fn pose_regression_dying_avatar_is_fixed() {
        with_game(|game| {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:mannequin")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(55, ty, Vec3::ZERO, 0.0));
            let living = game.living(game.entity(55).unwrap()).unwrap();
            assert!(game.living_set_attribute_base(living, "minecraft:scale", 2.0));
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Dying),
                Some([f64::from(0.2_f32), f64::from(0.2_f32), f64::from(1.62_f32)])
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn pose_regression_sleeping_stand() {
        with_game(|game| {
            game.armor_stands
                .push(horizon_client::entities::ArmorStand::new(
                    55,
                    Vec3::ZERO,
                    0.0,
                ));
            let living = game.living(game.entity(55).unwrap()).unwrap();
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Sleeping),
                Some([f64::from(0.2_f32); 3])
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn pose_regression_dragon_dimensions() {
        with_game(|game| {
            let mut dragon = super::super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
            dragon.id = 55;
            game.dragon.replace(dragon);
            let living = game.living(game.entity(55).unwrap()).unwrap();
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([16.0, 8.0, f64::from(6.8_f32)])
            );
            // A saved attribute exercises the live map without the public
            // attribute writer, which deliberately refuses dragon writes.
            use horizon_core::nbt::Tag;
            game.dragon.as_mut().unwrap().live.load(
                Some(&Tag::List(vec![Tag::compound([
                    ("id", Tag::String("minecraft:scale".into())),
                    ("base", Tag::Double(2.0_f64.to_bits())),
                ])])),
                |_| true,
            );
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([32.0, 16.0, f64::from(13.6_f32)])
            );
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Sleeping),
                Some([f64::from(0.2_f32); 3])
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn pose_regression_scaled_stand() {
        with_game(|game| {
            game.armor_stands
                .push(horizon_client::entities::ArmorStand::new(
                    55,
                    Vec3::ZERO,
                    0.0,
                ));
            let living = game.living(game.entity(55).unwrap()).unwrap();
            assert!(game.living_set_attribute_base(living, "minecraft:scale", 2.0));
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([1.0, f64::from(3.95_f32), f64::from(3.555_f32)])
            );
            game.armor_stands[0].small = true;
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([0.5, f64::from(1.975_f32), f64::from(1.975_f32)])
            );
            game.armor_stands[0].marker = true;
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([0.0; 3])
            );
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Sleeping),
                Some([f64::from(0.2_f32); 3])
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn pose_regression_scaled_digging_warden_is_fixed() {
        with_game(|game| {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:warden")
                .unwrap()
                .clone();
            let mut mob = Mob::new(55, ty, Vec3::ZERO, 0.0);
            horizon_client::mobs::sync::set_pose(&mut mob, horizon_client::mobs::sync::DIGGING);
            mob.extra.model_scale = 2.0;
            game.mobs.push(mob);
            let living = game.living(game.entity(55).unwrap()).unwrap();
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([f64::from(0.9_f32), 1.0, f64::from(0.85_f32)])
            );
        });
    }

    /// Lit empty chunks around the origin, a stone floor at y 63 over
    /// -8..8, and two zombies standing on it five blocks apart.
    pub(in crate::game) fn floor_and_zombies(game: &mut Game<'_>) {
        for (x, z) in (-1..=1).flat_map(|x| (-1..=1).map(move |z| (x, z))) {
            let mut chunk = horizon_client::chunk::Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), std::sync::Arc::new(chunk));
        }
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in -8..8 {
            for z in -8..8 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs
            .push(Mob::new(12, zombie.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0));
        game.mobs
            .push(Mob::new(13, zombie, Vec3::new(5.5, 64.0, 0.5), 0.0));
    }

    #[test]
    #[ignore = "daily"]
    fn line_of_sight_follows_the_blocks_between() {
        with_game(|game| {
            floor_and_zombies(game);
            let a = game.living(game.entity(12).unwrap()).unwrap();
            let b = game.entity(13).unwrap();
            assert_eq!(a.has_line_of_sight(game, b), Some(true));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for y in 64..67 {
                game.world.set(3, y, 0, stone).unwrap();
            }
            assert_eq!(
                a.has_line_of_sight(game, b),
                Some(false),
                "a wall in between"
            );
            game.mobs.retain(|m| m.id != 13);
            assert_eq!(a.has_line_of_sight(game, b), None, "a gone target");
        });
    }

    #[test]
    #[ignore = "daily"]
    fn random_teleport_drops_to_the_ground_and_honours_the_avoid_tag() {
        crate::game::entity::taken_pass_tests::in_both_dimensions(|game, balance| {
            floor_and_zombies(game);
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            let at = |game: &Game<'_>| game.mobs.iter().find(|m| m.id == 12).unwrap().position;
            // From six blocks up it lands on the floor, one block lower per
            // step down (`randomTeleport`).
            crate::game::entity::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert!(zombie.random_teleport(game, [4.5, 70.0, 4.5], None));
                },
            );
            assert_eq!(at(game), Vec3::new(4.5, 64.0, 4.5));
            assert_eq!(
                game.mobs.iter().find(|m| m.id == 12).unwrap().previous,
                at(game)
            );
            // No ground below (outside the floor): it stays.
            assert!(!zombie.random_teleport(game, [12.5, 70.0, 12.5], None));
            assert_eq!(at(game), Vec3::new(4.5, 64.0, 4.5));
            // Ground in the avoid tag: bedrock is in the enderman's.
            let bedrock = game
                .pack
                .assets
                .registry
                .default_state("minecraft:bedrock")
                .unwrap();
            game.world.set(-4, 63, -4, bedrock).unwrap();
            assert!(!zombie.random_teleport(
                game,
                [-3.5, 66.0, -3.5],
                Some("minecraft:enderman_does_not_teleport_to"),
            ));
            assert!(zombie.random_teleport(game, [-3.5, 66.0, -3.5], None));
            assert_eq!(at(game), Vec3::new(-3.5, 64.0, -3.5));
            assert!(!zombie.random_teleport(game, [f64::NAN, 66.0, 0.5], None));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dimensions_follow_the_pose_asked() {
        use super::super::lan::LanSettings;
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };

        with_game(|game| {
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
                level_seeds: game.levels.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            let (tx, _rx) = std::sync::mpsc::channel();
            let mut guest = Guest::new(
                Joined {
                    conn: 12,
                    uuid: 12,
                    name: "guest12".to_owned(),
                    properties: Vec::new(),
                    info: ClientInformation::new(8),
                    brand: String::new(),
                    sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                    latency: Arc::new(AtomicI32::new(0)),
                    address: "127.0.0.1:1".parse().unwrap(),
                    expected: false,
                },
                12,
                [3.0, 64.0, 5.0],
                10,
            );
            guest.placed = true;
            let motion =
                horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 5.0))
                    .unwrap();
            guest.motion = Some(Box::new(motion));
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let living = game.living(game.entity(12).unwrap()).unwrap();
            // Avatar.POSES: the box of each stance a player takes.
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Standing),
                Some([f64::from(0.6_f32), f64::from(1.8_f32), f64::from(1.62_f32)])
            );
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Crouching),
                Some([f64::from(0.6_f32), f64::from(1.5_f32), f64::from(1.27_f32)])
            );
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Swimming),
                Some([f64::from(0.6_f32), f64::from(0.6_f32), f64::from(0.4_f32)])
            );
            // LivingEntity.SLEEPING_DIMENSIONS: the fixed 0.2 box, scale off.
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Sleeping),
                Some([f64::from(0.2_f32), f64::from(0.2_f32), f64::from(0.2_f32)])
            );
            // The stances a player never takes answer standing, as
            // Avatar.getDefaultDimensions' fallback does.
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Roaring),
                Some([f64::from(0.6_f32), f64::from(1.8_f32), f64::from(1.62_f32)])
            );
            assert!(game.living_set_attribute_base(living, "minecraft:scale", 2.0));
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Dying),
                Some([f64::from(0.2_f32), f64::from(0.2_f32), f64::from(1.62_f32)]),
                "Avatar.POSES dying dimensions remain fixed even at scale 2"
            );
            game.lan.as_mut().unwrap().hub.guests.clear();
            assert_eq!(
                living.dimensions_for_pose(game, Pose::Sleeping),
                None,
                "a gone player must not acquire sleeping dimensions"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn camel_sitting_is_lower_even_while_standing() {
        with_game(|game| {
            floor_and_zombies(game);
            let ty = game.res.mob_types().get("minecraft:camel").unwrap().clone();
            game.mobs
                .push(Mob::new(20, ty, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let camel = game.living(game.entity(20).unwrap()).unwrap();
            let standing = camel.dimensions_for_pose(game, Pose::Standing).unwrap();
            // Camel.getDefaultDimensions: the sitting box 1.43 lower, its eyes
            // at 0.845 (`sitting_dims`), asked of a camel that stands.
            let sitting = camel.dimensions_for_pose(game, Pose::Sitting).unwrap();
            assert_eq!(
                sitting,
                [
                    f64::from(1.7_f32),
                    f64::from(horizon_client::mobs::camel::sitting_dims(false).1),
                    f64::from(horizon_client::mobs::camel::sitting_dims(false).2)
                ]
            );
            assert!(standing[1] > sitting[1]);
            // A camel that sits is in the sitting pose, so its own box is the
            // sitting one — but the standing pose still answers the standing
            // box, as `getDefaultDimensions` reads the pose asked for.
            let mob = game.mobs.iter_mut().find(|m| m.id == 20).unwrap();
            mob.extra.camel_sitting = true;
            let own = mob.dims();
            assert_eq!(own, (sitting[0], sitting[1], sitting[2]));
            let still_standing = camel.dimensions_for_pose(game, Pose::Standing).unwrap();
            assert_eq!(still_standing, standing);
            game.mobs.retain(|m| m.id != 20);
            assert_eq!(camel.dimensions_for_pose(game, Pose::Sitting), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn goat_long_jump_scales_the_box_by_seven_tenths() {
        with_game(|game| {
            floor_and_zombies(game);
            let ty = game.res.mob_types().get("minecraft:goat").unwrap().clone();
            game.mobs
                .push(Mob::new(21, ty, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let goat = game.living(game.entity(21).unwrap()).unwrap();
            let standing = goat.dimensions_for_pose(game, Pose::Standing).unwrap();
            let long_jump = goat.dimensions_for_pose(game, Pose::LongJumping).unwrap();
            for (asked, is) in standing.iter().zip(&long_jump) {
                assert!(
                    (is - asked * f64::from(0.7_f32)).abs() < 1.0e-6,
                    "{is} is seven tenths of {asked}"
                );
            }
            // The pose the goat is in (`Entity.setPose` of a long jump) gives
            // the same box through `dims`.
            let mob = game.mobs.iter_mut().find(|m| m.id == 21).unwrap();
            mob.extra.pose = horizon_client::mobs::sync::LONG_JUMPING;
            assert_eq!(mob.dims(), (long_jump[0], long_jump[1], long_jump[2]));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn warden_digging_answers_low_in_every_pose() {
        with_game(|game| {
            floor_and_zombies(game);
            let ty = game
                .res
                .mob_types()
                .get("minecraft:warden")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(22, ty, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let warden = game.living(game.entity(22).unwrap()).unwrap();
            let mob = game.mobs.iter_mut().find(|m| m.id == 22).unwrap();
            horizon_client::mobs::sync::set_pose(mob, horizon_client::mobs::sync::DIGGING);
            // Warden.getDefaultDimensions reads its own state, not the pose
            // asked for: a metre tall, its eyes at 0.85, however asked —
            // except the fixed 0.2 box every pose sleeps in.
            for pose in [Pose::Standing, Pose::LongJumping, Pose::Sitting] {
                let dims = warden.dimensions_for_pose(game, pose).unwrap();
                assert_eq!(dims[1], 1.0, "a metre tall in {pose:?}");
                assert_eq!(dims[2], f64::from(0.85_f32), "the eyes at 0.85 in {pose:?}");
            }
            assert_eq!(
                warden.dimensions_for_pose(game, Pose::Sleeping).unwrap(),
                [f64::from(0.2_f32), f64::from(0.2_f32), f64::from(0.2_f32)]
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn villager_sleeping_answers_the_sleeping_box() {
        with_game(|game| {
            floor_and_zombies(game);
            let ty = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(23, ty, Vec3::new(0.5, 64.0, 0.5), 0.0));
            let villager = game.living(game.entity(23).unwrap()).unwrap();
            let standing = villager.dimensions_for_pose(game, Pose::Standing).unwrap();
            let mob = game.mobs.iter_mut().find(|m| m.id == 23).unwrap();
            mob.extra.ints[1] = 1;
            let own = mob.dims();
            // LivingEntity.SLEEPING_DIMENSIONS, and the pose the villager is
            // in while it sleeps (`LivingEntity.startSleeping`).
            let sleeping = villager.dimensions_for_pose(game, Pose::Sleeping).unwrap();
            assert_eq!(
                sleeping,
                [f64::from(0.2_f32), f64::from(0.2_f32), f64::from(0.2_f32)]
            );
            assert_eq!(own, (sleeping[0], sleeping[1], sleeping[2]));
            // Asked for the standing pose, the sleeping villager answers its
            // standing box (the eye height that lays it in the bed).
            assert_eq!(
                villager.dimensions_for_pose(game, Pose::Standing).unwrap(),
                standing
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn no_action_time_and_pathfinding_malus_reach_the_mob() {
        with_game(|game| {
            floor_and_zombies(game);
            let mob = game.mob(game.entity(12).unwrap()).unwrap();
            assert_eq!(mob.no_action_time(game), Some(0));
            assert!(mob.set_no_action_time(game, 650));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 12)
                    .unwrap()
                    .ai
                    .no_action_time,
                650
            );
            assert_eq!(mob.no_action_time(game), Some(650));

            // `noActionTime` is a Java `int`: the tick past `i32::MAX` wraps
            // instead of panicking. Kept so it cannot despawn meanwhile.
            assert!(mob.set_persistence_required(game));
            assert!(mob.set_no_action_time(game, i32::MAX));
            game.tick_mobs().unwrap();
            let after = mob.no_action_time(game).unwrap();
            assert!(after < 0, "wrapped past i32::MAX, got {after}");
            assert!(mob.set_no_action_time(game, 650));

            assert_eq!(mob.pathfinding_malus(game, PathType::Water), Some(8.0));
            assert!(mob.set_pathfinding_malus(game, PathType::Water, -1.0));
            assert_eq!(mob.pathfinding_malus(game, PathType::Water), Some(-1.0));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|m| m.id == 12)
                    .unwrap()
                    .malus(PathType::Water),
                -1.0,
                "the pathfinder reads the same override"
            );
            assert_eq!(mob.pathfinding_malus(game, PathType::Lava), Some(-1.0));
            assert!(!mob.set_pathfinding_malus(game, PathType::Water, f32::NAN));

            game.mobs.retain(|m| m.id != 12);
            assert_eq!(mob.no_action_time(game), None);
            assert!(!mob.set_no_action_time(game, 1));
            assert_eq!(mob.pathfinding_malus(game, PathType::Water), None);
        });
    }
}
