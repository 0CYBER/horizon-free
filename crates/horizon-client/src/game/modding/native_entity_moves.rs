//! Moving and turning an entity for native server mods, through an
//! `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387):
//! teleport, rotation, head yaw, facing, size and pushes. Writes move the
//! entity as `/tp` and `/rotate` do, so clients see it as they see those.
//! Reads return `None`, writes false, for a stale handle or on a client.
use super::entity::{Entity as _, EntityHandle, with_entity_mut};
use super::{
    Dim, Game,
    commands::{Target, entity_pitch},
    vehicle_teleport::TeleportMotion,
};
use horizon_core::geometry::Vec3;
use mod_api::EntityRef;

/// `Level.isInSpawnableBounds` and `/tp`'s limits: 30,000,000 blocks out,
/// 20,000,000 up or down.
pub(super) fn in_bounds([x, y, z]: [f64; 3]) -> bool {
    (-30_000_000.0..30_000_000.0).contains(&x)
        && (-30_000_000.0..30_000_000.0).contains(&z)
        && (-20_000_000.0..20_000_000.0).contains(&y)
}

/// `Direction.fromYRot`: the horizontal direction a yaw faces.
pub(super) fn facing(yaw: f32) -> &'static str {
    const BY_2D: [&str; 4] = ["south", "west", "north", "east"];
    BY_2D[((f64::from(yaw) / 90.0 + 0.5).floor() as i32 & 3) as usize]
}

