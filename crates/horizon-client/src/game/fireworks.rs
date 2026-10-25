//! Firework rockets (`FireworkRocketItem`, `FireworkRocketEntity`): launching
//! from a block face, boosting a gliding player, the 1.15 horizontal
//! acceleration and 0.04 lift, the flight-duration lifetime, explosion
//! damage with line of sight, and the client explosion (`createFireworks`).
use super::{Game, Hit, item_draw::owner_at, view_vector};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    block_entity_render::vertex_light,
    collisions::SceneCollisions,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Damage, DamageKind},
    particles::{FireworkExplosion, ParticleOptions, ParticleType},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

pub(super) struct Rocket {
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub(super) position: Vec3,
    previous: Vec3,
    pub(super) velocity: Vec3,
    life: i32,
    lifetime: i32,
    pub(super) explosions: Vec<FireworkExplosion>,
    /// Boosting the player (attachedToEntity).
    pub(super) attached: bool,
    /// On a server, the player it boosts when not this game's own: a guest's
    /// connection.
    pub(super) guest: Option<u32>,
    /// `DATA_ATTACHED_TO_TARGET`: the server entity the rocket follows;
    /// boosting happens when it is the local player.
    pub(super) attached_to: Option<u32>,
    /// `DATA_SHOT_AT_ANGLE` (dispensers): constant velocity, exploding on
    /// the first block it hits.
    pub(super) angled: bool,
    /// `DATA_ID_FIREWORKS_ITEM`: the stack it was launched from.
    pub(super) item: ItemStack,
}
impl Rocket {
    /// A rocket a server sent (`handleAddEntity`): placed and moved by it.
    pub(super) fn remote(id: u32, position: Vec3, velocity: Vec3) -> Self {
        let item = ItemStack::new("minecraft:firework_rocket", 1);
        let (_, explosions) = fireworks_of(&item);
        Self {
            id,
            position,
            previous: position,
            velocity,
            life: 0,
            lifetime: i32::MAX,
            explosions,
            attached: false,
            guest: None,
            attached_to: None,
            angled: false,
            item,
        }
    }
    /// One tick of a server's client: the previous position, then `step`.
    pub(super) fn remote_step(&mut self, step: impl FnOnce(Vec3) -> Vec3) {
        self.previous = self.position;
        self.life += 1;
        self.position = step(self.position);
    }
    /// Position and motion (the dispenser parity fixture).
    pub(super) fn motion(&self) -> (Vec3, Vec3) {
        (self.position, self.velocity)
    }
}
/// `damageAmount * (float)Math.sqrt((5.0 - this.distanceTo(target)) / 5.0)`,
/// `distanceTo` being a float.
fn blast_damage(damage: f32, distance_sqr: f64) -> f32 {
    let distance = distance_sqr.sqrt() as f32;
    damage * ((5.0 - f64::from(distance)) / 5.0).sqrt() as f32
}
pub(super) fn fireworks_of(stack: &ItemStack) -> (u8, Vec<FireworkExplosion>) {
    // The item's default component: flight 1, no explosions.
    stack
        .data()
        .and_then(|d| d.fireworks.clone())
        .unwrap_or((1, Vec::new()))
}
impl Game<'_> {
    pub(super) fn new_rocket(&mut self, at: Vec3, stack: &ItemStack, attached: bool) -> Rocket {
        let (flight, explosions) = fireworks_of(stack);
        // FireworkRocketEntity: motion and lifetime come from the rocket's own random.
        let r = &mut super::dispenser::entity_random();
        let tri = |r: &mut horizon_core::random::LegacyRandom| r.triangle(0.0, 0.002297);
        let velocity = Vec3::new(tri(r), 0.05, tri(r));
        let lifetime = 10 * (1 + i32::from(flight))
            + r.next_int_bound(6).unwrap_or(0)
            + r.next_int_bound(7).unwrap_or(0);
        Rocket {
            id: 0,
            position: at,
            previous: at,
            velocity,
            life: 0,
            lifetime,
            explosions,
            attached,
            guest: None,
            attached_to: None,
            angled: false,
            item: stack.clone(),
        }
    }
    /// `FireworkRocketItem.useOn`: launch from the clicked point.
    pub(super) fn launch_firework(&mut self, hit: &Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:firework_rocket" {
            return false;
        }
        let at = Vec3::new(
            hit.point[0] + f64::from(hit.face[0]) * 0.15,
            hit.point[1] + f64::from(hit.face[1]) * 0.15,
            hit.point[2] + f64::from(hit.face[2]) * 0.15,
        );
        let rocket = self.new_rocket(at, stack, false);
        self.rockets.push(rocket);
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    /// A rocket shot at an angle (`DATA_SHOT_AT_ANGLE`): out of a dispenser
    /// (`FireworkRocketItem.asProjectile`) or a crossbow.
    pub(super) fn launch_angled_rocket(&mut self, at: Vec3, velocity: Vec3, stack: &ItemStack) {
        let mut rocket = self.new_rocket(at, stack, false);
        rocket.velocity = velocity;
        rocket.angled = true;
        self.rockets.push(rocket);
    }
    /// `FireworkRocketItem.use` while gliding: a rocket attached to the player.
    pub(super) fn boost_firework(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:firework_rocket"
            || !self.player.as_ref().is_some_and(|p| p.fall_flying)
        {
            return false;
        }
        let at = self.player.as_ref().map_or(Vec3::ZERO, |p| p.position);
        let mut rocket = self.new_rocket(at, stack, true);
        rocket.guest = self.lan_acting;
        self.rockets.push(rocket);
        if !self.creative {
            self.consume_held();
        }
        self.swing_arm();
        true
    }
    pub(super) fn tick_rockets(&mut self) {
        let mut i = 0;
        while i < self.rockets.len() {
            let mut r = self.rockets.remove(i);
            r.previous = r.position;
            if r.attached
                && let Some(conn) = r.guest
            {
                // A guest's rocket follows them; their own client boosts them.
                let guest = self
                    .lan
                    .as_ref()
                    .and_then(|host| host.hub.guests.iter().find(|g| g.conn == conn));
                if let Some(g) = guest {
                    r.position = Vec3::new(g.position[0], g.position[1], g.position[2]);
                }
            } else if r.attached {
                if let Some(p) = self.player.as_mut() {
                    if p.fall_flying {
                        let look = view_vector(self.yaw, self.pitch);
                        let m = p.velocity;
                        p.velocity = Vec3::new(
                            m.x + look[0] * 0.1 + (look[0] * 1.5 - m.x) * 0.5,
                            m.y + look[1] * 0.1 + (look[1] * 1.5 - m.y) * 0.5,
                            m.z + look[2] * 0.1 + (look[2] * 1.5 - m.z) * 0.5,
                        );
                    }
                    r.position = p.position;
                    r.velocity = p.velocity;
                }
            } else {
                // Not shot at an angle: accelerate (unless blocked sideways) and rise.
                let mut scene = SceneCollisions {
                    world: &self.world,
                    binding: self.collision,
                    subject: None,
                };
                let h = 0.125;
                let bounds = Aabb::new(
                    Vec3::new(r.position.x - h, r.position.y, r.position.z - h),
                    Vec3::new(r.position.x + h, r.position.y + 2.0 * h, r.position.z + h),
                );
                let wanted = if r.angled {
                    r.velocity
                } else {
                    Vec3::new(
                        r.velocity.x * 1.15,
                        r.velocity.y + 0.04,
                        r.velocity.z * 1.15,
                    )
                };
                if let Ok(moved) = scene.resolve(bounds, wanted, 0.0, false) {
                    let blocked = (moved.x - wanted.x).abs() > 1e-7
                        || (moved.y - wanted.y).abs() > 1e-7
                        || (moved.z - wanted.z).abs() > 1e-7;
                    if r.angled && blocked {
                        // hitTargetOrDeflectSelf: explode on the block.
                        r.life = r.lifetime;
                    }
                    let horizontal =
                        (moved.x - wanted.x).abs() > 1e-7 || (moved.z - wanted.z).abs() > 1e-7;
                    r.position = Vec3::new(
                        r.position.x + moved.x,
                        r.position.y + moved.y,
                        r.position.z + moved.z,
                    );
                    r.velocity = if horizontal {
                        Vec3::new(r.velocity.x, wanted.y, r.velocity.z)
                    } else {
                        wanted
                    };
                }
            }
            let at = [r.position.x, r.position.y, r.position.z];
            if r.life == 0 {
                self.audio.play(
                    "minecraft:entity.firework_rocket.launch",
                    Some(at),
                    3.0,
                    1.0,
                );
            }
            r.life += 1;
            // The trail: a spark each tick drifting against the rocket's motion.
            let g = |g: &mut horizon_core::random::LegacyRandom| g.next_gaussian() * 0.05;
            let v = [
                g(&mut self.world.random),
                -r.velocity.y * 0.5,
                g(&mut self.world.random),
            ];
            self.particles
                .add(&ParticleOptions::simple(ParticleType::Firework), at, v);
            if r.life > r.lifetime {
                self.explode_rocket(&r);
                continue;
            }
            self.rockets.insert(i, r);
            i += 1;
        }
    }
    /// `FireworkRocketEntity.explode` and `dealExplosionDamage`.
    fn explode_rocket(&mut self, r: &Rocket) {
        let at = [r.position.x, r.position.y, r.position.z];
        if !r.explosions.is_empty() {
            let damage = 5.0 + r.explosions.len() as f32 * 2.0;
            let from = r.position;
            // `attachedToEntity` takes the full damage, a guest as well; every
            // other player in range and in sight takes the blast.
            let boosted = r.guest.filter(|_| r.attached);
            if r.attached && boosted.is_none() {
                self.rocket_hurt_player(damage);
            } else {
                self.rocket_blast_player(from, damage);
            }
            if let Some(conn) = boosted
                && let Some(mut host) = self.lan.take()
            {
                self.as_guest(&mut host, conn, |g| g.rocket_hurt_player(damage));
                self.lan = Some(host);
            }
            // Every other player in the level (LAN guests, the players of a
            // server thread), each swapped in as the player.
            self.each_guest_hit_except(boosted, |g| g.rocket_blast_player(from, damage));
            let hits: Vec<(u32, f32)> = {
                let world = self.mob_world_base(&[]);
                self.mobs
                    .iter()
                    .filter(|m| m.alive())
                    .filter_map(|m| {
                        let d2 = (m.position.x - from.x).powi(2)
                            + (m.position.y - from.y).powi(2)
                            + (m.position.z - from.z).powi(2);
                        // Line of sight to the feet or the middle of the mob.
                        let sees = d2 <= 25.0
                            && (0..2).any(|step| {
                                let to = Vec3::new(
                                    m.position.x,
                                    m.position.y + f64::from(m.height()) * 0.5 * f64::from(step),
                                    m.position.z,
                                );
                                world.clear_between(from, to, false)
                            });
                        sees.then(|| (m.id, blast_damage(damage, d2)))
                    })
                    .collect()
            };
            for (id, amount) in hits {
                if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                    m.damage(
                        amount,
                        Damage {
                            kind: DamageKind::Explosion,
                            attacker: None,
                            from: Some(at),
                        },
                    );
                }
            }
        }
        let camera = self.eye_position().unwrap_or(at);
        let v = [r.velocity.x, r.velocity.y, r.velocity.z];
        self.particles
            .fireworks(at, v, r.explosions.clone(), camera);
    }
    /// `dealExplosionDamage` on a player that is not the rocket's
    /// `attachedToEntity`: within 5 blocks, when the rocket sees its feet or
    /// its middle (`ClipContext.Block.COLLIDER`, no fluids).
    fn rocket_blast_player(&mut self, from: Vec3, damage: f32) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let feet = p.position;
        let d2 = (feet.x - from.x).powi(2) + (feet.y - from.y).powi(2) + (feet.z - from.z).powi(2);
        if d2 > 25.0 {
            return;
        }
        let height = f64::from(p.dimensions.height);
        let sees = {
            let world = self.mob_world_base(&[]);
            (0..2).any(|step| {
                let to = Vec3::new(feet.x, feet.y + height * 0.5 * f64::from(step), feet.z);
                world.clear_between(from, to, false)
            })
        };
        if sees {
            self.rocket_hurt_player(blast_damage(damage, d2));
        }
    }
    /// `hurtServer` with `damageSources().fireworks`: armor absorbs it and
    /// wears (`hurtArmor`); creative and spectator players are immune.
    fn rocket_hurt_player(&mut self, amount: f32) {
        if self.creative || self.spectator() {
            return;
        }
        let hurt = if let Some(p) = self.player.as_mut()
            && !p.health.dead_or_dying()
            && let Some(id) = self.motion.health.damage.id("minecraft:fireworks")
        {
            p.hurt(&self.motion.health, self.world.health_rules, id, amount)
                .unwrap_or(false)
        } else {
            false
        };
        if hurt {
            self.hurt_armor(amount);
        }
    }
    /// `FireworkEntityRenderer.submit`: the item (GROUND) facing the camera,
    /// lit at the rocket's light probe.
    pub(super) fn rocket_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let orientation = glam::Mat4::from_quat(self.camera_orientation());
        for r in self.rockets.iter().filter(|r| !r.attached) {
            // FireworkRocketEntity.shouldRenderAtSqrDistance: within 64 blocks;
            // EntityRenderer.shouldRender: its 0.25 box in the view.
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(r.previous),
                super::entity_culling::at(r.position),
                alpha,
                0.25,
                0.25,
                64.0,
                0.0,
            ) {
                continue;
            }
            let p = glam::DVec3::new(
                r.previous.x + (r.position.x - r.previous.x) * alpha,
                r.previous.y + (r.position.y - r.previous.y) * alpha,
                r.previous.z + (r.position.z - r.previous.z) * alpha,
            );
            // `updateForNonLiving`: GROUND, owned by the rocket.
            let owner = owner_at(p.to_array(), 0.0, "minecraft:firework_rocket");
            let state = self.resolve_item(&r.item, DisplayContext::Ground, Some(&owner), 0);
            if state.is_empty() {
                continue;
            }
            let mut pose = glam::Mat4::from_translation((p - camera).as_vec3()) * orientation;
            if r.angled {
                pose *= glam::Mat4::from_rotation_z(std::f32::consts::PI)
                    * glam::Mat4::from_rotation_y(std::f32::consts::PI)
                    * glam::Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
            }
            // getLightProbePosition: the eye, 0.85 of its 0.25 height.
            let light = self.world.light(
                p.x.floor() as i32,
                (p.y + 0.2125).floor() as i32,
                p.z.floor() as i32,
            );
            self.submit_item_world(&state, pose, vertex_light(light), out);
        }
    }
}
