//! Client particle engine (`ParticleEngine`, `Particle`, `SingleQuadParticle`)
//! and the vanilla particle types players see most: block crumbs, smoke,
//! flames, lava pops, drips, splashes and rain, bubbles, portal and enchant
//! glyphs, crits, hearts, villager particles, explosions, campfire smoke,
//! notes, redstone dust, totem, end rod, biome ash and spores, falling leaves,
//! spells and dragon breath. Constructors, ticks, sizes, colours and light
//! follow the vanilla classes; sprite sets come from `particles/*.json`.
use crate::{
    blocks::{BlockFacts, Fluid},
    fast_hash::FxHashMap,
    hud::Rect,
    mesher::Vertex,
    world::World,
};
use glam::{DVec3, Quat, Vec3 as FVec3};
use horizon_assets::resource_source::ResourceSource;
use horizon_core::{
    geometry::{Aabb, Axis, Vec3, trig},
    random::{LegacyRandom, RandomSource},
};
use horizon_world::block_collision::BlockCollisionRegistry;
mod options;
use mod_api::{ParticleBlend, ParticleProvider};
pub use options::ParticleOptions;
use std::{
    array,
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

pub use horizon_client_core::particle_type::ParticleType;
pub use horizon_items::firework::{DYE_FIREWORK_COLORS, FireworkExplosion, FireworkShape};
mod collide;
mod explosions;
pub use explosions::ExplosionParticle;
mod groups;
mod mod_behaviour;
mod native;
mod spawn;
pub use groups::register_particle_groups;
use mod_behaviour::{ModBehaviour, mod_curve};
pub use native::{ParticleController, ParticleUpdate, register_particle_providers};
use spawn::next_double_between;
pub mod polytone;
mod polytone_gpu;
mod polytone_model;
mod simple;
use simple::{PARALLEL_MIN, parallel_threads, sprite_from_age_in, tick_simple};

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
    /// `ShriekParticle.getQuadSize`.
    Shriek,
    /// `FireworkParticles.OverlayParticle.getQuadSize`: a sine pulse.
    Flash,
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
    /// `FireflyParticle.getLightCoords`: block light fading in and out, no sky.
    Firefly,
    /// A Polytone particle's `light_level`: block light at least this level.
    MinBlock(u8),
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum Behaviour {
    Base,
    Controlled {
        translucent: bool,
    },
    Mod {
        size: [f32; 2],
        alpha: [f32; 2],
    },
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
    /// `SulfurBubbleParticle`: rises three blocks through still water,
    /// growing to 0.15.
    SulfurBubble {
        y_start: f64,
        y_end: f64,
        size_start: f32,
        y_prev: f64,
    },
    /// `NoxiousGasParticle`: fades out from `fade_start` ticks of age.
    NoxiousGas {
        fade_start: f32,
    },
    /// `NoxiousGasCloudParticle`: every second tick, a gas particle where the
    /// gas reaches.
    NoxiousGasCloud,
    /// `GeyserEruptionParticle`: the base puffs, the plume and the poofs of
    /// one eruption; `aux` is the speed its provider was given.
    GeyserEruption {
        water_blocks: i32,
        aux: [f64; 3],
    },
    /// `GeyserPlumeParticle`: shoots up `plumeHeight` blocks, then falls back.
    GeyserPlume {
        start_y: f64,
        max_y: f64,
        propulsion: f32,
        spray: [f32; 2],
        min_size: f32,
        max_size: f32,
        done: bool,
    },
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
    /// `GustSeedParticle`: three gusts around it every `delay + 1` ticks.
    GustSeed {
        scale: f64,
        delay: i32,
    },
    Campfire,
    SimpleAnimated {
        fade: Option<[f32; 3]>,
    },
    /// FireworkParticles.SparkParticle.
    Spark {
        fade: Option<[f32; 3]>,
        trail: bool,
        twinkle: bool,
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
    /// `ShriekParticle`: waits `delay` ticks, then rises.
    Shriek {
        delay: i32,
    },
    /// `VibrationSignalParticle`: flies to a listener.
    Vibration {
        dest: [f64; 3],
        rot: f32,
        rot_o: f32,
        pitch: f32,
        pitch_o: f32,
    },
    /// `DustColorTransitionParticle`: from one colour to the other.
    Transition {
        from: [f32; 3],
        to: [f32; 3],
    },
    /// `TrailParticle`: eases towards its target over its lifetime.
    Trail {
        target: [f64; 3],
    },
    /// `SquidInkParticle`: `SimpleAnimatedParticle`, sinking through air.
    SquidInk,
    /// `FireflyParticle`: wanders, fading in and out, gone inside blocks.
    Firefly,
    /// `WakeParticle` (fishing): grows and cycles four sprites.
    Wake,
    /// `DustPlumeParticle`: gravity and friction decay every tick.
    DustPlume,
    /// `FlyStraightTowardsParticle`: from its start along its motion back to
    /// the start, the colour moving from one to the other.
    FlyStraight {
        start: [f64; 3],
        colors: [u32; 2],
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

/// A native mod's sprite set: its handle in `set_sprites`, the behaviour its JSON declares,
/// the provider that moves its particles, if a mod registered one, and the render group they
/// draw in, if a mod listed the type in one. Resolved at load, so a spawn, a tick and a draw
/// never look a provider or a group up by name.
#[derive(Clone)]
struct ModSet {
    set: u32,
    behaviour: Option<ModBehaviour>,
    provider: Option<Arc<dyn ParticleProvider>>,
    /// The handle of the registered group (`groups::registered`), none for the built-in one.
    group: Option<u32>,
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
    pub gravity: f32,
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
    /// A mod particle's `loop` animation: ticks per sprite, 0 for none.
    frame_ticks: u16,
    /// Sub-rectangle of the sprite (u0, v0, u1, v1) in 0..1.
    pub uv: [f32; 4],
    size_curve: Size,
    light: Light,
    behaviour: Behaviour,
    no_render: bool,
    /// Ticked once when created (`CritParticle` constructor).
    pre_tick: bool,
    /// A Polytone particle with physics, which joins only where its box is clear of blocks.
    spawn_check: bool,
    /// `Particle.random`: the particle's own generator (`RandomSource.create()`).
    random: LegacyRandom,
    controller: Option<Box<dyn native::Controller>>,
    /// A resource pack's Polytone custom particle type, for its `limit` and `exclusion_radius`.
    polytone_type: Option<std::sync::Arc<str>>,
    /// A Polytone model particle's faces, drawn instead of its sprite quad.
    model: Option<std::sync::Arc<polytone_model::ParticleModel>>,
    /// How a Polytone custom particle's quad turns (`rotation_mode`).
    orientation: polytone_gpu::Orientation,
    /// Blended additively (a Polytone `additive_translucent` particle).
    additive: bool,
    /// The registered render group this particle belongs to, none for the built-in group.
    group: Option<u32>,
}
fn aabb_at(p: [f64; 3], w: f32, h: f32) -> Aabb {
    let w = f64::from(w) / 2.0;
    Aabb::new(
        Vec3::new(p[0] - w, p[1], p[2] - w),
        Vec3::new(p[0] + w, p[1] + f64::from(h), p[2] + w),
    )
}
/// ClientLevel.addBreakingParticles placement, using local outline boxes.
pub fn breaking_position(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    face: [i32; 3],
    boxes: &[[f64; 6]],
) -> Option<[f64; 3]> {
    let mut bounds = *boxes.first()?;
    for b in &boxes[1..] {
        for i in 0..3 {
            bounds[i] = bounds[i].min(b[i]);
            bounds[i + 3] = bounds[i + 3].max(b[i + 3]);
        }
    }
    // Vanilla consumes all three doubles before replacing the hit-face coordinate.
    let mut p = array::from_fn(|i| {
        f64::from(pos[i])
            + random.next_double() * (bounds[i + 3] - bounds[i] - f64::from(0.2f32))
            + f64::from(0.1f32)
            + bounds[i]
    });
    for i in 0..3 {
        match face[i] {
            -1 => p[i] = f64::from(pos[i]) + bounds[i] - f64::from(0.1f32),
            1 => p[i] = f64::from(pos[i]) + bounds[i + 3] + f64::from(0.1f32),
            _ => {}
        }
    }
    Some(p)
}

/// A burst of particles about an entity's box (`around_self`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burst {
    pub count: u32,
    /// Scale of the Gaussian speeds; `None` spawns them at rest without drawing any.
    pub speed: Option<f64>,
    /// Added to `getRandomY()`.
    pub y: f64,
}
impl Burst {
    /// `AbstractVillager.addParticlesAroundSelf`.
    pub const VILLAGER: Self = Self {
        count: 5,
        speed: Some(0.02),
        y: 1.0,
    };
    /// `TamableAnimal`/`AbstractHorse.spawnTamingParticles`,
    /// `Ocelot.spawnTrustingParticles`, `Animal`'s entity event 18.
    pub const TAMING: Self = Self {
        count: 7,
        speed: Some(0.02),
        y: 0.5,
    };
    /// `Allay`'s entity event 18: three `spawnHeartParticle`.
    pub const ALLAY: Self = Self {
        count: 3,
        speed: Some(0.02),
        y: 0.5,
    };
    /// `Dolphin.addParticlesAroundSelf`.
    pub const DOLPHIN: Self = Self {
        count: 7,
        speed: Some(0.01),
        y: 0.2,
    };
    /// `AbstractHorse.handleEating`'s growth particle.
    pub const HORSE_EATING: Self = Self {
        count: 1,
        speed: None,
        y: 0.5,
    };
}
/// The particle bursts entity events show on the client: the particle and
/// burst of `handleEntityEvent(event)` for the entity type `entity`.
pub fn entity_event_burst(entity: &str, event: u8) -> Option<(&'static str, Burst)> {
    let entity = entity.strip_prefix("minecraft:").unwrap_or(entity);
    let tamable = matches!(
        entity,
        "wolf" | "cat" | "parrot" | "nautilus" | "zombie_nautilus"
    );
    let horse = matches!(
        entity,
        "horse"
            | "donkey"
            | "mule"
            | "llama"
            | "trader_llama"
            | "camel"
            | "camel_husk"
            | "skeleton_horse"
            | "zombie_horse"
    );
    Some(match (entity, event) {
        ("villager", 12) => ("heart", Burst::VILLAGER),
        ("villager", 13) => ("angry_villager", Burst::VILLAGER),
        ("villager", 14) => ("happy_villager", Burst::VILLAGER),
        ("villager", 42) => ("splash", Burst::VILLAGER),
        ("allay", 18) => ("heart", Burst::ALLAY),
        ("dolphin", 38) => ("happy_villager", Burst::DOLPHIN),
        ("ocelot", 41) => ("heart", Burst::TAMING),
        ("ocelot", 40) => ("smoke", Burst::TAMING),
        (_, 7) if tamable || horse => ("heart", Burst::TAMING),
        (_, 6) if tamable || horse => ("smoke", Burst::TAMING),
        (_, 18) if !matches!(entity, "villager" | "allay") => ("heart", Burst::TAMING),
        _ => return None,
    })
}

/// Particles about an entity's box, as `AbstractVillager.addParticlesAroundSelf`,
/// `Animal`'s love hearts (entity event 18), `TamableAnimal` and
/// `AbstractHorse.spawnTamingParticles`, `Ocelot.spawnTrustingParticles`,
/// `Allay`'s hearts and `Dolphin.addParticlesAroundSelf` place them: `count`
/// times three Gaussian speeds scaled by `speed` (none drawn without one), then
/// `getRandomX(1.0)`, `getRandomY() + y` and `getRandomZ(1.0)`.
pub fn around_self(
    random: &mut impl RandomSource,
    feet: [f64; 3],
    width: f64,
    height: f64,
    burst: Burst,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    for _ in 0..burst.count {
        let v = burst.speed.map_or([0.0; 3], |s| {
            [
                random.next_gaussian() * s,
                random.next_gaussian() * s,
                random.next_gaussian() * s,
            ]
        });
        let p = [
            feet[0] + width * ((2.0 * random.next_double() - 1.0) * 1.0),
            feet[1] + height * random.next_double() + burst.y,
            feet[2] + width * ((2.0 * random.next_double() - 1.0) * 1.0),
        ];
        emit(p, v);
    }
}

/// `LivingEntity.handleEntityEvent(46)`: 128 `ParticleTypes.PORTAL` samples
/// from the entity's old position (`xo`, `yo`, `zo`) to its current one,
/// spread across the bounding box (`getBbWidth() * 2` on x and z, `getBbHeight`
/// on y). The speeds are `(nextFloat() - 0.5) * 0.2`.
pub fn living_teleport_particles(
    random: &mut impl RandomSource,
    from: [f64; 3],
    to: [f64; 3],
    width: f64,
    height: f64,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    for i in 0..128 {
        let d = i as f64 / 127.0;
        let velocity = [0; 3].map(|_| f64::from((random.next_float() - 0.5) * 0.2));
        let position = [
            from[0] + d * (to[0] - from[0]) + (random.next_double() - 0.5) * width * 2.0,
            from[1] + d * (to[1] - from[1]) + random.next_double() * height,
            from[2] + d * (to[2] - from[2]) + (random.next_double() - 0.5) * width * 2.0,
        ];
        emit(position, velocity);
    }
}

/// ClientLevel.addDestroyBlockEffect placement and pre-constructor velocities.
pub fn destroy_positions(
    pos: [i32; 3],
    boxes: &[[f64; 6]],
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    for b in boxes {
        let widths: [f64; 3] = array::from_fn(|i| (b[i + 3] - b[i]).min(1.0));
        let counts: [usize; 3] = widths.map(|w| ((w / 0.25).ceil() as usize).max(2));
        for x in 0..counts[0] {
            for y in 0..counts[1] {
                for z in 0..counts[2] {
                    let cells = [x, y, z];
                    let relative: [f64; 3] =
                        array::from_fn(|i| (cells[i] as f64 + 0.5) / counts[i] as f64);
                    emit(
                        array::from_fn(|i| f64::from(pos[i]) + (relative[i] * widths[i] + b[i])),
                        relative.map(|v| v - 0.5),
                    );
                }
            }
        }
    }
}

/// `LivingEntity.spawnItemParticles(stack, count)` (eating, drinking, an item
/// breaking): `count` crumbs in front of the eyes, thrown up and forwards,
/// both turned by the pitch and yaw in degrees.
pub fn item_use_particles(
    random: &mut impl RandomSource,
    pitch: f32,
    yaw: f32,
    position: [f64; 3],
    eye_y: f64,
    count: u32,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    let (x_rot, y_rot) = (-pitch * trig::DEG_TO_RAD, -yaw * trig::DEG_TO_RAD);
    for _ in 0..count {
        let d = Vec3::new(
            (f64::from(random.next_float()) - 0.5) * 0.1,
            f64::from(random.next_float()) * 0.1 + 0.1,
            0.0,
        )
        .rotate_x(x_rot)
        .rotate_y(y_rot);
        let y = f64::from(-random.next_float()) * 0.6 - 0.3;
        let p = Vec3::new((f64::from(random.next_float()) - 0.5) * 0.3, y, 0.6)
            .rotate_x(x_rot)
            .rotate_y(y_rot);
        emit(
            [p.x + position[0], p.y + eye_y, p.z + position[2]],
            [d.x, d.y + 0.05, d.z],
        );
    }
}

/// `ParticleUtils.spawnParticlesOnBlockFaces(level, pos, particle, UniformInt.of(3, 5))`
/// (level events 3002 without an axis, 3003 wax on, 3004 wax off, 3005 scrape):
/// three to five particles on each face, in `Direction.values()` order, moving
/// along the face at up to half a block a tick.
pub fn on_block_faces(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    emit: impl FnMut([f64; 3], [f64; 3]),
) {
    on_block_faces_between(random, pos, 3, 5, emit);
}
/// `ParticleUtils.spawnParticlesOnBlockFaces` with `UniformInt.of(min, max)`
/// particles per face (3009 sniffer egg cracks use 3 to 6).
pub fn on_block_faces_between(
    random: &mut impl RandomSource,
    pos: [i32; 3],
    min: i32,
    max: i32,
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    const FACES: [[i32; 3]; 6] = [
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
        [-1, 0, 0],
        [1, 0, 0],
    ];
    for face in FACES {
        let count = random.next_int_bound(max - min + 1).unwrap_or(0) + min;
        for _ in 0..count {
            // getRandomSpeedRanges, then spawnParticleOnFace with step factor 0.55.
            let speed: [f64; 3] = array::from_fn(|_| next_double_between(random, -0.5, 0.5));
            let mut p = [0.0; 3];
            let mut v = [0.0; 3];
            for i in 0..3 {
                let center = f64::from(pos[i]) + 0.5;
                if face[i] == 0 {
                    p[i] = center + next_double_between(random, -0.5, 0.5);
                    v[i] = speed[i];
                } else {
                    p[i] = center + f64::from(face[i]) * 0.55;
                }
            }
            emit(p, v);
        }
    }
}
/// `ParticleUtils.spawnParticlesAlongAxis(axis, level, pos, 0.125, particle,
/// UniformInt.of(min, max))` (`count` is the range; level event 3002 on a
/// lightning rod uses 10 to 19): sparks along the rod, moving along it.
/// `axis` is 0, 1, 2 for X, Y, Z.
pub fn along_axis(
    random: &mut impl RandomSource,
    axis: usize,
    pos: [i32; 3],
    (min, max): (i32, i32),
    mut emit: impl FnMut([f64; 3], [f64; 3]),
) {
    let count = random.next_int_bound(max - min + 1).unwrap_or(0) + min;
    for _ in 0..count {
        let p: [f64; 3] = array::from_fn(|i| {
            f64::from(pos[i])
                + 0.5
                + next_double_between(random, -1.0, 1.0) * if i == axis { 0.5 } else { 0.125 }
        });
        let v: [f64; 3] = array::from_fn(|i| {
            if i == axis {
                next_double_between(random, -1.0, 1.0)
            } else {
                0.0
            }
        });
        emit(p, v);
    }
}

impl Particle {
    /// `Particle(level, x, y, z)` + `SingleQuadParticle` (no initial velocity).
    fn at(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3]) -> Self {
        let mut particle = Self::base_at(random, ty, p);
        particle.size = 0.1 * (random.next_float() * 0.5 + 0.5) * 2.0;
        particle
    }
    /// `Particle(level, x, y, z)` without `SingleQuadParticle` (`NoRenderParticle`).
    fn base_at(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3]) -> Self {
        let mut particle = Self::blank(ty, p);
        particle.lifetime = (4.0 / (random.next_float() * 0.9 + 0.1)) as i32;
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
            spawn_check: false,
            frame_ticks: 0,
            random: LegacyRandom::new(0),
            controller: None,
            polytone_type: None,
            model: None,
            orientation: polytone_gpu::Orientation::Camera,
            additive: false,
            group: None,
        }
    }
    /// The normal age/removal test and previous pose, without vanilla motion.
    /// Returns false when this tick expires the particle.
    pub fn advance_age(&mut self) -> bool {
        self.previous = self.position;
        self.o_roll = self.roll;
        let alive = self.age < self.lifetime;
        self.age = self.age.saturating_add(1);
        self.removed |= !alive;
        alive
    }
    pub fn remove(&mut self) {
        self.removed = true;
    }
    /// Hide geometry while retaining the particle's lifetime and controller.
    pub fn set_visible(&mut self, visible: bool) {
        self.no_render = !visible;
    }
    /// The particle's own random, distinct from the engine and level generators.
    pub fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    pub fn set_full_bright(&mut self, full_bright: bool) {
        self.light = if full_bright {
            Light::FullBright
        } else {
            Light::World
        };
    }
    /// `Particle(level, x, y, z, xa, ya, za)`: jittered, normalized initial velocity.
    fn moving(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3], v: [f64; 3]) -> Self {
        let mut particle = Self::base_moving(random, ty, p, v);
        particle.size = 0.1 * (random.next_float() * 0.5 + 0.5) * 2.0;
        particle
    }
    /// `Particle(level, x, y, z, xa, ya, za)` without `SingleQuadParticle`.
    fn base_moving(random: &mut LegacyRandom, ty: ParticleType, p: [f64; 3], v: [f64; 3]) -> Self {
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
    /// What `CritParticle`'s providers do once its constructor has ticked it.
    fn after_constructor_tick(&mut self) {
        match self.ty {
            // MagicProvider.
            ParticleType::EnchantedHit => {
                self.rgba[0] *= 0.3;
                self.rgba[1] *= 0.8;
            }
            // DamageIndicatorProvider.
            ParticleType::DamageIndicator => self.lifetime = 20,
            _ => {}
        }
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
    /// `getLightCoords(a)` from the world's smooth sky and block light (0..=240).
    #[inline(always)]
    fn light_coords(&self, partial: f32, sky: u32, block: u32) -> (u32, u32) {
        match self.light {
            Light::World => (sky, block),
            Light::FullBright => (240, 240),
            Light::Block15 => (sky, 240),
            Light::AgeEmission => {
                let t = ((self.age as f32 + partial) / self.lifetime as f32).clamp(0.0, 1.0);
                (sky, (block + (t * 240.0) as u32).min(240))
            }
            Light::AgeEmission4 => {
                let b = self.age as f32 / self.lifetime as f32;
                let b = (b * b) * (b * b);
                (sky, (block + (b.clamp(0.0, 1.0) * 240.0) as u32).min(240))
            }
            Light::MinBlock(level) => (sky, block.max(u32::from(level) * 16)),
            Light::Firefly => (
                0,
                (255.0 * fade_amount(self.lifetime_progress(self.age as f32 + partial), 0.1, 0.3))
                    as u32,
            ),
        }
    }
    #[inline(always)]
    fn quad_size(&self, a: f32) -> f32 {
        match self.size_curve {
            Size::Fixed => self.size,
            Size::GrowIn => {
                let t = (self.age as f32 + a) / self.lifetime as f32;
                self.size * (t * 32.0).clamp(0.0, 1.0)
            }
            Size::Flame => {
                let t = (self.age as f32 + a) / self.lifetime as f32;
                self.size * (1.0 - t * t * 0.5)
            }
            Size::Lava => {
                let t = (self.age as f32 + a) / self.lifetime as f32;
                self.size * (1.0 - t * t)
            }
            Size::Portal => {
                let s = 1.0 - (self.age as f32 + a) / self.lifetime as f32;
                self.size * (1.0 - s * s)
            }
            Size::ReversePortal => {
                self.size * (1.0 - (self.age as f32 + a) / (self.lifetime as f32 * 1.5))
            }
            Size::Shriek => {
                let t = (self.age as f32 + a) / self.lifetime as f32;
                self.size * (t * 0.75).clamp(0.0, 1.0)
            }
            Size::Flash => {
                7.1 * trig::sin(f64::from(
                    (self.age as f32 + a - 1.0) * 0.25 * std::f64::consts::PI as f32,
                ))
            }
        }
    }
    /// `FireflyParticle.getLifetimeProgress`.
    fn lifetime_progress(&self, age: f32) -> f32 {
        (age / self.lifetime as f32).clamp(0.0, 1.0)
    }
}
/// `NoxiousGasCloudParticle.pickRandomParticleSpawnPoint`: a point up to three
/// blocks from the centre of `center_block` in a random horizontal direction,
/// a quarter block lower, from the level's random.
fn pick_random_particle_spawn_point(random: &mut LegacyRandom, center_block: [i32; 3]) -> [f64; 3] {
    let dx = f64::from(random.next_float() - 0.5);
    let dz = f64::from(random.next_float() - 0.5);
    // Vec3.normalize.
    let length = (dx * dx + dz * dz).sqrt();
    let (dx, dz) = if length < f64::from(1.0e-5f32) {
        (0.0, 0.0)
    } else {
        (dx / length, dz / length)
    };
    let distance = f64::from(random.next_float() * 3.0);
    [
        f64::from(center_block[0]) + 0.5 + dx * distance,
        f64::from(center_block[1]) + 0.5 - 0.25,
        f64::from(center_block[2]) + 0.5 + dz * distance,
    ]
}
/// `FireflyParticle.getFadeAmount`.
fn fade_amount(progress: f32, fade_in: f32, fade_out: f32) -> f32 {
    if progress >= 1.0 - fade_in {
        (1.0 - progress) / fade_in
    } else if progress <= fade_out {
        progress / fade_out
    } else {
        1.0
    }
}

/// A particle's fields as the parity probe reports them.
#[derive(Clone, Debug)]
pub struct ParticleSnapshot {
    pub age: i32,
    pub lifetime: i32,
    pub removed: bool,
    pub on_ground: bool,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub previous: [f64; 3],
    /// `SingleQuadParticle` state; `None` for `NoRenderParticle`s.
    pub quad: Option<QuadSnapshot>,
}
/// `SingleQuadParticle` state at a render.
#[derive(Clone, Debug)]
pub struct QuadSnapshot {
    /// `getQuadSize` at partial ticks 0 and 0.5.
    pub size: [f32; 2],
    pub rgba: [f32; 4],
    /// `roll`, `oRoll`.
    pub roll: [f32; 2],
    pub sprite: String,
    /// `getU0`, `getU1`, `getV0`, `getV1` within the sprite.
    pub uv: [f32; 4],
    /// `getLightCoords(0.5)` over full sky light and no block light.
    pub light: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackingTarget {
    LocalPlayer,
    Entity(i32),
}

/// The entity position and current dimensions that `getX/Y/Z(double)` use.
#[derive(Clone, Copy)]
pub struct TrackingBody {
    pub feet: [f64; 3],
    pub width: f64,
    pub height: f64,
}

struct TrackingEmitter {
    options: ParticleOptions,
    target: Option<TrackingTarget>,
    body: TrackingBody,
    remaining: u32,
}

/// World access for particle physics.
pub struct ParticleLevel<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    /// Local player feet position and vertical velocity (`PlayerCloudParticle`).
    pub player: Option<([f64; 3], f64)>,
    pub cached_chunk: std::cell::Cell<Option<((i32, i32), Option<&'a crate::chunk::Chunk>)>>,
}
impl<'a> ParticleLevel<'a> {
    #[inline(always)]
    pub(crate) fn block_at(p: [f64; 3]) -> [i32; 3] {
        [
            p[0].floor() as i32,
            p[1].floor() as i32,
            p[2].floor() as i32,
        ]
    }
    #[inline(always)]
    pub fn block(&self, x: i32, y: i32, z: i32) -> horizon_world::registry::BlockStateId {
        let (cx, cz) = (x >> 4, z >> 4);
        let chunk = match self.cached_chunk.get() {
            Some(((ccx, ccz), c)) if ccx == cx && ccz == cz => c,
            _ => {
                let c = self.world.chunks.get(&(cx, cz)).map(|c| c.as_ref());
                self.cached_chunk.set(Some(((cx, cz), c)));
                c
            }
        };
        chunk.map_or(self.world.air, |c| c.block(x & 15, y, z & 15))
    }
    /// The fluid at a block without reading the block above for its height.
    #[inline]
    fn fluid_kind(&self, b: [i32; 3]) -> Fluid {
        let state = self.block(b[0], b[1], b[2]);
        if state == self.world.air {
            return Fluid::None;
        }
        self.facts.get(state).fluid
    }
    /// The fluid at a block and `FluidState.getHeight`.
    #[inline]
    fn fluid(&self, b: [i32; 3]) -> (Fluid, f64) {
        let state = self.block(b[0], b[1], b[2]);
        if state == self.world.air {
            return (Fluid::None, 0.0);
        }
        self.fluid_of(state, b)
    }
    #[inline]
    fn fluid_of(&self, state: horizon_world::registry::BlockStateId, b: [i32; 3]) -> (Fluid, f64) {
        let s = self.facts.get(state);
        if s.fluid == Fluid::None {
            return (Fluid::None, 0.0);
        }
        let above = self.facts.get(self.block(b[0], b[1] + 1, b[2]));
        let h = if above.fluid == s.fluid {
            1.0
        } else {
            f64::from(s.fluid_level) / 9.0
        };
        (s.fluid, h)
    }
    /// The fluid at a block and whether it is a source (`FluidState.isSource`).
    #[inline]
    pub(crate) fn fluid_source(&self, b: [i32; 3]) -> (Fluid, bool) {
        let state = self.block(b[0], b[1], b[2]);
        if state == self.world.air {
            return (Fluid::None, false);
        }
        let s = self.facts.get(state);
        (s.fluid, s.fluid_level == 8 && !s.fluid_falling)
    }
    /// `getCollisionShape(...).max(Y, x, z)` for cached shapes.
    #[allow(dead_code)]
    #[inline]
    fn collision_top(&self, b: [i32; 3], fx: f64, fz: f64) -> f64 {
        self.collision_top_of(self.block(b[0], b[1], b[2]), fx, fz)
    }
    /// `getCollisionShape(...).max(Y, x, z)` for cached shapes of `state`.
    #[inline]
    fn collision_top_of(
        &self,
        state: horizon_world::registry::BlockStateId,
        fx: f64,
        fz: f64,
    ) -> f64 {
        match self.collision.cached_shapes().cached_shape(state) {
            Ok(Some(shape)) if !shape.is_empty() => shape.max_at(Axis::Y, fx, fz),
            Ok(Some(_)) => 0.0,
            _ if self.facts.get(state).full_collision => 1.0,
            _ => 0.0,
        }
    }
}

/// One `particlesToAdd` FIFO across render groups: admission shares the engine RNG.
#[allow(
    clippy::large_enum_variant,
    reason = "inline ordinary particles avoid an allocation per spawn"
)]
enum PendingParticle {
    Ordinary(Particle),
    Starter(Starter),
    ElderGuardian,
}

