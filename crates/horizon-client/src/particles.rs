//! Client particle engine (`ParticleEngine`, `Particle`, `SingleQuadParticle`)
//! and the vanilla particle types players see most: block crumbs, smoke,
//! flames, lava pops, drips, splashes and rain, bubbles, portal and enchant
//! glyphs, crits, hearts, villager particles, explosions, campfire smoke,
//! notes, redstone dust, totem, end rod, biome ash and spores, falling leaves,
//! spells and dragon breath. Constructors, ticks, sizes, colours and light
//! follow the vanilla classes; sprite sets come from `particles/*.json`.
use crate::{
    blocks::{BlockFacts, Fluid},
    collisions::SceneCollisions,
    hud::Rect,
    mesher::Vertex,
    world::World,
};
use glam::{DVec3, Quat, Vec3 as FVec3};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
use std::{collections::HashMap, path::Path};

macro_rules! particle_types {
    ($($variant:ident = $name:literal),* $(,)?) => {
        /// Vanilla particle types with a client implementation here.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ParticleType { $($variant),* }
        impl ParticleType {
            pub const ALL: &'static [ParticleType] = &[$(ParticleType::$variant),*];
            /// Registry path (`minecraft:<name>`), also the sprite-set file name.
            pub fn name(self) -> &'static str {
                match self { $(ParticleType::$variant => $name),* }
            }
            pub fn from_name(name: &str) -> Option<Self> {
                let name = name.strip_prefix("minecraft:").unwrap_or(name);
                match name { $($name => Some(ParticleType::$variant),)* _ => None }
            }
        }
    };
}
particle_types! {
    Block = "block",
    BlockCrumble = "block_crumble",
    DustPillar = "dust_pillar",
    FallingDust = "falling_dust",
    Smoke = "smoke",
    LargeSmoke = "large_smoke",
    WhiteSmoke = "white_smoke",
    Flame = "flame",
    SoulFireFlame = "soul_fire_flame",
    CopperFireFlame = "copper_fire_flame",
    SmallFlame = "small_flame",
    Lava = "lava",
    DrippingWater = "dripping_water",
    FallingWater = "falling_water",
    DrippingLava = "dripping_lava",
    FallingLava = "falling_lava",
    LandingLava = "landing_lava",
    DrippingHoney = "dripping_honey",
    FallingHoney = "falling_honey",
    LandingHoney = "landing_honey",
    FallingNectar = "falling_nectar",
    FallingSporeBlossom = "falling_spore_blossom",
    SporeBlossomAir = "spore_blossom_air",
    DrippingObsidianTear = "dripping_obsidian_tear",
    FallingObsidianTear = "falling_obsidian_tear",
    LandingObsidianTear = "landing_obsidian_tear",
    DrippingDripstoneWater = "dripping_dripstone_water",
    FallingDripstoneWater = "falling_dripstone_water",
    DrippingDripstoneLava = "dripping_dripstone_lava",
    FallingDripstoneLava = "falling_dripstone_lava",
    Splash = "splash",
    Rain = "rain",
    Bubble = "bubble",
    BubblePop = "bubble_pop",
    BubbleColumnUp = "bubble_column_up",
    CurrentDown = "current_down",
    Underwater = "underwater",
    Portal = "portal",
    ReversePortal = "reverse_portal",
    Enchant = "enchant",
    Nautilus = "nautilus",
    VaultConnection = "vault_connection",
    Crit = "crit",
    EnchantedHit = "enchanted_hit",
    DamageIndicator = "damage_indicator",
    Heart = "heart",
    AngryVillager = "angry_villager",
    HappyVillager = "happy_villager",
    Composter = "composter",
    Mycelium = "mycelium",
    Dolphin = "dolphin",
    EggCrack = "egg_crack",
    Poof = "poof",
    Explosion = "explosion",
    ExplosionEmitter = "explosion_emitter",
    CampfireCosySmoke = "campfire_cosy_smoke",
    CampfireSignalSmoke = "campfire_signal_smoke",
    Note = "note",
    Dust = "dust",
    TotemOfUndying = "totem_of_undying",
    EndRod = "end_rod",
    Ash = "ash",
    WhiteAsh = "white_ash",
    CrimsonSpore = "crimson_spore",
    WarpedSpore = "warped_spore",
    CherryLeaves = "cherry_leaves",
    PaleOakLeaves = "pale_oak_leaves",
    TintedLeaves = "tinted_leaves",
    Effect = "effect",
    InstantEffect = "instant_effect",
    EntityEffect = "entity_effect",
    Witch = "witch",
    Infested = "infested",
    Cloud = "cloud",
    Sneeze = "sneeze",
    Snowflake = "snowflake",
    Soul = "soul",
    SculkSoul = "sculk_soul",
    DragonBreath = "dragon_breath",
}
impl ParticleType {
    /// `ParticleType.getOverrideLimiter`: spawned regardless of distance.
    pub fn override_limiter(self) -> bool {
        matches!(
            self,
            Self::DamageIndicator | Self::ExplosionEmitter | Self::Explosion | Self::Poof
        )
    }
}

/// `ParticleOptions` with the parameters of the option classes used here.
#[derive(Clone, Debug)]
pub struct ParticleOptions {
    pub ty: ParticleType,
    /// `BlockParticleOption`: the block's particle sprite and terrain tint.
    pub block: Option<(String, u32)>,
    /// RGB(A) for dust, spells, entity effects and tinted leaves.
    pub color: [f32; 4],
    /// `DustParticleOptions.scale`, `PowerParticleOption.power`, `SpellParticleOption.power`.
    pub scale: f32,
}
impl ParticleOptions {
    pub fn simple(ty: ParticleType) -> Self {
        Self {
            ty,
            block: None,
            color: [1.0; 4],
            scale: 1.0,
        }
    }
    pub fn block(ty: ParticleType, sprite: String, tint: u32) -> Self {
        Self {
            block: Some((sprite, tint)),
            ..Self::simple(ty)
        }
    }
    /// `DustParticleOptions(color, scale)`.
    pub fn dust(rgb: [f32; 3], scale: f32) -> Self {
        Self {
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            scale: scale.clamp(0.01, 4.0),
            ..Self::simple(ParticleType::Dust)
        }
    }
    pub fn colored(ty: ParticleType, rgba: [f32; 4]) -> Self {
        Self {
            color: rgba,
            ..Self::simple(ty)
        }
    }
}

