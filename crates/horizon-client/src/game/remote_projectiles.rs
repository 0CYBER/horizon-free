//! Primed TNT, falling blocks, projectiles, tridents, firework rockets and
//! end crystals on a server's client (`ClientPacketListener.handleAddEntity`
//! and `handleSetEntityData`): the server simulates them, the client draws
//! them where it says and interpolates their moves.
use super::{FallStep, FallingBlock, Game};
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_protocol::entity_data::Value;
use horizon_world::registry::BlockStateId;

/// The projectile a type id names, as Horizon models it.
fn projectile_kind(kind: &str) -> Option<ProjectileKind> {
    Some(match kind {
        "minecraft:arrow" | "minecraft:spectral_arrow" => ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        "minecraft:snowball" => ProjectileKind::Snowball,
        "minecraft:egg" => ProjectileKind::Egg,
        "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
        "minecraft:small_fireball" => ProjectileKind::SmallFireball,
        "minecraft:fireball" => ProjectileKind::LargeFireball { power: 1 },
        "minecraft:llama_spit" => ProjectileKind::LlamaSpit,
        "minecraft:wind_charge" => ProjectileKind::WindCharge,
        "minecraft:breeze_wind_charge" => ProjectileKind::BreezeWindCharge,
        // Homing state stays on the server; the client interpolates its positions.
        "minecraft:shulker_bullet" => ProjectileKind::ShulkerBullet {
            target: horizon_client::mobs::Who::Player,
            dir: None,
            steps: 0,
            delta: [0.0; 3],
            seed: 0,
        },
        "minecraft:wither_skull" => ProjectileKind::WitherSkull { dangerous: false },
        _ => return None,
    })
}

