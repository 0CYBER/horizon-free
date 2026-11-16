use super::*;
use horizon_core::position::BlockPos;

impl Game<'_> {
    /// ServerGamePacketListenerImpl.handlePlayerCommand START_RIDING_JUMP: the
    /// vehicle the guest rides takes its own `handleStartJump` — a horse rears
    /// and slides on the rider's keys, a camel dashes — and the sound vanilla's
    /// server plays is broadcast to the players near the mount, the rider
    /// included (their own client plays none).
    pub(super) fn lan_start_riding_jump(&mut self, conn: u32, data: i32) {
        let index = self.as_guest(conn, |g| g.mount_index());
        let Some(Some(index)) = index else {
            return;
        };
        let m = &mut self.mobs[index];
        let camel = crate::mobs::camel::is_camel(m.species()).then_some(m.id);
        let Some((sound, volume)) = crate::game::mounts::start_riding_jump(m, data) else {
            return;
        };
        let at = m.center();
        // LivingEntity.getSoundSource: a mob's sounds are in the neutral source.
        // `Entity.playSound`: at the mount's position, not bound to it.
        let name = format!("minecraft:{sound}");
        self.send_level_sound(
            LevelSound::new(&name, at, volume, 1.0).in_source(SoundSource::Neutral),
            None,
        );
        if let Some(id) = camel {
            self.post_entity_event(
                "minecraft:entity_action",
                horizon_client::mobs::Who::Mob(id),
            );
        }
    }

    /// ServerGamePacketListenerImpl.handlePlayerCommand START_FALL_FLYING:
    /// `Player.tryToStartFallFlying`, or else `stopFallFlying` (a second
    /// start while gliding ends the glide, as in vanilla).
    pub(super) fn lan_start_gliding(&mut self, conn: u32) {
        let Some(g) = self.lan.as_ref().and_then(|host| host.hub.guest(conn)) else {
            return;
        };
        let start = !g.fall_flying && self.guest_can_glide(conn) == Some(true);
        let in_liquid = self
            .as_guest(conn, |game| {
                game.player
                    .as_ref()
                    .is_some_and(|p| p.fluids.was_touching_water || p.is_in_lava())
            })
            .unwrap_or(true);
        let mut changed = None;
        if let Some(host) = self.lan.as_mut() {
            let _ = host.hub.update_guest(conn, |g| {
                let gliding = start && !in_liquid;
                if g.fall_flying != gliding {
                    g.fall_flying = gliding;
                    changed = Some((g.uuid, g.name.clone(), gliding));
                } else if !gliding {
                    g.sent_shared ^= -128;
                }
            });
        }
        if let Some((uuid, name, gliding)) = changed {
            self.native_entity_toggle_glide(uuid, &name, gliding);
        }
    }

    /// `Player.canGlide`: not flying, airborne, not riding, no levitation, and
    /// a glider that its next point of wear won't break.
    pub(super) fn guest_can_glide(&mut self, conn: u32) -> Option<bool> {
        let g = self.lan.as_ref()?.hub.guest(conn)?;
        if g.abilities.flying || g.on_ground || g.vehicle.is_some() {
            return Some(false);
        }
        self.as_guest(conn, |game| {
            game.glider_worn() && game.potions.effects.get("levitation").is_none()
        })
    }

    /// `LivingEntity.updateFallFlying` on the server, once a tick for a
    /// gliding guest: the glide ends once `canGlide` fails; otherwise every
    /// 20th tick wears the glider by one, and every 10th is an
    /// `elytra_glide` game event. Entity.checkFallDistanceAccumulation runs
    /// first: a glide that isn't diving keeps the fall distance at 1.
    pub(in crate::game) fn guest_glide_tick(&mut self, conn: u32, fell: f64) {
        let Some(creative) = self.lan.as_mut().and_then(|host| {
            host.hub
                .update_guest(conn, |g| {
                    if !g.fall_flying {
                        if let Some(m) = g.motion.as_mut() {
                            m.fall_fly_ticks = 0;
                        }
                        return None;
                    }
                    let creative = g.creative();
                    if let Some(m) = g.motion.as_mut()
                        && fell > -0.5
                        && m.fall_distance > 1.0
                    {
                        m.fall_distance = 1.0;
                    }
                    Some(creative)
                })
                .flatten()
        }) else {
            return;
        };
        if self.guest_can_glide(conn) != Some(true) {
            if let Some(host) = self.lan.as_mut() {
                let _ = host.hub.update_guest(conn, |g| g.end_glide());
            }
            return;
        }
        self.as_guest(conn, |game| {
            let Some(p) = game.player.as_mut() else {
                return;
            };
            // updateFallFlying: last tick's count plus one; LivingEntity.tick
            // counts this tick after aiStep.
            let next = p.fall_fly_ticks + 1;
            p.fall_fly_ticks += 1;
            let at = [p.position.x, p.position.y, p.position.z];
            if next % 10 != 0 {
                return;
            }
            if (next / 10) % 2 == 0 && !creative {
                game.wear_glider(1);
            }
            let ctx = game.player_context(None);
            game.post_game_event("minecraft:elytra_glide", at, ctx);
        });
    }

    /// `Entity.isInWall`: a suffocating block in the box at the eyes as wide
    /// as 0.8 of the body (0.48 at scale 1, 0.048 at scale 0.1). Suffocating
    /// is a full collision box that is also a full render cube
    /// (`BlockBehaviour.isSuffocating`'s default; glass and the like opt
    /// out), as the mobs' check has it. A block sharing volume with one of
    /// guest `conn`'s collision exclusions is no wall (arismoko/horizon#3717).
    pub(super) fn in_wall(&self, conn: u32, at: [f64; 3], body: Dimensions) -> bool {
        let y = (at[1] + f64::from(body.eye_height)).floor() as i32;
        let [xs, zs] = eye_columns(at, body);
        let guest = self.lan.as_ref().and_then(|host| host.hub.guest(conn));
        let spared = |x: i32, z: i32| {
            guest.is_some_and(|g| g.collision_exclusions.spares(BlockPos::new(x, y, z)))
        };
        xs.into_iter().any(|x| {
            zs.clone().any(|z| {
                let f = self.res.facts().get(self.world.block(x, y, z));
                f.full_collision && f.solid_render && !spared(x, z)
            })
        })
    }
}

#[cfg(test)]
#[path = "tests/collision_exclusion_wall_tests.rs"]
mod collision_exclusion_wall_tests;