impl PendingParticle {
    fn ordinary(&self) -> Option<&Particle> {
        match self {
            Self::Ordinary(particle) => Some(particle),
            _ => None,
        }
    }
}

/// `ParticleEngine` plus the sprite sets of `ParticleResources`.
pub struct ParticleEngine {
    particles: Vec<Particle>,
    /// The mods' registered render groups, indexed by their handle: each keeps its own live
    /// particles, drawn after `particles`.
    groups: Vec<groups::Group>,
    /// The registered groups as this engine was built (`register_particle_groups`), which each
    /// type's group handle indexes; resolved at every load.
    group_defs: &'static [mod_api::ParticleGroup],
    pending: Vec<PendingParticle>,
    /// Particle spawns deferred until all live particles have ticked.
    spawned_in_tick: Option<Vec<(ParticleOptions, [f64; 3], [f64; 3])>>,
    /// `ParticleEngine.random`, handed to every `ParticleProvider`.
    random: LegacyRandom,
    /// `ClientLevel.random` as particle constructors use it (`level.getRandom()`).
    level_random: LegacyRandom,
    /// `SpellParticle.RANDOM`.
    spell_random: LegacyRandom,
    /// The local player's eye while it looks through a spyglass in first person.
    scope_eye: Option<[f64; 3]>,
    /// Seeds for each particle's own random (`RandomSource.create()`).
    seeds: LegacyRandom,
    /// When set, particles ask the level for other particles here instead
    /// (type, position, velocity), as the parity probe's stand-in level does.
    pub requested: Option<Vec<(ParticleOptions, [f64; 3], [f64; 3])>>,
    /// When set, `create` records every particle the level is asked for here and
    /// makes none: no particle's own random is drawn and no sprite is picked, so a
    /// parity probe compares exactly what a block, fluid or entity asked for.
    pub created_requests: Option<Vec<(ParticleOptions, [f64; 3], [f64; 3])>>,
    names: Vec<String>,
    rects: Vec<Option<Rect>>,
    /// Each named sprite, and whether `SingleQuadParticle.Layer.bySprite` puts
    /// it on the translucent layer (a texel with 0 < alpha < 255).
    atlas: HashMap<String, (Rect, bool)>,
    sets: FxHashMap<ParticleType, u32>,
    /// A native mod's sprite sets, by namespaced particle name.
    mod_sets: FxHashMap<String, ModSet>,
    /// The code providers of mods' particle types, by name: `register_particle_providers`' registrations as
    /// this engine was built, applied to `mod_sets` at every load.
    providers: BTreeMap<String, Arc<dyn ParticleProvider>>,
    set_sprites: Vec<Vec<u32>>,
    /// How many live particles it takes to tick them on several threads (`tick_all`), and how many threads.
    parallel_min: usize,
    parallel_threads: usize,
    /// Resource packs' Polytone custom particle types (#11486).
    polytone: Option<std::sync::Arc<polytone::Registry>>,
    /// Vanilla particle types a pack replaces with a custom one of the same id
    /// (`assets/minecraft/polytone/custom_particles/rain.json` is `minecraft:rain`).
    polytone_overrides: FxHashMap<ParticleType, String>,
    /// The packs' GPU particles' records, one store per type, and the ticks they count in.
    polytone_gpu: Vec<polytone_gpu::Store>,
    polytone_ticks: u64,
    /// The last frame's additively blended quads (`take_additive`).
    additive: Mutex<Vec<Vertex>>,
    /// Fast flag tracking whether `additive` holds unconsumed vertices so vanilla frames skip locking.
    has_additive: AtomicBool,
    /// Lazily-allocated 512-entry generation-stamped block-light cache reused across dense frames.
    dense_light_cache: Mutex<Option<DenseLightCache>>,
    emitters: Vec<TrackingEmitter>,
    /// Camera position for the 32-block spawn limiter.
    pub camera: [f64; 3],
    /// The Particles option (`ParticleStatus`): 0 all, 1 decreased, 2 minimal.
    pub particle_level: u8,
    /// The most live particles (`len`) this engine keeps: a new one past it is not
    /// added. None, the level's engine, has only each group's own limit; a mod screen's
    /// scene sets its own.
    pub live_limit: Option<usize>,
    /// FireworkParticles.Starter instances.
    starters: Vec<Starter>,
    elder_guardians: Vec<i32>,
    /// `ClientExplosionTracker` tracked explosions pending debris generation.
    tracked_explosions: Vec<explosions::TrackedExplosion>,
    /// Local sounds requested by particles (dripstone drips, firework bursts):
    /// (event, position, volume, pitch, distanceDelay).
    pub sounds: Vec<(&'static str, [f64; 3], f32, f32, bool)>,
    /// How many of each type were made, for `horizon-ctl status`.
    made: std::collections::BTreeMap<&'static str, u64>,
    /// The item particles' sprites (`BreakingItemParticle`'s slime ball, snowball
    /// and cobweb models, resolved for the ground), one per model layer.
    pub item_sprites: HashMap<ParticleType, Vec<String>>,
}
/// `ElderGuardianParticle.lifetime`.
pub const ELDER_GUARDIAN_LIFETIME: i32 = 30;

/// `ParticleGroup.MAX_PARTICLES` (per render group).
const MAX_PARTICLES: usize = 16384;
/// `ParticleLimit.SPORE_BLOSSOM`, the only limited particle.
const SPORE_BLOSSOM_LIMIT: usize = 1000;
const PARTICLE_LIGHT_CACHE_SIZE: usize = 64;
const DENSE_LIGHT_CACHE_THRESHOLD: usize = 32;
const SMALL_LIGHT_CACHE_SIZE: usize = 16;
const DENSE_LIGHT_CACHE_SIZE: usize = 512;
const MAX_INITIAL_PARTICLE_QUADS: usize = 256;

#[derive(Clone, Copy)]
struct CachedParticleLight {
    block: [i32; 3],
    packed: u8,
}

struct ParticleLightCache {
    entries: [Option<CachedParticleLight>; PARTICLE_LIGHT_CACHE_SIZE],
}

impl Default for ParticleLightCache {
    fn default() -> Self {
        Self {
            entries: [None; PARTICLE_LIGHT_CACHE_SIZE],
        }
    }
}

impl ParticleLightCache {
    #[inline]
    fn get(&mut self, block: [i32; 3], light_at: &dyn Fn([i32; 3]) -> u8) -> u8 {
        let hash = (block[0] as u32).wrapping_mul(73_856_093)
            ^ (block[1] as u32).wrapping_mul(19_349_663)
            ^ (block[2] as u32).wrapping_mul(83_492_791);
        let slot = hash as usize & (PARTICLE_LIGHT_CACHE_SIZE - 1);
        if let Some(entry) = self.entries[slot]
            && entry.block == block
        {
            return entry.packed;
        }
        let packed = light_at(block);
        self.entries[slot] = Some(CachedParticleLight { block, packed });
        packed
    }
}

#[derive(Clone, Copy)]
struct LightCacheSlot {
    block: [i32; 3],
    generation: u16,
    packed: u8,
}

impl LightCacheSlot {
    const EMPTY: Self = Self {
        block: [0; 3],
        generation: 0,
        packed: 0,
    };
}

/// Reusable 512-entry direct-mapped block-light cache cleared in O(1) via a
/// per-frame generation stamp, lazily allocated only on dense particle frames.
struct DenseLightCache {
    entries: Box<[LightCacheSlot]>,
    generation: u16,
}

impl DenseLightCache {
    fn new() -> Self {
        Self {
            entries: vec![LightCacheSlot::EMPTY; DENSE_LIGHT_CACHE_SIZE].into_boxed_slice(),
            generation: 0,
        }
    }