impl Game<'_> {
    /// `handleAddEntity` for these types; false when `kind` isn't one.
    pub(super) fn spawn_remote_projectile(
        &mut self,
        id: i32,
        kind: &str,
        position: [f64; 3],
        rotation: (f32, f32),
        velocity: [f64; 3],
        data: i32,
    ) -> bool {
        let id = id as u32;
        let at = Vec3::new(position[0], position[1], position[2]);
        let velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
        let (yaw, pitch) = rotation;
        self.remove_remote_projectile(id);
        match kind {
            "minecraft:tnt" => self.tnts.push(super::tnt::PrimedTnt {
                id,
                position: at,
                previous: at,
                velocity,
                fuse: 80,
                on_ground: false,
            }),
            // FallingBlockEntity.recreateFromPacket: the block state is the data.
            "minecraft:falling_block" => self.falling.push(FallingBlock {
                id,
                state: BlockStateId(data.max(0) as u32),
                start_pos: BlockPos::default(),
                position: at,
                previous: at,
                velocity,
                age: 0,
                no_gravity: false,
                drop_item: false,
                fall_distance: 0.0,
                stuck: Vec3::ZERO,
                damage: Default::default(),
                saved: None,
            }),
            "minecraft:trident" => {
                let mut t = super::trident::ThrownTrident::remote(id, at, yaw, pitch);
                t.velocity = velocity;
                self.tridents.push(t);
            }
            "minecraft:firework_rocket" => {
                self.rockets
                    .push(super::fireworks::Rocket::remote(id, at, velocity));
            }
            "minecraft:splash_potion"
            | "minecraft:lingering_potion"
            | "minecraft:experience_bottle" => {
                self.potions.add_remote_thrown(id, kind, at, velocity);
            }
            "minecraft:eye_of_ender" => {
                self.end
                    .add_remote_eye(id, position, [velocity.x, velocity.y, velocity.z])
            }
            // FishingHook.recreateFromPacket: the owner's id is the data.
            "minecraft:fishing_bobber" => self.spawn_remote_hook(id, at, data),
            "minecraft:end_crystal" => self.crystals.push(super::dragon::Crystal {
                id,
                position: at,
                show_bottom: true,
                time: 0,
                beam: None,
                invulnerable: false,
            }),
            _ => {
                let Some(k) = projectile_kind(kind) else {
                    return false;
                };
                let mut p = Projectile::new(k, at, None);
                p.id = id;
                p.velocity = velocity;
                p.yaw = yaw;
                p.pitch = pitch;
                self.creatures.projectiles.push(p);
            }
        }
        true
    }

    /// `handleSetEntityData` for these types: what changes how they look.
    pub(super) fn remote_projectile_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let id = id as u32;
        self.remote_hook_data(id, values);
        if self.potions.has_thrown(id) {
            // ThrowableItemProjectile.DATA_ITEM_STACK: the potion it is.
            for (index, _, value) in values {
                if let (8, Value::Item(stack)) = (index, value)
                    && let Some(stack) = self.to_stack(stack)
                {
                    self.potions.remote_thrown_item(id, stack);
                }
            }
            return;
        }
        for (index, _, value) in values {
            if let Some(t) = self.tnts.iter_mut().find(|t| t.id == id) {
                // PrimedTnt.DATA_FUSE_ID.
                if let (8, Value::Int(fuse)) = (index, value) {
                    t.fuse = *fuse;
                }
            } else if let Some(f) = self.falling.iter_mut().find(|f| f.id == id) {
                // FallingBlockEntity.DATA_START_POS (the model's seed).
                if let (8, Value::Pos(p)) = (index, value) {
                    f.start_pos = BlockPos::new(p[0], p[1], p[2]);
                }
            } else if let Some(c) = self.crystals.iter_mut().find(|c| c.id == id) {
                // EndCrystal.DATA_BEAM_TARGET and DATA_SHOW_BOTTOM.
                match (index, value) {
                    (8, Value::OptPos(beam)) => c.beam = *beam,
                    (9, Value::Bool(show)) => c.show_bottom = *show,
                    _ => {}
                }
            } else if let Some(p) = self.creatures.projectiles.iter_mut().find(|p| p.id == id) {
                match (&mut p.kind, index, value) {
                    // AbstractArrow.ID_FLAGS (critical) and IN_GROUND.
                    (ProjectileKind::Arrow { critical, .. }, 8, Value::Byte(flags)) => {
                        *critical = flags & 1 != 0;
                    }
                    (ProjectileKind::Arrow { .. }, 10, Value::Bool(in_ground)) => {
                        p.stuck = *in_ground;
                    }
                    // WitherSkull.DATA_DANGEROUS.
                    (ProjectileKind::WitherSkull { dangerous }, 8, Value::Bool(d)) => {
                        *dangerous = *d;
                    }
                    _ => {}
                }
            }
        }
    }

    /// A move's rotation for a projectile or trident.
    pub(super) fn turn_remote_projectile(&mut self, id: i32, rotation: Option<(f32, f32)>) {
        let (id, Some((yaw, pitch))) = (id as u32, rotation) else {
            return;
        };
        if let Some(p) = self.creatures.projectiles.iter_mut().find(|p| p.id == id) {
            p.yaw = yaw;
            p.pitch = pitch;
        } else if let Some(t) = self.tridents.iter_mut().find(|t| t.id == id) {
            t.yaw = yaw;
            t.pitch = pitch;
        }
    }

    /// One tick of interpolation, as `tick_remote_objects`.
    pub(super) fn tick_remote_projectiles(&mut self, mut step: impl FnMut(u32, Vec3) -> Vec3) {
        for t in &mut self.tnts {
            t.previous = t.position;
            t.position = step(t.id, t.position);
        }
        // FallingBlockEntity.tick runs on a client too: the block falls by
        // itself between the server's updates (every 20 ticks), then waits
        // where it lands for the server to place it and remove it.
        let mut falling = std::mem::take(&mut self.falling);
        for f in &mut falling {
            if let Ok(FallStep::Landed(_)) = self.fall_step(f, false) {
                // Block.updateEntityMovementAfterFallOn, then the air drag.
                f.velocity = Vec3::new(f.velocity.x * 0.98, 0.0, f.velocity.z * 0.98);
            }
        }
        self.falling = falling;
        for p in &mut self.creatures.projectiles {
            p.previous = p.position;
            p.age += 1;
            if matches!(p.kind, ProjectileKind::LargeFireball { .. }) {
                // AbstractHurtingProjectile.tick advances on the client between
                // the tracker updates (ten ticks apart for a ghast's fireball).
                let at = p.position;
                let water = self
                    .res
                    .facts()
                    .get(self.world.block(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ))
                    .fluid
                    == horizon_client::blocks::Fluid::Water;
                p.apply_hurting_inertia(water);
                p.position = p.position.add_vector(p.velocity);
            } else {
                p.position = step(p.id, p.position);
            }
        }
        for t in &mut self.tridents {
            let id = t.id;
            t.remote_step(|at| step(id, at));
        }
        for r in &mut self.rockets {
            let id = r.id;
            r.remote_step(|at| step(id, at));
        }
        for c in &mut self.crystals {
            c.time += 1;
            c.position = step(c.id, c.position);
        }
        self.potions.step_remote_thrown(&mut step);
        self.step_remote_hooks(&mut step);
        self.end.step_remote_eyes(|id, at| {
            let at = step(id, Vec3::new(at[0], at[1], at[2]));
            [at.x, at.y, at.z]
        });
    }

    pub(super) fn remove_remote_projectile(&mut self, id: u32) {
        self.tnts.retain(|t| t.id != id);
        self.falling.retain(|f| f.id != id);
        self.creatures.projectiles.retain(|p| p.id != id);
        self.tridents.retain(|t| t.id != id);
        self.rockets.retain(|r| r.id != id);
        self.crystals.retain(|c| c.id != id);
        self.potions.remove_remote_thrown(id);
        self.end.remove_remote_eye(id);
        self.remove_remote_hook(id);
    }

    /// Every one a server sent (a new connection, level or dimension).
    pub(super) fn clear_remote_projectiles(&mut self) {
        self.tnts.clear();
        self.falling.clear();
        self.creatures.projectiles.clear();
        self.tridents.clear();
        self.rockets.clear();
        self.crystals.clear();
        self.potions.clear_thrown();
        self.end.clear_remote_eyes();
        self.clear_remote_hooks();
    }

    /// Their kind and place, for `status`.
    pub(super) fn projectiles_status(&self) -> Vec<serde_json::Value> {
        use serde_json::json;
        let at = |p: Vec3| [p.x, p.y, p.z];
        let mut out = Vec::new();
        for t in &self.tnts {
            out.push(json!({"id": t.id, "kind": "minecraft:tnt", "position": at(t.position), "fuse": t.fuse}));
        }
        for f in &self.falling {
            out.push(json!({"id": f.id, "kind": "minecraft:falling_block", "position": at(f.position), "state": f.state.0}));
        }
        for c in &self.crystals {
            out.push(json!({"id": c.id, "kind": "minecraft:end_crystal", "position": at(c.position), "show_bottom": c.show_bottom}));
        }
        for t in &self.tridents {
            out.push(json!({"id": t.id, "kind": "minecraft:trident", "position": at(t.position)}));
        }
        for p in &self.creatures.projectiles {
            out.push(json!({"id": p.id, "kind": format!("{:?}", p.kind).split([' ', '{']).next().unwrap_or(""), "position": at(p.position)}));
        }
        for (kind, p, _) in self.potions.thrown_entities() {
            out.push(json!({"kind": kind, "position": at(p)}));
        }
        for (id, p) in self.end.eye_positions() {
            out.push(json!({"id": id, "kind": "minecraft:eye_of_ender", "position": p}));
        }
        out.extend(self.hooks_status());
        out
    }
}
