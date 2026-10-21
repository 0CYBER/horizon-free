//! Tridents: `TridentItem.releaseUsing` (throw after 10 ticks, or Riptide in
//! water or rain) and `ThrownTrident` (arrow flight with 0.99 water inertia,
//! 8 damage plus Impaling, the reversed bounce off a hit, sticking, Loyalty's
//! return to the owner, pickup) with `ThrownTridentRenderer`.
use super::{Game, view_vector};
use horizon_client::{
    collisions::SceneCollisions,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Damage, DamageKind, Who, anim::Poses},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};

pub(super) struct ThrownTrident {
    /// The entity id a server tracks it by (0 until it is tracked).
    pub id: u32,
    pub(super) stack: ItemStack,
    pub(super) position: Vec3,
    previous: Vec3,
    pub(super) velocity: Vec3,
    pub(super) yaw: f32,
    pub(super) pitch: f32,
    pub(super) in_ground: bool,
    in_ground_time: u32,
    dealt_damage: bool,
    pub(super) loyalty: u32,
    pub(super) no_physics: bool,
    returning: u32,
    creative_only: bool,
}
impl ThrownTrident {
    /// A trident a server sent (`handleAddEntity`): placed and moved by it.
    pub(super) fn remote(id: u32, position: Vec3, yaw: f32, pitch: f32) -> Self {
        Self {
            id,
            stack: ItemStack::new("minecraft:trident", 1),
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            pitch,
            in_ground: false,
            in_ground_time: 0,
            dealt_damage: false,
            loyalty: 0,
            no_physics: false,
            returning: 0,
            creative_only: false,
        }
    }
    /// One tick of a server's client: the previous position, then `step`.
    pub(super) fn remote_step(&mut self, step: impl FnOnce(Vec3) -> Vec3) {
        self.previous = self.position;
        self.position = step(self.position);
    }
}
const HALF: f64 = 0.25;
fn bounds(p: Vec3) -> Aabb {
    Aabb::new(
        Vec3::new(p.x - HALF, p.y, p.z - HALF),
        Vec3::new(p.x + HALF, p.y + 2.0 * HALF, p.z + HALF),
    )
}
/// The entry distance of a segment into a box, if it crosses it.
fn segment_box(from: Vec3, d: Vec3, b: Aabb) -> Option<f64> {
    let (lo, hi) = (b.min_position(), b.max_position());
    let (o, d, lo, hi) = (
        [from.x, from.y, from.z],
        [d.x, d.y, d.z],
        [lo.x, lo.y, lo.z],
        [hi.x, hi.y, hi.z],
    );
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for a in 0..3 {
        if d[a].abs() < 1e-12 {
            if o[a] < lo[a] || o[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (mut ta, mut tb) = ((lo[a] - o[a]) / d[a], (hi[a] - o[a]) / d[a]);
        if ta > tb {
            std::mem::swap(&mut ta, &mut tb);
        }
        t0 = t0.max(ta);
        t1 = t1.min(tb);
    }
    (t0 <= t1).then_some(t0)
}
impl Game<'_> {
    /// `TridentItem.use`: start the throw unless one more use would break it.
    pub(super) fn use_trident(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:trident" {
            return false;
        }
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(250, |i| i.max_damage);
        let riptide = stack.enchantment_level("minecraft:riptide") > 0;
        if stack.damage + 1 >= max || (riptide && !self.in_water_or_rain()) {
            return true;
        }
        if self.using.is_none() {
            self.using = Some((self.held_slot(), 0));
        }
        true
    }
    fn in_water_or_rain(&self) -> bool {
        let Some(p) = &self.player else { return false };
        let (x, y, z) = (
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        );
        let water = self.res.facts().get(self.world.block(x, y, z)).fluid
            == horizon_client::blocks::Fluid::Water;
        let sky = self.world.light(x, y + 1, z) >> 4 == 15;
        water || (self.world.weather.raining && sky)
    }
    /// The living entities a spinning player can touch this tick, for
    /// `LivingEntity.checkAutoSpinAttack` (none when not spinning).
    pub(super) fn spin_targets(&self) -> Vec<(u32, Aabb)> {
        let Some(p) = self
            .player
            .as_ref()
            .filter(|p| p.auto_spin_attack_ticks > 0)
        else {
            return Vec::new();
        };
        // A spin moves a few blocks a tick at most.
        let near = p.bounds().inflate(Vec3::new(16.0, 16.0, 16.0));
        self.mobs
            .iter()
            .filter(|m| m.alive() && m.bounds().intersects(near))
            .map(|m| (m.id, m.bounds()))
            .collect()
    }
    /// `Player.doAutoAttackOnTouch`: the touched entity takes `Player.attack`
    /// with the spin's damage; once the spin is over its damage is cleared.
    pub(super) fn end_spin_attack(&mut self, hit: Option<u32>) {
        if let Some(id) = hit
            && let Some(index) = self.mobs.iter().position(|m| m.id == id)
        {
            self.attack_mob(index);
        }
        if let Some(p) = self.player.as_mut()
            && p.auto_spin_attack_ticks <= 0
        {
            p.auto_spin_attack_dmg = 0.0;
        }
    }
    /// `TridentItem.releaseUsing`.
    pub(super) fn release_trident(&mut self, ticks: u32) {
        if ticks < 10 {
            return;
        }
        let Some(stack) = self.held().cloned() else {
            return;
        };
        let Some(eye) = self.eye_position() else {
            return;
        };
        let riptide = stack.enchantment_level("minecraft:riptide");
        let at = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        if riptide > 0 {
            if !self.in_water_or_rain() {
                return;
            }
            self.damage_held(1);
            // trident_spin_attack_strength: 1.5 + 0.75 per level above the first.
            let strength = 1.5 + 0.75 * (riptide as f32 - 1.0);
            let d = view_vector(self.yaw, self.pitch);
            if let Some(p) = self.player.as_mut() {
                p.velocity = Vec3::new(
                    p.velocity.x + d[0] * f64::from(strength),
                    p.velocity.y + d[1] * f64::from(strength),
                    p.velocity.z + d[2] * f64::from(strength),
                );
                // Player.startAutoSpinAttack(20, 8, stack).
                p.auto_spin_attack_ticks = 20;
                p.auto_spin_attack_dmg = 8.0;
                if p.ground.on_ground {
                    p.position.y += 1.2;
                }
            }
            let sound = format!("minecraft:item.trident.riptide_{}", riptide.min(3));
            self.audio.play(&sound, at, 1.0, 1.0);
            return;
        }
        // hurtWithoutBreaking, then the thrown copy.
        if !self.creative
            && let Some(s) = self.held_mut().as_mut()
        {
            s.damage += 1;
        }
        let thrown = self.held().cloned().unwrap_or(stack);
        if !self.creative {
            *self.held_mut() = None;
        }
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut projectile = horizon_client::projectiles::Projectile::new(
            horizon_client::projectiles::ProjectileKind::Snowball,
            start,
            Some(Who::Player),
        );
        let (v, on_ground) = self
            .player
            .as_ref()
            .map_or((Vec3::ZERO, true), |p| (p.velocity, p.ground.on_ground));
        projectile.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            2.5,
            1.0,
            v,
            on_ground,
        );
        self.tridents.push(ThrownTrident {
            id: 0,
            loyalty: thrown.enchantment_level("minecraft:loyalty"),
            stack: thrown,
            position: start,
            previous: start,
            velocity: projectile.velocity,
            yaw: projectile.yaw,
            pitch: projectile.pitch,
            in_ground: false,
            in_ground_time: 0,
            dealt_damage: false,
            no_physics: false,
            returning: 0,
            creative_only: self.creative,
        });
        self.player_projectile_shot();
        self.audio
            .play("minecraft:item.trident.throw", at, 1.0, 1.0);
    }
    pub(super) fn tick_tridents(&mut self) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        let owner_eye = Vec3::new(eye[0], eye[1], eye[2]);
        let feet = self.player.as_ref().map_or(owner_eye, |p| p.position);
        let mut i = 0;
        while i < self.tridents.len() {
            let mut t = self.tridents.remove(i);
            t.previous = t.position;
            if t.in_ground_time > 4 {
                t.dealt_damage = true;
            }
            if t.loyalty > 0 && (t.dealt_damage || t.no_physics) {
                t.no_physics = true;
                t.in_ground = false;
                let v = Vec3::new(
                    owner_eye.x - t.position.x,
                    owner_eye.y - t.position.y,
                    owner_eye.z - t.position.z,
                );
                t.position.y += v.y * 0.015 * f64::from(t.loyalty);
                let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1e-9);
                let accel = 0.05 * f64::from(t.loyalty);
                t.velocity = Vec3::new(
                    t.velocity.x * 0.95 + v.x / len * accel,
                    t.velocity.y * 0.95 + v.y / len * accel,
                    t.velocity.z * 0.95 + v.z / len * accel,
                );
                if t.returning == 0 {
                    self.audio.play(
                        "minecraft:item.trident.return",
                        Some([t.position.x, t.position.y, t.position.z]),
                        10.0,
                        1.0,
                    );
                }
                t.returning += 1;
            }
            if t.in_ground {
                t.in_ground_time += 1;
            } else if t.no_physics {
                t.position = Vec3::new(
                    t.position.x + t.velocity.x,
                    t.position.y + t.velocity.y,
                    t.position.z + t.velocity.z,
                );
            } else {
                // Mobs on the path, then blocks.
                let mut hit_mob = None;
                if !t.dealt_damage {
                    for m in self.mobs.iter().filter(|m| m.alive()) {
                        if let Some(d) = segment_box(t.position, t.velocity, m.bounds())
                            && hit_mob.is_none_or(|(_, best)| d < best)
                        {
                            hit_mob = Some((m.id, d));
                        }
                    }
                }
                if let Some((id, _)) = hit_mob {
                    let impaling = t.stack.enchantment_level("minecraft:impaling") as f32;
                    let aquatic = self.mobs.iter().find(|m| m.id == id).is_some_and(|m| {
                        m.kind.flags.aquatic_swimmer || m.kind.flags.breathes_water
                    });
                    let damage = 8.0 + if aquatic { 2.5 * impaling } else { 0.0 };
                    t.dealt_damage = true;
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        let from = Some([t.position.x, t.position.y, t.position.z]);
                        m.damage(
                            damage,
                            Damage {
                                kind: DamageKind::Arrow,
                                attacker: Some(Who::Player),
                                from,
                            },
                        );
                    }
                    self.audio.play(
                        "minecraft:item.trident.hit",
                        Some([t.position.x, t.position.y, t.position.z]),
                        1.0,
                        1.0,
                    );
                    // ProjectileDeflection.REVERSE scaled by (0.02, 0.2, 0.02).
                    t.velocity = Vec3::new(
                        -t.velocity.x * 0.02,
                        -t.velocity.y * 0.2,
                        -t.velocity.z * 0.02,
                    );
                } else {
                    let mut scene = SceneCollisions {
                        world: &self.world,
                        binding: self.collision,
                        subject: None,
                    };
                    if let Ok(moved) = scene.resolve(bounds(t.position), t.velocity, 0.0, false) {
                        let blocked = (moved.x - t.velocity.x).abs() > 1e-7
                            || (moved.y - t.velocity.y).abs() > 1e-7
                            || (moved.z - t.velocity.z).abs() > 1e-7;
                        t.position = Vec3::new(
                            t.position.x + moved.x,
                            t.position.y + moved.y,
                            t.position.z + moved.z,
                        );
                        if blocked {
                            t.in_ground = true;
                            t.velocity = Vec3::ZERO;
                            self.audio.play(
                                "minecraft:item.trident.hit_ground",
                                Some([t.position.x, t.position.y, t.position.z]),
                                1.0,
                                1.0,
                            );
                        }
                    }
                }
                if !t.in_ground {
                    // A trident's water inertia (0.99) equals the air drag; gravity 0.05.
                    let drag = 0.99;
                    t.velocity = Vec3::new(
                        t.velocity.x * drag,
                        t.velocity.y * drag - 0.05,
                        t.velocity.z * drag,
                    );
                    let h = (t.velocity.x * t.velocity.x + t.velocity.z * t.velocity.z).sqrt();
                    t.yaw = t.velocity.x.atan2(t.velocity.z).to_degrees() as f32;
                    t.pitch = t.velocity.y.atan2(h).to_degrees() as f32;
                }
            }
            // playerTouch: stuck or returning tridents go back to the player.
            let near = (t.position.x - feet.x).abs() < 1.0
                && (t.position.z - feet.z).abs() < 1.0
                && t.position.y > feet.y - 1.0
                && t.position.y < feet.y + 2.5;
            if near && (t.in_ground || t.no_physics) {
                if !t.creative_only {
                    let stack = t.stack.clone();
                    if let Some(rest) = self.give(stack) {
                        self.spawn_item(rest, [feet.x, feet.y, feet.z], [0.0; 3], 0);
                    }
                }
                let pitch =
                    ((self.world.random.next_float() - self.world.random.next_float()) * 0.7 + 1.0)
                        * 2.0;
                self.audio.play(
                    "minecraft:entity.item.pickup",
                    Some([feet.x, feet.y, feet.z]),
                    0.2,
                    pitch,
                );
                continue;
            }
            self.tridents.insert(i, t);
            i += 1;
        }
    }
    /// ThrownTridentRenderer.
    pub(super) fn trident_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let Some(model) = &self.trident_model else {
            return;
        };
        let Some(uv) = self
            .gui
            .atlas
            .sprites
            .get("entity/trident/trident")
            .map(|r| [r.u0, r.v0, r.u1, r.v1])
        else {
            return;
        };
        // AbstractArrow.shouldRenderAtSqrDistance: ten times the default.
        let range = 10.0
            * super::entity_culling::default_range(
                glam::DVec3::splat(2.0 * HALF),
                self.settings.entity_distance,
            );
        for t in &self.tridents {
            // EntityRenderer.shouldRender; ThrownTridentRenderer grows the box by 1.5.
            if !self.standing_entity_in_view(
                camera,
                super::entity_culling::at(t.previous),
                super::entity_culling::at(t.position),
                alpha,
                2.0 * HALF,
                2.0 * HALF,
                range,
                1.5,
            ) {
                continue;
            }
            let p = glam::DVec3::new(
                t.previous.x + (t.position.x - t.previous.x) * alpha,
                t.previous.y + (t.position.y - t.previous.y) * alpha,
                t.previous.z + (t.position.z - t.previous.z) * alpha,
            ) - camera;
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((t.yaw - 90.0).to_radians())
                * glam::Mat4::from_rotation_z((t.pitch + 90.0).to_radians());
            let l = self.world.light(
                t.position.x.floor() as i32,
                t.position.y.floor() as i32,
                t.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            let poses = Poses::new(model);
            horizon_client::mobs::render::draw(
                model,
                base,
                &poses,
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
        }
    }
}