    fn begin_frame(&mut self) -> (&mut [LightCacheSlot], u16) {
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            for entry in &mut self.entries {
                entry.generation = 0;
            }
            self.generation = 1;
        }
        (&mut self.entries, self.generation)
    }
}

struct FrameLightCache<'a> {
    entries: &'a mut [LightCacheSlot],
    mask: usize,
    generation: u16,
    last_block: [i32; 3],
    last_packed: u8,
    has_last: bool,
}

impl<'a> FrameLightCache<'a> {
    #[inline]
    fn new(entries: &'a mut [LightCacheSlot], generation: u16) -> Self {
        debug_assert!(entries.len().is_power_of_two());
        let mask = entries.len() - 1;
        Self {
            entries,
            mask,
            generation,
            last_block: [0; 3],
            last_packed: 0,
            has_last: false,
        }
    }

    #[inline(always)]
    fn get(&mut self, block: [i32; 3], light_at: &dyn Fn([i32; 3]) -> u8) -> u8 {
        if self.has_last && self.last_block == block {
            return self.last_packed;
        }
        let hash = (block[0] as u32).wrapping_mul(73_856_093)
            ^ (block[1] as u32).wrapping_mul(19_349_663)
            ^ (block[2] as u32).wrapping_mul(83_492_791);
        let slot = ((hash ^ (hash >> 9)) as usize) & self.mask;
        if let Some(entry) = self.entries.get_mut(slot) {
            if entry.generation == self.generation && entry.block == block {
                self.last_block = block;
                self.last_packed = entry.packed;
                self.has_last = true;
                return entry.packed;
            }
            let packed = light_at(block);
            *entry = LightCacheSlot {
                block,
                generation: self.generation,
                packed,
            };
            self.last_block = block;
            self.last_packed = packed;
            self.has_last = true;
            return packed;
        }
        light_at(block)
    }
}

mod view_cull;
use view_cull::{PARTICLE_FRUSTUM_OFFSET, ParticleSectionFrustumCache, SectionFrustumResult};

fn below_half_pixel(radius: f64, distance_squared: f64, pixel_focal_length: f32) -> bool {
    let projected_limit = 4.0 * radius * f64::from(pixel_focal_length);
    radius.is_finite()
        && radius >= 0.0
        && distance_squared.is_finite()
        && distance_squared > 0.0
        && pixel_focal_length.is_finite()
        && pixel_focal_length > 0.0
        && projected_limit.is_finite()
        && projected_limit * projected_limit < distance_squared
}

