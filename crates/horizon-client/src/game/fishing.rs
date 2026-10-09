//! Fishing: `FishingRodItem.use` (cast / reel in), the `FishingHook` (flight,
//! bobbing, lure and bite timers, open water, hooking mobs, retrieving the
//! `gameplay/fishing` loot) and `FishingHookRenderer` (bobber and line).
use super::{Game, view_vector};
use horizon_client::{
    blocks::Fluid, collisions::SceneCollisions, items::ItemStack, mesher::Vertex,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
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
                h.fish_angle += triangle(&mut self.world.random, 0.0, 9.188) as f32;
            } else {
                let pitch =
                    1.0 + (self.world.random.next_float() - self.world.random.next_float()) * 0.4;
                self.audio.play(
                    "minecraft:entity.fishing_bobber.splash",
                    Some(at),
                    0.25,
                    pitch,
                );
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
                // The fish particle draws: angle and distance.
                let _ = (r.next_float(), r.next_float());
            }
            if h.time_until_lured <= 0 {
                h.fish_angle = self.world.random.next_float() * 360.0;
                h.time_until_hooked = 20 + self.world.random.next_int_bound(61).unwrap_or(0);
            }
        } else {
            h.time_until_lured = 100 + self.world.random.next_int_bound(501).unwrap_or(0) - h.lure;
        }
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
    /// FishingHookRenderer: the bobber billboard and the line to the rod.
    pub(super) fn fishing_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let Some(h) = &self.hook else { return };
        let atlas = &self.res.atlas;
        let lerp = |a: f64, b: f64| a + (b - a) * alpha;
        let p = glam::DVec3::new(
            lerp(h.previous.x, h.position.x),
            lerp(h.previous.y, h.position.y),
            lerp(h.previous.z, h.position.z),
        );
        let light = self
            .world
            .light(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let forward = glam::DVec3::from_array(view_vector(self.yaw, self.pitch));
        let right = forward.cross(glam::DVec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let mut quad = |c: [glam::DVec3; 4], uv: [[f32; 2]; 4], color: u32| {
            for (v, uv) in c.iter().zip(uv) {
                out.push(Vertex {
                    pos: (*v - camera).as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color,
                    light: packed,
                });
            }
        };
        if let Some(r) = atlas.sprites.get("entity/fishing/fishing_hook").copied() {
            let c = p + glam::DVec3::new(0.0, SIZE / 2.0, 0.0);
            let s = 0.25;
            quad(
                [
                    c - right * s - up * s,
                    c + right * s - up * s,
                    c + right * s + up * s,
                    c - right * s + up * s,
                ],
                [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]],
                0xFFFF_FFFF,
            );
        }
        // The line: 16 segments sagging from the rod tip to the bobber.
        let Some(eye) = self.eye_position() else {
            return;
        };
        let tip = glam::DVec3::from_array(eye) + forward * 0.6 + right * 0.35 - up * 0.25;
        let end = p + glam::DVec3::new(0.0, SIZE * 0.7, 0.0);
        let w = atlas.font.white();
        let uv = [[w.u0, w.v1], [w.u1, w.v1], [w.u1, w.v0], [w.u0, w.v0]];
        let point = |t: f64| {
            let d = end - tip;
            tip + glam::DVec3::new(
                d.x * t,
                d.y * (t * t + t) * 0.5 + 0.25 * t * (1.0 - t),
                d.z * t,
            )
        };
        for i in 0..16 {
            let (a, b) = (point(f64::from(i) / 16.0), point(f64::from(i + 1) / 16.0));
            let side = (b - a).cross(forward).normalize_or_zero() * 0.0125;
            quad([a - side, b - side, b + side, a + side], uv, 0xFF00_0000);
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