#[derive(Clone, Debug)]
enum Sprite {
    /// Index into the engine's sprite table.
    Index(u32),
    /// An atlas sprite by name (block textures for terrain particles).
    Named(String),
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Size {
    Fixed,
    /// `quadSize * clamp((age + a) / lifetime * 32, 0, 1)`.
    GrowIn,
    Flame,
    Lava,
    Portal,
    ReversePortal,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Light {
    World,
    FullBright,
    /// `LightCoordsUtil.withBlock(light, 15)`.
    Block15,
    /// `addSmoothBlockEmission` with the age fraction.
    AgeEmission,
    /// `addSmoothBlockEmission` with the age fraction to the fourth power.
    AgeEmission4,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Behaviour {
    Base,
    /// BaseAshSmokeParticle / Explode / Dust / Snowflake / EmissiveRising: base tick, sprite from age.
    Animated,
    Lava,
    Drip {
        fluid: Fluid,
        stage: DripStage,
        next: Option<ParticleType>,
        sound: Option<&'static str>,
    },
    WaterDrop,
    Bubble,
    BubblePop,
    BubbleColumnUp,
    CurrentDown {
        angle: f32,
    },
    SuspendedTown,
    Portal {
        start: [f64; 3],
    },
    ReversePortal,
    FlyTowards {
        start: [f64; 3],
        alpha: [f32; 4],
    },
    Crit,
    HugeExplosion,
    ExplosionSeed,
    Campfire,
    SimpleAnimated {
        fade: Option<[f32; 3]>,
    },
    Spell {
        original_alpha: f32,
    },
    Cloud,
    Snowflake,
    Leaves {
        rot_speed: f32,
        spin: f32,
        wind: f32,
        swirl: bool,
        flow: bool,
        flow_scale: [f64; 2],
        swirl_period: f64,
    },
    FallingDust {
        rot_speed: f32,
    },
    DragonBreath {
        hit_ground: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum DripStage {
    Hang,
    CoolingHang,
    Fall,
    FallAndLand,
    Land,
}

/// One live particle (`Particle` + `SingleQuadParticle` state).
#[derive(Clone, Debug)]
pub struct Particle {
    pub ty: ParticleType,
    pub position: [f64; 3],
    pub previous: [f64; 3],
    pub velocity: [f64; 3],
    bb: Aabb,
    bb_width: f32,
    bb_height: f32,
    pub on_ground: bool,
    has_physics: bool,
    /// Particles that move without collision (`move` overridden).
    free_move: bool,
    stopped_by_collision: bool,
    removed: bool,
    pub age: i32,
    pub lifetime: i32,
    gravity: f32,
    friction: f32,
    speed_up_when_y_blocked: bool,
    /// `quadSize`.
    pub size: f32,
    pub rgba: [f32; 4],
    roll: f32,
    o_roll: f32,
    sprite: Sprite,
    /// Age-driven sprite set (`setSpriteFromAge`).
    set: Option<u32>,
    /// Sub-rectangle of the sprite (u0, v0, u1, v1) in 0..1.
    pub uv: [f32; 4],
    size_curve: Size,
    light: Light,
    behaviour: Behaviour,
    no_render: bool,
    /// Ticked once when created (`CritParticle` constructor).
    pre_tick: bool,
}
fn aabb_at(p: [f64; 3], w: f32, h: f32) -> Aabb {
    let w = f64::from(w) / 2.0;
    Aabb::new(
        Vec3::new(p[0] - w, p[1], p[2] - w),
        Vec3::new(p[0] + w, p[1] + f64::from(h), p[2] + w),
    )
}
impl Particle {
    /// `Particle(level, x, y, z)` + `SingleQuadParticle` (no initial velocity).
    fn at(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3]) -> Self {
        let mut particle = Self::blank(ty, p);
        particle.lifetime = (4.0 / (random.next_float() * 0.9 + 0.1)) as i32;
        particle.size = 0.1 * (random.next_float() * 0.5 + 0.5) * 2.0;
        particle
    }
    fn blank(ty: ParticleType, p: [f64; 3]) -> Self {
        Self {
            ty,
            position: p,
            previous: p,
            velocity: [0.0; 3],
            bb: aabb_at(p, 0.2, 0.2),
            bb_width: 0.2,
            bb_height: 0.2,
            on_ground: false,
            has_physics: true,
            free_move: false,
            stopped_by_collision: false,
            removed: false,
            age: 0,
            lifetime: 0,
            gravity: 0.0,
            friction: 0.98,
            speed_up_when_y_blocked: false,
            size: 0.0,
            rgba: [1.0; 4],
            roll: 0.0,
            o_roll: 0.0,
            sprite: Sprite::Index(u32::MAX),
            set: None,
            uv: [0.0, 0.0, 1.0, 1.0],
            size_curve: Size::Fixed,
            light: Light::World,
            behaviour: Behaviour::Base,
            no_render: false,
            pre_tick: false,
        }
    }
    /// `Particle(level, x, y, z, xa, ya, za)`: jittered, normalized initial velocity.
    fn moving(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3], v: [f64; 3]) -> Self {
        let lifetime = (4.0 / (random.next_float() * 0.9 + 0.1)) as i32;
        let mut d = [
            v[0] + f64::from((random.next_float() * 2.0 - 1.0) * 0.4),
            v[1] + f64::from((random.next_float() * 2.0 - 1.0) * 0.4),
            v[2] + f64::from((random.next_float() * 2.0 - 1.0) * 0.4),
        ];
        let speed = f64::from((random.next_float() + random.next_float() + 1.0) * 0.15);
        let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        for (i, c) in d.iter_mut().enumerate() {
            *c = *c / length * speed * f64::from(0.4f32);
            if i == 1 {
                *c += f64::from(0.1f32);
            }
        }
        let mut particle = Self::blank(ty, p);
        particle.lifetime = lifetime;
        particle.velocity = d;
        particle.size = 0.1 * (random.next_float() * 0.5 + 0.5) * 2.0;
        particle
    }
    /// `TerrainParticle(level, x, y, z, xa, ya, za, state)`: a crumb from a
    /// random quarter of the block's particle sprite, tinted like the block.
    pub fn terrain(
        random: &mut LegacyRandom,
        position: [f64; 3],
        velocity: [f64; 3],
        sprite: String,
        tint: u32,
    ) -> Self {
        let mut p = Self::moving(random, ParticleType::Block, position, velocity);
        p.sprite = Sprite::Named(sprite);
        p.gravity = 1.0;
        let c = |shift: u32| 0.6 * ((tint >> shift) & 255) as f32 / 255.0;
        p.rgba = [c(16), c(8), c(0), 1.0];
        p.size /= 2.0;
        let uo = random.next_float() * 3.0;
        let vo = random.next_float() * 3.0;
        // getU0 = (uo + 1) / 4, getU1 = uo / 4: the crumb is mirrored horizontally.
        p.uv = [(uo + 1.0) / 4.0, vo / 4.0, uo / 4.0, (vo + 1.0) / 4.0];
        p
    }
    /// `Particle.setPower`.
    pub fn set_power(&mut self, power: f32) {
        let power = f64::from(power);
        self.velocity[0] *= power;
        self.velocity[1] = (self.velocity[1] - f64::from(0.1f32)) * power + f64::from(0.1f32);
        self.velocity[2] *= power;
    }
    /// `SingleQuadParticle.scale` (quad and collision box).
    pub fn scale(&mut self, scale: f32) {
        self.size *= scale;
        self.set_size(0.2 * scale, 0.2 * scale);
    }
    fn set_size(&mut self, w: f32, h: f32) {
        if w != self.bb_width || h != self.bb_height {
            self.bb_width = w;
            self.bb_height = h;
            let min = self.bb.min_position();
            let max = self.bb.max_position();
            let nx = (min.x + max.x - f64::from(w)) / 2.0;
            let nz = (min.z + max.z - f64::from(w)) / 2.0;
            self.bb = Aabb::new(
                Vec3::new(nx, min.y, nz),
                Vec3::new(nx + f64::from(w), min.y + f64::from(h), nz + f64::from(w)),
            );
        }
    }
    fn set_pos(&mut self, p: [f64; 3]) {
        self.position = p;
        self.bb = aabb_at(p, self.bb_width, self.bb_height);
    }
    fn set_location_from_bb(&mut self) {
        let (min, max) = (self.bb.min_position(), self.bb.max_position());
        self.position = [(min.x + max.x) / 2.0, min.y, (min.z + max.z) / 2.0];
    }
    fn set_color(&mut self, r: f32, g: f32, b: f32) {
        self.rgba[0] = r;
        self.rgba[1] = g;
        self.rgba[2] = b;
    }
    pub fn is_alive(&self) -> bool {
        !self.removed
    }
    /// `Particle.move` with `Entity.collideBoundingBox` against block collision shapes.
    fn move_by(&mut self, level: &ParticleLevel, d: [f64; 3]) {
        if self.free_move {
            self.bb = self.bb.move_by(Vec3::new(d[0], d[1], d[2]));
            self.set_location_from_bb();
            return;
        }
        if self.stopped_by_collision {
            return;
        }
        let original = d;
        let mut moved = d;
        let length2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if self.has_physics && length2 != 0.0 && length2 < 100.0 * 100.0 {
            moved = level.collide(self.bb, d);
        }
        if moved != [0.0; 3] {
            self.bb = self.bb.move_by(Vec3::new(moved[0], moved[1], moved[2]));
            self.set_location_from_bb();
        }
        if original[1].abs() >= f64::from(1.0e-5f32) && moved[1].abs() < f64::from(1.0e-5f32) {
            self.stopped_by_collision = true;
        }
        self.on_ground = original[1] != moved[1] && original[1] < 0.0;
        if original[0] != moved[0] {
            self.velocity[0] = 0.0;
        }
        if original[2] != moved[2] {
            self.velocity[2] = 0.0;
        }
    }
    /// `Particle.tick`.
    fn base_tick(&mut self, level: &ParticleLevel) {
        self.previous = self.position;
        let age = self.age;
        self.age += 1;
        if age >= self.lifetime {
            self.removed = true;
            return;
        }
        self.velocity[1] -= 0.04 * f64::from(self.gravity);
        self.move_by(level, self.velocity);
        if self.speed_up_when_y_blocked && self.position[1] == self.previous[1] {
            self.velocity[0] *= 1.1;
            self.velocity[2] *= 1.1;
        }
        let f = f64::from(self.friction);
        for c in &mut self.velocity {
            *c *= f;
        }
        if self.on_ground {
            self.velocity[0] *= f64::from(0.7f32);
            self.velocity[2] *= f64::from(0.7f32);
        }
    }
    fn quad_size(&self, a: f32) -> f32 {
        let t = (self.age as f32 + a) / self.lifetime as f32;
        match self.size_curve {
            Size::Fixed => self.size,
            Size::GrowIn => self.size * (t * 32.0).clamp(0.0, 1.0),
            Size::Flame => self.size * (1.0 - t * t * 0.5),
            Size::Lava => self.size * (1.0 - t * t),
            Size::Portal => {
                let s = 1.0 - t;
                self.size * (1.0 - s * s)
            }
            Size::ReversePortal => {
                self.size * (1.0 - (self.age as f32 + a) / (self.lifetime as f32 * 1.5))
            }
        }
    }
}

/// World access for particle physics.
pub struct ParticleLevel<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    /// Local player feet position and vertical velocity (`PlayerCloudParticle`).
    pub player: Option<([f64; 3], f64)>,
}
impl ParticleLevel<'_> {
    fn collide(&self, bb: Aabb, d: [f64; 3]) -> [f64; 3] {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
        };
        match scene.resolve(bb, Vec3::new(d[0], d[1], d[2]), 0.0, false) {
            Ok(v) => [v.x, v.y, v.z],
            Err(_) => d,
        }
    }
    fn block_at(p: [f64; 3]) -> [i32; 3] {
        [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ]
    }
    /// The fluid at a block and `FluidState.getHeight`.
    fn fluid(&self, b: [i32; 3]) -> (Fluid, f64) {
        let s = self.facts.get(self.world.block(b[0], b[1], b[2]));
        if s.fluid == Fluid::None {
            return (Fluid::None, 0.0);
        }
        let above = self.facts.get(self.world.block(b[0], b[1] + 1, b[2]));
        let height = if above.fluid == s.fluid {
            1.0
        } else {
            f64::from(s.fluid_level) / 9.0
        };
        (s.fluid, height)
    }
    /// `getCollisionShape(...).max(Y, x, z)` for cached shapes.
    fn collision_top(&self, b: [i32; 3], fx: f64, fz: f64) -> f64 {
        let state = self.world.block(b[0], b[1], b[2]);
        match self.collision.cached_shapes().cached_shape(state) {
            Ok(Some(shape)) if !shape.is_empty() => shape.max_at(Axis::Y, fx, fz),
            Ok(Some(_)) => 0.0,
            _ => {
                if self.facts.get(state).full_collision {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// `ParticleEngine` plus the sprite sets of `ParticleResources`.
pub struct ParticleEngine {
    particles: Vec<Particle>,
    pending: Vec<Particle>,
    random: LegacyRandom,
    names: Vec<String>,
    rects: Vec<Option<Rect>>,
    atlas: HashMap<String, Rect>,
    sets: HashMap<ParticleType, u32>,
    set_sprites: Vec<Vec<u32>>,
    /// `TrackingEmitter`s: particle type, entity feet position, width, height, ticks left.
    emitters: Vec<(ParticleOptions, [f64; 3], f64, f64, u32)>,
    /// Camera position for the 32-block spawn limiter.
    pub camera: [f64; 3],
    /// Local sounds requested by particles (dripstone drips): (event, position, volume, pitch).
    pub sounds: Vec<(&'static str, [f64; 3], f32, f32)>,
}
/// `ParticleGroup.MAX_PARTICLES` (per render group; one group here).
const MAX_PARTICLES: usize = 16384;
impl ParticleEngine {
    pub fn new(root: &Path, atlas: &HashMap<String, Rect>) -> Self {
        let mut engine = Self {
            particles: Vec::new(),
            pending: Vec::new(),
            random: LegacyRandom::new(0x7061_7274),
            names: Vec::new(),
            rects: Vec::new(),
            atlas: atlas.clone(),
            sets: HashMap::new(),
            set_sprites: Vec::new(),
            emitters: Vec::new(),
            camera: [0.0; 3],
            sounds: Vec::new(),
        };
        let dir = root.join("assets/minecraft/particles");
        for &ty in ParticleType::ALL {
            let Ok(bytes) = std::fs::read(dir.join(format!("{}.json", ty.name()))) else {
                continue;
            };
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let sprites: Vec<u32> = json["textures"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .map(|t| {
                            engine
                                .intern(&format!("particle/{}", t.trim_start_matches("minecraft:")))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !sprites.is_empty() {
                engine.sets.insert(ty, engine.set_sprites.len() as u32);
                engine.set_sprites.push(sprites);
            }
        }
        engine
    }
    fn intern(&mut self, name: &str) -> u32 {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return i as u32;
        }
        self.names.push(name.to_owned());
        self.rects.push(self.atlas.get(name).copied());
        (self.names.len() - 1) as u32
    }
    pub fn len(&self) -> usize {
        self.particles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }
    pub fn clear(&mut self) {
        self.emitters.clear();
        self.particles.clear();
        self.pending.clear();
    }
    /// `ParticleEngine.add` (via `particlesToAdd`, applied on the next tick).
    pub fn push(&mut self, particle: Particle) {
        self.pending.push(particle);
    }
    /// Random generator shared by particle constructors and spawners.
    pub fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    /// `ClientLevel.addParticle`: dropped beyond 32 blocks of the camera unless
    /// the type overrides the limiter.
    pub fn add(&mut self, options: &ParticleOptions, p: [f64; 3], v: [f64; 3]) {
        let c = self.camera;
        let d2 = (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2);
        if !options.ty.override_limiter() && d2 > 1024.0 {
            return;
        }
        self.create(options, p, v);
    }
    /// `ClientPacketListener.handleExplosion` + `ClientExplosionTracker`: the
    /// explosion particle (emitter for radius >= 2) and up to 512 poof/smoke
    /// particles thrown from air inside the radius.
    pub fn explosion(
        &mut self,
        center: [f64; 3],
        radius: f32,
        block_count: usize,
        is_air: &dyn Fn([i32; 3]) -> bool,
    ) {
        let ty = if radius < 2.0 {
            ParticleType::Explosion
        } else {
            ParticleType::ExplosionEmitter
        };
        self.add(&ParticleOptions::simple(ty), center, [1.0, 0.0, 0.0]);
        for _ in 0..block_count.min(512) {
            let r = &mut self.random;
            let d = Vec3::new(
                f64::from(r.next_float() * 2.0 - 1.0),
                f64::from(r.next_float() * 2.0 - 1.0),
                f64::from(r.next_float() * 2.0 - 1.0),
            )
            .normalize();
            let distance = r.next_float().cbrt() * radius;
            let local = [
                d.x * f64::from(distance),
                d.y * f64::from(distance),
                d.z * f64::from(distance),
            ];
            let at = [
                center[0] + local[0],
                center[1] + local[1],
                center[2] + local[2],
            ];
            if !is_air(ParticleLevel::block_at(at)) {
                continue;
            }
            let speed = 0.5 / (distance / radius + 0.1) * r.next_float() * r.next_float() + 0.3;
            // Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES: poof (scaling 0.5) or smoke, equally weighted.
            let (ty, scaling) = if r.next_int_bound(2).unwrap_or(0) == 0 {
                (ParticleType::Poof, 0.5)
            } else {
                (ParticleType::Smoke, 1.0)
            };
            let p = [
                center[0] + local[0] * scaling,
                center[1] + local[1] * scaling,
                center[2] + local[2] * scaling,
            ];
            let v = [
                d.x * f64::from(speed),
                d.y * f64::from(speed),
                d.z * f64::from(speed),
            ];
            self.add(&ParticleOptions::simple(ty), p, v);
        }
    }
    /// `ParticleEngine.createTrackingEmitter` (crits): 16 tries per tick for
    /// three ticks inside a quarter of the entity's box, ticked once at creation.
    pub fn tracking_emitter(
        &mut self,
        options: ParticleOptions,
        feet: [f64; 3],
        width: f64,
        height: f64,
    ) {
        self.emit(&options, feet, width, height);
        self.emitters.push((options, feet, width, height, 2));
    }
    fn emit(&mut self, options: &ParticleOptions, feet: [f64; 3], width: f64, height: f64) {
        for _ in 0..16 {
            let r = &mut self.random;
            let xa = f64::from(r.next_float() * 2.0 - 1.0);
            let ya = f64::from(r.next_float() * 2.0 - 1.0);
            let za = f64::from(r.next_float() * 2.0 - 1.0);
            if xa * xa + ya * ya + za * za <= 1.0 {
                let p = [
                    feet[0] + width * xa / 4.0,
                    feet[1] + height * (0.5 + ya / 4.0),
                    feet[2] + width * za / 4.0,
                ];
                self.add(options, p, [xa, ya + 0.2, za]);
            }
        }
    }
    /// `ClientPacketListener.handleParticleEvent` for `ServerLevel.sendParticles`.
    pub fn send_particles(
        &mut self,
        options: &ParticleOptions,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: u32,
    ) {
        if count == 0 {
            let v = [speed * spread[0], speed * spread[1], speed * spread[2]];
            self.add(options, at, v);
            return;
        }
        for _ in 0..count {
            let r = &mut self.random;
            let p = [
                at[0] + r.next_gaussian() * spread[0],
                at[1] + r.next_gaussian() * spread[1],
                at[2] + r.next_gaussian() * spread[2],
            ];
            let v = [
                r.next_gaussian() * speed,
                r.next_gaussian() * speed,
                r.next_gaussian() * speed,
            ];
            self.add(options, p, v);
        }
    }
    /// `ParticleEngine.createParticle`: always spawned.
    pub fn create(&mut self, options: &ParticleOptions, p: [f64; 3], v: [f64; 3]) {
        if let Some(particle) = self.make(options, p, v) {
            self.pending.push(particle);
        }
    }
    /// `SpriteSet.get(random)`.
    fn sprite_from_set(&mut self, ty: ParticleType) -> Sprite {
        match self.sets.get(&ty) {
            Some(&set) => {
                let sprites = &self.set_sprites[set as usize];
                Sprite::Index(
                    sprites[self
                        .random
                        .next_int_bound(sprites.len() as i32)
                        .unwrap_or(0) as usize],
                )
            }
            None => Sprite::Index(u32::MAX),
        }
    }
    fn set_of(&self, ty: ParticleType) -> Option<u32> {
        self.sets.get(&ty).copied()
    }
    fn first(&self, ty: ParticleType) -> Sprite {
        self.set_of(ty).map_or(Sprite::Index(u32::MAX), |s| {
            Sprite::Index(self.set_sprites[s as usize][0])
        })
    }
    /// `SingleQuadParticle.setSpriteFromAge`.
    fn sprite_from_age(&self, p: &mut Particle) {
        if p.removed {
            return;
        }
        if let Some(set) = p.set {
            let sprites = &self.set_sprites[set as usize];
            let lifetime = p.lifetime.max(1);
            let index = (p.age.max(0) as i64 * (sprites.len() as i64 - 1) / i64::from(lifetime))
                .clamp(0, sprites.len() as i64 - 1);
            p.sprite = Sprite::Index(sprites[index as usize]);
        }
    }
    #[allow(clippy::too_many_lines)]
    fn make(&mut self, o: &ParticleOptions, p: [f64; 3], v: [f64; 3]) -> Option<Particle> {
        use ParticleType as T;
        let ty = o.ty;
        let set = self.set_of(ty);
        let r = &mut self.random;
        let mut particle = match ty {
            T::Block | T::BlockCrumble | T::DustPillar => {
                let (sprite, tint) = o.block.clone()?;
                let mut q = Particle::terrain(r, p, v, sprite, tint);
                q.ty = ty;
                if ty == T::BlockCrumble {
                    q.velocity = [0.0; 3];
                    q.lifetime = r.next_int_bound(10).unwrap_or(0) + 1;
                } else if ty == T::DustPillar {
                    q.velocity = [
                        r.next_gaussian() / 30.0,
                        v[1] + r.next_gaussian() / 2.0,
                        r.next_gaussian() / 30.0,
                    ];
                    q.lifetime = r.next_int_bound(20).unwrap_or(0) + 20;
                }
                return Some(q);
            }
            T::FallingDust => {
                let (_, tint) = o.block.clone()?;
                let mut q = Particle::at(r, ty, p);
                q.set_color(
                    ((tint >> 16) & 255) as f32 / 255.0,
                    ((tint >> 8) & 255) as f32 / 255.0,
                    (tint & 255) as f32 / 255.0,
                );
                q.size *= 0.674_999_95;
                let base = (32.0 / (r.next_float() * 0.8 + 0.2)) as i32;
                q.lifetime = ((base as f32 * 0.9) as i32).max(1);
                let rot_speed = (r.next_float() - 0.5) * 0.1;
                q.roll = r.next_float() * std::f32::consts::TAU;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::FallingDust { rot_speed };
                q.set = set;
                q
            }
            T::Smoke | T::LargeSmoke | T::WhiteSmoke | T::Ash | T::WhiteAsh => {
                // BaseAshSmokeParticle(dir, xa, scale, colorRandom, maxLifetime, gravity, hasPhysics).
                let (dir, scale, color_random, max_lifetime, gravity, physics, vel) = match ty {
                    T::Smoke | T::WhiteSmoke => ([0.1, 0.1, 0.1], 1.0, 0.3, 8, -0.1, true, v),
                    T::LargeSmoke => ([0.1, 0.1, 0.1], 2.5, 0.3, 8, -0.1, true, v),
                    T::Ash => ([0.1, -0.1, 0.1], 1.0, 0.5, 20, 0.1, false, [0.0; 3]),
                    _ => {
                        let xa = f64::from(r.next_float()) * -1.9 * f64::from(r.next_float()) * 0.1;
                        let ya = f64::from(r.next_float())
                            * -0.5
                            * f64::from(r.next_float())
                            * 0.1
                            * 5.0;
                        let za = f64::from(r.next_float()) * -1.9 * f64::from(r.next_float()) * 0.1;
                        ([0.1, -0.1, 0.1], 1.0, 0.0, 20, 0.0125, false, [xa, ya, za])
                    }
                };
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.96;
                q.gravity = gravity;
                q.speed_up_when_y_blocked = true;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(dir[i] as f32) + vel[i];
                }
                let col = r.next_float() * color_random;
                q.set_color(col, col, col);
                q.size *= 0.75 * scale;
                q.lifetime = ((f64::from(max_lifetime) / (f64::from(r.next_float()) * 0.8 + 0.2)
                    * f64::from(scale)) as i32)
                    .max(1);
                q.has_physics = physics;
                if ty == T::WhiteSmoke {
                    q.set_color(0.729_411_8, 0.694_117_67, 0.760_784_3);
                } else if ty == T::WhiteAsh {
                    q.set_color(186.0 / 255.0, 177.0 / 255.0, 194.0 / 255.0);
                }
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                q
            }
            T::Flame
            | T::SoulFireFlame
            | T::CopperFireFlame
            | T::SmallFlame
            | T::Soul
            | T::SculkSoul => {
                // RisingParticle.
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(0.01f32) + v[i];
                }
                for i in 0..3 {
                    q.position[i] += f64::from((r.next_float() - r.next_float()) * 0.05);
                }
                // RisingParticle moves x/y/z but not the bounding box.
                q.previous = p;
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32 + 4;
                if matches!(ty, T::Soul | T::SculkSoul) {
                    q.scale(1.5);
                    q.behaviour = Behaviour::Animated;
                    q.set = set;
                    if ty == T::SculkSoul {
                        q.light = Light::Block15;
                    }
                } else {
                    q.free_move = true;
                    q.size_curve = Size::Flame;
                    q.light = Light::AgeEmission;
                    if ty == T::SmallFlame {
                        q.scale(0.5);
                    }
                }
                q
            }
            T::Lava => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.gravity = 0.75;
                q.friction = 0.999;
                q.velocity[0] *= f64::from(0.8f32);
                q.velocity[1] *= f64::from(0.8f32);
                q.velocity[2] *= f64::from(0.8f32);
                q.velocity[1] = f64::from(r.next_float() * 0.4 + 0.05);
                q.size *= r.next_float() * 2.0 + 0.2;
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.size_curve = Size::Lava;
                q.light = Light::Block15;
                q.behaviour = Behaviour::Lava;
                q
            }
            T::DrippingWater
            | T::FallingWater
            | T::DrippingLava
            | T::FallingLava
            | T::LandingLava
            | T::DrippingHoney
            | T::FallingHoney
            | T::LandingHoney
            | T::FallingNectar
            | T::FallingSporeBlossom
            | T::DrippingObsidianTear
            | T::FallingObsidianTear
            | T::LandingObsidianTear
            | T::DrippingDripstoneWater
            | T::FallingDripstoneWater
            | T::DrippingDripstoneLava
            | T::FallingDripstoneLava => {
                let mut q = Particle::at(r, ty, p);
                q.set_size(0.01, 0.01);
                q.gravity = 0.06;
                let water = [0.2, 0.3, 1.0];
                let lava = [1.0, 0.285_714_3, 0.083_333_336];
                let tear = [0.511_718_75, 0.031_25, 0.890_625];
                let (fluid, stage, next, color, sound) = match ty {
                    T::DrippingWater => (
                        Fluid::Water,
                        DripStage::Hang,
                        Some(T::FallingWater),
                        Some(water),
                        None,
                    ),
                    T::FallingWater => (
                        Fluid::Water,
                        DripStage::FallAndLand,
                        Some(T::Splash),
                        Some(water),
                        None,
                    ),
                    T::DrippingLava => (
                        Fluid::Lava,
                        DripStage::CoolingHang,
                        Some(T::FallingLava),
                        None,
                        None,
                    ),
                    T::FallingLava => (
                        Fluid::Lava,
                        DripStage::FallAndLand,
                        Some(T::LandingLava),
                        Some(lava),
                        None,
                    ),
                    T::LandingLava => (Fluid::Lava, DripStage::Land, None, Some(lava), None),
                    T::DrippingHoney => (
                        Fluid::None,
                        DripStage::Hang,
                        Some(T::FallingHoney),
                        Some([0.622, 0.508, 0.082]),
                        None,
                    ),
                    T::FallingHoney => (
                        Fluid::None,
                        DripStage::FallAndLand,
                        Some(T::LandingHoney),
                        Some([0.582, 0.448, 0.082]),
                        Some("minecraft:block.beehive.drip"),
                    ),
                    T::LandingHoney => (
                        Fluid::None,
                        DripStage::Land,
                        None,
                        Some([0.522, 0.408, 0.082]),
                        None,
                    ),
                    T::FallingNectar => (
                        Fluid::None,
                        DripStage::Fall,
                        None,
                        Some([0.92, 0.782, 0.72]),
                        None,
                    ),
                    T::FallingSporeBlossom => (
                        Fluid::None,
                        DripStage::Fall,
                        None,
                        Some([0.32, 0.5, 0.22]),
                        None,
                    ),
                    T::DrippingObsidianTear => (
                        Fluid::None,
                        DripStage::Hang,
                        Some(T::FallingObsidianTear),
                        Some(tear),
                        None,
                    ),
                    T::FallingObsidianTear => (
                        Fluid::None,
                        DripStage::FallAndLand,
                        Some(T::LandingObsidianTear),
                        Some(tear),
                        None,
                    ),
                    T::LandingObsidianTear => {
                        (Fluid::None, DripStage::Land, None, Some(tear), None)
                    }
                    T::DrippingDripstoneWater => (
                        Fluid::Water,
                        DripStage::Hang,
                        Some(T::FallingDripstoneWater),
                        Some(water),
                        None,
                    ),
                    T::FallingDripstoneWater => (
                        Fluid::Water,
                        DripStage::FallAndLand,
                        Some(T::Splash),
                        Some(water),
                        Some("minecraft:block.pointed_dripstone.drip_water"),
                    ),
                    T::DrippingDripstoneLava => (
                        Fluid::Lava,
                        DripStage::CoolingHang,
                        Some(T::FallingDripstoneLava),
                        None,
                        None,
                    ),
                    _ => (
                        Fluid::Lava,
                        DripStage::FallAndLand,
                        Some(T::LandingLava),
                        Some(lava),
                        Some("minecraft:block.pointed_dripstone.drip_lava"),
                    ),
                };
                match stage {
                    DripStage::Hang | DripStage::CoolingHang => {
                        q.gravity *= 0.02;
                        q.lifetime = 40;
                    }
                    DripStage::FallAndLand => {
                        q.lifetime = (64.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32
                    }
                    DripStage::Land => {
                        q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32
                    }
                    DripStage::Fall => {}
                }
                match ty {
                    T::FallingHoney | T::FallingObsidianTear => q.gravity = 0.01,
                    T::DrippingHoney | T::DrippingObsidianTear => {
                        q.gravity *= 0.01;
                        q.lifetime = 100;
                    }
                    T::LandingHoney => {
                        q.lifetime = (128.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32
                    }
                    T::LandingObsidianTear => {
                        q.lifetime = (28.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32
                    }
                    T::FallingNectar => {
                        q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                        q.gravity = 0.007;
                    }
                    T::FallingSporeBlossom => {
                        q.lifetime = (64.0 / (r.next_float() * (0.9 - 0.1) + 0.1)) as i32;
                        q.gravity = 0.005;
                    }
                    _ => {}
                }
                if matches!(
                    ty,
                    T::DrippingObsidianTear | T::FallingObsidianTear | T::LandingObsidianTear
                ) {
                    q.light = Light::Block15;
                }
                if let Some(c) = color {
                    q.set_color(c[0], c[1], c[2]);
                }
                q.behaviour = Behaviour::Drip {
                    fluid,
                    stage,
                    next,
                    sound,
                };
                q
            }
            T::Rain | T::Splash => {
                // WaterDropParticle.
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.velocity[0] *= f64::from(0.3f32);
                q.velocity[1] = f64::from(r.next_float() * 0.2 + 0.1);
                q.velocity[2] *= f64::from(0.3f32);
                q.set_size(0.01, 0.01);
                q.gravity = 0.06;
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                if ty == T::Splash {
                    q.gravity = 0.04;
                    if v[1] == 0.0 && (v[0] != 0.0 || v[2] != 0.0) {
                        q.velocity = [v[0], 0.1, v[2]];
                    }
                }
                q.behaviour = Behaviour::WaterDrop;
                q
            }
            T::Bubble | T::BubbleColumnUp => {
                let mut q = Particle::at(r, ty, p);
                q.set_size(0.02, 0.02);
                q.size *= r.next_float() * 0.6 + 0.2;
                for i in 0..3 {
                    q.velocity[i] =
                        v[i] * f64::from(0.2f32) + f64::from((r.next_float() * 2.0 - 1.0) * 0.02);
                }
                if ty == T::Bubble {
                    q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                    q.behaviour = Behaviour::Bubble;
                } else {
                    q.gravity = -0.125;
                    q.friction = 0.85;
                    q.lifetime = (40.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                    q.behaviour = Behaviour::BubbleColumnUp;
                }
                q
            }
            T::BubblePop => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = 4;
                q.gravity = 0.008;
                q.velocity = v;
                q.behaviour = Behaviour::BubblePop;
                q.set = set;
                q
            }
            T::CurrentDown => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = (r.next_float() * 60.0) as i32 + 30;
                q.has_physics = false;
                q.velocity = [0.0, -0.05, 0.0];
                q.set_size(0.02, 0.02);
                q.size *= r.next_float() * 0.6 + 0.2;
                q.gravity = 0.002;
                q.behaviour = Behaviour::CurrentDown { angle: 0.0 };
                q
            }
            T::Underwater | T::CrimsonSpore | T::WarpedSpore | T::SporeBlossomAir => {
                let below = [p[0], p[1] - 0.125, p[2]];
                let mut q = if ty == T::Underwater {
                    let mut q = Particle::at(r, ty, below);
                    q.set_size(0.01, 0.01);
                    q.size *= r.next_float() * 0.6 + 0.2;
                    q
                } else {
                    let vel = match ty {
                        T::CrimsonSpore => [
                            r.next_gaussian() * 1.0e-6,
                            r.next_gaussian() * 1.0e-4,
                            r.next_gaussian() * 1.0e-6,
                        ],
                        T::WarpedSpore => [
                            0.0,
                            f64::from(r.next_float()) * -1.9 * f64::from(r.next_float()) * 0.1,
                            0.0,
                        ],
                        _ => [0.0, f64::from(-0.8f32), 0.0],
                    };
                    let mut q = Particle::moving(r, ty, below, vel);
                    q.set_size(0.01, 0.01);
                    q.size *= r.next_float() * 0.6 + 0.6;
                    q
                };
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                q.friction = 1.0;
                q.gravity = 0.0;
                match ty {
                    T::Underwater => q.set_color(0.4, 0.4, 0.7),
                    T::CrimsonSpore => q.set_color(0.9, 0.4, 0.5),
                    T::WarpedSpore => {
                        q.set_color(0.1, 0.1, 0.3);
                        q.set_size(0.001, 0.001);
                    }
                    _ => {
                        q.lifetime = r.next_int_bound(501).unwrap_or(0) + 500;
                        q.gravity = 0.01;
                        q.set_color(0.32, 0.5, 0.22);
                    }
                }
                q
            }
            T::Portal | T::ReversePortal => {
                let mut q = Particle::at(r, ty, p);
                q.velocity = v;
                q.size = 0.1 * (r.next_float() * 0.2 + 0.5);
                let br = r.next_float() * 0.6 + 0.4;
                q.set_color(br * 0.9, br * 0.3, br);
                q.lifetime = (r.next_float() * 10.0) as i32 + 40;
                q.free_move = true;
                q.light = Light::AgeEmission4;
                if ty == T::Portal {
                    q.size_curve = Size::Portal;
                    q.behaviour = Behaviour::Portal { start: p };
                } else {
                    q.size *= 1.5;
                    q.lifetime = (r.next_float() * 2.0) as i32 + 60;
                    q.size_curve = Size::ReversePortal;
                    q.behaviour = Behaviour::ReversePortal;
                }
                q
            }
            T::Enchant | T::Nautilus | T::VaultConnection => {
                let mut q = Particle::at(r, ty, p);
                let alpha = if ty == T::VaultConnection {
                    [0.0, 0.6, 0.25, 1.0]
                } else {
                    [1.0, 1.0, 0.0, 1.0]
                };
                q.rgba[3] = alpha[0];
                q.velocity = v;
                let start = [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
                q.previous = start;
                q.set_pos(start);
                q.position = start;
                q.size = 0.1 * (r.next_float() * 0.5 + 0.2);
                let br = r.next_float() * 0.6 + 0.4;
                q.set_color(0.9 * br, 0.9 * br, br);
                q.has_physics = false;
                q.lifetime = (r.next_float() * 10.0) as i32 + 30;
                q.free_move = true;
                q.light = if ty == T::VaultConnection {
                    Light::Block15
                } else {
                    Light::AgeEmission4
                };
                q.behaviour = Behaviour::FlyTowards { start: p, alpha };
                if ty == T::VaultConnection {
                    q.scale(1.5);
                }
                q
            }
            T::Crit | T::EnchantedHit | T::DamageIndicator => {
                let vel = if ty == T::DamageIndicator {
                    [v[0], v[1] + 1.0, v[2]]
                } else {
                    v
                };
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.7;
                q.gravity = 0.5;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(0.1f32) + vel[i] * 0.4;
                }
                let col = r.next_float() * 0.3 + 0.6;
                q.set_color(col, col, col);
                q.size *= 0.75;
                q.lifetime = ((6.0 / (f64::from(r.next_float()) * 0.8 + 0.6)) as i32).max(1);
                q.has_physics = false;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Crit;
                q.pre_tick = true;
                q
            }
            T::Heart | T::AngryVillager => {
                let at = if ty == T::AngryVillager {
                    [p[0], p[1] + 0.5, p[2]]
                } else {
                    p
                };
                let mut q = Particle::moving(r, ty, at, [0.0; 3]);
                q.speed_up_when_y_blocked = true;
                q.friction = 0.86;
                for c in &mut q.velocity {
                    *c *= f64::from(0.01f32);
                }
                q.velocity[1] += 0.1;
                q.size *= 1.5;
                q.lifetime = 16;
                q.has_physics = false;
                q.size_curve = Size::GrowIn;
                q
            }
            T::HappyVillager | T::Composter | T::Mycelium | T::Dolphin | T::EggCrack => {
                let mut q = Particle::moving(r, ty, p, v);
                let br = r.next_float() * 0.1 + 0.2;
                q.set_color(br, br, br);
                q.set_size(0.02, 0.02);
                q.size *= r.next_float() * 0.6 + 0.5;
                for c in &mut q.velocity {
                    *c *= f64::from(0.02f32);
                }
                q.lifetime = (20.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.free_move = true;
                q.behaviour = Behaviour::SuspendedTown;
                match ty {
                    T::HappyVillager | T::EggCrack => q.set_color(1.0, 1.0, 1.0),
                    T::Composter => {
                        q.set_color(1.0, 1.0, 1.0);
                        q.lifetime = 3 + r.next_int_bound(5).unwrap_or(0);
                    }
                    T::Dolphin => {
                        q.set_color(0.3, 0.5, 1.0);
                        q.rgba[3] = 1.0 - r.next_float() * 0.7;
                        q.lifetime /= 2;
                    }
                    _ => {}
                }
                q
            }
            T::Poof | T::Snowflake => {
                let mut q = Particle::at(r, ty, p);
                if ty == T::Poof {
                    q.gravity = -0.1;
                    q.friction = 0.9;
                } else {
                    q.gravity = 0.225;
                    q.friction = 1.0;
                }
                for i in 0..3 {
                    q.velocity[i] = v[i] + f64::from((r.next_float() * 2.0 - 1.0) * 0.05);
                }
                if ty == T::Poof {
                    let col = r.next_float() * 0.3 + 0.7;
                    q.set_color(col, col, col);
                    q.size = 0.1 * (r.next_float() * r.next_float() * 6.0 + 1.0);
                } else {
                    q.size = 0.1 * (r.next_float() * r.next_float() + 1.0);
                }
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32 + 2;
                q.behaviour = if ty == T::Poof {
                    Behaviour::Animated
                } else {
                    Behaviour::Snowflake
                };
                if ty == T::Snowflake {
                    q.set_color(0.923, 0.964, 0.999);
                }
                q.set = set;
                q
            }
            T::Explosion => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.lifetime = 6 + r.next_int_bound(4).unwrap_or(0);
                let col = r.next_float() * 0.6 + 0.4;
                q.set_color(col, col, col);
                q.size = 2.0 * (1.0 - v[0] as f32 * 0.5);
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            T::ExplosionEmitter => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.lifetime = 8;
                q.no_render = true;
                q.behaviour = Behaviour::ExplosionSeed;
                q
            }
            T::CampfireCosySmoke | T::CampfireSignalSmoke => {
                let mut q = Particle::at(r, ty, p);
                q.scale(3.0);
                q.set_size(0.25, 0.25);
                q.lifetime = r.next_int_bound(50).unwrap_or(0)
                    + if ty == T::CampfireSignalSmoke {
                        280
                    } else {
                        80
                    };
                q.gravity = 3.0e-6;
                q.velocity = [v[0], v[1] + f64::from(r.next_float() / 500.0), v[2]];
                q.rgba[3] = if ty == T::CampfireSignalSmoke {
                    0.95
                } else {
                    0.9
                };
                q.behaviour = Behaviour::Campfire;
                q
            }
            T::Note => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.66;
                q.speed_up_when_y_blocked = true;
                for c in &mut q.velocity {
                    *c *= f64::from(0.01f32);
                }
                q.velocity[1] += 0.2;
                let color = v[0] as f32;
                let channel = |offset: f32| {
                    ((color + offset) * std::f32::consts::TAU)
                        .sin()
                        .mul_add(0.65, 0.35)
                        .max(0.0)
                };
                q.set_color(channel(0.0), channel(1.0 / 3.0), channel(2.0 / 3.0));
                q.size *= 1.5;
                q.lifetime = 6;
                q.size_curve = Size::GrowIn;
                q
            }
            T::Dust => {
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                q.speed_up_when_y_blocked = true;
                for c in &mut q.velocity {
                    *c *= f64::from(0.1f32);
                }
                q.size *= 0.75 * o.scale;
                let base = (8.0 / (r.next_double() * 0.8 + 0.2)) as i32;
                q.lifetime = ((base as f32 * o.scale).max(1.0)) as i32;
                let factor = r.next_float() * 0.4 + 0.6;
                let mut randomize = |c: f32| (r.next_float() * 0.2 + 0.8) * c * factor;
                let rgb = [
                    randomize(o.color[0]),
                    randomize(o.color[1]),
                    randomize(o.color[2]),
                ];
                q.set_color(rgb[0], rgb[1], rgb[2]);
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                q
            }
            T::TotemOfUndying | T::EndRod => {
                let mut q = Particle::at(r, ty, p);
                q.friction = 0.91;
                q.gravity = if ty == T::TotemOfUndying {
                    1.25
                } else {
                    0.0125
                };
                q.velocity = v;
                q.size *= 0.75;
                q.lifetime = 60 + r.next_int_bound(12).unwrap_or(0);
                let mut fade = None;
                if ty == T::TotemOfUndying {
                    q.friction = 0.6;
                    if r.next_int_bound(4).unwrap_or(1) == 0 {
                        q.set_color(
                            0.6 + r.next_float() * 0.2,
                            0.6 + r.next_float() * 0.3,
                            r.next_float() * 0.2,
                        );
                    } else {
                        q.set_color(
                            0.1 + r.next_float() * 0.2,
                            0.4 + r.next_float() * 0.3,
                            r.next_float() * 0.2,
                        );
                    }
                } else {
                    fade = Some([
                        0xF2 as f32 / 255.0,
                        0xDE as f32 / 255.0,
                        0xC9 as f32 / 255.0,
                    ]);
                    q.free_move = true;
                }
                q.light = Light::FullBright;
                q.behaviour = Behaviour::SimpleAnimated { fade };
                q.set = set;
                q
            }
            T::CherryLeaves | T::PaleOakLeaves | T::TintedLeaves => {
                let (fall, side, swirl, flow, scale, start) = if ty == T::CherryLeaves {
                    (0.25f32, 2.0f32, false, true, 1.0f32, 0.0f32)
                } else {
                    (0.07, 10.0, true, false, 2.0, 0.021)
                };
                let mut q = Particle::at(r, ty, p);
                let rot_speed = (if r.next_boolean() { -30.0f32 } else { 30.0 }).to_radians();
                let spin = (if r.next_boolean() { -5.0f32 } else { 5.0 }).to_radians();
                q.lifetime = 300;
                q.gravity = fall * 1.2 * 0.0025;
                let s = scale * if r.next_boolean() { 0.05 } else { 0.075 };
                q.size = s;
                q.set_size(s, s);
                q.friction = 1.0;
                q.velocity[1] = -f64::from(start);
                let pr = r.next_float();
                let flow_scale = [
                    f64::from(pr * 60.0).to_radians().cos() * f64::from(side),
                    f64::from(pr * 60.0).to_radians().sin() * f64::from(side),
                ];
                let swirl_period = f64::from(1000.0 + pr * 3000.0).to_radians();
                if ty == T::TintedLeaves {
                    q.set_color(o.color[0], o.color[1], o.color[2]);
                }
                q.behaviour = Behaviour::Leaves {
                    rot_speed,
                    spin,
                    wind: side,
                    swirl,
                    flow,
                    flow_scale,
                    swirl_period,
                };
                q
            }
            T::Effect | T::InstantEffect | T::EntityEffect | T::Witch | T::Infested => {
                // SpellParticle: x/z velocity from a shared random, y from the option.
                let xa = 0.5 - r.next_double();
                let za = 0.5 - r.next_double();
                let mut q = Particle::moving(r, ty, p, [xa, v[1], za]);
                q.friction = 0.96;
                q.gravity = -0.1;
                q.speed_up_when_y_blocked = true;
                q.velocity[1] *= f64::from(0.2f32);
                if v[0] == 0.0 && v[2] == 0.0 {
                    q.velocity[0] *= f64::from(0.1f32);
                    q.velocity[2] *= f64::from(0.1f32);
                }
                q.size *= 0.75;
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                match ty {
                    T::Witch => {
                        let b = r.next_float() * 0.5 + 0.35;
                        q.set_color(b, 0.0, b);
                    }
                    T::EntityEffect => {
                        q.set_color(o.color[0], o.color[1], o.color[2]);
                        q.rgba[3] = o.color[3];
                    }
                    T::Effect | T::InstantEffect => {
                        q.set_color(o.color[0], o.color[1], o.color[2]);
                        q.set_power(o.scale);
                    }
                    _ => {}
                }
                q.behaviour = Behaviour::Spell {
                    original_alpha: q.rgba[3],
                };
                q.set = set;
                q
            }
            T::Cloud | T::Sneeze => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.96;
                for i in 0..3 {
                    q.velocity[i] = q.velocity[i] * f64::from(0.1f32) + v[i];
                }
                let col = 1.0 - r.next_float() * 0.3;
                q.set_color(col, col, col);
                q.size *= 1.875;
                let base = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.3)) as i32;
                q.lifetime = ((base as f32 * 2.5).max(1.0)) as i32;
                q.has_physics = false;
                if ty == T::Sneeze {
                    q.set_color(0.22, 1.0, 0.53);
                    q.rgba[3] = 0.4;
                }
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Cloud;
                q.set = set;
                q
            }
            T::DragonBreath => {
                let mut q = Particle::at(r, ty, p);
                q.friction = 0.96;
                q.velocity = v;
                q.set_color(
                    r.next_float() * (0.874_509_8 - 0.717_647_1) + 0.717_647_1,
                    0.0,
                    r.next_float() * (0.976_470_6 - 0.823_529_4) + 0.823_529_4,
                );
                q.size *= 0.75;
                q.lifetime = (20.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::DragonBreath { hit_ground: false };
                q.set = set;
                q.set_power(o.scale);
                q
            }
        };
        if particle.set.is_some() {
            particle.sprite = self.first(ty);
            self.sprite_from_age(&mut particle);
        } else if matches!(particle.sprite, Sprite::Index(u32::MAX)) {
            particle.sprite = self.sprite_from_set(ty);
        }
        Some(particle)
    }

    /// `ParticleEngine.tick`: tick live particles, then admit queued ones.
    pub fn tick(&mut self, level: &ParticleLevel) {
        for (options, feet, width, height, life) in std::mem::take(&mut self.emitters) {
            self.emit(&options, feet, width, height);
            if life > 1 {
                self.emitters.push((options, feet, width, height, life - 1));
            }
        }
        let mut particles = std::mem::take(&mut self.particles);
        for p in &mut particles {
            self.tick_particle(p, level);
        }
        particles.retain(|p| !p.removed);
        self.particles = particles;
        let pending = std::mem::take(&mut self.pending);
        for mut p in pending {
            if p.pre_tick {
                p.pre_tick = false;
                self.tick_particle(&mut p, level);
            }
            // ParticleGroup.add: the last quarter fills with decreasing probability.
            let size = self.particles.len();
            if size >= MAX_PARTICLES {
                continue;
            }
            if size >= 12288 {
                let free = (MAX_PARTICLES - size) as f32 / 4096.0;
                if self.random.next_float() >= free * free {
                    continue;
                }
            }
            self.particles.push(p);
        }
    }
    #[allow(clippy::too_many_lines)]
    fn tick_particle(&mut self, p: &mut Particle, level: &ParticleLevel) {
        match p.behaviour {
            Behaviour::Base => p.base_tick(level),
            Behaviour::Animated => {
                p.base_tick(level);
                self.sprite_from_age(p);
            }
            Behaviour::Snowflake => {
                p.base_tick(level);
                self.sprite_from_age(p);
                p.velocity[0] *= f64::from(0.95f32);
                p.velocity[1] *= f64::from(0.9f32);
                p.velocity[2] *= f64::from(0.95f32);
            }
            Behaviour::Lava => {
                p.base_tick(level);
                if !p.removed {
                    let odds = p.age as f32 / p.lifetime as f32;
                    if self.random.next_float() > odds {
                        self.add(
                            &ParticleOptions::simple(ParticleType::Smoke),
                            p.position,
                            p.velocity,
                        );
                    }
                }
            }
            Behaviour::Crit => {
                p.base_tick(level);
                p.rgba[1] *= 0.96;
                p.rgba[2] *= 0.9;
            }
            Behaviour::Drip {
                fluid,
                stage,
                next,
                sound,
            } => {
                p.previous = p.position;
                // preMoveUpdate
                if stage == DripStage::CoolingHang {
                    p.rgba[0] = 1.0;
                    p.rgba[1] = 16.0 / (40 - p.lifetime + 16) as f32;
                    p.rgba[2] = 4.0 / (40 - p.lifetime + 8) as f32;
                }
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    if matches!(stage, DripStage::Hang | DripStage::CoolingHang)
                        && let Some(next) = next
                    {
                        self.add(&ParticleOptions::simple(next), p.position, p.velocity);
                    }
                }
                if p.removed {
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                // postMoveUpdate
                match stage {
                    DripStage::Hang | DripStage::CoolingHang => {
                        for c in &mut p.velocity {
                            *c *= 0.02;
                        }
                    }
                    DripStage::Fall if p.on_ground => p.removed = true,
                    DripStage::FallAndLand if p.on_ground => {
                        p.removed = true;
                        if let Some(next) = next {
                            self.add(&ParticleOptions::simple(next), p.position, [0.0; 3]);
                        }
                        if let Some(sound) = sound {
                            let volume = self.random.next_float() * 0.7 + 0.3;
                            self.sounds.push((sound, p.position, volume, 1.0));
                        }
                    }
                    _ => {}
                }
                if p.removed {
                    return;
                }
                for c in &mut p.velocity {
                    *c *= f64::from(0.98f32);
                }
                if fluid != Fluid::None {
                    let b = ParticleLevel::block_at(p.position);
                    let (f, height) = level.fluid(b);
                    if f == fluid && p.position[1] < f64::from(b[1]) + height {
                        p.removed = true;
                    }
                }
            }
            Behaviour::WaterDrop => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= f64::from(0.98f32);
                }
                if p.on_ground {
                    if self.random.next_float() < 0.5 {
                        p.removed = true;
                    }
                    p.velocity[0] *= f64::from(0.7f32);
                    p.velocity[2] *= f64::from(0.7f32);
                }
                let b = ParticleLevel::block_at(p.position);
                let top = level.collision_top(
                    b,
                    p.position[0] - f64::from(b[0]),
                    p.position[2] - f64::from(b[2]),
                );
                let offset = top.max(level.fluid(b).1);
                if offset > 0.0 && p.position[1] < f64::from(b[1]) + offset {
                    p.removed = true;
                }
            }
            Behaviour::Bubble => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.velocity[1] += 0.002;
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= f64::from(0.85f32);
                }
                if level.fluid(ParticleLevel::block_at(p.position)).0 != Fluid::Water {
                    p.removed = true;
                }
            }
            Behaviour::BubbleColumnUp => {
                p.base_tick(level);
                if !p.removed && level.fluid(ParticleLevel::block_at(p.position)).0 != Fluid::Water
                {
                    p.removed = true;
                }
            }
            Behaviour::BubblePop => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                p.velocity[1] -= f64::from(p.gravity);
                p.move_by(level, p.velocity);
                self.sprite_from_age(p);
            }
            Behaviour::CurrentDown { angle } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                p.velocity[0] += f64::from(0.6 * angle.cos());
                p.velocity[2] += f64::from(0.6 * angle.sin());
                p.velocity[0] *= 0.07;
                p.velocity[2] *= 0.07;
                p.move_by(level, p.velocity);
                if level.fluid(ParticleLevel::block_at(p.position)).0 != Fluid::Water || p.on_ground
                {
                    p.removed = true;
                }
                p.behaviour = Behaviour::CurrentDown {
                    angle: angle + 0.08,
                };
            }
            Behaviour::SuspendedTown => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                    return;
                }
                p.move_by(level, p.velocity);
                for c in &mut p.velocity {
                    *c *= 0.99;
                }
            }
            Behaviour::Portal { start } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let a = p.age as f32 / p.lifetime as f32;
                let pos = 1.0 - (-a + a * a * 2.0);
                p.position = [
                    start[0] + p.velocity[0] * f64::from(pos),
                    start[1] + p.velocity[1] * f64::from(pos) + f64::from(1.0 - a),
                    start[2] + p.velocity[2] * f64::from(pos),
                ];
            }
            Behaviour::ReversePortal => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let m = f64::from(p.age as f32 / p.lifetime as f32);
                for i in 0..3 {
                    p.position[i] += p.velocity[i] * m;
                }
            }
            Behaviour::FlyTowards { start, .. } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let pos = 1.0 - p.age as f32 / p.lifetime as f32;
                let mut pp = 1.0 - pos;
                pp *= pp;
                pp *= pp;
                p.position = [
                    start[0] + p.velocity[0] * f64::from(pos),
                    start[1] + p.velocity[1] * f64::from(pos) - f64::from(pp * 1.2),
                    start[2] + p.velocity[2] * f64::from(pos),
                ];
            }
            Behaviour::HugeExplosion => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                self.sprite_from_age(p);
            }
            Behaviour::ExplosionSeed => {
                for _ in 0..6 {
                    let r = &mut self.random;
                    let at = [
                        p.position[0] + (r.next_double() - r.next_double()) * 4.0,
                        p.position[1] + (r.next_double() - r.next_double()) * 4.0,
                        p.position[2] + (r.next_double() - r.next_double()) * 4.0,
                    ];
                    let f = f64::from(p.age as f32 / p.lifetime as f32);
                    self.add(
                        &ParticleOptions::simple(ParticleType::Explosion),
                        at,
                        [f, 0.0, 0.0],
                    );
                }
                p.age += 1;
                if p.age == p.lifetime {
                    p.removed = true;
                }
            }
            Behaviour::Campfire => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age < p.lifetime && p.rgba[3] > 0.0 {
                    let r = &mut self.random;
                    let dx = r.next_float() / 5000.0 * if r.next_boolean() { 1.0 } else { -1.0 };
                    p.velocity[0] += f64::from(dx);
                    let dz = r.next_float() / 5000.0 * if r.next_boolean() { 1.0 } else { -1.0 };
                    p.velocity[2] += f64::from(dz);
                    p.velocity[1] -= f64::from(p.gravity);
                    p.move_by(level, p.velocity);
                    if p.age >= p.lifetime - 60 && p.rgba[3] > 0.01 {
                        p.rgba[3] -= 0.015;
                    }
                } else {
                    p.removed = true;
                }
            }
            Behaviour::SimpleAnimated { fade } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                if p.age > p.lifetime / 2 {
                    p.rgba[3] = 1.0 - (p.age as f32 - (p.lifetime / 2) as f32) / p.lifetime as f32;
                    if let Some(f) = fade {
                        for i in 0..3 {
                            p.rgba[i] += (f[i] - p.rgba[i]) * 0.2;
                        }
                    }
                }
            }
            Behaviour::Spell { original_alpha } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                p.rgba[3] += (original_alpha - p.rgba[3]) * 0.05;
            }
            Behaviour::Cloud => {
                p.base_tick(level);
                if !p.removed {
                    self.sprite_from_age(p);
                    if let Some((player, vy)) = level.player {
                        let d2 = (player[0] - p.position[0]).powi(2)
                            + (player[1] - p.position[1]).powi(2)
                            + (player[2] - p.position[2]).powi(2);
                        if d2 < 4.0 && p.position[1] > player[1] {
                            let y = p.position[1] + (player[1] - p.position[1]) * 0.2;
                            p.velocity[1] += (vy - p.velocity[1]) * 0.2;
                            p.set_pos([p.position[0], y, p.position[2]]);
                        }
                    }
                }
            }
            Behaviour::Leaves {
                mut rot_speed,
                spin,
                wind,
                swirl,
                flow,
                flow_scale,
                swirl_period,
            } => {
                p.previous = p.position;
                let lifetime = p.lifetime;
                p.lifetime -= 1;
                if lifetime <= 0 {
                    p.removed = true;
                }
                if !p.removed {
                    let alive = (300 - p.lifetime) as f32;
                    let relative = (alive / 300.0).min(1.0);
                    let (mut xa, mut za) = (0.0f64, 0.0f64);
                    if flow {
                        xa += flow_scale[0] * f64::from(relative).powf(1.25);
                        za += flow_scale[1] * f64::from(relative).powf(1.25);
                    }
                    if swirl {
                        let rel = f64::from(relative);
                        xa += rel * (rel * swirl_period).cos() * f64::from(wind);
                        za += rel * (rel * swirl_period).sin() * f64::from(wind);
                    }
                    p.velocity[0] += xa * f64::from(0.0025f32);
                    p.velocity[2] += za * f64::from(0.0025f32);
                    p.velocity[1] -= f64::from(p.gravity);
                    rot_speed += spin / 20.0;
                    p.o_roll = p.roll;
                    p.roll += rot_speed / 20.0;
                    p.move_by(level, p.velocity);
                    if p.on_ground
                        || p.lifetime < 299 && (p.velocity[0] == 0.0 || p.velocity[2] == 0.0)
                    {
                        p.removed = true;
                    }
                    if !p.removed {
                        let f = f64::from(p.friction);
                        for c in &mut p.velocity {
                            *c *= f;
                        }
                    }
                }
                p.behaviour = Behaviour::Leaves {
                    rot_speed,
                    spin,
                    wind,
                    swirl,
                    flow,
                    flow_scale,
                    swirl_period,
                };
            }
            Behaviour::FallingDust { rot_speed } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                self.sprite_from_age(p);
                p.o_roll = p.roll;
                p.roll += std::f32::consts::PI * rot_speed * 2.0;
                if p.on_ground {
                    p.o_roll = 0.0;
                    p.roll = 0.0;
                }
                p.move_by(level, p.velocity);
                p.velocity[1] -= f64::from(0.003f32);
                p.velocity[1] = p.velocity[1].max(f64::from(-0.14f32));
            }
            Behaviour::DragonBreath { mut hit_ground } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                self.sprite_from_age(p);
                if p.on_ground {
                    p.velocity[1] = 0.0;
                    hit_ground = true;
                }
                if hit_ground {
                    p.velocity[1] += 0.002;
                }
                p.move_by(level, p.velocity);
                if p.position[1] == p.previous[1] {
                    p.velocity[0] *= 1.1;
                    p.velocity[2] *= 1.1;
                }
                let f = f64::from(p.friction);
                p.velocity[0] *= f;
                p.velocity[2] *= f;
                if hit_ground {
                    p.velocity[1] *= f;
                }
                p.behaviour = Behaviour::DragonBreath { hit_ground };
            }
        }
    }

    /// Camera-facing quads (`QuadParticleRenderState.renderRotatedQuad`).
    pub fn render(
        &self,
        camera: DVec3,
        yaw: f32,
        pitch: f32,
        partial: f32,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        // Camera.setRotation: rotationYXZ(PI - yRot, -xRot, 0).
        let base = Quat::from_rotation_y(std::f32::consts::PI - yaw.to_radians())
            * Quat::from_rotation_x(-pitch.to_radians());
        for p in &self.particles {
            if p.no_render {
                continue;
            }
            let rect = match &p.sprite {
                Sprite::Index(i) => self.rects.get(*i as usize).copied().flatten(),
                Sprite::Named(name) => self.atlas.get(name).copied(),
            };
            let Some(r) = rect else { continue };
            let a = f64::from(partial);
            let pos = DVec3::new(
                p.previous[0] + (p.position[0] - p.previous[0]) * a,
                p.previous[1] + (p.position[1] - p.previous[1]) * a,
                p.previous[2] + (p.position[2] - p.previous[2]) * a,
            ) - camera;
            let center = pos.as_vec3();
            let rotation = if p.roll != 0.0 {
                base * Quat::from_rotation_z(p.o_roll + (p.roll - p.o_roll) * partial)
            } else {
                base
            };
            let size = p.quad_size(partial);
            let mut alpha = p.rgba[3];
            if let Behaviour::FlyTowards { alpha: la, .. } = p.behaviour {
                alpha = lifetime_alpha(la, p.age, p.lifetime, partial);
            }
            let block = ParticleLevel::block_at(p.position);
            let packed = light_at(block);
            let (sky, mut block_light) = (u32::from(packed >> 4) * 16, u32::from(packed & 15) * 16);
            let mut sky_light = sky;
            let t = ((p.age as f32 + partial) / p.lifetime as f32).clamp(0.0, 1.0);
            match p.light {
                Light::World => {}
                Light::FullBright => {
                    block_light = 240;
                    sky_light = 240;
                }
                Light::Block15 => block_light = 240,
                Light::AgeEmission => block_light = (block_light + (t * 240.0) as u32).min(240),
                Light::AgeEmission4 => {
                    let b = p.age as f32 / p.lifetime as f32;
                    let b = (b * b) * (b * b);
                    block_light = (block_light + (b.clamp(0.0, 1.0) * 240.0) as u32).min(240);
                }
            }
            let light = sky_light | block_light << 8;
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u32;
            let color = c(alpha) << 24 | c(p.rgba[2]) << 16 | c(p.rgba[1]) << 8 | c(p.rgba[0]);
            let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
            let (u0, v0, u1, v1) = (
                r.u0 + du * p.uv[0],
                r.v0 + dv * p.uv[1],
                r.u0 + du * p.uv[2],
                r.v0 + dv * p.uv[3],
            );
            for (n, uv) in [
                ((1.0, -1.0), [u1, v1]),
                ((1.0, 1.0), [u1, v0]),
                ((-1.0, 1.0), [u0, v0]),
                ((-1.0, -1.0), [u0, v1]),
            ] {
                let v = center + rotation * FVec3::new(n.0, n.1, 0.0) * size;
                out.push(Vertex {
                    pos: v.to_array(),
                    uv,
                    layer: 0,
                    color,
                    light,
                });
            }
        }
    }
}
/// `Particle.LifetimeAlpha.currentAlphaForAge`.
fn lifetime_alpha(a: [f32; 4], age: i32, lifetime: i32, partial: f32) -> f32 {
    if (a[0] - a[1]).abs() < 1.0e-5 {
        return a[0];
    }
    let t = ((age as f32 + partial) / lifetime as f32 - a[2]) / (a[3] - a[2]);
    a[0] + (a[1] - a[0]) * t.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn particle_names_round_trip() {
        for &ty in ParticleType::ALL {
            assert_eq!(
                ParticleType::from_name(&format!("minecraft:{}", ty.name())),
                Some(ty)
            );
        }
        assert_eq!(ParticleType::from_name("minecraft:not_a_particle"), None);
    }
    #[test]
    fn terrain_crumb_samples_a_mirrored_quarter() {
        let mut r = LegacyRandom::new(1);
        let p = Particle::terrain(&mut r, [0.5; 3], [0.0; 3], "block/stone".into(), 0xFFFFFF);
        assert!((p.uv[0] - p.uv[2] - 0.25).abs() < 1e-6);
        assert!((p.uv[3] - p.uv[1] - 0.25).abs() < 1e-6);
        assert!((p.rgba[0] - 0.6).abs() < 1e-6);
        // The initial speed is normalized to 0.15..0.45 * 0.4 plus 0.1 upwards.
        let s =
            (p.velocity[0].powi(2) + (p.velocity[1] - 0.1).powi(2) + p.velocity[2].powi(2)).sqrt();
        assert!((0.15 * 0.4 - 1e-6..=0.45 * 0.4 + 1e-6).contains(&s));
    }
    #[test]
    fn sizes_follow_vanilla_curves() {
        let mut r = LegacyRandom::new(2);
        let mut p = Particle::at(&mut r, ParticleType::Flame, [0.0; 3]);
        p.size = 1.0;
        p.lifetime = 10;
        p.age = 5;
        p.size_curve = Size::Flame;
        assert!((p.quad_size(0.0) - (1.0 - 0.25 * 0.5)).abs() < 1e-6);
        p.size_curve = Size::GrowIn;
        p.age = 0;
        assert!((p.quad_size(0.1) - (0.1f32 / 10.0 * 32.0)).abs() < 1e-6);
    }
    #[test]
    fn lifetime_alpha_interpolates() {
        let a = [0.0, 0.6, 0.25, 1.0];
        assert_eq!(lifetime_alpha(a, 0, 40, 0.0), 0.0);
        assert!((lifetime_alpha(a, 40, 40, 0.0) - 0.6).abs() < 1e-6);
    }
}
