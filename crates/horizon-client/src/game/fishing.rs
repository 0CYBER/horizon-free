//! Fishing: `FishingRodItem.use` (cast / reel in), the `FishingHook` (flight,
//! bobbing, lure and bite timers, open water, hooking mobs, retrieving the
//! `gameplay/fishing` loot) and `FishingHookRenderer` (bobber and line).
use super::{Game, view_vector};
use horizon_client::{
    blocks::Fluid,
    collisions::SceneCollisions,
    gizmos::Line,
    items::ItemStack,
    mesher::Vertex,
    particles::{ParticleOptions, ParticleType},
    pose::Pose,
};
use horizon_core::{
    geometry::{Aabb, Vec3, trig},
    random::RandomSource,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Flying,
    Bobbing,
    Hooked(u32),
}
pub(super) struct Hook {
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    state: State,
    life: i32,
    on_ground: bool,
    nibble: i32,
    time_until_lured: i32,
    time_until_hooked: i32,
    fish_angle: f32,
    out_of_water: i32,
    open_water: bool,
    luck: i32,
    lure: i32,
}
const SIZE: f64 = 0.25;
fn bounds(p: Vec3) -> Aabb {
    let h = SIZE / 2.0;
    Aabb::new(
        Vec3::new(p.x - h, p.y, p.z - h),
        Vec3::new(p.x + h, p.y + SIZE, p.z + h),
    )
}
/// RandomSource.triangle(mode, deviation).
fn triangle(r: &mut impl RandomSource, mode: f64, deviation: f64) -> f64 {
    mode + deviation * (f64::from(r.next_float()) - f64::from(r.next_float()))
}
impl Game<'_> {
    fn holds_rod(&self) -> bool {
        self.held()
            .is_some_and(|s| &*s.item == "minecraft:fishing_rod")
            || self
                .offhand
                .as_ref()
                .is_some_and(|s| &*s.item == "minecraft:fishing_rod")
    }
    /// `FishingRodItem.use`.
    pub(super) fn use_fishing_rod(&mut self, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:fishing_rod" {
            return false;
        }
        let Some(p) = self.player.as_ref() else {
            return false;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let pitch = 0.4 / (self.world.random.next_float() * 0.4 + 0.8);
        if self.hook.is_some() {
            let damage = self.retrieve_hook();
            self.damage_held(damage);
            self.audio.play(
                "minecraft:entity.fishing_bobber.retrieve",
                Some(at),
                1.0,
                pitch,
            );
        } else {
            self.audio.play(
                "minecraft:entity.fishing_bobber.throw",
                Some(at),
                0.5,
                pitch,
            );
            // FishingHook(player, level, luck, lureSpeed)
            let lure = stack.enchantment_level("minecraft:lure") as i32 * 5 * 20;
            let luck = stack.enchantment_level("minecraft:luck_of_the_sea") as i32;
            let (yaw, pitch) = (self.yaw, self.pitch);
            let y_cos = (-yaw.to_radians() - std::f32::consts::PI).cos();
            let y_sin = (-yaw.to_radians() - std::f32::consts::PI).sin();
            let x_cos = -(-pitch.to_radians()).cos();
            let x_sin = (-pitch.to_radians()).sin();
            let eye_y = p.position.y + f64::from(p.dimensions.eye_height);
            let start = Vec3::new(
                p.position.x - f64::from(y_sin) * 0.3,
                eye_y,
                p.position.z - f64::from(y_cos) * 0.3,
            );
            let mut v = Vec3::new(
                f64::from(-y_sin),
                f64::from((-(x_sin / x_cos)).clamp(-5.0, 5.0)),
                f64::from(-y_cos),
            );
            let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
            let r = &mut self.world.random;
            v = Vec3::new(
                v.x * (0.6 / len + triangle(r, 0.5, 0.0103365)),
                v.y * (0.6 / len + triangle(r, 0.5, 0.0103365)),
                v.z * (0.6 / len + triangle(r, 0.5, 0.0103365)),
            );
            // A spawned hook's start position is its feet: centre the box on the eye.
            let start = Vec3::new(start.x, start.y - SIZE / 2.0, start.z);
            self.hook = Some(Hook {
                position: start,
                previous: start,
                velocity: v,
                state: State::Flying,
                life: 0,
                on_ground: false,
                nibble: 0,
                time_until_lured: 0,
                time_until_hooked: 0,
                fish_angle: 0.0,
                out_of_water: 0,
                open_water: true,
                luck,
                lure,
            });
        }
        self.swing_arm();
        true
    }
    /// `FishingHook.retrieve`: the rod damage it costs.
    fn retrieve_hook(&mut self) -> u32 {
        let Some(hook) = self.hook.take() else {
            return 0;
        };
        let Some(p) = self.player.as_ref() else {
            return 0;
        };
        let owner = p.position;
        let mut damage = 0;
        if let State::Hooked(id) = hook.state {
            // pullEntity: 0.1 of the way to the owner.
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                let d = Vec3::new(
                    owner.x - hook.position.x,
                    owner.y - hook.position.y,
                    owner.z - hook.position.z,
                );
                m.velocity = Vec3::new(
                    m.velocity.x + d.x * 0.1,
                    m.velocity.y + d.y * 0.1,
                    m.velocity.z + d.z * 0.1,
                );
            }
            damage = 5;
        } else if hook.nibble > 0 {
            let biome = self.biome_name_at(hook.position);
            let Some(loot) = horizon_client::structure_blocks::loot_data(&self.options.resources)
            else {
                return damage;
            };
            let mut random = self.world.random.clone();
            let items = loot.roll(
                "minecraft:gameplay/fishing",
                &mut random,
                hook.luck as f32,
                biome.as_deref(),
                Some(hook.open_water),
            );
            self.world.random = random;
            // FishingHook.retrieve: fish_caught for a catch.
            self.stats.custom("fish_caught", 1);
            for stack in items {
                let mut item = ItemStack {
                    item: Arc::from(stack.item.as_str()),
                    count: stack.count.max(1) as u32,
                    damage: stack.damage.max(0) as u32,
                    extra: None,
                };
                for (id, level) in &stack.enchantments {
                    if &*item.item == "minecraft:enchanted_book" {
                        horizon_client::item_data::set_level(
                            &mut item.data_mut().stored_enchantments,
                            id,
                            (*level).max(0) as u32,
                        );
                    } else {
                        item.enchant(id, (*level).max(0) as u32);
                    }
                }
                if let Some(potion) = &stack.potion {
                    item = item.with_potion(potion);
                }
                let (dx, dy, dz) = (
                    owner.x - hook.position.x,
                    owner.y - hook.position.y,
                    owner.z - hook.position.z,
                );
                let v = [
                    dx * 0.1,
                    dy * 0.1 + (dx * dx + dy * dy + dz * dz).sqrt().sqrt() * 0.08,
                    dz * 0.1,
                ];
                self.spawn_item(
                    item,
                    [hook.position.x, hook.position.y, hook.position.z],
                    v,
                    10,
                );
                let xp = self.world.random.next_int_bound(6).unwrap_or(0) as u32 + 1;
                self.spawn_orbs([owner.x, owner.y + 0.5, owner.z + 0.5], xp);
            }
            damage = 1;
        }
        if hook.on_ground {
            damage = 2;
        }
        damage
    }
    fn biome_name_at(&self, p: Vec3) -> Option<String> {
        let (x, y, z) = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let c = self.world.chunks.get(&(x >> 4, z >> 4))?;
        Some(self.res.biomes.list[c.biome(x, y, z) as usize].name.clone())
    }
    fn water_height(&self, x: i32, y: i32, z: i32) -> Option<(f32, bool)> {
        let f = self.res.facts.get(self.world.block(x, y, z));
        if f.fluid != Fluid::Water {
            return None;
        }
        let above = self.res.facts.get(self.world.block(x, y + 1, z));
        let h = if above.fluid == Fluid::Water {
            1.0
        } else {
            f32::from(f.fluid_level) / 9.0
        };
        Some((h, f.fluid_level == 8 && !f.fluid_falling))
    }
    /// getOpenWaterTypeForBlock: 0 above water, 1 inside water, 2 invalid.
    fn open_water_type(&self, x: i32, y: i32, z: i32) -> u8 {
        let f = self.res.facts.get(self.world.block(x, y, z));
        if self.world.is_air(self.world.block(x, y, z)) || &*f.name == "minecraft:lily_pad" {
            return 0;
        }
        if f.fluid == Fluid::Water
            && f.fluid_level == 8
            && !f.fluid_falling
            && !f.full_collision
            && !f.legacy_solid
        {
            1
        } else {
            2
        }
    }
    /// calculateOpenWater over the 5x4x5 area around the hook.
    fn calculate_open_water(&self, b: [i32; 3]) -> bool {
        let mut previous = 2u8;
        for dy in -1..=2 {
            let mut layer = None;
            for dx in -2..=2 {
                for dz in -2..=2 {
                    let t = self.open_water_type(b[0] + dx, b[1] + dy, b[2] + dz);
                    layer = Some(match layer {
                        None => t,
                        Some(l) if l == t => l,
                        Some(_) => 2,
                    });
                }
            }
            let layer = layer.unwrap_or(2);
            match layer {
                0 if previous == 2 => return false,
                1 if previous == 0 => return false,
                2 => return false,
                _ => {}
            }
            previous = layer;
        }
        true
    }
    /// `FishingHook.catchingFish`.
    fn catching_fish(&mut self, h: &mut Hook, b: [i32; 3]) {
        let mut speed = 1;
        let above = [b[0], b[1] + 1, b[2]];
        let sky = self.world.light(above[0], above[1], above[2]) >> 4 == 15;
        let raining = self.world.weather.raining;
        let r = &mut self.world.random;
        if r.next_float() < 0.25 && raining && sky {
            speed += 1;
        }
        let r = &mut self.world.random;
        if r.next_float() < 0.5 && !sky {
            speed -= 1;
        }
        let at = [h.position.x, h.position.y, h.position.z];
        if h.nibble > 0 {
            h.nibble -= 1;
            if h.nibble <= 0 {
                h.time_until_lured = 0;
                h.time_until_hooked = 0;
            }
        } else if h.time_until_hooked > 0 {
            h.time_until_hooked -= speed;
            if h.time_until_hooked > 0 {
                // The fish swimming up: a bubble now and then, and a fishing
                // particle to each side of its path.
                h.fish_angle += triangle(&mut self.world.random, 0.0, 9.188) as f32;
                let angle = h.fish_angle * (std::f64::consts::PI / 180.0) as f32;
                let (sin, cos) = (trig::sin(f64::from(angle)), trig::cos(f64::from(angle)));
                let distance = h.time_until_hooked as f32 * 0.1;
                let fish = [
                    h.position.x + f64::from(sin * distance),
                    f64::from(h.position.y.floor() as f32 + 1.0),
                    h.position.z + f64::from(cos * distance),
                ];
                if self.water_block(fish) {
                    if self.world.random.next_float() < 0.15 {
                        let spread = [f64::from(sin), 0.1, f64::from(cos)];
                        let at = [fish[0], fish[1] - f64::from(0.1f32), fish[2]];
                        self.send_particles(ParticleType::Bubble, at, spread, 0.0, 1);
                    }
                    let (x, z) = (sin * 0.04, cos * 0.04);
                    for side in [1.0f32, -1.0] {
                        let spread = [f64::from(z * side), 0.01, f64::from(-x * side)];
                        self.send_particles(ParticleType::Fishing, fish, spread, 1.0, 0);
                    }
                }
            } else {
                let pitch =
                    1.0 + (self.world.random.next_float() - self.world.random.next_float()) * 0.4;
                self.audio.play(
                    "minecraft:entity.fishing_bobber.splash",
                    Some(at),
                    0.25,
                    pitch,
                );
                // The bite: bubbles and fishing particles around the bobber.
                let at = [h.position.x, h.position.y + 0.5, h.position.z];
                let count = (1.0 + SIZE as f32 * 20.0) as u32;
                let spread = [SIZE, 0.0, SIZE];
                for kind in [ParticleType::Bubble, ParticleType::Fishing] {
                    self.send_particles(kind, at, spread, f64::from(0.2f32), count);
                }
                h.nibble = 20 + self.world.random.next_int_bound(21).unwrap_or(0);
            }
        } else if h.time_until_lured > 0 {
            h.time_until_lured -= speed;
            let mut tease = 0.15f32;
            if h.time_until_lured < 20 {
                tease += (20 - h.time_until_lured) as f32 * 0.05;
            } else if h.time_until_lured < 40 {
                tease += (40 - h.time_until_lured) as f32 * 0.02;
            } else if h.time_until_lured < 60 {
                tease += (60 - h.time_until_lured) as f32 * 0.01;
            }
            let r = &mut self.world.random;
            if r.next_float() < tease {
                // A splash where a fish is teasing the bait.
                let angle = (r.next_float() * 360.0) * (std::f64::consts::PI / 180.0) as f32;
                let distance = r.next_float() * 35.0 + 25.0;
                let fish = [
                    h.position.x + f64::from(trig::sin(f64::from(angle)) * distance) * 0.1,
                    f64::from(h.position.y.floor() as f32 + 1.0),
                    h.position.z + f64::from(trig::cos(f64::from(angle)) * distance) * 0.1,
                ];
                if self.water_block(fish) {
                    let count = 2 + self.world.random.next_int_bound(2).unwrap_or(0) as u32;
                    let spread = [f64::from(0.1f32), 0.0, f64::from(0.1f32)];
                    self.send_particles(ParticleType::Splash, fish, spread, 0.0, count);
                }
            }
            if h.time_until_lured <= 0 {
                h.fish_angle = self.world.random.next_float() * 360.0;
                h.time_until_hooked = 20 + self.world.random.next_int_bound(61).unwrap_or(0);
            }
        } else {
            h.time_until_lured = 100 + self.world.random.next_int_bound(501).unwrap_or(0) - h.lure;
        }
    }
    /// `catchingFish`'s check below a fish: the block under it is water.
    fn water_block(&self, fish: [f64; 3]) -> bool {
        let state = self.world.block(
            fish[0].floor() as i32,
            (fish[1] - 1.0).floor() as i32,
            fish[2].floor() as i32,
        );
        &*self.res.facts.get(state).name == "minecraft:water"
    }
    /// `ServerLevel.sendParticles` of a simple particle.
    fn send_particles(
        &mut self,
        kind: ParticleType,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
    ) {
        let options = ParticleOptions::simple(kind);
        self.particles
            .send_particles(&options, at, spread, speed, count);
    }
    /// `FishingHook.tick`.
    pub(super) fn tick_fishing(&mut self) {
        let Some(mut h) = self.hook.take() else {
            return;
        };
        let Some(owner) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        // shouldStopFishing: a rod in either hand, within 32 blocks.
        let d2 = (h.position.x - owner.x).powi(2)
            + (h.position.y - owner.y).powi(2)
            + (h.position.z - owner.z).powi(2);
        if !self.holds_rod() || d2 > 1024.0 {
            return;
        }
        h.previous = h.position;
        if h.on_ground {
            h.life += 1;
            if h.life >= 1200 {
                return;
            }
        } else {
            h.life = 0;
        }
        let b = [
            h.position.x.floor() as i32,
            h.position.y.floor() as i32,
            h.position.z.floor() as i32,
        ];
        let liquid = self.water_height(b[0], b[1], b[2]).map_or(0.0, |w| w.0);
        let in_water = liquid > 0.0;
        match h.state {
            State::Flying => {
                if in_water {
                    h.velocity =
                        Vec3::new(h.velocity.x * 0.3, h.velocity.y * 0.2, h.velocity.z * 0.3);
                    h.state = State::Bobbing;
                    self.hook = Some(h);
                    return;
                }
                // checkCollision: hook the first living mob it touches.
                let bb = bounds(h.position);
                let (lo, hi) = (bb.min_position(), bb.max_position());
                if let Some(m) = self.mobs.iter().find(|m| {
                    let mb = m.bounds();
                    let (a, z) = (mb.min_position(), mb.max_position());
                    m.alive()
                        && a.x < hi.x
                        && lo.x < z.x
                        && a.y < hi.y
                        && lo.y < z.y
                        && a.z < hi.z
                        && lo.z < z.z
                }) {
                    h.state = State::Hooked(m.id);
                    h.velocity = Vec3::ZERO;
                    self.hook = Some(h);
                    return;
                }
            }
            State::Hooked(id) => {
                match self.mobs.iter().find(|m| m.id == id && m.alive()) {
                    Some(m) => {
                        h.position = Vec3::new(
                            m.position.x,
                            m.position.y + f64::from(m.height()) * 0.8,
                            m.position.z,
                        )
                    }
                    None => h.state = State::Flying,
                }
                self.hook = Some(h);
                return;
            }
            State::Bobbing => {
                let v = h.velocity;
                let mut force = h.position.y + v.y - f64::from(b[1]) - f64::from(liquid);
                if force.abs() < 0.01 {
                    force += force.signum() * 0.1;
                }
                let r = f64::from(self.world.random.next_float());
                h.velocity = Vec3::new(v.x * 0.9, v.y - force * r * 0.2, v.z * 0.9);
                if h.nibble <= 0 && h.time_until_hooked <= 0 {
                    h.open_water = true;
                } else {
                    h.open_water =
                        h.open_water && h.out_of_water < 10 && self.calculate_open_water(b);
                }
                if in_water {
                    h.out_of_water = (h.out_of_water - 1).max(0);
                    if h.nibble > 0 {
                        let r = &mut self.world.random;
                        let dip = -0.1 * f64::from(r.next_float()) * f64::from(r.next_float());
                        h.velocity.y += dip;
                    }
                    self.catching_fish(&mut h, b);
                } else {
                    h.out_of_water = (h.out_of_water + 1).min(10);
                }
            }
        }
        if !in_water && !h.on_ground {
            h.velocity.y -= 0.03;
        }
        let mut scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        if let Ok(moved) = scene.resolve(bounds(h.position), h.velocity, 0.0, h.on_ground) {
            let horizontal =
                (moved.x - h.velocity.x).abs() > 1.0e-7 || (moved.z - h.velocity.z).abs() > 1.0e-7;
            h.on_ground = h.velocity.y < 0.0 && moved.y > h.velocity.y;
            h.position = Vec3::new(
                h.position.x + moved.x,
                h.position.y + moved.y,
                h.position.z + moved.z,
            );
            if h.state == State::Flying && (h.on_ground || horizontal) {
                h.velocity = Vec3::ZERO;
            }
        }
        h.velocity = Vec3::new(
            h.velocity.x * 0.92,
            h.velocity.y * 0.92,
            h.velocity.z * 0.92,
        );
        self.hook = Some(h);
    }
    /// `FishingRodCast`: the stack in the arm holding the rod
    /// (`FishingHookRenderer.getHoldingArm`) while the hook is out.
    pub(super) fn cast_rod(&self) -> Option<&ItemStack> {
        self.hook.as_ref()?;
        if self.main_hand_rod() {
            self.held()
        } else {
            self.offhand.as_ref()
        }
    }
    pub(super) fn main_hand_rod(&self) -> bool {
        self.held()
            .is_some_and(|s| &*s.item == "minecraft:fishing_rod")
    }
    /// The hook's position this frame.
    fn hook_position(&self, alpha: f64) -> Option<glam::DVec3> {
        let h = self.hook.as_ref()?;
        let lerp = |a: f64, b: f64| a + (b - a) * alpha;
        Some(glam::DVec3::new(
            lerp(h.previous.x, h.position.x),
            lerp(h.previous.y, h.position.y),
            lerp(h.previous.z, h.position.z),
        ))
    }
    /// FishingHookRenderer: the bobber, a camera-facing half-block quad
    /// centred on the hook.
    pub(super) fn fishing_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let Some(p) = self.hook_position(alpha) else {
            return;
        };
        let Some(r) = self
            .gui
            .atlas
            .sprites
            .get("entity/fishing/fishing_hook")
            .copied()
        else {
            return;
        };
        let light = self
            .world
            .light(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let forward = glam::DVec3::from_array(view_vector(self.yaw, self.pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let s = 0.25;
        let corners = [
            p - right * s - up * s,
            p + right * s - up * s,
            p + right * s + up * s,
            p - right * s + up * s,
        ];
        let uv = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
        for (v, uv) in corners.iter().zip(uv) {
            out.push(Vertex {
                normal: [0; 4],
                pos: (*v - camera).as_vec3().to_array(),
                uv,
                layer: 0,
                color: 0xFFFF_FFFF,
                light: packed,
            });
        }
    }
    /// FishingHookRenderer: the line, 16 black `lines` segments sagging from
    /// a quarter block above the hook to the rod in the player's hand,
    /// `width` pixels wide (`Window.getAppropriateLineWidth`).
    pub(super) fn fishing_line(&self, alpha: f64, size: [u32; 2]) -> Vec<Line> {
        let (Some(hook), Some(hand)) = (
            self.hook_position(alpha),
            self.rod_hand_position(alpha, size),
        ) else {
            return Vec::new();
        };
        let offset = (hand - (hook + glam::DVec3::new(0.0, 0.25, 0.0))).as_vec3();
        let point = |a: f32| {
            let y = offset.y * (a * a + a) * 0.5 + 0.25;
            hook + glam::Vec3::new(offset.x * a, y, offset.z * a).as_dvec3()
        };
        let width = horizon_client::gfx::outline_width(size[0]);
        (0..16)
            .map(|i| Line {
                start: point(i as f32 / 16.0).to_array(),
                end: point((i + 1) as f32 / 16.0).to_array(),
                color: 0xFF00_0000,
                width,
            })
            .collect()
    }
    /// `FishingHookRenderer.getPlayerHandPos`: on the near plane in first
    /// person, beside the body otherwise.
    fn rod_hand_position(&self, alpha: f64, size: [u32; 2]) -> Option<glam::DVec3> {
        let p = self.player.as_ref()?;
        let right_arm = self.main_hand_rod() != self.settings.main_hand_left;
        let invert: f32 = if right_arm { 1.0 } else { -1.0 };
        let swing = trig::sin(f64::from(
            self.swing.animation(alpha as f32).sqrt() * std::f32::consts::PI,
        ));
        let feet = self.previous_position.lerp(p.position, alpha);
        let eye = glam::DVec3::new(feet.x, feet.y + f64::from(p.dimensions.eye_height), feet.z);
        if self.camera_type.first_person() {
            let fov = self.settings.fov as i32 as f32;
            // Camera.getNearPlane(fov).getPointOnPlane(invert * 0.525, -0.1).
            let aspect = f64::from(size[0]) / f64::from(size[1].max(1));
            let height =
                (f64::from(fov * (std::f64::consts::PI / 180.0) as f32) / 2.0).tan() * 0.05;
            let forward = glam::DVec3::from_array(view_vector(self.yaw, self.pitch));
            let left = -forward.cross(glam::DVec3::Y).normalize_or_zero();
            let up = forward.cross(left).normalize_or_zero();
            let point = forward * 0.05 + up * (height * -0.1)
                - left * (height * aspect * f64::from(invert * 0.525));
            let point = point * (960.0 / f64::from(fov));
            // Vec3.yRot(swing * 0.5), then xRot(-swing * 0.7).
            let (s, c) = (
                f64::from(trig::sin(f64::from(swing * 0.5))),
                f64::from(trig::cos(f64::from(swing * 0.5))),
            );
            let point = glam::DVec3::new(
                point.x * c + point.z * s,
                point.y,
                point.z * c - point.x * s,
            );
            let (s, c) = (
                f64::from(trig::sin(f64::from(-swing * 0.7))),
                f64::from(trig::cos(f64::from(-swing * 0.7))),
            );
            let point = glam::DVec3::new(
                point.x,
                point.y * c + point.z * s,
                point.z * c - point.y * s,
            );
            Some(eye + point)
        } else {
            let model = self.player_model.as_ref()?;
            let body =
                horizon_client::camera::lerp(alpha as f32, model.previous_body_yaw, model.body_yaw);
            let angle = f64::from(body * (std::f64::consts::PI / 180.0) as f32);
            let (sin, cos) = (f64::from(trig::sin(angle)), f64::from(trig::cos(angle)));
            let right = f64::from(invert) * 0.35;
            let crouch = if p.pose == Pose::Crouching {
                -0.1875
            } else {
                0.0
            };
            Some(
                eye + glam::DVec3::new(
                    -cos * right - sin * 0.8,
                    crouch - 0.45,
                    -sin * right + cos * 0.8,
                ),
            )
        }
    }
}
impl Game<'_> {
    /// The hook for `horizon-ctl status`.
    pub(super) fn hook_status(&self) -> Option<serde_json::Value> {
        let h = self.hook.as_ref()?;
        Some(serde_json::json!({
            "state": format!("{:?}", h.state),
            "nibble": h.nibble,
            "lured_in": h.time_until_lured,
            "hooked_in": h.time_until_hooked,
            "open_water": h.open_water,
        }))
    }
}