impl ParticleEngine {
    pub fn new(
        root: &ResourceSource,
        atlas: &HashMap<String, Rect>,
        translucent: &HashSet<String>,
    ) -> Self {
        let mut engine = Self {
            particles: Vec::new(),
            groups: groups::Group::all(groups::registered()),
            group_defs: groups::registered(),
            pending: Vec::new(),
            spawned_in_tick: None,
            random: LegacyRandom::new(0x7061_7274),
            level_random: LegacyRandom::new(0x6c65_7665),
            spell_random: LegacyRandom::new(0x7370_656c),
            scope_eye: None,
            seeds: LegacyRandom::new(0x7365_6564),
            requested: None,
            created_requests: None,
            names: Vec::new(),
            rects: Vec::new(),
            atlas: HashMap::new(),
            sets: FxHashMap::default(),
            mod_sets: FxHashMap::default(),
            providers: native::registered().clone(),
            set_sprites: Vec::new(),
            parallel_min: PARALLEL_MIN,
            live_limit: None,
            parallel_threads: parallel_threads(),
            polytone: None,
            polytone_overrides: FxHashMap::default(),
            polytone_gpu: Vec::new(),
            polytone_ticks: 0,
            additive: Mutex::default(),
            has_additive: AtomicBool::new(false),
            dense_light_cache: Mutex::new(None),
            emitters: Vec::new(),
            starters: Vec::new(),
            elder_guardians: Vec::new(),
            tracked_explosions: Vec::new(),
            camera: [0.0; 3],
            particle_level: 0,
            sounds: Vec::new(),
            made: Default::default(),
            item_sprites: HashMap::new(),
        };
        engine.reload(root, atlas, translucent);
        engine
    }
    /// `onResourceManagerReload`: sprite rects and sets come from the
    /// reloaded atlas and pack; a live particle keeps drawing its sprite by
    /// name or index.
    pub fn reload(
        &mut self,
        root: &ResourceSource,
        atlas: &HashMap<String, Rect>,
        translucent: &HashSet<String>,
    ) {
        // Controllers' numeric sprite-set handles belong to this resource
        // generation. Discard them rather than selecting another set after reload.
        self.particles.retain(|p| p.controller.is_none());
        for group in &mut self.groups {
            group.particles.retain(|p| p.controller.is_none());
        }
        self.pending
            .retain(|p| p.ordinary().is_none_or(|p| p.controller.is_none()));
        self.atlas = atlas
            .iter()
            .map(|(name, &rect)| (name.clone(), (rect, translucent.contains(name))))
            .collect();
        self.rects = self
            .names
            .iter()
            .map(|name| self.atlas.get(name).map(|&(rect, _)| rect))
            .collect();
        self.sets.clear();
        self.mod_sets.clear();
        self.set_sprites.clear();
        self.item_sprites.clear();
        self.load_sprites(root);
        for name in self.providers.keys() {
            if !self.mod_sets.contains_key(name) {
                log::warn!("Particle provider {name} has no particles/<path>.json sprite set");
            }
        }
        self.resolve_groups();
    }
    /// Gives each type a registered group lists the group's handle, once per load. A type no
    /// sprite set matches, or another group lists already, is logged and left out.
    fn resolve_groups(&mut self) {
        for (handle, group) in self.group_defs.iter().enumerate() {
            for name in &group.particles {
                match self.mod_sets.get_mut(name) {
                    Some(set) if set.group.is_none() => set.group = Some(handle as u32),
                    Some(_) => log::warn!(
                        "Particle {name} is in another particle group already; left out of {}",
                        group.id
                    ),
                    None => log::warn!(
                        "Particle {name} of group {} has no particles/<path>.json sprite set",
                        group.id
                    ),
                }
            }
        }
    }
    /// The sprite sets of every `particles/*.json` under `root`: vanilla's own
    /// and a native mod's (`assets/<namespace>/particles/<name>.json`).
    fn load_sprites(&mut self, root: &ResourceSource) {
        for ns in root.namespaces("assets") {
            if ns != "minecraft" {
                self.load_mod_sprites(root, &ns);
            }
        }
        for &ty in ParticleType::ALL {
            let Ok(bytes) = root.read(&format!("assets/minecraft/particles/{}.json", ty.name()))
            else {
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
                            self.intern(&format!("particle/{}", t.trim_start_matches("minecraft:")))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !sprites.is_empty() {
                self.sets.insert(ty, self.set_sprites.len() as u32);
                self.set_sprites.push(sprites);
            }
        }
    }
    /// A namespace's particle JSONs that vanilla has no type for: each becomes
    /// one sprite set under `<ns>:<file name>`.
    fn load_mod_sprites(&mut self, root: &ResourceSource, ns: &str) {
        let prefix = format!("assets/{ns}/particles/");
        for path in root.list(&prefix) {
            let Some(stem) = path
                .strip_prefix(&prefix)
                .and_then(|p| p.strip_suffix(".json"))
            else {
                continue;
            };
            let key = format!("{ns}:{stem}");
            if ParticleType::from_name(&key).is_some() {
                continue;
            }
            let Ok(bytes) = root.read(&path) else {
                continue;
            };
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let behaviour = json.get("horizon:behaviour").and_then(|value| {
                let behaviour = ModBehaviour::parse(value);
                if behaviour.is_none() {
                    log::warn!("Invalid particle behaviour in {}", path);
                }
                behaviour
            });
            let sprites: Vec<u32> = json["textures"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .map(|t| {
                            // A texture is `<ns>:<path>` (bare means
                            // `minecraft:`), and the atlas names a sprite
                            // `<ns>:particle/<path>` — `minecraft`'s own
                            // without its namespace.
                            let name = match t.split_once(':') {
                                Some(("minecraft", path)) => format!("particle/{path}"),
                                Some((ns, path)) => format!("{ns}:particle/{path}"),
                                None => format!("particle/{t}"),
                            };
                            self.intern(&name)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !sprites.is_empty() {
                let provider = self.providers.get(&key).cloned();
                self.mod_sets.insert(
                    key,
                    ModSet {
                        set: self.set_sprites.len() as u32,
                        behaviour,
                        provider,
                        group: None,
                    },
                );
                self.set_sprites.push(sprites);
            }
        }
    }
    fn intern(&mut self, name: &str) -> u32 {
        if let Some(i) = self.names.iter().position(|n| n == name) {
            return i as u32;
        }
        self.names.push(name.to_owned());
        self.rects.push(self.atlas.get(name).map(|&(rect, _)| rect));
        (self.names.len() - 1) as u32
    }
    pub fn len(&self) -> usize {
        self.particles.len() + self.groups.iter().map(|g| g.particles.len()).sum::<usize>()
    }
    pub fn is_empty(&self) -> bool {
        self.particles.is_empty() && self.groups.iter().all(|g| g.particles.is_empty())
    }
    /// The live particles `len` counts: the engine's own, then each group's.
    pub fn live(&self) -> impl Iterator<Item = &Particle> {
        self.particles
            .iter()
            .chain(self.groups.iter().flat_map(|g| g.particles.iter()))
    }
    /// The live particles of the groups this engine holds, as `ParticleEngine.countParticles`
    /// counts them: ordinary particles, firework starters (`NO_RENDER`) and elder guardian
    /// apparitions (`ELDER_GUARDIANS`). Queued particles and tracking emitters are not counted.
    pub fn group_count(&self) -> usize {
        self.len() + self.starters.len() + self.elder_guardians.len()
    }
    /// `ParticleEngine.clearParticles`, for the groups this engine holds: the live
    /// particles, firework starters, elder guardians, queued particles and tracking emitters.
    pub fn clear(&mut self) {
        self.emitters.clear();
        self.particles.clear();
        for group in &mut self.groups {
            group.particles.clear();
        }
        self.starters.clear();
        self.elder_guardians.clear();
        self.pending.clear();
    }
    /// `VibrationSignalParticle`'s body: the signal leaves `from` aimed at the
    /// listener at `to` and lands there in `ticks`.
    fn vibration_particle(from: [f64; 3], to: [f64; 3], ticks: i32) -> Particle {
        let mut q = Particle::blank(ParticleType::Vibration, from);
        q.size = 0.3;
        q.lifetime = ticks;
        q.has_physics = false;
        q.light = Light::Block15;
        let d = [from[0] - to[0], from[1] - to[1], from[2] - to[2]];
        let rot = d[0].atan2(d[2]) as f32;
        let pitch = d[1].atan2((d[0] * d[0] + d[2] * d[2]).sqrt()) as f32;
        q.behaviour = Behaviour::Vibration {
            dest: to,
            rot,
            rot_o: rot,
            pitch,
            pitch_o: pitch,
        };
        q
    }
    /// A vibration signal flying from `from` to `to` in `ticks`
    /// (`VibrationParticleOption`), as the client's own vibration system
    /// (`VibrationSystem.Ticker`) spawns one.
    pub fn vibration(&mut self, from: [f64; 3], to: [f64; 3], ticks: i32) {
        let mut q = Self::vibration_particle(from, to, ticks);
        q.set = self.set_of(ParticleType::Vibration);
        self.pending.push(PendingParticle::Ordinary(q));
    }
    /// `ParticleEngine.add` (via `particlesToAdd`, applied on the next tick).
    pub fn push(&mut self, particle: Particle) {
        self.pending.push(PendingParticle::Ordinary(particle));
    }
    /// Create a named billboard with custom compiled behavior and no vanilla
    /// motion. Like `add`, respects camera distance and the Particles setting.
    /// Configure its public fields, then `push` it to the ordinary engine queue.
    /// Missing sprite sets return None. The constructor has the ordinary
    /// stationary SingleQuadParticle random size/lifetime and no velocity draws.
    pub fn controlled<C: ParticleController>(
        &mut self,
        name: &str,
        position: [f64; 3],
        translucent: bool,
        controller: C,
    ) -> Option<Particle> {
        let minimal = self.calculate_particle_level(false) == 2;
        let distance: f64 = position
            .iter()
            .zip(self.camera)
            .map(|(p, c)| (p - c).powi(2))
            .sum();
        if minimal || !distance.is_finite() || distance > 1024.0 {
            return None;
        }
        let entry = self.mod_sets.get(name)?;
        let (set, group) = (entry.set, entry.group);
        let mut random = LegacyRandom::new(self.seeds.next_long());
        let mut particle = Particle::at(&mut random, ParticleType::Smoke, position);
        particle.random = random;
        particle.set = Some(set);
        particle.group = group;
        particle.sprite = Sprite::Index(*self.set_sprites.get(set as usize)?.first()?);
        particle.behaviour = Behaviour::Controlled { translucent };
        particle.controller = Some(Box::new(controller));
        Some(particle)
    }
    /// The quads the last render blends additively (`RenderPipelines` additive, src alpha and
    /// one), for the frame's additive pass.
    pub fn take_additive(&self) -> Vec<Vertex> {
        if !self.has_additive.swap(false, Ordering::AcqRel) {
            return Vec::new();
        }
        std::mem::take(&mut *self.additive.lock().unwrap_or_else(|e| e.into_inner()))
    }
    /// Live GPU particle records (spawns, before `area` spreads them), for tests and status.
    pub fn polytone_gpu_records(&self) -> usize {
        self.polytone_gpu.iter().map(polytone_gpu::Store::len).sum()
    }
    /// Choose a frame from this particle's declared sprite set.
    pub fn set_sprite_frame(&self, particle: &mut Particle, frame: usize) {
        if let Some(sprites) = particle.set.and_then(|s| self.set_sprites.get(s as usize))
            && !sprites.is_empty()
        {
            particle.sprite = Sprite::Index(sprites[frame.min(sprites.len() - 1)]);
        }
    }
    /// Random generator shared by particle constructors and spawners.
    pub fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    /// `ClientLevel.random` as client effects use it (`LevelEventHandler`).
    pub fn level_random(&mut self) -> &mut LegacyRandom {
        &mut self.level_random
    }
    /// `ClientLevel.addParticle`: dropped beyond 32 blocks of the camera unless
    /// the type overrides the limiter, and thinned by the Particles option.
    pub fn add(&mut self, options: &ParticleOptions, p: [f64; 3], v: [f64; 3]) {
        self.add_with(options, p, v, false, false);
    }
    /// `ClientLevel.addParticle(particle, overrideLimiter, alwaysShow, ...)`
    /// (`doAddParticle`). `always_show` is `addAlwaysVisibleParticle`'s flag.
    #[horizon_mod_api_macros::replaceable]
    pub fn add_with(
        &mut self,
        options: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
        override_limiter: bool,
        always_show: bool,
    ) {
        let c = self.camera;
        let d2 = (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2);
        if let Some(requested) = &mut self.requested {
            requested.push((options.clone(), p, v));
            return;
        }
        let minimal = self.calculate_particle_level(always_show) == 2;
        if override_limiter || options.ty.override_limiter() || (d2 <= 1024.0 && !minimal) {
            self.create(options, p, v);
        }
    }
    /// `NoxiousGasCloudParticle.spawnNoxiousGasParticle`: the gas, as an
    /// always visible particle (`ClientLevel.addAlwaysVisibleParticle`).
    fn spawn_noxious_gas_particle(&mut self, pos: [f64; 3]) {
        self.add_with(
            &ParticleOptions::simple(ParticleType::NoxiousGas),
            pos,
            [0.0; 3],
            false,
            true,
        );
    }
    /// `ClientLevel.calculateParticleLevel`: minimal still shows one in ten
    /// always-visible particles as decreased, and decreased drops a third.
    fn calculate_particle_level(&mut self, always_show: bool) -> u8 {
        let mut level = self.particle_level.min(2);
        if always_show && level == 2 && self.level_random.next_int_bound(10).unwrap_or(0) == 0 {
            level = 1;
        }
        if level == 1 && self.level_random.next_int_bound(3).unwrap_or(0) == 0 {
            level = 2;
        }
        level
    }
    /// `LevelEventHandler.singleBlockTeleportationParticles` towards
    /// `BlockUtil.unpackDifferenceInPosition(from, data, radius)`: the dragon
    /// egg's event 2015 packs offsets within (16, 8, 16), a consumable's
    /// teleport (2017) within (127, 127, 127).
    pub fn block_teleportation(&mut self, from: [i32; 3], data: i32, radius: [i32; 3]) {
        let to = [
            from[0].wrapping_add(((data >> 16) & 255) - radius[0]),
            from[1].wrapping_add(((data >> 8) & 255) - radius[1]),
            from[2].wrapping_add((data & 255) - radius[2]),
        ]
        .map(f64::from);
        let from = from.map(f64::from);
        let options = ParticleOptions::simple(ParticleType::Portal);
        for _ in 0..128 {
            let r = &mut self.level_random;
            let t = r.next_double();
            let velocity = array::from_fn(|_| f64::from((r.next_float() - 0.5) * 0.2));
            let position = [
                to[0] + t * (from[0] - to[0]) + (r.next_double() - 0.5) + 0.5,
                to[1] + t * (from[1] - to[1]) + r.next_double() - 0.5,
                to[2] + t * (from[2] - to[2]) + (r.next_double() - 0.5) + 0.5,
            ];
            self.add(&options, position, velocity);
        }
    }
    /// `LevelEventHandler.levelEvent` 2018: an enderman's portal trail from
    /// where it was (`from`) to where it went, which `data` packs within
    /// (127, 127, 127). 128 portal particles along the way, spread about the
    /// width and height of an enderman.
    pub fn enderman_teleportation(&mut self, from: [i32; 3], data: i32, particle_level: u8) {
        // EntityTypes.ENDERMAN's dimensions.
        const WIDTH: f64 = 0.6_f32 as f64;
        const HEIGHT: f64 = 2.9_f32 as f64;
        let to = [
            from[0].wrapping_add(((data >> 16) & 255) - 127),
            from[1].wrapping_add(((data >> 8) & 255) - 127),
            from[2].wrapping_add((data & 255) - 127),
        ]
        .map(f64::from);
        let from = from.map(f64::from);
        let options = ParticleOptions::simple(ParticleType::Portal);
        for _ in 0..128 {
            let r = &mut self.level_random;
            let t = r.next_double();
            let velocity = array::from_fn(|_| f64::from((r.next_float() - 0.5) * 0.2));
            // Mth.lerp(t, to, from).
            let position = [
                to[0] + t * (from[0] - to[0]) + (r.next_double() - 0.5) * WIDTH * 2.0,
                to[1] + t * (from[1] - to[1]) + r.next_double() * HEIGHT,
                to[2] + t * (from[2] - to[2]) + (r.next_double() - 0.5) * WIDTH * 2.0,
            ];
            // The option roll shares the level random and precedes the distance check.
            let limited = match particle_level {
                1 => r.next_int_bound(3).unwrap_or(0) == 0,
                2 => true,
                _ => false,
            };
            if !limited {
                self.add(&options, position, velocity);
            }
        }
    }
    /// `LlamaSpit.recreateFromPacket`: seven spit particles thrown ahead of a
    /// new spit, each faster than the last along x and z.
    pub fn llama_spit_burst(&mut self, at: [f64; 3], movement: [f64; 3]) {
        for i in 0..7 {
            let k = 0.4 + 0.1 * f64::from(i);
            self.add(
                &ParticleOptions::simple(ParticleType::Spit),
                at,
                [movement[0] * k, movement[1], movement[2] * k],
            );
        }
    }
    /// `ParticleEngine.createTrackingEmitter` (crits): 16 tries per tick for
    /// three ticks inside a quarter of the entity's box, ticked once at creation.
    pub fn tracking_emitter(
        &mut self,
        options: ParticleOptions,
        target: TrackingTarget,
        body: TrackingBody,
    ) {
        self.tracking_emitter_for(options, target, body, 3);
    }
    /// `createTrackingEmitter(entity, type, lifetime)`.
    pub fn tracking_emitter_for(
        &mut self,
        options: ParticleOptions,
        target: TrackingTarget,
        body: TrackingBody,
        life: u32,
    ) {
        self.emit(&options, body);
        if life > 1 {
            self.emitters.push(TrackingEmitter {
                options,
                target: Some(target),
                body,
                remaining: life - 1,
            });
        }
    }
    /// Vanilla keeps the old entity reference until the emitter expires. Keep
    /// its final body if it leaves the level, even when the id is paired again.
    pub fn detach_tracking_target(
        &mut self,
        target: TrackingTarget,
        mut last_body: impl FnMut() -> Option<TrackingBody>,
    ) {
        for emitter in &mut self.emitters {
            if emitter.target == Some(target) {
                emitter.body = last_body().unwrap_or(emitter.body);
                emitter.target = None;
            }
        }
    }
    /// `TrackingEmitter.tick`: the spawn attempts use the tracked entity's
    /// current feet and dimensions.
    fn emit(&mut self, options: &ParticleOptions, body: TrackingBody) {
        let TrackingBody {
            feet,
            width,
            height,
        } = body;
        for _ in 0..16 {
            let r = &mut self.random;
            let xa = f64::from(r.next_float() * 2.0 - 1.0);
            let ya = f64::from(r.next_float() * 2.0 - 1.0);
            let za = f64::from(r.next_float() * 2.0 - 1.0);
            if xa * xa + ya * ya + za * za <= 1.0 {
                let p = [
                    feet[0] + width * (xa / 4.0),
                    feet[1] + height * (0.5 + ya / 4.0),
                    feet[2] + width * (za / 4.0),
                ];
                self.add(options, p, [xa, ya + 0.2, za]);
            }
        }
    }
    /// Set each frame to the local player's eye while it scopes in first person.
    pub fn set_scope_eye(&mut self, eye: Option<[f64; 3]>) {
        self.scope_eye = eye;
    }
    /// `SpellParticle.isCloseToScopingPlayer`: within 3 blocks of that eye.
    fn close_to_scoping_player(&self, pos: [f64; 3]) -> bool {
        self.scope_eye.is_some_and(|eye| {
            (eye[0] - pos[0]).powi(2) + (eye[1] - pos[1]).powi(2) + (eye[2] - pos[2]).powi(2) <= 9.0
        })
    }
    /// How many particles of each type were made so far.
    pub fn made(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.made
    }
    /// Parity probes: seed the provider (`ParticleEngine.random`), level and
    /// `SpellParticle.RANDOM` generators.
    pub fn seed_randoms(&mut self, provider: i64, level: i64, spell: i64) {
        self.random = LegacyRandom::new(provider);
        self.level_random = LegacyRandom::new(level);
        self.spell_random = LegacyRandom::new(spell);
    }
    /// Seeds each new particle's own random from `seed` (a screen scene's tick), so the
    /// same ticks give the same particles whatever the engine made before.
    pub fn seed_particle_randoms(&mut self, seed: i64) {
        self.seeds = LegacyRandom::new(seed);
    }
    /// Parity probes: the next long of the provider, level and spell generators.
    pub fn next_randoms(&mut self) -> [i64; 3] {
        [
            self.random.next_long(),
            self.level_random.next_long(),
            self.spell_random.next_long(),
        ]
    }
    /// Parity probes: `ParticleProvider.createParticle` with the particle's own
    /// random seeded `seed`, ticked in its constructor where vanilla does.
    pub fn probe_create(
        &mut self,
        options: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
        seed: i64,
        level: &ParticleLevel,
    ) -> Option<Particle> {
        let mut particle = self.make(options, p, v, seed)?;
        if particle.pre_tick {
            particle.pre_tick = false;
            self.tick_particle(&mut particle, level);
            particle.after_constructor_tick();
        }
        Some(particle)
    }
    /// Parity probes: `Particle.tick`.
    pub fn probe_tick(&mut self, particle: &mut Particle, level: &ParticleLevel) {
        self.tick_particle(particle, level);
    }
    /// Parity probes: the state vanilla's particle fields hold.
    pub fn snapshot(&self, p: &Particle) -> ParticleSnapshot {
        let sprite = match &p.sprite {
            Sprite::Index(i) => self.names.get(*i as usize).map_or_else(
                || "-".to_owned(),
                |n| n.strip_prefix("particle/").unwrap_or(n).to_owned(),
            ),
            Sprite::Named(n) => n.clone(),
        };
        let (sky, block) = p.light_coords(0.5, 240, 0);
        ParticleSnapshot {
            age: p.age,
            lifetime: p.lifetime,
            removed: p.removed,
            on_ground: p.on_ground,
            position: p.position,
            velocity: p.velocity,
            previous: p.previous,
            quad: (!p.no_render).then(|| QuadSnapshot {
                size: [p.quad_size(0.0), p.quad_size(0.5)],
                rgba: p.rgba,
                roll: [p.roll, p.o_roll],
                sprite,
                uv: [p.uv[0], p.uv[2], p.uv[1], p.uv[3]],
                light: block | sky << 16,
            }),
        }
    }
    /// `SpriteSet.get(random)`.
    #[allow(dead_code)]
    #[inline]
    fn sprite_from_set(&mut self, ty: ParticleType) -> Sprite {
        self.sprite_from_set_index(self.set_of(ty))
    }
    /// `SpriteSet.get(random)` from an already-resolved sprite set index.
    #[inline]
    fn sprite_from_set_index(&mut self, set: Option<u32>) -> Sprite {
        match set {
            Some(set) => {
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
    #[inline]
    fn set_of(&self, ty: ParticleType) -> Option<u32> {
        self.sets.get(&ty).copied()
    }
    #[allow(dead_code)]
    fn first(&self, ty: ParticleType) -> Sprite {
        self.set_of(ty).map_or(Sprite::Index(u32::MAX), |s| {
            Sprite::Index(self.set_sprites[s as usize][0])
        })
    }
    /// `SingleQuadParticle.setSpriteFromAge`.
    fn sprite_from_age(&self, p: &mut Particle) {
        sprite_from_age_in(&self.set_sprites, p);
    }
    #[allow(clippy::too_many_lines)]
    /// `ParticleProvider.createParticle`: provider draws use the engine's
    /// random, constructor and tick draws the particle's own (seeded `seed`).
    fn make(
        &mut self,
        o: &ParticleOptions,
        p: [f64; 3],
        v: [f64; 3],
        seed: i64,
    ) -> Option<Particle> {
        use ParticleType as T;
        let ty = o.ty;
        // A native mod's SimpleParticleType: vanilla's TextureSheetParticle with
        // the sprite set its particles/<name>.json names, no tint, 20..40 ticks.
        // A pack's custom particle: a new type, or one replacing the vanilla type of its id.
        if let Some(registry) = &self.polytone {
            let custom = match &o.mod_particle {
                Some(name) => Some(name.as_ref()),
                None => self.polytone_overrides.get(&ty).map(String::as_str),
            };
            if let Some(name) = custom
                && let Some(def) = registry.get(name)
            {
                let (registry, def, name) = (registry.clone(), def.clone(), name.to_owned());
                let particle = polytone::make(self, &registry, &name, &def, o, p, v, seed)?;
                if def.gpu.is_some() {
                    // A GPU particle is only a record: it never becomes an engine particle.
                    let now = self.polytone_ticks;
                    let store = match self.polytone_gpu.iter().position(|s| s.is(&def)) {
                        Some(i) => &mut self.polytone_gpu[i],
                        None => {
                            self.polytone_gpu.push(polytone_gpu::Store::new(&def)?);
                            self.polytone_gpu.last_mut()?
                        }
                    };
                    store.spawn(&particle, now, seed as u64);
                    return None;
                }
                return Some(particle);
            }
        }
        if let Some(name) = &o.mod_particle {
            let entry = self.mod_sets.get(name.as_ref())?;
            let (set, behaviour, provider, group) = (
                entry.set,
                entry.behaviour,
                entry.provider.clone(),
                entry.group,
            );
            let mut r = LegacyRandom::new(seed);
            let mut q = Particle::moving(&mut r, ParticleType::Smoke, p, v);
            q.ty = ParticleType::Smoke;
            q.gravity = 0.0;
            q.rgba = [1.0; 4];
            q.lifetime = 20 + r.next_int_bound(20).unwrap_or(0);
            q.set = Some(set);
            q.group = group;
            let sprites = &self.set_sprites[set as usize];
            q.sprite = Sprite::Index(
                sprites[r.next_int_bound(sprites.len() as i32).unwrap_or(0) as usize],
            );
            if let Some(behaviour) = behaviour {
                behaviour.apply(&mut q, v, &mut r);
                if behaviour.starts_on_first_sprite() {
                    q.sprite = Sprite::Index(sprites[0]);
                }
            }
            if let Some(provider) = provider {
                // The declared values seed the particle; the provider owns its tick from here.
                // Its sprite is chosen by frame, so the set stays attached.
                let frame = sprites
                    .iter()
                    .position(|&s| matches!(q.sprite, Sprite::Index(i) if i == s))
                    .unwrap_or(0);
                q.set = Some(set);
                q.behaviour = Behaviour::Controlled { translucent: true };
                q.controller = Some(Box::new(native::Provided::new(provider, frame as u32)));
            }
            return Some(q);
        }
        let set = self.set_of(ty);
        let mut local = LegacyRandom::new(seed);
        let r = &mut local;
        let mut particle = match ty {
            T::Block | T::BlockCrumble | T::DustPillar => {
                let (sprite, tint) = o.block.clone()?;
                let mut q = Particle::terrain(r, p, v, sprite, tint);
                q.ty = ty;
                let pr = &mut self.random;
                if ty == T::BlockCrumble {
                    q.velocity = [0.0; 3];
                    q.lifetime = pr.next_int_bound(10).unwrap_or(0) + 1;
                } else if ty == T::DustPillar {
                    q.velocity = [
                        pr.next_gaussian() / 30.0,
                        v[1] + pr.next_gaussian() / 2.0,
                        pr.next_gaussian() / 30.0,
                    ];
                    q.lifetime = pr.next_int_bound(20).unwrap_or(0) + 20;
                }
                q
            }
            T::FallingDust => {
                let &(_, tint) = o.block.as_ref()?;
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
            T::Smoke
            | T::LargeSmoke
            | T::WhiteSmoke
            | T::Ash
            | T::WhiteAsh
            | T::DustPlume
            | T::NoxiousGas
            | T::GeyserBase
            | T::GeyserPoof => {
                // BaseAshSmokeParticle(dir, xa, scale, colorRandom, maxLifetime, gravity, hasPhysics).
                let (dir, scale, color_random, max_lifetime, gravity, physics, vel) = match ty {
                    T::Smoke | T::WhiteSmoke => ([0.1, 0.1, 0.1], 1.0, 0.3, 8, -0.1, true, v),
                    T::LargeSmoke => ([0.1, 0.1, 0.1], 2.5, 0.3, 8, -0.1, true, v),
                    T::Ash => ([0.1, -0.1, 0.1], 1.0, 0.5, 20, 0.1, false, [0.0; 3]),
                    // DustPlumeParticle: ya + 0.15F.
                    T::DustPlume => (
                        [0.7, 0.6, 0.7],
                        1.0,
                        0.5,
                        7,
                        0.5,
                        false,
                        [v[0], v[1] + f64::from(0.15f32), v[2]],
                    ),
                    // NoxiousGasParticle.Provider: scale 3.
                    T::NoxiousGas => ([0.1, 0.1, 0.1], 3.0, 0.3, 5, -0.02, true, v),
                    // GeyserBaseParticle: the burst impulse scales every axis.
                    T::GeyserBase | T::GeyserPoof => {
                        let (water_blocks, burst_impulse_base) = o.geyser?;
                        let burst_impulse = burst_impulse_base + 0.25 * water_blocks as f32;
                        (
                            [f64::from(burst_impulse); 3],
                            3.0 + 0.125 * water_blocks as f32,
                            0.0,
                            0,
                            0.0,
                            true,
                            v,
                        )
                    }
                    _ => {
                        // WhiteAshParticle.Provider draws the motion.
                        let pr = &mut self.random;
                        let xa =
                            f64::from(pr.next_float()) * -1.9 * f64::from(pr.next_float()) * 0.1;
                        let ya = f64::from(pr.next_float())
                            * -0.5
                            * f64::from(pr.next_float())
                            * 0.1
                            * 5.0;
                        let za =
                            f64::from(pr.next_float()) * -1.9 * f64::from(pr.next_float()) * 0.1;
                        ([0.1, -0.1, 0.1], 1.0, 0.0, 20, 0.0125, false, [xa, ya, za])
                    }
                };
                // GeyserBaseParticle.Provider: a random spot half a block
                // around the point, a fifth of a block up (the provider's
                // random, as float arithmetic added to the double position).
                let p = if matches!(ty, T::GeyserBase | T::GeyserPoof) {
                    let pr = &mut self.random;
                    let x = p[0] + f64::from((pr.next_float() - 0.5) * 0.5);
                    let y = p[1] + f64::from((pr.next_float() - 0.5) * 0.5) + f64::from(0.2f32);
                    let z = p[2] + f64::from((pr.next_float() - 0.5) * 0.5);
                    [x, y, z]
                } else {
                    p
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
                if matches!(ty, T::WhiteSmoke | T::WhiteAsh) {
                    q.set_color(186.0 / 255.0, 177.0 / 255.0, 194.0 / 255.0);
                }
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                if ty == T::DustPlume {
                    let shift = r.next_float() * 0.2;
                    q.set_color(
                        186.0 / 255.0 - shift,
                        177.0 / 255.0 - shift,
                        194.0 / 255.0 - shift,
                    );
                    q.behaviour = Behaviour::DustPlume;
                }
                match ty {
                    // NoxiousGasParticle: white, living `6 / (0.5 to 1) * 3`
                    // ticks, fading out over its second half.
                    T::NoxiousGas => {
                        q.set_color(1.0, 1.0, 1.0);
                        q.lifetime = (6.0 / (f64::from(r.next_float()) * 0.5 + 0.5)
                            * f64::from(3.0f32)) as i32;
                        q.behaviour = Behaviour::NoxiousGas {
                            fade_start: q.lifetime as f32 / 2.0,
                        };
                    }
                    // GeyserBaseParticle: white, only ever rising, 20 to 25 ticks.
                    T::GeyserBase | T::GeyserPoof => {
                        q.friction = 0.725;
                        q.set_color(1.0, 1.0, 1.0);
                        q.velocity[1] = q.velocity[1].abs();
                        let lifetime_factor = 0.8 + 0.2 * self.level_random.next_float();
                        q.lifetime = (25.0 * lifetime_factor) as i32;
                    }
                    _ => {}
                }
                q.set = set;
                q
            }
            // NoxiousGasCloudParticle: lives 20 ticks, draws nothing itself.
            T::NoxiousGasCloud => {
                let mut q = Particle::base_at(r, ty, p);
                q.lifetime = 20;
                q.no_render = true;
                q.behaviour = Behaviour::NoxiousGasCloud;
                q
            }
            // GeyserEruptionParticle: lives 20 ticks, draws nothing itself.
            T::Geyser => {
                let (water_blocks, _) = o.geyser?;
                let mut q = Particle::base_at(r, ty, p);
                q.lifetime = 20;
                q.no_render = true;
                q.behaviour = Behaviour::GeyserEruption {
                    water_blocks,
                    aux: v,
                };
                q
            }
            // GeyserPlumeParticle, after its provider's random placement.
            T::GeyserPlume => {
                let (water_blocks, _) = o.geyser?;
                let pr = &mut self.random;
                let at = [
                    p[0] + f64::from((pr.next_float() - 0.5) * 0.2),
                    p[1] + f64::from(pr.next_float()),
                    p[2] + f64::from((pr.next_float() - 0.5) * 0.2),
                ];
                let plume_height = 5 * water_blocks.max(1);
                let mut q = Particle::moving(r, ty, at, v);
                q.has_physics = true;
                q.speed_up_when_y_blocked = true;
                q.lifetime = plume_height * 5;
                q.velocity[1] = 0.0;
                let level = &mut self.level_random;
                let spray = [
                    (level.next_float() - 0.5) * 0.2,
                    (level.next_float() - 0.5) * 0.2,
                ];
                q.friction = 1.0;
                let propulsion =
                    (if water_blocks == 1 { 1.5 } else { 1.0 }) * plume_height as f32 * 1.45;
                q.gravity = -propulsion;
                let initial_size = q.size * 0.75;
                let min_size = initial_size * (2.0 + plume_height as f32 / 8.0);
                let max_size = initial_size * (3.0 + plume_height as f32 / 8.0);
                q.size = min_size;
                q.behaviour = Behaviour::GeyserPlume {
                    start_y: at[1],
                    max_y: at[1] + f64::from(plume_height) - 1.0,
                    propulsion,
                    spray,
                    min_size,
                    max_size,
                    done: false,
                };
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
                // The provider picks the sprite first (`sprite.get(random)`).
                let sprite = self.sprite_from_set_index(set);
                let mut q = Particle::at(r, ty, p);
                q.sprite = sprite;
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
                    // Lifetimes the providers draw from their own random.
                    T::LandingHoney => {
                        q.lifetime =
                            (128.0 / (f64::from(self.random.next_float()) * 0.8 + 0.2)) as i32
                    }
                    T::LandingObsidianTear => {
                        q.lifetime =
                            (28.0 / (f64::from(self.random.next_float()) * 0.8 + 0.2)) as i32
                    }
                    T::FallingNectar => {
                        q.lifetime =
                            (16.0 / (f64::from(self.random.next_float()) * 0.8 + 0.2)) as i32;
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
            T::SulfurBubbles => {
                let mut q = Particle::at(r, ty, p);
                q.gravity = -0.04;
                q.friction = 0.85;
                q.set_size(0.02, 0.02);
                q.velocity[0] =
                    v[0] * f64::from(0.2f32) + f64::from((r.next_float() * 2.0 - 1.0) * 0.02);
                // The provider passes `yAux` as `za`, so the Z speed comes from Y.
                q.velocity[2] =
                    v[1] * f64::from(0.2f32) + f64::from((r.next_float() * 2.0 - 1.0) * 0.02);
                let size_start = 0.02 + 0.02 * r.next_float();
                q.size = size_start;
                q.lifetime = i32::MAX;
                q.behaviour = Behaviour::SulfurBubble {
                    y_start: p[1],
                    y_end: p[1] + 4.0 - 1.0,
                    size_start,
                    y_prev: p[1],
                };
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
                // SuspendedParticle; the providers draw motion, then the sprite.
                let below = [p[0], p[1] - 0.125, p[2]];
                let pr = &mut self.random;
                let vel = match ty {
                    T::CrimsonSpore => [
                        pr.next_gaussian() * f64::from(1.0e-6f32),
                        pr.next_gaussian() * f64::from(1.0e-4f32),
                        pr.next_gaussian() * f64::from(1.0e-6f32),
                    ],
                    T::WarpedSpore => [
                        0.0,
                        f64::from(pr.next_float()) * -1.9 * f64::from(pr.next_float()) * 0.1,
                        0.0,
                    ],
                    _ => [0.0, f64::from(-0.8f32), 0.0],
                };
                let sprite = self.sprite_from_set_index(set);
                let mut q = if ty == T::Underwater {
                    let mut q = Particle::at(r, ty, below);
                    q.set_size(0.01, 0.01);
                    q.size *= r.next_float() * 0.6 + 0.2;
                    q
                } else {
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
                        q.lifetime = self.random.next_int_bound(501).unwrap_or(0) + 500;
                        q.gravity = 0.01;
                        q.set_color(0.32, 0.5, 0.22);
                    }
                }
                q.sprite = sprite;
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
                // SuspendedTownParticle; the provider picks the sprite first.
                let sprite = self.sprite_from_set_index(set);
                let mut q = Particle::moving(r, ty, p, v);
                q.sprite = sprite;
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
                        q.lifetime = 3 + self.level_random.next_int_bound(5).unwrap_or(0);
                    }
                    T::Dolphin => {
                        q.set_color(0.3, 0.5, 1.0);
                        q.rgba[3] = 1.0 - self.random.next_float() * 0.7;
                        q.lifetime /= 2;
                    }
                    _ => {}
                }
                q
            }
            T::Poof | T::Snowflake | T::Spit => {
                let mut q = Particle::at(r, ty, p);
                if ty != T::Snowflake {
                    q.gravity = -0.1;
                    q.friction = 0.9;
                } else {
                    q.gravity = 0.225;
                    q.friction = 1.0;
                }
                for i in 0..3 {
                    q.velocity[i] = v[i] + f64::from((r.next_float() * 2.0 - 1.0) * 0.05);
                }
                if ty != T::Snowflake {
                    let col = r.next_float() * 0.3 + 0.7;
                    q.set_color(col, col, col);
                    q.size = 0.1 * (r.next_float() * r.next_float() * 6.0 + 1.0);
                } else {
                    q.size = 0.1 * (r.next_float() * r.next_float() + 1.0);
                }
                q.lifetime = (16.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32 + 2;
                q.behaviour = if ty == T::Snowflake {
                    Behaviour::Snowflake
                } else {
                    Behaviour::Animated
                };
                // SpitParticle: ExplodeParticle falling at gravity 0.5.
                if ty == T::Spit {
                    q.gravity = 0.5;
                }
                if ty == T::Snowflake {
                    q.set_color(0.923, 0.964, 0.999);
                }
                q.set = set;
                q
            }
            T::Explosion | T::SonicBoom => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.lifetime = 6 + r.next_int_bound(4).unwrap_or(0);
                let col = r.next_float() * 0.6 + 0.4;
                q.set_color(col, col, col);
                q.size = 2.0 * (1.0 - v[0] as f32 * 0.5);
                if ty == T::SonicBoom {
                    // SonicBoomParticle.
                    q.lifetime = 16;
                    q.size = 1.5;
                }
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            T::ExplosionEmitter => {
                let mut q = Particle::base_moving(r, ty, p, [0.0; 3]);
                q.lifetime = 8;
                q.no_render = true;
                q.behaviour = Behaviour::ExplosionSeed;
                q
            }
            // GustParticle: a still, bright, opaque quad that animates over
            // its 12 to 15 ticks; the small one is scaled to 15%.
            T::Gust | T::SmallGust => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = 12 + r.next_int_bound(4).unwrap_or(0);
                q.size = 1.0;
                q.set_size(1.0, 1.0);
                if ty == T::SmallGust {
                    q.scale(0.15);
                }
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            T::GustEmitterLarge | T::GustEmitterSmall => {
                let mut q = Particle::base_moving(r, ty, p, [0.0; 3]);
                let (scale, lifetime, delay) = if ty == T::GustEmitterLarge {
                    (3.0, 7, 0)
                } else {
                    (1.0, 3, 2)
                };
                q.lifetime = lifetime;
                q.no_render = true;
                q.behaviour = Behaviour::GustSeed { scale, delay };
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
                // `Mth.sin` (the quantized table) of a float angle.
                let channel = |offset: f32| {
                    (trig::sin(f64::from((color + offset) * std::f64::consts::TAU as f32)) * 0.65
                        + 0.35)
                        .max(0.0)
                };
                q.set_color(channel(0.0), channel(0.333_333_34), channel(0.666_666_7));
                q.size *= 1.5;
                q.lifetime = 6;
                q.size_curve = Size::GrowIn;
                q
            }
            T::DustColorTransition => {
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
                let from = [
                    randomize(o.color[0]),
                    randomize(o.color[1]),
                    randomize(o.color[2]),
                ];
                let to = [
                    randomize(o.to_color[0]),
                    randomize(o.to_color[1]),
                    randomize(o.to_color[2]),
                ];
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Transition { from, to };
                q.set = set;
                q
            }
            // SculkChargeParticle / SculkChargePopParticle.
            T::SculkCharge | T::SculkChargePop => {
                let mut q = Particle::moving(r, ty, p, v);
                q.friction = 0.96;
                q.scale(if ty == T::SculkCharge { 1.5 } else { 1.0 });
                q.has_physics = false;
                q.velocity = v;
                q.light = Light::Block15;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                if ty == T::SculkCharge {
                    q.roll = o.scale;
                    q.o_roll = o.scale;
                    q.lifetime = self.random.next_int_bound(12).unwrap_or(0) + 8;
                } else {
                    q.lifetime = self.random.next_int_bound(4).unwrap_or(0) + 6;
                }
                q
            }
            // VibrationSignalParticle.Provider: `VibrationParticleOption` carries
            // the listener the signal flies to and its arrival time; the sprite
            // is the set's own draw from the engine's random.
            T::Vibration => {
                let (_, dest, ticks) = o.vibration.as_ref()?;
                let mut q = Self::vibration_particle(p, *dest, *ticks);
                q.set = set;
                q.sprite = self.sprite_from_set_index(set);
                q
            }
            // ShriekParticle.
            T::Shriek => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.size = 0.85;
                q.lifetime = 30;
                q.gravity = 0.0;
                q.velocity = [0.0, 0.1, 0.0];
                q.light = Light::Block15;
                q.size_curve = Size::Shriek;
                q.behaviour = Behaviour::Shriek {
                    delay: o.scale as i32,
                };
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
            // FireworkParticles.SparkProvider (a rocket's trail).
            T::Firework => {
                let mut q = Particle::at(r, ty, p);
                q.velocity = v;
                q.gravity = 0.1;
                q.friction = 0.91;
                q.size *= 0.75;
                q.lifetime = 48 + r.next_int_bound(12).unwrap_or(0);
                q.rgba[3] = 0.99;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::Spark {
                    fade: None,
                    trail: false,
                    twinkle: false,
                };
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
            T::CherryLeaves
            | T::PaleOakLeaves
            | T::TintedLeaves
            | T::RedPoplarLeaves
            | T::OrangePoplarLeaves
            | T::YellowPoplarLeaves => {
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
            T::Effect
            | T::InstantEffect
            | T::EntityEffect
            | T::Witch
            | T::Infested
            | T::TrialOmen
            | T::RaidOmen => {
                // SpellParticle: x/z velocity from `SpellParticle.RANDOM`, y from the option.
                let xa = 0.5 - self.spell_random.next_double();
                let za = 0.5 - self.spell_random.next_double();
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
                // SpellParticle's constructor: setAlpha(0.0F), which also sets originalAlpha.
                // MobEffectProvider sets the option's alpha after it, so an entity
                // effect starts visible and the first tick hides it.
                if self.close_to_scoping_player(q.position) {
                    q.rgba[3] = 0.0;
                }
                match ty {
                    T::Witch => {
                        let b = self.random.next_float() * 0.5 + 0.35;
                        q.set_color(b, 0.0, b);
                    }
                    T::EntityEffect => {
                        if let Some(ambient) = o.legacy_entity_effect {
                            q.set_color(v[0] as f32, v[1] as f32, v[2] as f32);
                            // AmbientMobProvider overrides constructor alpha; MobProvider only sets RGB.
                            if ambient {
                                q.rgba[3] = 0.15;
                            }
                        } else {
                            q.set_color(o.color[0], o.color[1], o.color[2]);
                            q.rgba[3] = o.color[3];
                        }
                    }
                    T::Effect | T::InstantEffect => {
                        q.set_color(o.color[0], o.color[1], o.color[2]);
                        q.set_power(o.scale);
                    }
                    _ => {}
                }
                q.behaviour = Behaviour::Spell {
                    // The old SpellParticle.tick always eased back to 1, including ambient effects.
                    original_alpha: if o.legacy_entity_effect.is_some() {
                        1.0
                    } else {
                        q.rgba[3]
                    },
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
            // AttackSweepParticle: `xAux` shrinks it; still, bright, animated over 4 ticks.
            T::SweepAttack => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.lifetime = 4;
                let col = r.next_float() * 0.6 + 0.4;
                q.set_color(col, col, col);
                q.size = 1.0 - v[0] as f32 * 0.5;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::HugeExplosion;
                q.set = set;
                q
            }
            // GlowParticle and its providers.
            T::Glow | T::WaxOn | T::WaxOff | T::ElectricSpark | T::Scrape => {
                let pr = &mut self.random;
                let start = if ty == T::Glow {
                    let xa = 0.5 - pr.next_double();
                    [xa, v[1], 0.5 - pr.next_double()]
                } else {
                    [0.0; 3]
                };
                let mut q = Particle::moving(r, ty, p, start);
                q.friction = 0.96;
                q.speed_up_when_y_blocked = true;
                q.size *= 0.75;
                q.has_physics = false;
                q.light = Light::AgeEmission;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                match ty {
                    T::ElectricSpark => {
                        q.set_color(1.0, 0.9, 1.0);
                        q.velocity = [v[0] * 0.25, v[1] * 0.25, v[2] * 0.25];
                        q.lifetime = pr.next_int_bound(2).unwrap_or(0) + 2;
                    }
                    T::Glow => {
                        if pr.next_boolean() {
                            q.set_color(0.6, 1.0, 0.8);
                        } else {
                            q.set_color(0.08, 0.4, 0.4);
                        }
                        q.velocity[1] *= f64::from(0.2f32);
                        if v[0] == 0.0 && v[2] == 0.0 {
                            q.velocity[0] *= f64::from(0.1f32);
                            q.velocity[2] *= f64::from(0.1f32);
                        }
                        q.lifetime = (8.0 / (pr.next_double() * 0.8 + 0.2)) as i32;
                    }
                    T::Scrape => {
                        if pr.next_boolean() {
                            q.set_color(0.29, 0.58, 0.51);
                        } else {
                            q.set_color(0.43, 0.77, 0.62);
                        }
                        q.velocity = [v[0] * 0.01, v[1] * 0.01, v[2] * 0.01];
                        q.lifetime = pr.next_int_bound(30).unwrap_or(0) + 10;
                    }
                    _ => {
                        if ty == T::WaxOn {
                            q.set_color(0.91, 0.55, 0.08);
                        } else {
                            q.set_color(1.0, 0.9, 1.0);
                        }
                        q.velocity = [v[0] * 0.01 / 2.0, v[1] * 0.01, v[2] * 0.01 / 2.0];
                        q.lifetime = pr.next_int_bound(30).unwrap_or(0) + 10;
                    }
                }
                q
            }
            // SquidInkParticle (SimpleAnimatedParticle, gravity 0).
            T::SquidInk | T::GlowSquidInk => {
                let mut q = Particle::at(r, ty, p);
                q.friction = 0.92;
                q.size = 0.5;
                if ty == T::GlowSquidInk {
                    // ARGB.colorFromFloat(1, 0.2, 0.8, 0.6), back to floats.
                    q.set_color(51.0 / 255.0, 204.0 / 255.0, 153.0 / 255.0);
                } else {
                    q.set_color(0.0, 0.0, 0.0);
                }
                q.lifetime = (q.size * 12.0 / (r.next_float() * 0.8 + 0.2)) as i32;
                q.has_physics = false;
                q.velocity = v;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::SquidInk;
                q.set = set;
                q
            }
            // BreakingItemParticle: a crumb of the item's sprite.
            T::Item | T::ItemSlime | T::ItemSnowball | T::ItemCobweb | T::SulfurCubeGoo => {
                let sprite = if ty == T::SulfurCubeGoo {
                    set.map_or(Sprite::Index(u32::MAX), |s| {
                        Sprite::Index(self.set_sprites[s as usize][0])
                    })
                } else {
                    // ItemStackRenderState.pickParticleMaterial.
                    let layers: &[String] = match ty {
                        T::Item => o.item.as_deref().unwrap_or(&[]),
                        _ => self.item_sprites.get(&ty).map_or(&[], Vec::as_slice),
                    };
                    if layers.is_empty() {
                        Sprite::Index(u32::MAX)
                    } else {
                        let i = self.random.next_int_bound(layers.len() as i32).unwrap_or(0);
                        Sprite::Named(layers[i as usize].clone())
                    }
                };
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.sprite = sprite;
                q.gravity = 1.0;
                q.size /= 2.0;
                let uo = r.next_float() * 3.0;
                let vo = r.next_float() * 3.0;
                q.uv = [(uo + 1.0) / 4.0, vo / 4.0, uo / 4.0, (vo + 1.0) / 4.0];
                if ty == T::Item {
                    for i in 0..3 {
                        q.velocity[i] = q.velocity[i] * f64::from(0.1f32) + v[i];
                    }
                }
                q
            }
            // FireworkParticles.OverlayParticle: the flash of an explosion.
            T::Flash => {
                let mut q = Particle::at(r, ty, p);
                q.lifetime = 4;
                q.set_color(o.color[0], o.color[1], o.color[2]);
                q.rgba[3] = o.color[3];
                q.size_curve = Size::Flash;
                q
            }
            // FireflyParticle; the provider draws its motion, sprite and lifetime.
            T::Firefly => {
                let pr = &mut self.random;
                let xa = 0.5 - pr.next_double();
                let ya = if pr.next_boolean() { v[1] } else { -v[1] };
                let za = 0.5 - pr.next_double();
                let sprite = self.sprite_from_set_index(set);
                let mut q = Particle::moving(r, ty, p, [xa, ya, za]);
                q.sprite = sprite;
                q.speed_up_when_y_blocked = true;
                q.friction = 0.96;
                q.size *= 0.75;
                q.velocity[1] *= f64::from(0.8f32);
                q.velocity[0] *= f64::from(0.8f32);
                q.velocity[2] *= f64::from(0.8f32);
                q.lifetime = self
                    .random
                    .next_int_between_inclusive(200, 300)
                    .unwrap_or(200);
                q.scale(1.5);
                q.rgba[3] = 0.0;
                q.light = Light::Firefly;
                q.behaviour = Behaviour::Firefly;
                q
            }
            // TrialSpawnerDetectionParticle (scale 1.5 from the provider).
            T::TrialSpawnerDetection | T::TrialSpawnerDetectionOminous => {
                let scale = 1.5f32;
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.friction = 0.96;
                q.gravity = -0.1;
                q.speed_up_when_y_blocked = true;
                q.velocity[0] *= 0.0;
                q.velocity[1] *= 0.9;
                q.velocity[2] *= 0.0;
                for i in 0..3 {
                    q.velocity[i] += v[i];
                }
                q.size *= 0.75 * scale;
                // Mth.randomBetween(random, 0.5, 1.0).
                q.lifetime = ((8.0 / (r.next_float() * (1.0 - 0.5) + 0.5) * scale) as i32).max(1);
                q.light = Light::Block15;
                q.size_curve = Size::GrowIn;
                q.behaviour = Behaviour::Animated;
                q.set = set;
                q
            }
            // FlyStraightTowardsParticle.OminousSpawnProvider.
            T::OminousSpawning => {
                let sprite = self.sprite_from_set_index(set);
                let mut q = Particle::at(r, ty, p);
                q.sprite = sprite;
                q.velocity = v;
                let start = [p[0] + v[0], p[1] + v[1], p[2] + v[2]];
                q.previous = start;
                q.position = start;
                q.size = 0.1 * (r.next_float() * 0.5 + 0.2);
                q.has_physics = false;
                q.free_move = true;
                q.lifetime = (r.next_float() * 5.0) as i32 + 25;
                q.light = Light::Block15;
                q.behaviour = Behaviour::FlyStraight {
                    start: p,
                    colors: [0xFF45_AEFE, 0xFFFF_FFFF],
                };
                // Mth.randomBetween(level.getRandom(), 3, 5).
                let s = self.level_random.next_float() * (5.0 - 3.0) + 3.0;
                q.scale(s);
                q
            }
            // WakeParticle.
            T::Fishing => {
                let mut q = Particle::moving(r, ty, p, [0.0; 3]);
                q.velocity[0] *= f64::from(0.3f32);
                q.velocity[1] = f64::from(r.next_float() * 0.2 + 0.1);
                q.velocity[2] *= f64::from(0.3f32);
                q.set_size(0.01, 0.01);
                q.lifetime = (8.0 / (f64::from(r.next_float()) * 0.8 + 0.2)) as i32;
                q.gravity = 0.0;
                q.velocity = v;
                q.behaviour = Behaviour::Wake;
                q.set = set;
                q
            }
            // SimpleVerticalParticle.
            T::PauseMobGrowth | T::ResetMobGrowth => {
                let mut q = Particle::moving(r, ty, p, v);
                q.velocity = v;
                q.gravity = 0.0;
                q.velocity[1] += if ty == T::ResetMobGrowth { 0.03 } else { -0.03 };
                q.size *= r.next_float() * 0.6 + 0.5;
                q.lifetime = 8;
                q
            }
            // BlockMarker: the block's particle sprite, half a block wide, 80 ticks.
            T::BlockMarker => {
                let (sprite, _) = o.block.clone()?;
                let mut q = Particle::at(r, ty, p);
                q.sprite = Sprite::Named(sprite);
                q.gravity = 0.0;
                q.lifetime = 80;
                q.has_physics = false;
                q.size = 0.5;
                q
            }
            T::Trail => {
                let (target, duration) = o.trail?;
                let mut q = Particle::moving(r, ty, p, v);
                let jitter = |r: &mut LegacyRandom| 0.875 + r.next_float() * 0.25;
                let (jr, jg, jb) = (jitter(r), jitter(r), jitter(r));
                // `ARGB.scaleRGB` on the 8-bit channels.
                let scaled = |c: f32, s: f32| {
                    ((((c * 255.0).round() as i32) as f32 * s) as i32).clamp(0, 255) as f32 / 255.0
                };
                q.set_color(
                    scaled(o.color[0], jr),
                    scaled(o.color[1], jg),
                    scaled(o.color[2], jb),
                );
                q.size = 0.26;
                q.lifetime = if o.legacy_trail_lifetime {
                    self.level_random.next_int_bound(40).unwrap_or(0) + 10
                } else {
                    duration
                };
                q.has_physics = false;
                q.free_move = true;
                q.light = Light::FullBright;
                q.behaviour = Behaviour::Trail { target };
                q
            }
        };
        if particle.set.is_some() {
            self.sprite_from_age(&mut particle);
        } else if matches!(particle.sprite, Sprite::Index(u32::MAX)) {
            particle.sprite = self.sprite_from_set_index(set);
        }
        particle.random = local;
        Some(particle)
    }

    /// `ParticleEngine.tick`: tick live particles, then admit queued ones.
    /// A firework spark (`SparkParticle`: gravity 0.1, friction 0.91, 3/4 size,
    /// 48..59 ticks).
    fn spark(
        &mut self,
        at: [f64; 3],
        v: [f64; 3],
        rgba: [f32; 4],
        fade: Option<[f32; 3]>,
        trail: bool,
        twinkle: bool,
    ) -> Particle {
        let mut local = LegacyRandom::new(self.seeds.next_long());
        let mut q = Particle::at(&mut local, ParticleType::Firework, at);
        q.velocity = v;
        q.gravity = 0.1;
        q.friction = 0.91;
        q.size *= 0.75;
        q.lifetime = 48 + local.next_int_bound(12).unwrap_or(0);
        q.random = local;
        q.rgba = [rgba[0], rgba[1], rgba[2], 0.99];
        q.light = Light::FullBright;
        q.behaviour = Behaviour::Spark {
            fade,
            trail,
            twinkle,
        };
        q.set = self.set_of(ParticleType::Firework);
        self.sprite_from_age(&mut q);
        q
    }
    /// Queue an elder guardian apparition, admitted with the other particles after ticking.
    pub fn elder_guardian(&mut self) {
        self.pending.push(PendingParticle::ElderGuardian);
    }
    /// Ticks left for each admitted `ElderGuardianParticle`, for its model renderer.
    pub fn elder_guardians(&self) -> &[i32] {
        &self.elder_guardians
    }
    /// ClientLevel.createFireworks: queue a Starter (or poofs without explosions).
    /// Like other particles it is admitted after the next tick, without being aged.
    pub fn fireworks(
        &mut self,
        at: [f64; 3],
        v: [f64; 3],
        explosions: Vec<FireworkExplosion>,
        camera: [f64; 3],
        play_sound: bool,
    ) {
        if explosions.is_empty() {
            for _ in 0..self.random.next_int_bound(3).unwrap_or(0) + 2 {
                let g = |r: &mut LegacyRandom| r.next_gaussian() * 0.05;
                let (vx, vz) = (g(&mut self.random), g(&mut self.random));
                self.add(
                    &ParticleOptions::simple(ParticleType::Poof),
                    at,
                    [vx, 0.005, vz],
                );
            }
            return;
        }
        let twinkle = explosions.iter().any(|e| e.twinkle);
        let lifetime = explosions.len() as i32 * 2 - 1 + if twinkle { 15 } else { 0 };
        let far = (0..3).map(|i| (at[i] - camera[i]).powi(2)).sum::<f64>() >= 256.0;
        self.pending.push(PendingParticle::Starter(Starter {
            at,
            v,
            explosions,
            life: 0,
            lifetime,
            twinkle_delay: twinkle,
            sound: play_sound.then_some(far),
        }));
    }
    fn tick_starters(&mut self) {
        if self.starters.is_empty() {
            return;
        }
        let mut starters = std::mem::take(&mut self.starters);
        starters.retain_mut(|s| {
            if s.life == 0 && s.sound.is_some() {
                let large = s.explosions.len() >= 3
                    || s.explosions
                        .iter()
                        .any(|e| e.shape == FireworkShape::LargeBall);
                let sound = match (large, s.sound == Some(true)) {
                    (true, true) => "minecraft:entity.firework_rocket.large_blast_far",
                    (true, false) => "minecraft:entity.firework_rocket.large_blast",
                    (false, true) => "minecraft:entity.firework_rocket.blast_far",
                    (false, false) => "minecraft:entity.firework_rocket.blast",
                };
                let pitch = 0.95 + self.random.next_float() * 0.1;
                self.sounds.push((sound, s.at, 20.0, pitch, true));
            }
            if s.life % 2 == 0 && ((s.life / 2) as usize) < s.explosions.len() {
                let e = &s.explosions[(s.life / 2) as usize];
                // DyeColor.BLACK.getFireworkColor without colours.
                let colors: &[u32] = if e.colors.is_empty() {
                    &[0x1E1B1B]
                } else {
                    &e.colors
                };
                match e.shape {
                    FireworkShape::SmallBall => self.spark_ball(s, 0.25, 2, colors, e),
                    FireworkShape::LargeBall => self.spark_ball(s, 0.5, 4, colors, e),
                    FireworkShape::Star => self.spark_shape(s, 0.5, &STAR, colors, e, false),
                    FireworkShape::Creeper => self.spark_shape(s, 0.5, &CREEPER, colors, e, true),
                    FireworkShape::Burst => {
                        let g = |r: &mut LegacyRandom| r.next_gaussian();
                        let (bx, bz) = (g(&mut self.random) * 0.05, g(&mut self.random) * 0.05);
                        for _ in 0..70 {
                            let xa = s.v[0] * 0.5 + g(&mut self.random) * 0.15 + bx;
                            let za = s.v[2] * 0.5 + g(&mut self.random) * 0.15 + bz;
                            let ya = s.v[1] * 0.5 + self.random.next_double() * 0.5;
                            self.spark_from(s, [xa, ya, za], colors, e);
                        }
                    }
                }
                // The flash: ColorParticleOption.create(FLASH, first colour), alpha 0.
                let c = colors[0];
                let rgba = [
                    ((c >> 16) & 255) as f32 / 255.0,
                    ((c >> 8) & 255) as f32 / 255.0,
                    (c & 255) as f32 / 255.0,
                    ((c >> 24) & 255) as f32 / 255.0,
                ];
                let flash = ParticleOptions::colored(ParticleType::Flash, rgba);
                self.create(&flash, s.at, [0.0; 3]);
            }
            s.life += 1;
            if s.life > s.lifetime {
                if s.twinkle_delay && s.sound.is_some() {
                    let sound = if s.sound == Some(true) {
                        "minecraft:entity.firework_rocket.twinkle_far"
                    } else {
                        "minecraft:entity.firework_rocket.twinkle"
                    };
                    let pitch = 0.9 + self.random.next_float() * 0.15;
                    self.sounds.push((sound, s.at, 20.0, pitch, true));
                }
                false
            } else {
                true
            }
        });
        if self.starters.is_empty() {
            self.starters = starters;
        } else {
            starters.append(&mut self.starters);
            self.starters = starters;
        }
    }
    /// Starter.createParticle: a random colour and fade colour.
    fn spark_from(&mut self, s: &Starter, v: [f64; 3], colors: &[u32], e: &FireworkExplosion) {
        let rgb = |c: u32| {
            [
                ((c >> 16) & 255) as f32 / 255.0,
                ((c >> 8) & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        let c = colors[self.random.next_int_bound(colors.len() as i32).unwrap_or(0) as usize];
        let fade = (!e.fade.is_empty()).then(|| {
            rgb(e.fade[self.random.next_int_bound(e.fade.len() as i32).unwrap_or(0) as usize])
        });
        let [r, g, b] = rgb(c);
        let p = self.spark(s.at, v, [r, g, b, 0.99], fade, e.trail, e.twinkle);
        self.pending.push(PendingParticle::Ordinary(p));
    }
    fn spark_ball(
        &mut self,
        s: &Starter,
        speed: f64,
        steps: i32,
        colors: &[u32],
        e: &FireworkExplosion,
    ) {
        for y in -steps..=steps {
            for x in -steps..=steps {
                let mut z = -steps;
                while z <= steps {
                    let d = |r: &mut LegacyRandom| (r.next_double() - r.next_double()) * 0.5;
                    let xa = f64::from(x) + d(&mut self.random);
                    let ya = f64::from(y) + d(&mut self.random);
                    let za = f64::from(z) + d(&mut self.random);
                    let len = (xa * xa + ya * ya + za * za).sqrt() / speed
                        + self.random.next_gaussian() * 0.05;
                    self.spark_from(s, [xa / len, ya / len, za / len], colors, e);
                    if y != -steps && y != steps && x != -steps && x != steps {
                        z += steps * 2 - 1;
                    }
                    z += 1;
                }
            }
        }
    }
    fn spark_shape(
        &mut self,
        s: &Starter,
        speed: f64,
        coords: &[[f64; 2]],
        colors: &[u32],
        e: &FireworkExplosion,
        flat: bool,
    ) {
        let (sx, sy) = (coords[0][0], coords[0][1]);
        self.spark_from(s, [sx * speed, sy * speed, 0.0], colors, e);
        let base = f64::from(self.random.next_float() * std::f32::consts::PI);
        let modifier = if flat { 0.034 } else { 0.34 };
        for step in 0..3 {
            let angle = base + f64::from(step as f32 * std::f32::consts::PI) * modifier;
            let (mut ox, mut oy) = (sx, sy);
            for c in &coords[1..] {
                let (tx, ty) = (c[0], c[1]);
                let mut sub = 0.25;
                while sub <= 1.0 {
                    let mut xa = (ox + (tx - ox) * sub) * speed;
                    let ya = (oy + (ty - oy) * sub) * speed;
                    let za = xa * angle.sin();
                    xa *= angle.cos();
                    for flip in [-1.0, 1.0] {
                        self.spark_from(s, [xa * flip, ya, za * flip], colors, e);
                    }
                    sub += 0.25;
                }
                (ox, oy) = (tx, ty);
            }
        }
    }
    /// A tick where no emitter follows an entity's live body (tests, fixtures, benches).
    pub fn tick(&mut self, level: &ParticleLevel) {
        self.tick_tracking(level, |_| None);
    }
    /// A tick whose tracking emitters read each followed entity's live body.
    pub fn tick_tracking(
        &mut self,
        level: &ParticleLevel,
        mut tracking_body: impl FnMut(TrackingTarget) -> Option<TrackingBody>,
    ) {
        if !self.elder_guardians.is_empty() {
            self.elder_guardians.retain_mut(|left| {
                *left -= 1;
                *left >= 0
            });
        }
        self.polytone_ticks += 1;
        for store in &mut self.polytone_gpu {
            store.expire(self.polytone_ticks);
        }
        // The sounds the packs' custom particles played last tick.
        if let Some(registry) = &self.polytone {
            self.sounds.extend(registry.take_sounds());
        }
        self.tick_starters();
        self.tick_explosions(level);
        if !self.emitters.is_empty() {
            let mut emitters = std::mem::take(&mut self.emitters);
            emitters.retain_mut(|emitter| {
                if let Some(target) = emitter.target {
                    match tracking_body(target) {
                        Some(body) => emitter.body = body,
                        None => emitter.target = None,
                    }
                }
                self.emit(&emitter.options, emitter.body);
                emitter.remaining -= 1;
                emitter.remaining > 0
            });
            self.emitters = emitters;
        }
        let has_live_groups = self.groups.iter().any(|g| !g.particles.is_empty());
        if !self.particles.is_empty() || has_live_groups {
            self.spawned_in_tick = Some(Vec::new());
            if !self.particles.is_empty() {
                let mut particles = std::mem::take(&mut self.particles);
                self.tick_all(&mut particles, level);
                particles.retain(|p| !p.removed);
                self.particles = particles;
            }
            if has_live_groups {
                for handle in 0..self.groups.len() {
                    if self.groups[handle].particles.is_empty() {
                        continue;
                    }
                    let mut particles = std::mem::take(&mut self.groups[handle].particles);
                    for p in &mut particles {
                        self.tick_particle(p, level);
                    }
                    particles.retain(|p| !p.removed);
                    self.groups[handle].particles = particles;
                }
            }
            if let Some(spawned) = self.spawned_in_tick.take() {
                for (options, pos, velocity) in spawned {
                    self.add(&options, pos, velocity);
                }
            }
        }
        if !self.pending.is_empty() {
            let mut pending = std::mem::take(&mut self.pending);
            for item in pending.drain(..) {
                match item {
                    PendingParticle::Starter(starter) => {
                        if self.group_has_space(self.starters.len()) {
                            self.starters.push(starter);
                        }
                    }
                    PendingParticle::ElderGuardian => {
                        if self.group_has_space(self.elder_guardians.len()) {
                            self.elder_guardians.push(ELDER_GUARDIAN_LIFETIME);
                        }
                    }
                    PendingParticle::Ordinary(mut p) => {
                        if !self.polytone_joins(&p, level) {
                            continue;
                        }
                        if p.pre_tick {
                            p.pre_tick = false;
                            self.tick_particle(&mut p, level);
                            p.after_constructor_tick();
                        }
                        let live = p.group.map_or(self.particles.len(), |handle| {
                            self.groups[handle as usize].particles.len()
                        });
                        // A scene's own limit first: the level's engine has none, and draws
                        // `group_has_space`'s random exactly as before.
                        if self.live_limit.is_none_or(|limit| self.len() < limit)
                            && self.group_has_space(live)
                        {
                            match p.group {
                                Some(handle) => self.groups[handle as usize].particles.push(p),
                                None => self.particles.push(p),
                            }
                        }
                    }
                }
            }
            // Keep the pending buffer warm across ticks without hoarding after giant spikes.
            if pending.capacity() > 4096 {
                pending.shrink_to(1024);
            }
            if self.pending.is_empty() {
                self.pending = pending;
            } else {
                pending.append(&mut self.pending);
                self.pending = pending;
            }
        }
    }
    /// `ParticleGroup.add`: each group's last quarter fills with decreasing probability.
    fn group_has_space(&mut self, size: usize) -> bool {
        if size >= MAX_PARTICLES {
            return false;
        }
        if size >= 12288 {
            let free = (MAX_PARTICLES - size) as f32 / 4096.0;
            return self.random.next_float() < free * free;
        }
        true
    }
    #[allow(clippy::too_many_lines)]
    fn tick_particle(&mut self, p: &mut Particle, level: &ParticleLevel) {
        match p.behaviour {
            Behaviour::Base
            | Behaviour::Mod { .. }
            | Behaviour::Animated
            | Behaviour::Snowflake
            | Behaviour::Crit
            | Behaviour::Spark { .. } => {
                let simple = tick_simple(&self.set_sprites, p, level);
                self.finish_simple(p, simple);
            }
            Behaviour::Controlled { .. } => {
                if let Some(mut controller) = p.controller.take() {
                    let update = controller.tick(p, level);
                    p.controller = Some(controller);
                    if let Some(frame) = update.frame {
                        self.set_sprite_frame(p, frame);
                    }
                    if let Some(later) = &mut self.spawned_in_tick {
                        later.extend(update.spawned);
                    } else {
                        for (options, pos, velocity) in update.spawned {
                            self.add(&options, pos, velocity);
                        }
                    }
                } else {
                    p.remove();
                }
            }
            Behaviour::Lava => {
                p.base_tick(level);
                if !p.removed && p.random.next_float() > p.age as f32 / p.lifetime as f32 {
                    self.add(
                        &ParticleOptions::simple(ParticleType::Smoke),
                        p.position,
                        p.velocity,
                    );
                }
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
                            let volume = p.random.next_float() * 0.7 + 0.3;
                            self.sounds.push((sound, p.position, volume, 1.0, false));
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
                    p.removed |= p.random.next_float() < 0.5;
                    p.velocity[0] *= f64::from(0.7f32);
                    p.velocity[2] *= f64::from(0.7f32);
                }
                let b = ParticleLevel::block_at(p.position);
                let state = level.block(b[0], b[1], b[2]);
                if state != level.world.air {
                    let top = level.collision_top_of(
                        state,
                        p.position[0] - f64::from(b[0]),
                        p.position[2] - f64::from(b[2]),
                    );
                    let offset = if top >= 1.0 {
                        top
                    } else {
                        top.max(level.fluid_of(state, b).1)
                    };
                    if offset > 0.0 && p.position[1] < f64::from(b[1]) + offset {
                        p.removed = true;
                    }
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
                if level.fluid_kind(ParticleLevel::block_at(p.position)) != Fluid::Water {
                    p.removed = true;
                }
            }
            Behaviour::SulfurBubble {
                y_start,
                y_end,
                size_start,
                y_prev,
            } => {
                p.base_tick(level);
                let (fluid, source) = level.fluid_source(ParticleLevel::block_at(p.position));
                if !(fluid == Fluid::Water && source)
                    || p.position[1] >= y_end
                    || p.position[1] <= y_prev
                {
                    p.removed = true;
                }
                // randomHorizontalWiggling, then a second, horizontal move.
                let wiggle = |r: &mut LegacyRandom| {
                    let f = r.next_float() * 0.003;
                    f64::from(if r.next_boolean() { f } else { -f }) * 0.5
                };
                p.velocity[0] += wiggle(&mut p.random);
                p.velocity[2] += wiggle(&mut p.random);
                p.move_by(level, [p.velocity[0], 0.0, p.velocity[2]]);
                let progress = ((p.position[1] - y_start) / (y_end - y_start)) as f32;
                p.size = size_start + progress * (0.15 - size_start);
                p.behaviour = Behaviour::SulfurBubble {
                    y_start,
                    y_end,
                    size_start,
                    y_prev: p.position[1],
                };
            }
            Behaviour::NoxiousGas { fade_start } => {
                p.base_tick(level);
                self.sprite_from_age(p);
                if p.age as f32 > fade_start {
                    p.rgba[3] =
                        (p.lifetime as f32 - (p.age as f32 - fade_start)) / p.lifetime as f32;
                }
            }
            Behaviour::NoxiousGasCloud => {
                p.base_tick(level);
                if p.age % 2 == 0 {
                    let source = ParticleLevel::block_at(p.position);
                    let spawn = pick_random_particle_spawn_point(&mut self.level_random, source);
                    if crate::potent_sulfur::can_be_reached_by_noxious_gas(level, source, spawn) {
                        self.spawn_noxious_gas_particle(spawn);
                    }
                }
            }
            Behaviour::GeyserEruption { water_blocks, aux } => {
                p.base_tick(level);
                if p.age % 2 == 0 {
                    let base =
                        ParticleOptions::geyser_base(ParticleType::GeyserBase, water_blocks, 1.5);
                    for _ in 0..2 {
                        self.add(&base, p.position, aux);
                    }
                }
                let plume = ParticleOptions::geyser(ParticleType::GeyserPlume, water_blocks);
                for _ in 0..water_blocks + 2 {
                    self.add(&plume, p.position, aux);
                }
                if p.age % 10 == 0 {
                    let poof =
                        ParticleOptions::geyser_base(ParticleType::GeyserPoof, water_blocks, 2.0);
                    for _ in 0..20 {
                        self.add(&poof, p.position, aux);
                    }
                }
            }
            Behaviour::GeyserPlume {
                start_y,
                max_y,
                propulsion,
                spray,
                min_size,
                max_size,
                mut done,
            } => {
                p.base_tick(level);
                if !done
                    && (p.velocity[1] < 0.0
                        || p.position[1] > max_y
                        || p.position[1] == p.previous[1])
                {
                    p.lifetime = p.lifetime.min(p.age + 5);
                    p.friction = 0.0;
                    done = true;
                }
                let y_progress_linear =
                    ((p.position[1] - start_y) / (max_y - start_y)).clamp(0.0, 1.0);
                let y_progress_exponential = horizon_core::jmath::pow(y_progress_linear, 3.0);
                p.gravity = propulsion * y_progress_exponential as f32 * 0.12;
                p.velocity[0] = y_progress_linear * f64::from(spray[0]);
                p.velocity[2] = y_progress_linear * f64::from(spray[1]);
                self.sprite_from_age(p);
                p.size = min_size + (y_progress_linear * f64::from(max_size - min_size)) as f32;
                p.behaviour = Behaviour::GeyserPlume {
                    start_y,
                    max_y,
                    propulsion,
                    spray,
                    min_size,
                    max_size,
                    done,
                };
            }
            Behaviour::BubbleColumnUp => {
                p.base_tick(level);
                if !p.removed
                    && level.fluid_kind(ParticleLevel::block_at(p.position)) != Fluid::Water
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
                if level.fluid_kind(ParticleLevel::block_at(p.position)) != Fluid::Water
                    || p.on_ground
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
            Behaviour::Transition { .. } => {
                p.base_tick(level);
                self.sprite_from_age(p);
            }
            Behaviour::Shriek { delay } => {
                if delay > 0 {
                    p.behaviour = Behaviour::Shriek { delay: delay - 1 };
                } else {
                    p.base_tick(level);
                }
            }
            Behaviour::Vibration {
                dest, rot, pitch, ..
            } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let remaining = p.lifetime - p.age;
                if remaining > 0 {
                    let alpha = 1.0 / f64::from(remaining);
                    for i in 0..3 {
                        p.position[i] += (dest[i] - p.position[i]) * alpha;
                    }
                }
                let d = [
                    p.position[0] - dest[0],
                    p.position[1] - dest[1],
                    p.position[2] - dest[2],
                ];
                p.behaviour = Behaviour::Vibration {
                    dest,
                    rot: d[0].atan2(d[2]) as f32,
                    rot_o: rot,
                    pitch: d[1].atan2((d[0] * d[0] + d[2] * d[2]).sqrt()) as f32,
                    pitch_o: pitch,
                };
                self.sprite_from_age(p);
            }
            Behaviour::Trail { target } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                // `Mth.lerp(1.0 / ticksRemaining, x, target)`, infinite on the last tick.
                let alpha = 1.0 / f64::from(p.lifetime - p.age);
                for i in 0..3 {
                    p.position[i] += alpha * (target[i] - p.position[i]);
                }
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
            Behaviour::GustSeed { scale, delay } => {
                if p.age % (delay + 1) == 0 {
                    let gust = ParticleOptions::simple(ParticleType::Gust);
                    for _ in 0..3 {
                        let r = &mut p.random;
                        let at = [
                            p.position[0] + (r.next_double() - r.next_double()) * scale,
                            p.position[1] + (r.next_double() - r.next_double()) * scale,
                            p.position[2] + (r.next_double() - r.next_double()) * scale,
                        ];
                        let f = f64::from(p.age as f32 / p.lifetime as f32);
                        self.add(&gust, at, [f, 0.0, 0.0]);
                    }
                }
                let age = p.age;
                p.age += 1;
                if age == p.lifetime {
                    p.removed = true;
                }
            }
            Behaviour::ExplosionSeed => {
                let explosion = ParticleOptions::simple(ParticleType::Explosion);
                for _ in 0..6 {
                    let r = &mut p.random;
                    let at = [
                        p.position[0] + (r.next_double() - r.next_double()) * 4.0,
                        p.position[1] + (r.next_double() - r.next_double()) * 4.0,
                        p.position[2] + (r.next_double() - r.next_double()) * 4.0,
                    ];
                    let f = f64::from(p.age as f32 / p.lifetime as f32);
                    self.add(&explosion, at, [f, 0.0, 0.0]);
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
                    let r = &mut p.random;
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
                // SpellParticle.tick: alpha 0 while close, else eased back to originalAlpha.
                if self.close_to_scoping_player(p.position) {
                    p.rgba[3] = 0.0;
                } else {
                    p.rgba[3] += (original_alpha - p.rgba[3]) * 0.05;
                }
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
                        let factor = f64::from(relative).powf(1.25);
                        xa += flow_scale[0] * factor;
                        za += flow_scale[1] * factor;
                    }
                    if swirl {
                        let rel = f64::from(relative);
                        let (sin, cos) = (rel * swirl_period).sin_cos();
                        xa += rel * cos * f64::from(wind);
                        za += rel * sin * f64::from(wind);
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
            Behaviour::SquidInk => {
                // SimpleAnimatedParticle.tick, then SquidInkParticle's.
                p.base_tick(level);
                self.sprite_from_age(p);
                let half = |p: &mut Particle| {
                    if p.age > p.lifetime / 2 {
                        p.rgba[3] =
                            1.0 - (p.age as f32 - (p.lifetime / 2) as f32) / p.lifetime as f32;
                    }
                };
                half(p);
                if !p.removed {
                    self.sprite_from_age(p);
                    half(p);
                    let b = ParticleLevel::block_at(p.position);
                    if level.facts.is_air(level.block(b[0], b[1], b[2])) {
                        p.velocity[1] -= f64::from(0.0074f32);
                    }
                }
            }
            Behaviour::Firefly => {
                p.base_tick(level);
                let b = ParticleLevel::block_at(p.position);
                if !level.facts.is_air(level.block(b[0], b[1], b[2])) {
                    p.removed = true;
                } else {
                    p.rgba[3] = fade_amount(p.lifetime_progress(p.age as f32), 0.3, 0.5);
                    if p.random.next_float() > 0.95 || p.age == 1 {
                        let r = &mut p.random;
                        let mut speed = || f64::from(-0.05f32 + 0.1 * r.next_float());
                        p.velocity = [speed(), speed(), speed()];
                    }
                }
            }
            Behaviour::Wake => {
                p.previous = p.position;
                let life = 60 - p.lifetime;
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
                let size = life as f32 * 0.001;
                p.set_size(size, size);
                // SpriteSet.get(life % 4, 4).
                if let Some(set) = p.set {
                    let sprites = &self.set_sprites[set as usize];
                    let index = (life % 4) as usize * (sprites.len() - 1) / 4;
                    p.sprite = Sprite::Index(sprites[index]);
                }
            }
            Behaviour::DustPlume => {
                p.gravity *= 0.88;
                p.friction *= 0.92;
                p.base_tick(level);
                self.sprite_from_age(p);
            }
            Behaviour::FlyStraight { start, colors } => {
                p.previous = p.position;
                let age = p.age;
                p.age += 1;
                if age >= p.lifetime {
                    p.removed = true;
                    return;
                }
                let normalized = p.age as f32 / p.lifetime as f32;
                let along = 1.0 - normalized;
                for i in 0..3 {
                    p.position[i] = start[i] + p.velocity[i] * f64::from(along);
                }
                // ARGB.srgbLerp: Mth.lerpInt on each 8-bit channel.
                let lerp = |shift: u32| {
                    let (a, b) = (
                        (colors[0] >> shift & 255) as i32,
                        (colors[1] >> shift & 255) as i32,
                    );
                    (a + (normalized * (b - a) as f32).floor() as i32) as f32 / 255.0
                };
                p.set_color(lerp(16), lerp(8), lerp(0));
                p.rgba[3] = lerp(24);
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

    /// The particles that `entries` name, as lights: where each is this frame, its entry's colour
    /// as RGB and reach, and its alpha as the intensity.
    pub fn lights(
        &self,
        entries: &[horizon_shader_api::manifest::ParticleLight],
        partial: f32,
    ) -> Vec<([f64; 3], u32, f32, f32)> {
        if self.particles.is_empty() || entries.is_empty() {
            return Vec::new();
        }
        let mut shining: smallvec::SmallVec<[(ParticleType, u32, f32); 16]> =
            smallvec::SmallVec::new();
        let mut mask = 0u128;
        let mut has_high = false;
        for e in entries {
            let [r, g, b] = e.color.map(|c| (c.clamp(0.0, 1.0) * 255.0) as u32);
            let rgb = (r << 16) | (g << 8) | b;
            for name in &e.particles {
                let Some(ty) = ParticleType::from_name(name) else {
                    continue;
                };
                shining.push((ty, rgb, e.reach));
                let idx = ty as usize;
                if idx < 128 {
                    mask |= 1u128 << idx;
                } else {
                    has_high = true;
                }
            }
        }
        if shining.is_empty() {
            return Vec::new();
        }
        let f = f64::from(partial);
        let mut out = Vec::new();
        for p in &self.particles {
            let idx = p.ty as usize;
            let maybe_shining = if idx < 128 {
                (mask & (1u128 << idx)) != 0
            } else {
                has_high
            };
            if maybe_shining
                && p.rgba[3] > 0.0
                && let Some(&(_, rgb, reach)) = shining.iter().find(|&&(ty, ..)| ty == p.ty)
            {
                let at = [
                    p.previous[0] + (p.position[0] - p.previous[0]) * f,
                    p.previous[1] + (p.position[1] - p.previous[1]) * f,
                    p.previous[2] + (p.position[2] - p.previous[2]) * f,
                ];
                out.push((at, rgb, reach, p.rgba[3].clamp(0.0, 1.0)));
            }
        }
        out
    }

    /// Camera-facing quads (`QuadParticleRenderState.renderRotatedQuad`), turned by
    /// `Camera.rotation()`: `rotationYXZ(PI - yRot, -xRot, 0)`.
    pub fn render(
        &self,
        camera: DVec3,
        base: Quat,
        partial: f32,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        self.render_in_view(camera, base, partial, None, light_at, out);
    }

    /// Render particles after rejecting view-frustum misses and any particles
    /// the caller proves hidden behind opaque terrain.
    pub fn render_culled(
        &self,
        camera: DVec3,
        base: Quat,
        partial: f32,
        pixel_focal_length: f32,
        culling_enabled: bool,
        frustum: Option<&crate::gfx::JoinedFrusta>,
        mut occluded: impl FnMut([f64; 3], [f64; 3]) -> bool,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        if !culling_enabled {
            self.render_in_view(camera, base, partial, frustum, light_at, out);
            return;
        }
        self.render_inner::<true>(
            camera,
            base,
            partial,
            pixel_focal_length,
            frustum,
            &mut occluded,
            light_at,
            out,
        );
    }

    /// Every live particle in draw order: the built-in group's, then each registered group's in
    /// registration order, with the blend that group sets (none for the built-in group).
    #[allow(dead_code)]
    fn drawn(&self) -> impl Iterator<Item = (&Particle, Option<ParticleBlend>)> {
        self.particles.iter().map(|p| (p, None)).chain(
            self.groups
                .iter()
                .flat_map(|g| g.particles.iter().map(|p| (p, Some(g.blend)))),
        )
    }

    #[allow(clippy::too_many_lines)]
    fn render_slice<'s, const CULL: bool>(
        &'s self,
        particles: &'s [Particle],
        blend: Option<ParticleBlend>,
        camera: DVec3,
        basis: BillboardBasis,
        partial: f32,
        pixel_focal_length: f32,
        has_focal: bool,
        frustum: Option<&crate::gfx::JoinedFrusta>,
        view: Option<&crate::gfx::JoinedFrusta>,
        section_cache: &mut Option<ParticleSectionFrustumCache>,
        occluded: &mut dyn FnMut([f64; 3], [f64; 3]) -> bool,
        light_cache: &mut FrameLightCache<'_>,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
        additive_guard: &mut Option<MutexGuard<'s, Vec<Vertex>>>,
        additive_cleared: &mut bool,
    ) {
        let partial_f64 = f64::from(partial);
        let mut last_named: Option<(&'s str, Option<(Rect, bool)>)> = None;
        let mut out_reserved = false;
        for (idx, p) in particles.iter().enumerate() {
            if p.no_render {
                continue;
            }
            if let Some(view) = view
                && !view_cull::in_view(view, p, camera)
            {
                continue;
            }
            match p.behaviour {
                // SparkParticle.extract: twinkling sparks blink after a third of their life.
                Behaviour::Spark { twinkle: true, .. }
                    if p.age >= p.lifetime / 3 && ((p.age + p.lifetime) / 3) % 2 != 0 =>
                {
                    continue;
                }
                Behaviour::Shriek { delay } if delay > 0 => continue,
                _ => {}
            }
            let culled_geometry = if CULL {
                let wx = p.previous[0] + (p.position[0] - p.previous[0]) * partial_f64;
                let wy = p.previous[1] + (p.position[1] - p.previous[1]) * partial_f64;
                let wz = p.previous[2] + (p.position[2] - p.previous[2]) * partial_f64;
                let center = FVec3::new(
                    (wx - camera.x) as f32,
                    (wy - camera.y) as f32,
                    (wz - camera.z) as f32,
                );
                let size = p.quad_size(partial);
                // Every generated quad vertex is sqrt(2) * size from its center,
                // including the rotated two-quad particle types.
                let radius = f64::from(size.abs()) * std::f64::consts::SQRT_2 + 0.05;
                if let Some(frustum) = frustum {
                    if let Some(section_cache) = section_cache.as_mut() {
                        match section_cache.result([wx, wy, wz], camera, radius, frustum) {
                            SectionFrustumResult::Outside => continue,
                            SectionFrustumResult::Intersects
                                if !frustum.intersects_sphere(center, radius as f32) =>
                            {
                                continue;
                            }
                            SectionFrustumResult::Inside | SectionFrustumResult::Intersects => {}
                        }
                    } else if !frustum.intersects_sphere(center, radius as f32) {
                        continue;
                    }
                }
                if has_focal
                    && below_half_pixel(
                        radius,
                        f64::from(center.length_squared()),
                        pixel_focal_length,
                    )
                {
                    continue;
                }
                if occluded(
                    [wx - radius, wy - radius, wz - radius],
                    [wx + radius, wy + radius, wz + radius],
                ) {
                    continue;
                }
                Some((center, size))
            } else {
                None
            };
            // `SubmitNodeCollection.submitQuadParticleGroup`: the translucent
            // layer draws after translucent terrain, so a crumb of stained
            // glass cannot hide the glass behind it (`getLayer`).
            let (r, sprite_translucent) = match &p.sprite {
                Sprite::Index(i) => {
                    let Some(rect) = self.rects.get(*i as usize).copied().flatten() else {
                        continue;
                    };
                    let translucent = if p.additive || blend.is_some() {
                        false
                    } else {
                        p.ty.translucent_layer()
                            || matches!(p.behaviour, Behaviour::Controlled { translucent: true })
                            || matches!(p.behaviour, Behaviour::Mod { alpha, .. } if alpha != [1.0; 2])
                    };
                    (rect, translucent)
                }
                Sprite::Named(name) => {
                    let found = match last_named {
                        Some((prev, cached)) if prev == name.as_str() => cached,
                        _ => {
                            let entry = self.atlas.get(name).copied();
                            last_named = Some((name.as_str(), entry));
                            entry
                        }
                    };
                    let Some((rect, translucent)) = found else {
                        continue;
                    };
                    (rect, translucent)
                }
            };
            let (center, size) = culled_geometry.unwrap_or_else(|| {
                let wx = p.previous[0] + (p.position[0] - p.previous[0]) * partial_f64;
                let wy = p.previous[1] + (p.position[1] - p.previous[1]) * partial_f64;
                let wz = p.previous[2] + (p.position[2] - p.previous[2]) * partial_f64;
                (
                    FVec3::new(
                        (wx - camera.x) as f32,
                        (wy - camera.y) as f32,
                        (wz - camera.z) as f32,
                    ),
                    p.quad_size(partial),
                )
            });
            let mut alpha = p.rgba[3];
            // OverlayParticle.extract: the flash fades as it pulses.
            if p.ty == ParticleType::Flash {
                alpha = 0.6 - (p.age as f32 + partial - 1.0) * 0.25 * 0.5;
            }
            let mut rgb = [p.rgba[0], p.rgba[1], p.rgba[2]];
            match p.behaviour {
                Behaviour::FlyTowards { alpha: la, .. } => {
                    alpha = lifetime_alpha(la, p.age, p.lifetime, partial);
                }
                // ShriekParticle.extract: two tilted quads, fading out.
                Behaviour::Shriek { .. } => {
                    alpha = 1.0 - ((p.age as f32 + partial) / p.lifetime as f32).clamp(0.0, 1.0);
                }
                // DustColorTransitionParticle.lerpColors.
                Behaviour::Transition { from, to } => {
                    let t = (p.age as f32 + partial) / (p.lifetime as f32 + 1.0);
                    rgb = [
                        from[0] + (to[0] - from[0]) * t,
                        from[1] + (to[1] - from[1]) * t,
                        from[2] + (to[2] - from[2]) * t,
                    ];
                }
                _ => {}
            }
            let light = particle_light(p, partial, light_cache, light_at);
            let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as u32;
            let color = (c(alpha) << 24) | (c(rgb[2]) << 16) | (c(rgb[1]) << 8) | c(rgb[0]);
            let overlay = if p.additive {
                crate::gfx::ADDITIVE_PARTICLE_OVERLAY
            } else {
                let layer = match blend {
                    Some(ParticleBlend::Opaque) => false,
                    Some(ParticleBlend::Translucent) => true,
                    None => sprite_translucent,
                };
                if layer {
                    crate::gfx::TRANSLUCENT_PARTICLE_OVERLAY
                } else {
                    0
                }
            };
            let layer_hdr = overlay | (u32::from(hdr_tier(p.ty)) << 16);
            let target: &mut Vec<Vertex> = if p.additive {
                additive_guard.get_or_insert_with(|| {
                    let mut guard = self.additive.lock().unwrap_or_else(|e| e.into_inner());
                    if !*additive_cleared {
                        guard.clear();
                        *additive_cleared = true;
                    }
                    guard
                })
            } else {
                if !out_reserved {
                    out_reserved = true;
                    let remaining = particles.len() - idx;
                    let est_quads = if !CULL && frustum.is_none() {
                        remaining
                    } else {
                        (remaining / 2).max(1)
                    };
                    out.reserve(est_quads.min(MAX_INITIAL_PARTICLE_QUADS) * 4);
                }
                out
            };
            // A Polytone model particle: its model's faces, scaled by its size and turned as
            // its quad would be.
            if let Some(model) = &p.model {
                let rotation = particle_rotation(p, center, partial, basis.base);
                target.reserve(model.quads.len() * 4);
                for quad in &model.quads {
                    let Some(&(r, _)) = self.atlas.get(&quad.sprite) else {
                        continue;
                    };
                    let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
                    let verts = array::from_fn::<_, 4, _>(|i| Vertex {
                        normal: [0; 4],
                        pos: (center + rotation * (quad.corners[i] * size)).to_array(),
                        uv: [r.u0 + du * quad.uv[i][0], r.v0 + dv * quad.uv[i][1]],
                        layer: layer_hdr,
                        color,
                        light,
                        glint_uv: [0.0, 0.0],
                    });
                    target.extend_from_slice(&verts);
                }
                continue;
            }
            let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
            let uv = [
                r.u0 + du * p.uv[0],
                r.v0 + dv * p.uv[1],
                r.u0 + du * p.uv[2],
                r.v0 + dv * p.uv[3],
            ];
            match p.behaviour {
                Behaviour::Shriek { .. } => {
                    emit_rotated_quad(
                        target,
                        center,
                        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_3),
                        size,
                        uv,
                        layer_hdr,
                        color,
                        light,
                    );
                    emit_rotated_quad(
                        target,
                        center,
                        Quat::from_euler(
                            glam::EulerRot::YXZ,
                            -std::f32::consts::PI,
                            std::f32::consts::FRAC_PI_3,
                            0.0,
                        ),
                        size,
                        uv,
                        layer_hdr,
                        color,
                        light,
                    );
                }
                // VibrationSignalParticle.extract: two quads along the flight.
                Behaviour::Vibration {
                    rot,
                    rot_o,
                    pitch,
                    pitch_o,
                    ..
                } => {
                    let sway =
                        ((p.age as f32 + partial - std::f32::consts::TAU) * 0.05).sin() * 2.0;
                    let lerped_rot = rot_o + (rot - rot_o) * partial;
                    let lerped_pitch =
                        pitch_o + (pitch - pitch_o) * partial + std::f32::consts::FRAC_PI_2;
                    let q_sway = Quat::from_rotation_y(sway);
                    emit_rotated_quad(
                        target,
                        center,
                        Quat::from_rotation_y(lerped_rot)
                            * Quat::from_rotation_x(-lerped_pitch)
                            * q_sway,
                        size,
                        uv,
                        layer_hdr,
                        color,
                        light,
                    );
                    emit_rotated_quad(
                        target,
                        center,
                        Quat::from_rotation_y(-std::f32::consts::PI + lerped_rot)
                            * Quat::from_rotation_x(lerped_pitch)
                            * q_sway,
                        size,
                        uv,
                        layer_hdr,
                        color,
                        light,
                    );
                }
                _ if p.orientation != polytone_gpu::Orientation::Camera => {
                    let rotation = particle_rotation(p, center, partial, basis.base);
                    emit_rotated_quad(target, center, rotation, size, uv, layer_hdr, color, light);
                }
                _ if p.roll != 0.0 => {
                    let (sin, cos) = (p.o_roll + (p.roll - p.o_roll) * partial).sin_cos();
                    let a = cos + sin;
                    let b = cos - sin;
                    let d0 = (basis.right * a - basis.up * b) * size;
                    let d1 = (basis.right * b + basis.up * a) * size;
                    emit_quad(target, center, d0, d1, uv, layer_hdr, color, light);
                }
                _ => {
                    emit_quad(
                        target,
                        center,
                        basis.c0 * size,
                        basis.c1 * size,
                        uv,
                        layer_hdr,
                        color,
                        light,
                    );
                }
            }
        }
    }

    fn render_inner<const CULL: bool>(
        &self,
        camera: DVec3,
        base: Quat,
        partial: f32,
        pixel_focal_length: f32,
        frustum: Option<&crate::gfx::JoinedFrusta>,
        occluded: &mut dyn FnMut([f64; 3], [f64; 3]) -> bool,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        let live_count = self.len();
        let has_gpu = self.polytone_gpu.iter().any(|store| store.len() > 0);
        if live_count == 0 && !has_gpu {
            if self.has_additive.swap(false, Ordering::AcqRel) {
                self.additive
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clear();
            }
            return;
        }
        let mut additive_cleared = !self.has_additive.swap(false, Ordering::AcqRel);
        let mut additive_guard: Option<MutexGuard<'_, Vec<Vertex>>> = None;

        if live_count > 0 {
            let mut small_light = [LightCacheSlot::EMPTY; SMALL_LIGHT_CACHE_SIZE];
            let mut dense_guard = (live_count > DENSE_LIGHT_CACHE_THRESHOLD).then(|| {
                self.dense_light_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
            });
            let mut light_cache = if let Some(guard) = dense_guard.as_mut() {
                let cache = guard.get_or_insert_with(DenseLightCache::new);
                let (entries, generation) = cache.begin_frame();
                FrameLightCache::new(entries, generation)
            } else {
                FrameLightCache::new(&mut small_light, 1)
            };
            // For single-group particle counts (up to MAX_PARTICLES), testing the
            // sphere directly against the frustum planes in registers is faster
            // than hashing a 3 KiB section cache (and avoids zeroing it per frame).
            let mut section_cache = (CULL && frustum.is_some() && live_count > MAX_PARTICLES)
                .then(ParticleSectionFrustumCache::default);
            let view = frustum.map(|f| f.pulled_back(PARTICLE_FRUSTUM_OFFSET));
            let basis = BillboardBasis::new(base);
            let has_focal = pixel_focal_length.is_finite() && pixel_focal_length > 0.0;
            if !self.particles.is_empty() {
                self.render_slice::<CULL>(
                    &self.particles,
                    None,
                    camera,
                    basis,
                    partial,
                    pixel_focal_length,
                    has_focal,
                    frustum,
                    view.as_ref(),
                    &mut section_cache,
                    occluded,
                    &mut light_cache,
                    light_at,
                    out,
                    &mut additive_guard,
                    &mut additive_cleared,
                );
            }
            for group in &self.groups {
                if !group.particles.is_empty() {
                    self.render_slice::<CULL>(
                        &group.particles,
                        Some(group.blend),
                        camera,
                        basis,
                        partial,
                        pixel_focal_length,
                        has_focal,
                        frustum,
                        view.as_ref(),
                        &mut section_cache,
                        occluded,
                        &mut light_cache,
                        light_at,
                        out,
                        &mut additive_guard,
                        &mut additive_cleared,
                    );
                }
            }
        }
        if has_gpu {
            let mut gpu_light_cache = ParticleLightCache::default();
            let additive = additive_guard.get_or_insert_with(|| {
                let mut guard = self.additive.lock().unwrap_or_else(|e| e.into_inner());
                if !additive_cleared {
                    guard.clear();
                    additive_cleared = true;
                }
                guard
            });
            for store in &self.polytone_gpu {
                if store.len() > 0 {
                    store.render(
                        &self.atlas,
                        camera,
                        base,
                        self.polytone_ticks,
                        partial,
                        &mut gpu_light_cache,
                        light_at,
                        out,
                        additive,
                    );
                }
            }
        }
        let produced_additive = additive_guard.as_ref().is_some_and(|v| !v.is_empty());
        drop(additive_guard);
        if !additive_cleared {
            self.additive
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
        } else if produced_additive {
            self.has_additive.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Copy)]
struct BillboardBasis {
    right: FVec3,
    up: FVec3,
    c0: FVec3,
    c1: FVec3,
    base: Quat,
}

impl BillboardBasis {
    #[inline]
    fn new(base: Quat) -> Self {
        Self {
            right: base * FVec3::X,
            up: base * FVec3::Y,
            c0: base * FVec3::new(1.0, -1.0, 0.0),
            c1: base * FVec3::new(1.0, 1.0, 0.0),
            base,
        }
    }
}

#[inline]
fn particle_rotation(p: &Particle, center: FVec3, partial: f32, base: Quat) -> Quat {
    if p.orientation != polytone_gpu::Orientation::Camera {
        // A custom particle's `movement_aligned` quad has its texture's top against
        // its motion (measured in Java: a light ray rising toward the noon sun hangs
        // below the leaves it starts at); a GPU particle's has it along its motion.
        let velocity = -FVec3::new(
            p.velocity[0] as f32,
            p.velocity[1] as f32,
            p.velocity[2] as f32,
        );
        let roll = p.o_roll + (p.roll - p.o_roll) * partial;
        polytone_gpu::orient(p.orientation, center, velocity, roll, base)
    } else if p.roll != 0.0 {
        base * Quat::from_rotation_z(p.o_roll + (p.roll - p.o_roll) * partial)
    } else {
        base
    }
}

#[inline(always)]
fn particle_light(
    p: &Particle,
    partial: f32,
    light_cache: &mut FrameLightCache<'_>,
    light_at: &dyn Fn([i32; 3]) -> u8,
) -> u32 {
    match p.light {
        Light::World => {
            let packed = light_cache.get(ParticleLevel::block_at(p.position), light_at);
            u32::from(packed & 0xF0) | (u32::from(packed & 0x0F) << 12)
        }
        Light::FullBright => 240 | (240 << 8),
        Light::Block15 => {
            let packed = light_cache.get(ParticleLevel::block_at(p.position), light_at);
            u32::from(packed & 0xF0) | (240 << 8)
        }
        Light::Firefly => {
            let block_light =
                (255.0 * fade_amount(p.lifetime_progress(p.age as f32 + partial), 0.1, 0.3)) as u32;
            block_light << 8
        }
        _ => {
            let packed = light_cache.get(ParticleLevel::block_at(p.position), light_at);
            let (sky_light, block_light) = p.light_coords(
                partial,
                u32::from(packed & 0xF0),
                u32::from(packed & 0x0F) * 16,
            );
            sky_light | (block_light << 8)
        }
    }
}

#[inline(always)]
fn emit_quad(
    target: &mut Vec<Vertex>,
    center: FVec3,
    d0: FVec3,
    d1: FVec3,
    uv: [f32; 4],
    layer: u32,
    color: u32,
    light: u32,
) {
    let [u0, v0, u1, v1] = uv;
    let quad = [
        Vertex {
            normal: [0; 4],
            pos: (center + d0).to_array(),
            uv: [u1, v1],
            layer,
            color,
            light,
            glint_uv: [0.0, 0.0],
        },
        Vertex {
            normal: [0; 4],
            pos: (center + d1).to_array(),
            uv: [u1, v0],
            layer,
            color,
            light,
            glint_uv: [0.0, 0.0],
        },
        Vertex {
            normal: [0; 4],
            pos: (center - d0).to_array(),
            uv: [u0, v0],
            layer,
            color,
            light,
            glint_uv: [0.0, 0.0],
        },
        Vertex {
            normal: [0; 4],
            pos: (center - d1).to_array(),
            uv: [u0, v1],
            layer,
            color,
            light,
            glint_uv: [0.0, 0.0],
        },
    ];
    target.extend_from_slice(&quad);
}

#[inline]
fn emit_rotated_quad(
    target: &mut Vec<Vertex>,
    center: FVec3,
    rotation: Quat,
    size: f32,
    uv: [f32; 4],
    layer: u32,
    color: u32,
    light: u32,
) {
    let d0 = rotation * FVec3::new(1.0, -1.0, 0.0) * size;
    let d1 = rotation * FVec3::new(1.0, 1.0, 0.0) * size;
    emit_quad(target, center, d0, d1, uv, layer, color, light);
}
/// How brightly particle `ty` shines in HDR output, as `mesher::hdr_tier`: flames,
/// glowing drops and fireworks, not their smoke.
fn hdr_tier(ty: ParticleType) -> u8 {
    use ParticleType as P;
    match ty {
        P::Flame
        | P::SoulFireFlame
        | P::CopperFireFlame
        | P::SmallFlame
        | P::Lava
        | P::DrippingLava
        | P::FallingLava
        | P::LandingLava
        | P::DrippingDripstoneLava
        | P::FallingDripstoneLava => crate::mesher::INCANDESCENT,
        P::DrippingObsidianTear | P::FallingObsidianTear | P::LandingObsidianTear => {
            crate::mesher::LOW
        }
        // FireworkParticles: the sparks and the flash of the burst.
        P::Firework | P::Flash => crate::mesher::BRIGHT,
        _ => 0,
    }
}
/// `Particle.LifetimeAlpha.currentAlphaForAge`.
fn lifetime_alpha(a: [f32; 4], age: i32, lifetime: i32, partial: f32) -> f32 {
    let t = ((age as f32 + partial) / lifetime as f32 - a[2]) / (a[3] - a[2]);
    if (a[0] - a[1]).abs() < 1.0e-5 {
        a[0]
    } else {
        a[0] + (a[1] - a[0]) * t.clamp(0.0, 1.0)
    }
}

struct Starter {
    at: [f64; 3],
    v: [f64; 3],
    explosions: Vec<FireworkExplosion>,
    life: i32,
    lifetime: i32,
    twinkle_delay: bool,
    /// `FireworkParticles.Starter.playSound`, with `isFarAwayFromCamera` when
    /// it plays: `None` for a silent rocket.
    sound: Option<bool>,
}
const STAR: [[f64; 2]; 6] = [
    [0.0, 1.0],
    [0.3455, 0.309],
    [0.9511, 0.309],
    [0.379_591_836_734_693_9, -0.126_530_612_244_897_95],
    [0.612_244_897_959_183_7, -0.804_081_632_653_061_2],
    [0.0, -0.359_183_673_469_387_73],
];
const CREEPER: [[f64; 2]; 12] = [
    [0.0, 0.2],
    [0.2, 0.2],
    [0.2, 0.6],
    [0.6, 0.6],
    [0.6, 0.2],
    [0.2, 0.2],
    [0.2, 0.0],
    [0.4, 0.0],
    [0.4, -0.6],
    [0.2, -0.6],
    [0.2, -0.4],
    [0.0, -0.4],
];

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tick_bench;