impl Game<'_> {
    /// Moves the entity to `pos` in its level (`Entity.teleportTo`), as
    /// `/tp` does: a player is placed there and told, a rider gets off. With
    /// `rotation` (yaw, pitch in degrees) it also turns. False for a position
    /// outside the world's bounds, for wall-mounted entities or dragon parts, or when the
    /// requested rotation cannot be stored.
    pub(super) fn entity_teleport_to(
        &mut self,
        entity: EntityRef,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        let Some(t) = self.teleport_target(entity, pos, rotation) else {
            return false;
        };
        self.teleport_to(t, pos, rotation, true);
        self.teleport_stops(t);
        true
    }
    /// Sends the entity to `pos` in the level `dimension` (`Entity.teleport`
    /// with a portal's transition, `TeleportMotion::Keep`): the level opens if
    /// it isn't, the entity keeps its motion, a player arrives after the
    /// current tick, a vehicle takes its passengers along. In the entity's own
    /// level it is `entity_teleport_to` with the motion kept. False for what
    /// `entity_teleport_to` refuses, a dimension that isn't registered, and
    /// kinds that cannot change level here (`teleport_target_across`).
    pub(super) fn entity_teleport_to_level(
        &mut self,
        entity: EntityRef,
        dimension: &str,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        let Some(t) = self.teleport_target(entity, pos, rotation) else {
            return false;
        };
        if !Dim::exists(dimension) {
            return false;
        }
        self.teleport_to_level(t, Dim::from_id(dimension), pos, rotation)
    }
    /// A mod sends `t` to `pos` in `level` (`Entity.teleport` with a
    /// `TeleportTransition` that keeps the motion, `TeleportMotion::Keep`): in
    /// its own level when that is `level`, else as `/tp` into another level
    /// does. False for a kind that only moves within its level (armor stands,
    /// falling blocks, ...; `teleport_target_across`), a player who is not a
    /// server guest, or a transfer the game could not make. A fake player
    /// stays, and the call is true.
    pub(in crate::game) fn teleport_to_level(
        &mut self,
        t: Target,
        level: Dim,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> bool {
        if self.target_level(t) == level {
            let motion = self.transition_motion(t);
            self.teleport_to(t, pos, rotation, true);
            if let Some(motion) = motion {
                self.restore_transition_motion(t, motion);
            }
            return true;
        }
        self.teleport_across(t, level, pos, rotation, true, TeleportMotion::Keep)
            .unwrap_or(false)
    }
    /// What a teleport of `entity` to `pos` moves, when `/tp` could place it there.
    fn teleport_target(
        &self,
        entity: EntityRef,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
    ) -> Option<Target> {
        let t = self.native_entity_target(entity)?;
        if !in_bounds(pos)
            || matches!(
                t,
                Target::Frame(_) | Target::Painting(_) | Target::DragonPart(_)
            )
            || rotation.is_some_and(|r| {
                !r.iter().all(|v| v.is_finite())
                    || !matches!(
                        t,
                        Target::Player | Target::Guest(_) | Target::Mob(_) | Target::Other(_)
                    )
            })
        {
            return None;
        }
        Some(t)
    }
    /// Turns the entity to `yaw` and `pitch` in degrees (`Entity.setYRot`,
    /// `setXRot`): the pitch wraps and is clamped to -90..90. False for kinds
    /// whose rotation Horizon does not fully store, or an unavailable player.
    /// Does not teleport or dismount.
    pub(super) fn entity_set_rotation(&mut self, entity: EntityRef, yaw: f32, pitch: f32) -> bool {
        let Some(t) = self.native_entity_target(entity) else {
            return false;
        };
        if !yaw.is_finite() || !pitch.is_finite() {
            return false;
        }
        let pitch = entity_pitch(pitch);
        match t {
            Target::Player | Target::Guest(_) => {
                self.for_player(t, |g| {
                    if g.player.is_none() {
                        return false;
                    }
                    g.yaw = yaw;
                    g.pitch = pitch;
                    true
                }) == Some(true)
            }
            Target::Mob(id) => {
                let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
                    return false;
                };
                m.yaw = yaw;
                m.head_pitch = pitch;
                m.needs_sync = true;
                true
            }
            Target::Other(id) => {
                let Some(i) = self.others.iter().position(|o| o.id == id) else {
                    return false;
                };
                self.others.update(i, |o| {
                    o.yaw = yaw;
                    o.pitch = pitch;
                });
                true
            }
            _ => false,
        }
    }
    /// The yaw of the entity's head (`Entity.getYHeadRot`): a mob's or a
    /// player's own, zero for the kinds without a head.
    pub(super) fn entity_head_yaw(&self, entity: EntityRef) -> Option<f32> {
        let t = self.native_entity_target(entity)?;
        if let Some(g) = self.native_target_guest(t) {
            return Some(g.head_yaw);
        }
        if let Target::Mob(id) = t {
            return Some(self.mobs.iter().find(|m| m.id == id)?.head_yaw);
        }
        Some(0.0)
    }
    /// The yaw of a mob's body (`LivingEntity.yBodyRot`), which its model turns
    /// with and its head turns against; `entity_head_yaw` reads the head. None
    /// for the other kinds, whose body yaw Horizon doesn't keep.
    pub(super) fn entity_body_yaw(&self, entity: EntityRef) -> Option<f32> {
        let Target::Mob(id) = self.native_entity_target(entity)? else {
            return None;
        };
        Some(self.mobs.iter().find(|m| m.id == id)?.body_yaw)
    }
    /// Turns a mob's head to `yaw` (`Entity.setYHeadRot`); its body follows
    /// as its AI turns it. Players' heads can turn too; false for other kinds
    /// or an unavailable player.
    pub(super) fn entity_set_head_yaw(&mut self, entity: EntityRef, yaw: f32) -> bool {
        if !yaw.is_finite() {
            return false;
        }
        match self.native_entity_target(entity) {
            Some(Target::Mob(id)) => match self.mobs.iter_mut().find(|m| m.id == id) {
                Some(m) => {
                    m.head_yaw = yaw;
                    m.needs_sync = true;
                    true
                }
                None => false,
            },
            Some(t @ (Target::Player | Target::Guest(_))) => {
                let conn = match t {
                    Target::Player => self.lan_acting,
                    Target::Guest(conn) => Some(conn),
                    _ => None,
                };
                conn.and_then(|conn| {
                    self.lan
                        .as_mut()?
                        .hub
                        .update_guest(conn, |g| g.head_yaw = yaw)
                })
                .is_some()
            }
            _ => false,
        }
    }
    /// The horizontal direction the entity faces (`Entity.getDirection`):
    /// `north`, `south`, `east` or `west`.
    pub(super) fn entity_direction(&self, entity: EntityRef) -> Option<&'static str> {
        let t = self.native_entity_target(entity)?;
        Some(facing(self.target_rotation(t)[0]))
    }
    /// The entity's width and height (`Entity.getBbWidth`, `getBbHeight`):
    /// its box's, so a baby's or a scaled entity's own.
    pub(super) fn entity_dimensions(&self, entity: EntityRef) -> Option<[f64; 2]> {
        let [min, max] = self.entity_bounding_box(entity)?;
        Some([max[0] - min[0], max[1] - min[1]])
    }
    /// Adds `delta` to the entity's velocity (`Entity.push`,
    /// `addDeltaMovement`). False for an unavailable player, an entity
    /// Horizon doesn't simulate, or a value that isn't finite.
    pub(super) fn entity_push(&mut self, entity: EntityRef, delta: [f64; 3]) -> bool {
        if !delta.iter().all(|v| v.is_finite()) {
            return false;
        }
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        if matches!(target, Target::Player | Target::Guest(_)) {
            let conn = match target {
                Target::Player => self.lan_acting,
                Target::Guest(conn) => Some(conn),
                _ => None,
            };
            let Some(conn) = conn else {
                return false;
            };
            let Some(host) = self.lan.as_mut() else {
                return false;
            };
            return host
                .hub
                .update_guest(conn, |g| {
                    let motion = if matches!(target, Target::Player) {
                        self.player.as_mut()
                    } else {
                        g.motion.as_deref_mut()
                    };
                    let Some(motion) = motion else {
                        return false;
                    };
                    let v = motion.velocity;
                    motion.velocity = Vec3::new(v.x + delta[0], v.y + delta[1], v.z + delta[2]);
                    let v = motion.velocity;
                    g.send(
                        &horizon_protocol::packets::play::Clientbound::SetEntityMotion(
                            horizon_protocol::packets::play::SetEntityMotion {
                                id: g.entity_id,
                                velocity: [v.x, v.y, v.z],
                            },
                        ),
                    );
                    true
                })
                .unwrap_or(false);
        }
        let Some(h) = self.native_walk_entity(entity.id()) else {
            return false;
        };
        let [x, y, z] = delta;
        with_entity_mut!(self, h, e => {
            let v = e.velocity_mut();
            *v = Vec3::new(v.x + x, v.y + y, v.z + z);
        });
        if let EntityHandle::Mob(i) = h {
            self.mobs[i].needs_sync = true;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn moves_and_pushes_a_live_mob() {
        use super::super::native_entity_refs::tests::with_game;
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;

        with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut mob = Mob::new(12, kind, Vec3::ZERO, 0.0);
            mob.ai.nav.path = Some(horizon_client::mobs::path::Path {
                nodes: Vec::new(),
                next: 0,
                target: [1, 64, 1],
                dist_to_target: 0.0,
                reached: true,
            });
            mob.extra.mount = Some(42);
            mob.extra.passenger = true;
            mob.fall_distance = 5.0;
            mob.velocity.y = -2.0;
            game.mobs.push(mob);
            let entity = game.entity(12).unwrap();
            assert!(entity.teleport_to(game, [5.0, 70.0, -3.0], Some([90.0, 120.0])));
            let mob = &game.mobs[0];
            assert!(mob.ai.nav.path.is_none());
            assert!(mob.extra.mount.is_none());
            assert!(!mob.extra.passenger);
            assert_eq!(mob.fall_distance, 5.0);
            assert_eq!(mob.velocity.y, 0.0);
            assert_eq!(entity.position(game), Some([5.0, 70.0, -3.0]));
            assert_eq!(entity.rotation(game), Some([90.0, 90.0]));
            assert_eq!(entity.head_yaw(game), Some(90.0));
            game.mobs[0].body_yaw = 33.0;
            assert_eq!(entity.body_yaw(game), Some(33.0));
            assert_eq!(entity.head_yaw(game), Some(90.0));
            let before = entity.velocity(game).unwrap();
            assert!(entity.set_rotation(game, 15.0, 360.0));
            assert_eq!(entity.rotation(game), Some([15.0, 0.0]));
            assert_eq!(entity.head_yaw(game), Some(90.0));
            assert_eq!(entity.position(game), Some([5.0, 70.0, -3.0]));
            assert_eq!(entity.velocity(game), Some(before));
            assert!(entity.teleport_to(game, [5.0, 70.0, -3.0], Some([0.0, 450.0])));
            assert_eq!(entity.rotation(game), Some([0.0, 90.0]));
            assert!(entity.push(game, [1.0, 2.0, -1.0]));
            assert_eq!(
                entity.velocity(game),
                Some([before[0] + 1.0, before[1] + 2.0, before[2] - 1.0])
            );
            assert!(!entity.teleport_to(game, [f64::NAN, 0.0, 0.0], None));
            assert!(!entity.push(game, [0.0, f64::INFINITY, 0.0]));
            game.armor_stands
                .push(horizon_client::entities::ArmorStand::new(
                    13,
                    Vec3::ZERO,
                    0.0,
                ));
            let stand = game.entity(13).unwrap();
            assert!(!stand.set_rotation(game, 30.0, 10.0));
            game.others.push(super::super::commands::OtherEntity {
                attachment_lifetime: Default::default(),
                knot_check: None,
                eye: None,
                join: Default::default(),
                item_transformation: None,
                interaction: None,
                id: 14,
                kind: "minecraft:marker".into(),
                position: Vec3::ZERO,
                yaw: 60.0,
                pitch: 0.0,
            });
            let marker = game.entity(14).unwrap();
            assert_eq!(marker.head_yaw(game), Some(0.0));
            assert!(!marker.set_head_yaw(game, 10.0));
            game.paintings.push(horizon_client::entities::Painting {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 15,
                pos: [0, 64, 0],
                facing: 0,
                variant: "minecraft:kebab".into(),
                width: 1,
                height: 1,
                check_interval: 0,
            });
            let painting = game.entity(15).unwrap();
            assert!(!painting.teleport_to(game, [1.0, 64.0, 0.0], None));
            assert!(!painting.set_rotation(game, 30.0, 10.0));
        });
    }

    #[test]
    fn a_yaw_faces_vanillas_direction() {
        assert_eq!(super::facing(0.0), "south");
        assert_eq!(super::facing(90.0), "west");
        assert_eq!(super::facing(180.0), "north");
        assert_eq!(super::facing(-90.0), "east");
        assert_eq!(super::facing(-45.0), "south");
        assert_eq!(super::facing(-46.0), "east");
        assert_eq!(super::facing(720.0 + 170.0), "north");
    }

    #[test]
    fn positions_stay_inside_the_worlds_bounds() {
        assert!(super::in_bounds([29_999_999.0, -64.0, 0.0]));
        assert!(super::in_bounds([
            -30_000_000.0,
            -20_000_000.0,
            -30_000_000.0
        ]));
        assert!(!super::in_bounds([30_000_000.0, 64.0, 0.0]));
        assert!(!super::in_bounds([0.0, 20_000_001.0, 0.0]));
        assert!(!super::in_bounds([f64::NAN, 64.0, 0.0]));
    }
    /// Independent vanilla outcomes for spawn bounds and the yaw-to-direction conversion.
    #[test]
    fn move_boundaries_match_vanilla() {
        use serde_json::Value;
        let cases = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/native_entity_moves.jsonl"
        ));
        let expected = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/native_entity_moves_expected.jsonl"
        ));
        let rows = |text: &str| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>()
        };
        let (cases, expected) = (rows(cases), rows(expected));
        assert_eq!(cases.len(), expected.len());
        assert!(cases.len() >= 20);
        for (input, expected) in cases.iter().zip(&expected) {
            assert_eq!(input["name"], expected["name"]);
            let pos = std::array::from_fn(|i| input["pos"][i].as_f64().unwrap());
            let yaw = input["yaw"].as_f64().unwrap() as f32;
            assert_eq!(
                super::in_bounds(pos),
                expected["spawnable"].as_bool().unwrap(),
                "{}",
                input["name"]
            );
            assert_eq!(
                super::facing(yaw),
                expected["direction"].as_str().unwrap(),
                "{}",
                input["name"]
            );
        }
    }
}
