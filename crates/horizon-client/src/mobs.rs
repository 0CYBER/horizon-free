//! Mobs: entity types (dimensions and default attributes exported from vanilla),
//! models (vanilla `LayerDefinitions` baked part trees), `LivingEntity.travel`
//! movement with step-up and jumping, and goal-style AI for the common
//! overworld mobs (strolling, panicking, chasing, melee, creeper swelling).
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
    mesher::Vertex,
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path, sync::Arc};

#[derive(Clone, Debug)]
pub struct Polygon {
    pub vertices: [[f32; 5]; 4],
}
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    /// x, y, z, xRot, yRot, zRot, xScale, yScale, zScale
    pub pose: [f32; 9],
    pub polygons: Vec<Polygon>,
    pub children: Vec<Part>,
}
impl Part {
    fn parse(name: &str, v: &Value) -> Self {
        let pose: Vec<f32> = v["pose"].as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
        let mut p = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        for (i, x) in pose.iter().enumerate().take(9) {
            p[i] = *x;
        }
        let polygons = v["polygons"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|poly| {
                let verts: Vec<[f32; 5]> = poly
                    .as_array()?
                    .iter()
                    .map(|vx| {
                        let a: Vec<f32> = vx.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
                        [a[0], a[1], a[2], a[3], a[4]]
                    })
                    .collect();
                (verts.len() == 4).then(|| Polygon { vertices: [verts[0], verts[1], verts[2], verts[3]] })
            })
            .collect();
        let mut children: Vec<Part> = v["children"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, c)| Part::parse(k, c))
            .collect();
        children.sort_by(|a, b| a.name.cmp(&b.name));
        Self { name: name.to_owned(), pose: p, polygons, children }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brain {
    Passive,
    Chicken,
    Zombie,
    Skeleton,
    Spider,
    Creeper,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anim {
    Quadruped,
    Humanoid { zombie_arms: bool },
    Creeper,
    Spider,
    Chicken,
}
#[derive(Clone, Debug)]
pub struct MobType {
    pub id: String,
    pub width: f64,
    pub height: f64,
    pub eye_height: f64,
    pub category: String,
    pub max_health: f32,
    pub speed: f64,
    pub attack_damage: f32,
    pub follow_range: f64,
    pub step_height: f64,
    pub model: Arc<Part>,
    pub texture: String,
    pub brain: Brain,
    pub anim: Anim,
    pub sounds: [&'static str; 3],
}
pub struct MobTypes {
    pub types: HashMap<String, MobType>,
}
impl MobTypes {
    pub fn load(root: &Path) -> crate::Result<Self> {
        let types: Value = serde_json::from_slice(&fs::read(root.join("entity-types.json"))?)?;
        let models: Value = serde_json::from_slice(&fs::read(root.join("entity-models.json"))?)?;
        // Vanilla renderers' default texture per supported mob.
        let table: [(&str, &str, Brain, Anim, [&str; 3]); 9] = [
            ("pig", "entity/pig/pig_temperate", Brain::Passive, Anim::Quadruped, ["entity.pig.ambient", "entity.pig.hurt", "entity.pig.death"]),
            ("cow", "entity/cow/cow_temperate", Brain::Passive, Anim::Quadruped, ["entity.cow.ambient", "entity.cow.hurt", "entity.cow.death"]),
            ("sheep", "entity/sheep/sheep", Brain::Passive, Anim::Quadruped, ["entity.sheep.ambient", "entity.sheep.hurt", "entity.sheep.death"]),
            ("chicken", "entity/chicken/chicken_temperate", Brain::Chicken, Anim::Chicken, ["entity.chicken.ambient", "entity.chicken.hurt", "entity.chicken.death"]),
            ("zombie", "entity/zombie/zombie", Brain::Zombie, Anim::Humanoid { zombie_arms: true }, ["entity.zombie.ambient", "entity.zombie.hurt", "entity.zombie.death"]),
            ("skeleton", "entity/skeleton/skeleton", Brain::Skeleton, Anim::Humanoid { zombie_arms: false }, ["entity.skeleton.ambient", "entity.skeleton.hurt", "entity.skeleton.death"]),
            ("spider", "entity/spider/spider", Brain::Spider, Anim::Spider, ["entity.spider.ambient", "entity.spider.hurt", "entity.spider.death"]),
            ("creeper", "entity/creeper/creeper", Brain::Creeper, Anim::Creeper, ["entity.creeper.primed", "entity.creeper.hurt", "entity.creeper.death"]),
            ("cave_spider", "entity/spider/cave_spider", Brain::Spider, Anim::Spider, ["entity.spider.ambient", "entity.spider.hurt", "entity.spider.death"]),
        ];
        let mut out = HashMap::new();
        for (name, texture, brain, anim, sounds) in table {
            let id = format!("minecraft:{name}");
            let t = &types[&id];
            let model_key = if name == "cave_spider" { "minecraft:cave_spider#main".to_owned() } else { format!("{id}#main") };
            let model_value = models.get(&model_key).or_else(|| models.get(&format!("{id}#main")));
            let Some(model) = model_value else { continue };
            let attr = |k: &str| t["attributes"][k].as_f64();
            let texture = if root.join(format!("assets/minecraft/textures/{texture}.png")).exists() {
                texture.to_owned()
            } else {
                // Fall back to the first texture in the entity's folder.
                let folder = texture.rsplit_once('/').map_or(texture, |(a, _)| a);
                fs::read_dir(root.join(format!("assets/minecraft/textures/{folder}")))
                    .ok()
                    .and_then(|d| {
                        let mut names: Vec<String> = d.flatten().filter_map(|e| e.file_name().to_str().map(str::to_owned)).filter(|n| n.ends_with(".png") && !n.contains("baby")).collect();
                        names.sort();
                        names.first().map(|n| format!("{folder}/{}", n.trim_end_matches(".png")))
                    })
                    .unwrap_or_else(|| texture.to_owned())
            };
            out.insert(
                id.clone(),
                MobType {
                    width: t["width"].as_f64().unwrap_or(0.6),
                    height: t["height"].as_f64().unwrap_or(1.8),
                    eye_height: t["eye_height"].as_f64().unwrap_or(1.6),
                    category: t["category"].as_str().unwrap_or("creature").to_owned(),
                    max_health: attr("minecraft:max_health").unwrap_or(10.0) as f32,
                    speed: attr("minecraft:movement_speed").unwrap_or(0.25),
                    attack_damage: attr("minecraft:attack_damage").unwrap_or(0.0) as f32,
                    follow_range: attr("minecraft:follow_range").unwrap_or(16.0),
                    step_height: attr("minecraft:step_height").unwrap_or(0.6),
                    model: Arc::new(Part::parse("root", model)),
                    texture,
                    brain,
                    anim,
                    sounds: sounds.map(|s| s),
                    id,
                },
            );
        }
        Ok(Self { types: out })
    }
}
#[derive(Clone, Debug)]
pub struct Mob {
    pub id: u32,
    pub kind: Arc<MobType>,
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub yaw: f32,
    pub previous_yaw: f32,
    pub body_yaw: f32,
    pub previous_body_yaw: f32,
    pub head_pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub health: f32,
    pub hurt_time: i32,
    pub invulnerable: i32,
    pub death_time: i32,
    pub age: u32,
    pub limb_position: f32,
    pub limb_speed: f32,
    pub previous_limb_speed: f32,
    pub target: Option<[f64; 3]>,
    pub speed_modifier: f64,
    pub panic: i32,
    pub attack_cooldown: i32,
    pub swell: i32,
    pub previous_swell: i32,
    pub ambient_timer: i32,
    pub fall_distance: f64,
    pub persistent: bool,
    pub idle: u32,
}
/// Events a mob tick asks the game to act on.
#[derive(Clone, Debug)]
pub enum MobEvent {
    Sound(&'static str, [f64; 3], f32, f32),
    AttackPlayer(f32, [f64; 3]),
    Shoot([f64; 3], [f64; 3]),
    Explode([f64; 3], f32),
}
pub struct MobWorld<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub player: Option<([f64; 3], bool)>,
    pub daylight: bool,
}
fn wrap(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}
impl Mob {
    pub fn new(id: u32, kind: Arc<MobType>, position: Vec3, yaw: f32) -> Self {
        Self {
            id,
            health: kind.max_health,
            kind,
            position,
            previous: position,
            velocity: Vec3::ZERO,
            yaw,
            previous_yaw: yaw,
            body_yaw: yaw,
            previous_body_yaw: yaw,
            head_pitch: 0.0,
            on_ground: false,
            horizontal_collision: false,
            hurt_time: 0,
            invulnerable: 0,
            death_time: 0,
            age: 0,
            limb_position: 0.0,
            limb_speed: 0.0,
            previous_limb_speed: 0.0,
            target: None,
            speed_modifier: 1.0,
            panic: 0,
            attack_cooldown: 0,
            swell: 0,
            previous_swell: 0,
            ambient_timer: -80,
            fall_distance: 0.0,
            persistent: false,
            idle: 0,
        }
    }
    pub fn bounds(&self) -> Aabb {
        let w = self.kind.width / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - w, self.position.y, self.position.z - w),
            Vec3::new(self.position.x + w, self.position.y + self.kind.height, self.position.z + w),
        )
    }
    pub fn alive(&self) -> bool {
        self.health > 0.0
    }
    pub fn hostile(&self) -> bool {
        self.kind.category == "monster"
    }
    /// LivingEntity.hurt with invulnerability frames and knockback.
    pub fn hurt(&mut self, amount: f32, from: Option<[f64; 3]>, random: &mut LegacyRandom) -> bool {
        if !self.alive() || self.invulnerable > 10 {
            return false;
        }
        self.health -= amount;
        self.invulnerable = 20;
        self.hurt_time = 10;
        if let Some(f) = from {
            // LivingEntity.knockback(0.4, dx, dz)
            let (dx, dz) = (f[0] - self.position.x, f[2] - self.position.z);
            let len = (dx * dx + dz * dz).sqrt().max(1e-4);
            let strength = 0.4;
            self.velocity = Vec3::new(
                self.velocity.x / 2.0 - dx / len * strength,
                if self.on_ground { (self.velocity.y / 2.0 + strength).min(0.4) } else { self.velocity.y },
                self.velocity.z / 2.0 - dz / len * strength,
            );
            if !self.hostile() {
                self.panic = 60 + random.next_int_bound(40).unwrap_or(0);
                self.target = None;
            }
        }
        true
    }
    fn random_target(&self, random: &mut LegacyRandom, w: &MobWorld, radius: i32) -> Option<[f64; 3]> {
        // DefaultRandomPos.getPos: ten tries, prefer walkable spots.
        for _ in 0..10 {
            let x = self.position.x.floor() as i32 + random.next_int_bound(2 * radius + 1).ok()? - radius;
            let z = self.position.z.floor() as i32 + random.next_int_bound(2 * radius + 1).ok()? - radius;
            let mut y = self.position.y.floor() as i32 + random.next_int_bound(15).ok()? - 7;
            if !w.world.loaded(x, z) {
                continue;
            }
            let solid = |y: i32| w.facts.get(w.world.block(x, y, z)).full_collision;
            while y > crate::chunk::MIN_Y && !solid(y - 1) {
                y -= 1;
            }
            while solid(y) && y < 319 {
                y += 1;
            }
            let below = w.facts.get(w.world.block(x, y - 1, z));
            if below.fluid != Fluid::None || &*below.name == "minecraft:lava" {
                continue;
            }
            return Some([f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]);
        }
        None
    }
    pub fn tick(&mut self, w: &MobWorld, random: &mut LegacyRandom, events: &mut Vec<MobEvent>) -> bool {
        self.previous = self.position;
        self.previous_yaw = self.yaw;
        self.previous_body_yaw = self.body_yaw;
        self.previous_swell = self.swell;
        self.age += 1;
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
        if self.invulnerable > 0 {
            self.invulnerable -= 1;
        }
        if !self.alive() {
            self.death_time += 1;
            self.velocity = Vec3::new(0.0, self.velocity.y - 0.08, 0.0);
            self.travel(w, 0.0);
            return self.death_time < 20;
        }
        let center = [self.position.x, self.position.y + self.kind.eye_height, self.position.z];
        // Ambient sounds (Mob.playAmbientSound every ~80 ticks on average).
        self.ambient_timer += 1;
        if random.next_int_bound(1000).unwrap_or(1) < self.ambient_timer {
            self.ambient_timer = -80;
            if self.kind.brain != Brain::Creeper {
                let pitch = (random.next_float() - random.next_float()) * 0.2 + 1.0;
                events.push(MobEvent::Sound(self.kind.sounds[0], center, 1.0, pitch));
            }
        }
        let mut forward = 0.0f32;
        let player = w.player.filter(|(_, alive)| *alive).map(|(p, _)| p);
        let aggressive = match self.kind.brain {
            Brain::Zombie | Brain::Skeleton | Brain::Creeper => true,
            Brain::Spider => !w.daylight,
            _ => false,
        };
        let chase = if aggressive {
            player.filter(|p| self.distance_to(*p) < self.kind.follow_range.min(24.0))
        } else {
            None
        };
        if self.attack_cooldown > 0 {
            self.attack_cooldown -= 1;
        }
        if let Some(p) = chase {
            let d = self.distance_to(p);
            self.face(p, 30.0);
            match self.kind.brain {
                Brain::Creeper => {
                    // SwellGoal: fuse within 3 blocks, defuse beyond 7.
                    if d < 3.0 {
                        if self.swell == 0 {
                            events.push(MobEvent::Sound("entity.creeper.primed", center, 1.0, 0.5));
                        }
                        self.swell += 1;
                        if self.swell >= 30 {
                            events.push(MobEvent::Explode([self.position.x, self.position.y, self.position.z], 3.0));
                            self.health = 0.0;
                            self.death_time = 20;
                            return false;
                        }
                    } else {
                        self.swell = (self.swell - 1).max(0);
                        forward = 1.0;
                    }
                }
                Brain::Skeleton => {
                    // RangedBowAttackGoal: keep ~15 blocks, shoot every 20-40 ticks.
                    if d > 12.0 {
                        forward = 1.0;
                    }
                    if d < 15.0 && self.attack_cooldown == 0 {
                        events.push(MobEvent::Shoot(center, p));
                        self.attack_cooldown = 20 + random.next_int_bound(20).unwrap_or(0);
                    }
                }
                _ => {
                    forward = 1.0;
                    let reach = self.kind.width * 2.0 * self.kind.width * 2.0 + 0.6;
                    let dx = p[0] - self.position.x;
                    let dz = p[2] - self.position.z;
                    if dx * dx + dz * dz <= reach.max(2.0) && (p[1] - self.position.y).abs() < 2.0 && self.attack_cooldown == 0 {
                        events.push(MobEvent::AttackPlayer(self.kind.attack_damage.max(2.0), [self.position.x, self.position.y, self.position.z]));
                        self.attack_cooldown = 20;
                    }
                }
            }
            self.speed_modifier = 1.0;
            self.idle = 0;
        } else {
            self.swell = (self.swell - 1).max(0);
            if self.panic > 0 {
                self.panic -= 1;
                if self.target.is_none() || random.next_int_bound(20).unwrap_or(1) == 0 {
                    self.target = self.random_target(random, w, 5);
                }
                self.speed_modifier = if self.kind.brain == Brain::Chicken { 1.4 } else { 1.25 };
            } else if self.target.is_none() && random.next_int_bound(120).unwrap_or(1) == 0 {
                // RandomStrollGoal(speed 1.0, interval 120)
                self.target = self.random_target(random, w, 10);
                self.speed_modifier = 1.0;
            }
            if let Some(t) = self.target {
                let dx = t[0] - self.position.x;
                let dz = t[2] - self.position.z;
                if dx * dx + dz * dz < 0.5 || (self.horizontal_collision && random.next_int_bound(40).unwrap_or(1) == 0) {
                    self.target = None;
                } else {
                    self.face(t, 30.0);
                    forward = 1.0;
                }
            } else if let Some(p) = player
                && self.distance_to(p) < 8.0
                && random.next_float() < 0.02
            {
                // LookAtPlayerGoal.
                self.face(p, 10.0);
            }
            self.idle += 1;
        }
        // Body follows head (BodyRotationControl).
        self.body_yaw += wrap(self.yaw - self.body_yaw) * 0.3;
        // Daylight burning for undead.
        if matches!(self.kind.brain, Brain::Zombie | Brain::Skeleton) && w.daylight && self.age % 20 == 0 {
            let x = self.position.x.floor() as i32;
            let z = self.position.z.floor() as i32;
            let y = (self.position.y + self.kind.eye_height).floor() as i32;
            if w.world.light(x, y, z) >> 4 == 15 && w.facts.get(w.world.block(x, y, z)).fluid == Fluid::None {
                self.hurt(1.0, None, random);
            }
        }
        let speed = (self.kind.speed * self.speed_modifier) as f32 * forward;
        self.travel(w, speed);
        // Limb animation (WalkAnimationState.update).
        let dx = self.position.x - self.previous.x;
        let dz = self.position.z - self.previous.z;
        let moved = ((dx * dx + dz * dz).sqrt() * 4.0).min(1.0) as f32;
        self.previous_limb_speed = self.limb_speed;
        self.limb_speed += (moved - self.limb_speed) * 0.4;
        self.limb_position += self.limb_speed;
        // Despawn far monsters.
        if self.hostile() && !self.persistent
            && let Some(p) = player
        {
            let d = self.distance_to(p);
            if d > 128.0 || (d > 32.0 && self.idle > 600 && random.next_int_bound(800).unwrap_or(1) == 0) {
                return false;
            }
        }
        self.position.y > -128.0
    }
    fn distance_to(&self, p: [f64; 3]) -> f64 {
        let d = [p[0] - self.position.x, p[1] - self.position.y, p[2] - self.position.z];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }
    fn face(&mut self, p: [f64; 3], max: f32) {
        let dx = p[0] - self.position.x;
        let dz = p[2] - self.position.z;
        let target = (dz.atan2(dx).to_degrees() as f32) - 90.0;
        self.yaw += wrap(target - self.yaw).clamp(-max, max);
    }
    /// LivingEntity.travel (ground/air friction, gravity, step-up and jumping).
    fn travel(&mut self, w: &MobWorld, speed: f32) {
        let x = self.position.x.floor() as i32;
        let z = self.position.z.floor() as i32;
        let below = w.world.block(x, (self.position.y - 0.5).floor() as i32, z);
        let in_water = w.facts.get(w.world.block(x, (self.position.y + 0.2).floor() as i32, z)).fluid != Fluid::None;
        let friction = if self.on_ground { w.facts.get(below).friction * 0.91 } else { 0.91 };
        let acceleration = if in_water {
            0.02
        } else if self.on_ground {
            f64::from(speed) * (0.216_000_02 / f64::from(friction * friction * friction))
        } else {
            0.02 * f64::from(speed > 0.0)
        };
        if speed > 0.0 {
            let yaw = f64::from(self.yaw).to_radians();
            self.velocity.x += -yaw.sin() * acceleration;
            self.velocity.z += yaw.cos() * acceleration;
            // JumpControl: hop when blocked by a full block.
            if self.horizontal_collision && self.on_ground {
                self.velocity.y = 0.42;
            }
        }
        if in_water {
            if speed > 0.0 || self.kind.brain != Brain::Passive {
                self.velocity.y += 0.04;
            }
            self.velocity.y = self.velocity.y.min(0.1);
        }
        let mut scene = SceneCollisions { world: w.world, binding: w.collision };
        let wanted = self.velocity;
        let moved = scene
            .resolve(self.bounds(), wanted, self.kind.step_height as f32, self.on_ground)
            .unwrap_or(Vec3::ZERO);
        self.position = Vec3::new(self.position.x + moved.x, self.position.y + moved.y, self.position.z + moved.z);
        self.horizontal_collision = (moved.x - wanted.x).abs() > 1e-6 || (moved.z - wanted.z).abs() > 1e-6;
        let vertical = (moved.y - wanted.y).abs() > 1e-6;
        self.on_ground = vertical && wanted.y < 0.0;
        if vertical {
            self.velocity.y = 0.0;
        }
        if self.horizontal_collision {
            if (moved.x - wanted.x).abs() > 1e-6 {
                self.velocity.x = 0.0;
            }
            if (moved.z - wanted.z).abs() > 1e-6 {
                self.velocity.z = 0.0;
            }
        }
        // Spiders climb walls.
        if self.kind.brain == Brain::Spider && self.horizontal_collision {
            self.velocity.y = 0.2;
        }
        if self.on_ground {
            if self.fall_distance > 3.0 {
                self.health -= (self.fall_distance - 3.0).ceil() as f32;
                self.hurt_time = 10;
            }
            self.fall_distance = 0.0;
        } else if moved.y < 0.0 {
            self.fall_distance -= moved.y;
        }
        let gravity = if self.kind.brain == Brain::Chicken && self.velocity.y < 0.0 { 0.0 } else { 0.08 };
        if !in_water {
            self.velocity.y -= gravity;
        }
        if self.kind.brain == Brain::Chicken && !self.on_ground && self.velocity.y < 0.0 {
            // Chicken flapping: slow fall.
            self.velocity.y *= 0.6;
        }
        let drag = if in_water { 0.8 } else { f64::from(friction) };
        self.velocity = Vec3::new(self.velocity.x * drag, self.velocity.y * 0.98, self.velocity.z * drag);
    }
}
/// Model geometry for a mob, posed for its animation state, as camera-relative
/// vertices textured from the shared atlas.
pub fn render(mob: &Mob, alpha: f32, camera: glam::DVec3, uv: [f32; 4], light: u32, out: &mut Vec<Vertex>) {
    let a = f64::from(alpha);
    let p = glam::DVec3::new(
        mob.previous.x + (mob.position.x - mob.previous.x) * a,
        mob.previous.y + (mob.position.y - mob.previous.y) * a,
        mob.previous.z + (mob.position.z - mob.previous.z) * a,
    ) - camera;
    let body = mob.previous_body_yaw + wrap(mob.body_yaw - mob.previous_body_yaw) * alpha;
    let head_yaw = wrap((mob.previous_yaw + wrap(mob.yaw - mob.previous_yaw) * alpha) - body);
    let limb_speed = (mob.previous_limb_speed + (mob.limb_speed - mob.previous_limb_speed) * alpha).min(1.0);
    let limb = mob.limb_position - mob.limb_speed * (1.0 - alpha);
    let age = mob.age as f32 + alpha;
    // LivingEntityRenderer: rotate 180-body, flip, lift 1.501.
    let mut base = glam::Mat4::from_translation(p.as_vec3())
        * glam::Mat4::from_rotation_y((180.0 - body).to_radians());
    if mob.death_time > 0 {
        let f = ((mob.death_time as f32 + alpha - 1.0) / 20.0 * 1.6).sqrt().min(1.0);
        base *= glam::Mat4::from_rotation_z(f * 90.0_f32.to_radians());
    }
    if mob.kind.brain == Brain::Creeper {
        // CreeperRenderer.scale: swelling.
        let s = (mob.previous_swell as f32 + (mob.swell - mob.previous_swell) as f32 * alpha) / 28.0;
        let s = s.clamp(0.0, 1.0);
        let wobble = 1.0 + (s * 100.0).sin() * s * 0.01;
        let s4 = s * s * s * s;
        let xz = (1.0 + s4 * 0.4) * wobble;
        let y = (1.0 + s4 * 0.1) / wobble;
        base *= glam::Mat4::from_scale(glam::Vec3::new(xz, y, xz));
    }
    base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0)) * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
    let overlay = u32::from(mob.hurt_time > 0 || mob.death_time > 0);
    let swing = |phase: f32| (limb * 0.6662 + phase).cos() * 1.4 * limb_speed;
    let pose_for = |part: &Part| -> [f32; 9] {
        let mut pose = part.pose;
        match (&mob.kind.anim, part.name.as_str()) {
            (_, "head") => {
                pose[4] = head_yaw.to_radians();
                pose[3] += mob.head_pitch.to_radians();
            }
            (Anim::Quadruped, "right_hind_leg") | (Anim::Quadruped, "left_front_leg") => pose[3] = swing(0.0),
            (Anim::Quadruped, "left_hind_leg") | (Anim::Quadruped, "right_front_leg") => pose[3] = swing(std::f32::consts::PI),
            (Anim::Chicken, "right_leg") => pose[3] = swing(0.0),
            (Anim::Chicken, "left_leg") => pose[3] = swing(std::f32::consts::PI),
            (Anim::Chicken, "right_wing") => pose[5] = (age * 0.3).sin().abs() * if mob.on_ground { 0.0 } else { 1.0 },
            (Anim::Chicken, "left_wing") => pose[5] = -(age * 0.3).sin().abs() * if mob.on_ground { 0.0 } else { 1.0 },
            (Anim::Humanoid { .. }, "right_leg") => pose[3] = (limb * 0.6662).cos() * 1.4 * limb_speed * 0.5 * 2.0 / 2.0 * 1.0,
            (Anim::Humanoid { .. }, "left_leg") => pose[3] = (limb * 0.6662 + std::f32::consts::PI).cos() * 1.4 * limb_speed,
            (Anim::Humanoid { zombie_arms: true }, "right_arm") | (Anim::Humanoid { zombie_arms: true }, "left_arm") => {
                // AnimationUtils.animateZombieArms
                pose[3] = -std::f32::consts::FRAC_PI_2 + (age * 0.067).sin() * 0.05;
            }
            (Anim::Humanoid { .. }, "right_arm") => pose[3] = (limb * 0.6662 + std::f32::consts::PI).cos() * 2.0 * limb_speed * 0.5,
            (Anim::Humanoid { .. }, "left_arm") => pose[3] = (limb * 0.6662).cos() * 2.0 * limb_speed * 0.5,
            (Anim::Creeper, "right_hind_leg") | (Anim::Creeper, "left_front_leg") => pose[3] = swing(0.0),
            (Anim::Creeper, "left_hind_leg") | (Anim::Creeper, "right_front_leg") => pose[3] = swing(std::f32::consts::PI),
            (Anim::Spider, name) if name.contains("leg") => {
                let n: f32 = name.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0.0);
                let phase = (n % 4.0) * std::f32::consts::FRAC_PI_2;
                pose[4] += (limb * 0.6662 * 2.0 + phase).cos() * 0.4 * limb_speed;
                pose[5] += (limb * 0.6662 + phase).sin().abs() * 0.4 * limb_speed;
            }
            _ => {}
        }
        pose
    };
    fn visit(part: &Part, parent: glam::Mat4, pose_for: &dyn Fn(&Part) -> [f32; 9], uv: [f32; 4], light: u32, overlay: u32, out: &mut Vec<Vertex>) {
        let p = pose_for(part);
        let m = parent
            * glam::Mat4::from_translation(glam::Vec3::new(p[0], p[1], p[2]) / 16.0)
            * glam::Mat4::from_quat(glam::Quat::from_euler(glam::EulerRot::ZYX, p[5], p[4], p[3]))
            * glam::Mat4::from_scale(glam::Vec3::new(p[6], p[7], p[8]));
        for poly in &part.polygons {
            for v in poly.vertices {
                let pos = m.transform_point3(glam::Vec3::new(v[0], v[1], v[2]) / 16.0);
                out.push(Vertex {
                    pos: pos.to_array(),
                    uv: [uv[0] + (uv[2] - uv[0]) * v[3], uv[1] + (uv[3] - uv[1]) * v[4]],
                    layer: overlay,
                    color: 0xFFFF_FFFF,
                    light,
                });
            }
        }
        for c in &part.children {
            visit(c, m, pose_for, uv, light, overlay, out);
        }
    }
    visit(&mob.kind.model, base, &pose_for, uv, light, overlay, out);
}
