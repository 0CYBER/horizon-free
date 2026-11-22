//! Resource packs' Polytone custom particles and block particle emitters (#11486, wiki "Custom
//! Particle Types" and "Block Properties Modifiers"): a particle whose start, motion, look and
//! death are expressions, and emitters that spawn particles from blocks and from particles. Each
//! custom particle is an ordinary engine particle (sprite set, interpolation, rendering, the
//! total limit) with a controller that runs the type's ticker.
#[cfg(test)]
use super::ModSet;
use super::{
    Behaviour, Fluid, Particle, ParticleEngine, ParticleLevel, ParticleOptions, ParticleType,
    ParticleUpdate, PendingParticle, Sprite,
};
use crate::environment::Frame;
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_polytone::expr::{Host, Program, Root, Value as V};
use horizon_polytone::particles::{
    BlockEmitters, CustomParticle, Emitter, EntityEmitters, LiquidAffinity, ParticleModifier,
    RenderType, SoundEmitter,
};
use horizon_world::registry::BlockRegistry;
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

/// The packs' custom particle types and block emitters, and what their expressions read beyond
/// the particle and its blocks.
pub struct Registry {
    types: HashMap<String, Arc<CustomParticle>>,
    /// Block state id → the highest priority block modifiers with emitters aimed at its block.
    block_emitters: Vec<Option<Arc<[Arc<BlockEmitters>]>>>,
    /// The entity modifiers with emitters, and by entity type the ones aimed at it.
    entity_modifiers: Vec<Arc<EntityEmitters>>,
    entity_emitters: Mutex<HashMap<String, Arc<[Arc<EntityEmitters>]>>>,
    /// The world's biomes, for `o.biome()` and emitters' `biomes` (what the game last set).
    biomes: RwLock<Option<Arc<crate::biome::Biomes>>>,
    /// The frame's global, player and camera context (what the game last set).
    pub frame: Arc<RwLock<Frame>>,
    configs: horizon_polytone::pack::Configs,
    blocks: &'static BlockRegistry,
    /// Each custom particle type's colormap (`colormap`), by type id.
    tints: HashMap<String, Arc<Tint>>,
    /// The tags modifiers' `targets` and emitters' `biomes` name.
    tags: crate::pack_tags::Tags,
    /// Each model particle type's faces (`model`), by type id.
    models: HashMap<String, Arc<super::polytone_model::ParticleModel>>,
    /// Sound ids as the engine's sound queue holds them, interned once.
    sound_names: Mutex<HashMap<String, &'static str>>,
    /// Sounds the custom particles played this tick (`ParticleEngine::tick` plays them).
    sounds: Mutex<Vec<Sound>>,
    /// The particle modifiers with their colormaps, and by particle type the ones aimed at it.
    particle_modifiers: Vec<(ParticleModifier, Option<Tint>, Option<Filter>)>,
    modifiers_by_type: Mutex<HashMap<String, Arc<[usize]>>>,
}

/// A sound for the engine's queue: id, position, volume, pitch, delayed by the distance.
type Sound = (&'static str, [f64; 3], f32, f32, bool);

/// Where a particle's colormap colour comes from.
enum TintSource {
    /// A vanilla biome colour (`grass_color`, `foliage_color`, `dry_foliage_color`,
    /// `water_color`): the biome's own, as blocks get it.
    Biome(fn(&crate::biome::BiomeInfo) -> u32),
    /// A pack's colormap (`polytone/colormaps`), its axes run against the particle.
    Pack(crate::environment::Colormap),
}

/// When a particle's colormap is sampled again (`cache`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum TintCache {
    EveryTick,
    PerPosition,
    OnSpawn,
}

/// A custom particle type's colormap: multiplied into its colour.
struct Tint {
    source: TintSource,
    cache: TintCache,
}

impl Tint {
    /// `colormap`: an id, or `{"colormap": id, "cache": ...}`. Inline colormaps aren't read.
    fn read(
        pack_root: &horizon_assets::resource_source::ResourceSource,
        json: &Json,
    ) -> Option<Self> {
        let (id, cache) = match json {
            Json::String(id) => (id.as_str(), TintCache::EveryTick),
            Json::Object(o) => (
                o.get("colormap")?.as_str()?,
                match o.get("cache").and_then(Json::as_str) {
                    Some("per_position") => TintCache::PerPosition,
                    Some("on_spawn") => TintCache::OnSpawn,
                    _ => TintCache::EveryTick,
                },
            ),
            _ => return None,
        };
        let id = if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        };
        let source = match id.as_str() {
            "minecraft:grass_color" => TintSource::Biome(|b| b.grass),
            "minecraft:foliage_color" => TintSource::Biome(|b| b.foliage),
            "minecraft:dry_foliage_color" => TintSource::Biome(|b| b.dry_foliage),
            "minecraft:water_color" => TintSource::Biome(|b| b.water),
            _ => TintSource::Pack(crate::environment::Colormap::load(pack_root, &id)?),
        };
        Some(Self { source, cache })
    }

    /// The colour (0 to 1 per channel) at the particle in `ctx`; `None` keeps its own.
    fn sample(&self, ctx: &mut Ctx) -> Option<[f32; 3]> {
        let rgb = match &self.source {
            TintSource::Biome(colour) => {
                let at = ParticleLevel::block_at(ctx.position());
                let info = ctx.biome_info(at)?;
                colour(&info)
            }
            TintSource::Pack(colormap) => colormap.sample(ctx)?,
        };
        Some([16, 8, 0].map(|shift| ((rgb >> shift) & 0xff) as f32 / 255.0))
    }
}

/// Every `.json` under `dir`, with its path relative to `root`.
fn json_files(
    root: &horizon_assets::resource_source::ResourceSource,
    dir: &Path,
) -> Vec<(String, Json)> {
    // Resource names use `/`; a `PathBuf` join puts a `\` in on Windows, which lists nothing.
    root.list(&dir.to_string_lossy().replace('\\', "/"))
        .into_iter()
        .filter(|p| p.ends_with(".json"))
        .filter_map(|p| {
            let json = serde_json::from_str(&root.read_to_string(&p).ok()?).ok()?;
            Some((p, json))
        })
        .collect()
}

impl super::ParticleOptions {
    /// These options with the block or item they carry (`source`), for particle modifier
    /// filters.
    pub fn with_source(mut self, id: &str) -> Self {
        self.source = Some(Arc::from(id));
        self
    }
}

impl Registry {
    /// Every `assets/<namespace>/polytone/{custom_particles,block_modifiers,entity_modifiers}/**.json` under a
    /// pack overlay root.
    pub fn read(
        pack_root: &horizon_assets::resource_source::ResourceSource,
        configs: horizon_polytone::pack::Configs,
        blocks: &'static BlockRegistry,
    ) -> Self {
        let mut types = HashMap::new();
        let mut modifiers = Vec::new();
        let mut entity_modifiers = Vec::new();
        let mut particle_modifiers = Vec::new();
        // Custom colours' `particle.<id>`: a modifier that sets only the colour, below the files'.
        for (id, color) in horizon_polytone::colors::PackColors::from_files(
            pack_root.namespaces("assets").into_iter().map(|namespace| {
                let base = format!("assets/{namespace}/polytone");
                (
                    pack_root
                        .read_to_string(&format!("{base}/color.properties"))
                        .ok(),
                    pack_root
                        .read_to_string(&format!("{base}/colors.json"))
                        .ok(),
                )
            }),
        )
        .particles()
        {
            let json = serde_json::json!({"targets": [id], "color": color});
            match ParticleModifier::parse(&json, None) {
                Ok(Some(m)) => particle_modifiers.push((m, None, None)),
                Ok(None) => {}
                Err(e) => log::warn!("colors particle.{id}: {e}"),
            }
        }
        for namespace in pack_root.namespaces("assets") {
            let polytone = std::path::PathBuf::from(format!("assets/{namespace}/polytone"));
            for (relative, json) in json_files(pack_root, &polytone.join("custom_particles")) {
                let Some(id) = horizon_polytone::implicit_id(&relative, "custom_particles") else {
                    continue;
                };
                match CustomParticle::parse(&json) {
                    Ok(particle) => {
                        types.insert(id, Arc::new(particle));
                    }
                    Err(e) => log::warn!("{relative}: {e}"),
                }
            }
            for (relative, json) in json_files(pack_root, &polytone.join("entity_modifiers")) {
                let implicit = horizon_polytone::implicit_id(&relative, "entity_modifiers");
                match EntityEmitters::parse(&json, implicit.as_deref()) {
                    Ok(Some(m)) if !m.emitters.is_empty() => entity_modifiers.push(Arc::new(m)),
                    Ok(_) => {}
                    Err(e) => log::warn!("{relative}: {e}"),
                }
            }
            for (relative, json) in json_files(pack_root, &polytone.join("particle_modifiers")) {
                let implicit = horizon_polytone::implicit_id(&relative, "particle_modifiers");
                match ParticleModifier::parse(&json, implicit.as_deref()) {
                    Ok(Some(m)) => {
                        let tint = m.colormap.as_ref().and_then(|json| {
                            let tint = Tint::read(pack_root, json);
                            if tint.is_none() {
                                log::warn!("{relative}: colormap {json} not found");
                            }
                            tint
                        });
                        let filter = m.filter.as_ref().map(Filter::read);
                        particle_modifiers.push((m, tint, filter));
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("{relative}: {e}"),
                }
            }
            for (relative, json) in json_files(pack_root, &polytone.join("block_modifiers")) {
                let implicit = horizon_polytone::implicit_id(&relative, "block_modifiers");
                match BlockEmitters::parse(&json, implicit.as_deref()) {
                    Ok(Some(m)) => modifiers.push(Arc::new(m)),
                    Ok(_) => {}
                    Err(e) => log::warn!("{relative}: {e}"),
                }
            }
        }
        // Each block's modifiers: the highest priority ones.
        let tags = crate::pack_tags::Tags::new(pack_root);
        let mut by_name: HashMap<&str, Option<Arc<[Arc<BlockEmitters>]>>> = HashMap::new();
        let block_emitters = (0..blocks.len() as u32)
            .map(|i| {
                let name = blocks
                    .state(horizon_world::registry::BlockStateId(i))?
                    .name
                    .as_str();
                by_name
                    .entry(name)
                    .or_insert_with(|| {
                        let aimed: Vec<_> = modifiers
                            .iter()
                            .filter(|m| {
                                m.targets
                                    .matches(name, |tag| tags.contains("block", tag, name))
                            })
                            .collect();
                        let top = aimed.iter().map(|m| m.priority).max()?;
                        Some(
                            aimed
                                .into_iter()
                                .filter(|m| m.priority == top)
                                .cloned()
                                .collect(),
                        )
                    })
                    .clone()
            })
            .collect();
        let models = super::polytone_model::read_models(
            pack_root,
            types
                .iter()
                .filter_map(|(id, def)| Some((id, def.render.get("model")?))),
        );
        let tints = types
            .iter()
            .filter_map(|(id, def)| {
                let json = def.render.get("colormap")?;
                match Tint::read(pack_root, json) {
                    Some(tint) => Some((id.clone(), Arc::new(tint))),
                    None => {
                        log::warn!("custom particle {id}: colormap {json} not found");
                        None
                    }
                }
            })
            .collect();
        Self {
            types,
            block_emitters,
            entity_modifiers,
            entity_emitters: Mutex::default(),
            biomes: RwLock::default(),
            frame: Arc::new(RwLock::new(Frame::default())),
            configs,
            blocks,
            tints,
            models,
            tags,
            sound_names: Mutex::default(),
            sounds: Mutex::default(),
            particle_modifiers,
            modifiers_by_type: Mutex::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
            && self.block_emitters.iter().all(Option::is_none)
            && self.entity_modifiers.is_empty()
            && self.particle_modifiers.is_empty()
    }

    /// Whether any pack modifies vanilla or mod particles (`particle_modifiers`).
    pub(super) fn modifies_particles(&self) -> bool {
        !self.particle_modifiers.is_empty()
    }

    /// The particle modifiers aimed at a particle type (`minecraft:flame`), in the order read;
    /// `#tags` in `targets` are particle type tags.
    fn modifiers_for(&self, id: &str) -> Arc<[usize]> {
        let mut cache = self
            .modifiers_by_type
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(found) = cache.get(id) {
            return found.clone();
        }
        let found: Arc<[usize]> = (0..self.particle_modifiers.len())
            .filter(|&i| {
                self.particle_modifiers[i]
                    .0
                    .targets
                    .matches(id, |tag| self.tags.contains("particle_type", tag, id))
            })
            .collect();
        cache.insert(id.to_owned(), found.clone());
        found
    }

    /// A sound id as the engine's sound queue holds it.
    fn sound_name(&self, id: &str) -> &'static str {
        let mut names = self.sound_names.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(name) = names.get(id) {
            return name;
        }
        let name: &'static str = Box::leak(id.to_owned().into_boxed_str());
        names.insert(id.to_owned(), name);
        name
    }

    /// The sounds custom particles played since the last call.
    pub(super) fn take_sounds(&self) -> Vec<Sound> {
        std::mem::take(&mut *self.sounds.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn get(&self, id: &str) -> Option<&Arc<CustomParticle>> {
        self.types.get(id)
    }

    /// Sets the world's biome table (the world's own, or the default one).
    pub fn set_biomes(&self, biomes: &Arc<crate::biome::Biomes>) {
        let same = self
            .biomes
            .read()
            .is_ok_and(|b| b.as_ref().is_some_and(|b| Arc::ptr_eq(b, biomes)));
        if !same && let Ok(mut b) = self.biomes.write() {
            *b = Some(biomes.clone());
        }
    }

    /// The entity modifiers aimed at an entity type (`minecraft:player`); `#tags` in `targets`
    /// are entity type tags.
    fn entity_emitters(&self, entity: &str) -> Arc<[Arc<EntityEmitters>]> {
        let mut cache = self
            .entity_emitters
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache
            .entry(entity.to_owned())
            .or_insert_with(|| {
                self.entity_modifiers
                    .iter()
                    .filter(|m| {
                        m.targets
                            .matches(entity, |tag| self.tags.contains("entity_type", tag, entity))
                    })
                    .cloned()
                    .collect()
            })
            .clone()
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Registry({} types, {} emitting states)",
            self.types.len(),
            self.block_emitters.iter().filter(|e| e.is_some()).count()
        )
    }
}

/// What an emitter seeds on the custom particle it spawns: its own `red`... `custom` and customs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seed {
    pub fields: BTreeMap<String, f64>,
    pub customs: BTreeMap<String, f64>,
}

/// A custom particle's state beyond the engine particle's.
#[derive(Clone)]
struct Controller {
    def: Arc<CustomParticle>,
    registry: Arc<Registry>,
    custom: f64,
    customs: BTreeMap<String, f64>,
    ticks: u32,
    /// Sprites in its set, for the age-driven frame.
    frames: usize,
    /// Its colormap, and what the last sample left.
    tint: Option<Arc<Tint>>,
    tinting: Tinting,
}

/// A particle's colormap state: its colour before the colormap, the block it was last
/// sampled at, and that sample.
#[derive(Clone, Copy, Debug)]
struct Tinting {
    base: Option<[f32; 3]>,
    at: Option<[i32; 3]>,
    rgb: [f32; 3],
}

impl Default for Tinting {
    fn default() -> Self {
        Self {
            base: None,
            at: None,
            rgb: [1.0; 3],
        }
    }
}

impl Tinting {
    /// Samples the colormap as its `cache` says and multiplies it into the colour.
    fn apply(&mut self, tint: &Tint, ctx: &mut Ctx) {
        let Subject::Particle(p) = &ctx.subject else {
            return;
        };
        let base = [p.rgba[0], p.rgba[1], p.rgba[2]];
        let at = ParticleLevel::block_at(p.position);
        let resample = match tint.cache {
            TintCache::EveryTick => true,
            TintCache::PerPosition => self.at != Some(at),
            TintCache::OnSpawn => self.at.is_none(),
        };
        if resample {
            self.at = Some(at);
            self.rgb = tint.sample(ctx).unwrap_or([1.0; 3]);
        }
        self.base = Some(base);
        if let Subject::Particle(p) = &mut ctx.subject {
            for i in 0..3 {
                p.rgba[i] = base[i] * self.rgb[i];
            }
        }
    }
}

impl std::fmt::Debug for Controller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Controller")
            .field("custom", &self.custom)
            .field("customs", &self.customs)
            .finish()
    }
}

const SELF: u32 = 1;
const PLAYER: u32 = 2;
const CAMERA: u32 = 3;
const GLOBAL: u32 = 4;
const RANDOM: u32 = 5;
/// Block objects (`o.relative(...)`) are ids from here, indexes into `Ctx::blocks`.
const BLOCKS: u32 = 16;

/// What `o` is: a custom particle (tickers, particle emitters), a block (block emitters) or an
/// entity (entity emitters).
enum Subject<'a> {
    Particle(&'a mut Particle),
    Block([i32; 3]),
    Entity([f64; 3]),
}

/// What one expression runs against: its subject, the frame and the level.
struct Ctx<'a, 'l> {
    subject: Subject<'a>,
    random: &'a mut LegacyRandom,
    custom: &'a mut f64,
    customs: &'a mut BTreeMap<String, f64>,
    frame: &'a Frame,
    registry: &'a Registry,
    level: Option<&'a ParticleLevel<'l>>,
    blocks: Vec<[i32; 3]>,
}

/// The fluid at `at` (none for a state the facts lack).
fn fluid_at(level: &ParticleLevel, at: [i32; 3]) -> Fluid {
    let state = level.world.block(at[0], at[1], at[2]);
    level
        .facts
        .states
        .get(state.0 as usize)
        .map_or(Fluid::None, |f| f.fluid)
}

fn axis(name: &str) -> usize {
    usize::from(name.as_bytes()[0] - b'x')
}

impl Ctx<'_, '_> {
    fn position(&self) -> [f64; 3] {
        match &self.subject {
            Subject::Particle(p) => p.position,
            Subject::Block(b) => b.map(f64::from),
            Subject::Entity(at) => *at,
        }
    }
    /// The biome at a block, when the level and biomes are known.
    fn biome_info(&self, at: [i32; 3]) -> Option<crate::biome::BiomeInfo> {
        let chunk = self.level?.world.chunks.get(&(at[0] >> 4, at[2] >> 4))?;
        let id = chunk.biome(at[0], at[1], at[2]);
        let biomes = self.registry.biomes.read().ok()?;
        Some(biomes.as_ref()?.get(id).clone())
    }
    /// The biome at a block (`minecraft:plains`), when the level and biomes are known.
    fn biome(&self, at: [i32; 3]) -> Option<String> {
        let chunk = self.level?.world.chunks.get(&(at[0] >> 4, at[2] >> 4))?;
        let id = chunk.biome(at[0], at[1], at[2]);
        let biomes = self.registry.biomes.read().ok()?;
        Some(biomes.as_ref()?.get(id).name.to_string())
    }
    fn block_object(&mut self, at: [i32; 3]) -> V {
        self.blocks.push(at);
        V::Object(BLOCKS + self.blocks.len() as u32 - 1)
    }
    fn rand(&mut self) -> f64 {
        f64::from(self.random.next_float())
    }
    /// The positional methods of the subject's block and of block objects.
    fn positional(&mut self, at: [i32; 3], name: &str, args: &[V]) -> Result<V, String> {
        let num = |n: f64| Ok(V::Num(n));
        if name == "biome" {
            return Ok(V::Str(self.biome(at).unwrap_or_default().into()));
        }
        // The biome's climate where it is known, else the camera's.
        if name == "temperature" {
            let t = self
                .biome_info(at)
                .map_or(self.frame.temperature, |b| b.temperature);
            return num(f64::from(t));
        }
        if name == "downfall" {
            let d = self
                .biome_info(at)
                .map_or(self.frame.downfall, |b| b.downfall);
            return num(f64::from(d));
        }
        let arg = |i: usize| args.get(i).map_or(0.0, V::num) as i32;
        let neighbour = match name {
            "relative" => Some([at[0] + arg(0), at[1] + arg(1), at[2] + arg(2)]),
            "north" => Some([at[0], at[1], at[2] - 1]),
            "south" => Some([at[0], at[1], at[2] + 1]),
            "west" => Some([at[0] - 1, at[1], at[2]]),
            "east" => Some([at[0] + 1, at[1], at[2]]),
            "up" => Some([at[0], at[1] + 1, at[2]]),
            "down" => Some([at[0], at[1] - 1, at[2]]),
            _ => None,
        };
        if let Some(n) = neighbour {
            return Ok(self.block_object(n));
        }
        let level = self.level.ok_or("no level here")?;
        let state = level.world.block(at[0], at[1], at[2]);
        let fluid = fluid_at(level, at);
        let light = level.world.light(at[0], at[1], at[2]);
        match name {
            "block" => Ok(V::Str(
                self.registry
                    .blocks
                    .state(state)
                    .map_or("minecraft:air", |s| s.name.as_str())
                    .into(),
            )),
            "hasAirAt" => Ok(V::Bool(state == level.facts.air)),
            "hasFluid" => Ok(V::Bool(fluid != Fluid::None)),
            "fluid" => Ok(V::Str(
                match fluid {
                    Fluid::Water => "minecraft:water",
                    Fluid::Lava => "minecraft:lava",
                    Fluid::Modded | Fluid::None => "minecraft:empty",
                }
                .into(),
            )),
            "skyLight" => num(f64::from(light >> 4)),
            "blockLight" => num(f64::from(light & 15)),
            "canSeeSky" => Ok(V::Bool(light >> 4 == 15)),
            "blockStateValue" => {
                let Some(V::Str(property)) = args.first() else {
                    return Err("blockStateValue needs a property".into());
                };
                let value = self
                    .registry
                    .blocks
                    .state(state)
                    .and_then(|s| s.properties.get(property.as_ref()));
                Ok(match value {
                    Some(v) => v
                        .parse::<f64>()
                        .map_or_else(|_| V::Str(v.as_str().into()), V::Num),
                    None => V::Null,
                })
            }
            _ => Err(format!("{name} is not a block method")),
        }
    }
    /// `o.<name>(...)` on a custom particle: its fields, setters and named customs.
    fn particle_method(&mut self, name: &str, args: &[V]) -> Option<Result<V, String>> {
        let value = args.first().map(V::num);
        let custom = &mut *self.custom;
        let customs = &mut *self.customs;
        let Subject::Particle(p) = &mut self.subject else {
            return None;
        };
        let f = |n: f64| Some(Ok(V::Num(n)));
        let set = |slot: &mut f32| {
            if let Some(v) = value {
                *slot = v as f32;
            }
            Some(Ok(V::Num(f64::from(*slot))))
        };
        match name {
            "age" => f(f64::from(p.age)),
            "life" => f(f64::from(p.lifetime)),
            "x" | "y" | "z" => f(p.position[axis(name)]),
            "xd" | "yd" | "zd" => f(p.velocity[axis(name)]),
            "setX" | "setY" | "setZ" => {
                p.position[usize::from(name.as_bytes()[3] - b'X')] = value?;
                f(value?)
            }
            "setXd" | "setYd" | "setZd" => {
                p.velocity[usize::from(name.as_bytes()[3] - b'X')] = value?;
                f(value?)
            }
            "red" | "setRed" => set(&mut p.rgba[0]),
            "green" | "setGreen" => set(&mut p.rgba[1]),
            "blue" | "setBlue" => set(&mut p.rgba[2]),
            "alpha" | "setAlpha" => set(&mut p.rgba[3]),
            "roll" | "setRoll" => set(&mut p.roll),
            "size" | "setSize" => set(&mut p.size),
            "color" => {
                let c = |i: usize| (p.rgba[i].clamp(0.0, 1.0) * 255.0) as u32;
                f(f64::from(c(0) << 16 | c(1) << 8 | c(2)))
            }
            "setRemoved" | "removed" => {
                p.remove();
                Some(Ok(V::Bool(true)))
            }
            "custom" | "setCustom" => Some(match args {
                [] => Ok(V::Num(*custom)),
                [V::Str(n)] => Ok(V::Num(
                    customs.get(&n.to_lowercase()).copied().unwrap_or(0.0),
                )),
                [V::Str(n), v] => {
                    customs.insert(n.to_lowercase(), v.num());
                    Ok(V::Num(v.num()))
                }
                [v] => {
                    *custom = v.num();
                    Ok(V::Num(v.num()))
                }
                _ => Err("custom takes a name and a value at most".into()),
            }),
            _ => None,
        }
    }
}

impl Host for Ctx<'_, '_> {
    fn root(&mut self, root: Root) -> V {
        match root {
            Root::Object => V::Object(SELF),
            Root::Player => V::Object(PLAYER),
            Root::Camera => V::Object(CAMERA),
            Root::Global => V::Object(GLOBAL),
            Root::Random => V::Object(RANDOM),
            Root::Value => V::Num(0.0),
        }
    }

    fn method(&mut self, target: &V, name: &str, args: &[V]) -> Result<V, String> {
        let V::Object(id) = target else {
            return Err(format!("{name} on a value that is no object"));
        };
        let num = |n: f64| Ok(V::Num(n));
        let frame = self.frame;
        let other = |v: Option<&V>| match v {
            Some(V::Object(PLAYER)) => Some(frame.player),
            Some(V::Object(CAMERA)) => Some(frame.camera),
            _ => None,
        };
        // Positional objects all measure distances the same way.
        if matches!(name, "distSq" | "dist") {
            let from = match *id {
                SELF => self.position(),
                PLAYER => frame.player,
                CAMERA => frame.camera,
                b if b >= BLOCKS => self
                    .blocks
                    .get((b - BLOCKS) as usize)
                    .ok_or("no such block object")?
                    .map(f64::from),
                _ => return Err(format!("{name} on no position")),
            };
            let to =
                other(args.first()).ok_or_else(|| format!("{name} needs the player or camera"))?;
            let d2: f64 = (0..3).map(|i| (from[i] - to[i]).powi(2)).sum();
            return num(if name == "dist" { d2.sqrt() } else { d2 });
        }
        match (*id, name) {
            (SELF, _) => {
                if let Some(result) = self.particle_method(name, args) {
                    return result;
                }
                if matches!(name, "x" | "y" | "z") && !matches!(self.subject, Subject::Particle(_))
                {
                    return num(self.position()[axis(name)]);
                }
                let at = ParticleLevel::block_at(self.position());
                self.positional(at, name, args)
            }
            (PLAYER, "x" | "y" | "z") => num(frame.player[axis(name)]),
            (PLAYER, "inWater") => Ok(V::Bool(frame.in_water)),
            (PLAYER, "onGround") => Ok(V::Bool(frame.player_on_ground)),
            (PLAYER, "speedSq") => num(frame.player_velocity.iter().map(|v| v * v).sum()),
            (PLAYER, "speed") => num(frame
                .player_velocity
                .iter()
                .map(|v| v * v)
                .sum::<f64>()
                .sqrt()),
            (PLAYER, _) => self.positional(ParticleLevel::block_at(frame.player), name, args),
            (CAMERA, "x" | "y" | "z") => num(frame.camera[axis(name)]),
            (CAMERA, "viewDistance") => num(f64::from(frame.render_distance * 16)),
            (CAMERA, _) => self.positional(ParticleLevel::block_at(frame.camera), name, args),
            (GLOBAL, "rain") => num(frame.global_rain()),
            (GLOBAL, "time") => num(frame.game_time as f64),
            (GLOBAL, "dayTime") => num(frame.day_time as f64),
            (GLOBAL, "skyType") => num(f64::from(frame.sky_type)),
            (GLOBAL, "dimensionType") => Ok(V::Str(frame.dimension.as_str().into())),
            (GLOBAL, "renderDistance") => num(f64::from(frame.render_distance)),
            (RANDOM, "rand") => {
                let r = self.rand();
                let a = |i: usize| args.get(i).map_or(0.0, V::num);
                num(match args.len() {
                    0 => r,
                    1 => r * a(0),
                    _ => a(0) + r * (a(1) - a(0)),
                })
            }
            (RANDOM, "gaussian") => {
                let g = self.random.next_gaussian();
                let a = |i: usize| args.get(i).map_or(0.0, V::num);
                num(if args.len() >= 2 { a(0) + g * a(1) } else { g })
            }
            (RANDOM, "noise") => {
                // `r.noise(x, y)`: one simplex noise (named noises aren't read yet).
                let mut coords = args.iter().filter(|v| !matches!(v, V::Str(_))).map(V::num);
                let (x, y) = (coords.next().unwrap_or(0.0), coords.next().unwrap_or(0.0));
                num(crate::environment::polytone_noise().get(x, y))
            }
            (id, _) if id >= BLOCKS => {
                let at = *self
                    .blocks
                    .get((id - BLOCKS) as usize)
                    .ok_or("no such block object")?;
                match name {
                    "x" | "y" | "z" => num(f64::from(at[axis(name)])),
                    _ => self.positional(at, name, args),
                }
            }
            _ => Err(format!("{name} is not available here")),
        }
    }

    fn function(&mut self, name: &str, args: &[V]) -> Result<V, String> {
        match (name, args) {
            ("config", [V::Str(id)]) => self
                .registry
                .configs
                .get(id)
                .map(horizon_polytone::pack::ConfigValue::to_value)
                .ok_or_else(|| format!("no config {id}")),
            _ => Err(format!("{name} is not available here")),
        }
    }

    /// A particle's own fields, as `particle_method` answers them.
    fn field(&mut self, root: Root, field: horizon_polytone::expr::Field) -> Option<V> {
        use horizon_polytone::expr::Field as F;
        let Subject::Particle(p) = &self.subject else {
            return None;
        };
        if root != Root::Object {
            return None;
        }
        Some(V::Num(match field {
            F::Age => f64::from(p.age),
            F::Life => f64::from(p.lifetime),
            F::Custom => *self.custom,
            F::X => p.position[0],
            F::Y => p.position[1],
            F::Z => p.position[2],
            F::Temperature | F::Downfall => return None,
        }))
    }
}

fn eval(program: &Program, ctx: &mut Ctx) -> Option<f64> {
    program.eval(ctx).ok().map(|v| v.num())
}

/// A rule test on a block state (`state_predicate`): `always_true`, `block_match` and
/// `blockstate_match`; other tests don't hold.
fn state_matches(predicate: &Json, ctx: &Ctx, at: [i32; 3]) -> bool {
    let Some(level) = ctx.level else {
        return false;
    };
    let state = level.world.block(at[0], at[1], at[2]);
    let kind = predicate["predicate_type"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("minecraft:");
    let name = |id: &str| {
        if id.contains(':') {
            id.to_owned()
        } else {
            format!("minecraft:{id}")
        }
    };
    let Some(block) = ctx.registry.blocks.state(state) else {
        return false;
    };
    match kind {
        "always_true" => true,
        "block_match" => predicate["block"]
            .as_str()
            .is_some_and(|b| name(b) == block.name),
        "blockstate_match" => {
            let wanted = &predicate["block_state"];
            let id = wanted["Name"].as_str().unwrap_or("");
            name(id) == block.name
                && wanted["Properties"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .all(|(k, v)| {
                        block.properties.get(k.as_str()).map(String::as_str) == v.as_str()
                    })
        }
        _ => false,
    }
}

/// Whether `biomes` (an id, a `#tag`, or a list of them) holds the biome
/// at `at`.
fn biome_matches(biomes: &Json, ctx: &Ctx, at: [i32; 3]) -> bool {
    let Some(biome) = ctx.biome(at) else {
        return true;
    };
    let id = |s: &str| {
        if s.contains(':') {
            s.to_owned()
        } else {
            format!("minecraft:{s}")
        }
    };
    // An id, or a `#tag` of biomes.
    let holds = |entry: &str| match entry.strip_prefix('#') {
        Some(tag) => ctx.registry.tags.contains("worldgen/biome", tag, &biome),
        None => id(entry) == biome,
    };
    match biomes {
        Json::String(s) => holds(s),
        Json::Array(list) => list.iter().filter_map(Json::as_str).any(holds),
        _ => true,
    }
}

/// The options to spawn `id` with: a pack's custom particle, a vanilla particle, or a mod's
/// sprite-set particle.
fn options_for(id: &str, registry: &Registry) -> ParticleOptions {
    if registry.types.contains_key(id) {
        return ParticleOptions::named(id);
    }
    match ParticleType::from_name(id) {
        Some(ty) if id.starts_with("minecraft:") => ParticleOptions::simple(ty),
        _ => ParticleOptions::named(id),
    }
}

/// Runs `emitters` from the subject: each fires `count` times when its `chance` beats a random
/// draw, its position relative to `base` (the particle, or the block's spawn location).
fn emit(emitters: &[Emitter], ctx: &mut Ctx, out: &mut Vec<(ParticleOptions, [f64; 3], [f64; 3])>) {
    for emitter in emitters {
        if emitter
            .tick_source
            .as_deref()
            .is_some_and(|s| s.trim_start_matches("minecraft:") != "animate_tick")
        {
            continue;
        }
        // No `chance` always fires; one that fails to run never does.
        let chance = emitter
            .fields
            .get("chance")
            .map_or(Some(1.0), |c| eval(c, ctx))
            .unwrap_or(0.0);
        if ctx.rand() >= chance {
            continue;
        }
        let field = |ctx: &mut Ctx, key: &str| emitter.fields.get(key).and_then(|p| eval(p, ctx));
        let base = match &ctx.subject {
            Subject::Particle(p) => p.position,
            // Bones aren't resolved yet: offsets count from the entity's feet, in world axes.
            Subject::Entity(at) => *at,
            Subject::Block(b) => {
                let corner = b.map(f64::from);
                match emitter.spawn_location.as_deref() {
                    Some("lower_corner") => corner,
                    _ => corner.map(|c| c + 0.5),
                }
            }
        };
        if let Some(biomes) = &emitter.biomes
            && !biome_matches(biomes, ctx, ParticleLevel::block_at(base))
        {
            continue;
        }
        let count = field(ctx, "count").map_or(1, |c| c.max(0.0) as u32).min(64);
        for _ in 0..count {
            let at = [
                base[0] + field(ctx, "x").unwrap_or(0.0),
                base[1] + field(ctx, "y").unwrap_or(0.0),
                base[2] + field(ctx, "z").unwrap_or(0.0),
            ];
            if let Some(predicate) = &emitter.state_predicate
                && !state_matches(predicate, ctx, ParticleLevel::block_at(at))
            {
                continue;
            }
            let velocity = [
                field(ctx, "dx").unwrap_or(0.0),
                field(ctx, "dy").unwrap_or(0.0),
                field(ctx, "dz").unwrap_or(0.0),
            ];
            let id = emitter.particle.id();
            let mut options = options_for(&id, ctx.registry);
            if ctx.registry.types.contains_key(&id) {
                let mut seed = Seed::default();
                for (key, short) in [
                    ("red", "r"),
                    ("green", "g"),
                    ("blue", "b"),
                    ("alpha", "a"),
                    ("roll", ""),
                    ("size", ""),
                    ("custom", ""),
                ] {
                    let program = emitter.fields.get(key).or(emitter.fields.get(short));
                    if let Some(v) = program.and_then(|p| eval(p, ctx)) {
                        seed.fields.insert(key.to_owned(), v);
                    }
                }
                for (name, p) in &emitter.customs {
                    if let Some(v) = eval(p, ctx) {
                        seed.customs.insert(name.to_lowercase(), v);
                    }
                }
                options.polytone_seed = (seed != Seed::default()).then(|| Arc::new(seed));
            }
            out.push((options, at, velocity));
        }
    }
}

/// Runs sound emitters from the subject: each plays when its `chance` beats a random draw, at
/// its position relative to the subject (a block's centre).
fn emit_sounds(emitters: &[SoundEmitter], ctx: &mut Ctx, out: &mut Vec<Sound>) {
    for emitter in emitters {
        let chance = emitter
            .fields
            .get("chance")
            .map_or(Some(1.0), |c| eval(c, ctx))
            .unwrap_or(0.0);
        if ctx.rand() >= chance {
            continue;
        }
        let field = |ctx: &mut Ctx, key: &str| emitter.fields.get(key).and_then(|p| eval(p, ctx));
        let base = match &ctx.subject {
            Subject::Block(b) => b.map(|c| f64::from(c) + 0.5),
            _ => ctx.position(),
        };
        if let Some(biomes) = &emitter.biomes
            && !biome_matches(biomes, ctx, ParticleLevel::block_at(base))
        {
            continue;
        }
        let at = [
            base[0] + field(ctx, "x").unwrap_or(0.0),
            base[1] + field(ctx, "y").unwrap_or(0.0),
            base[2] + field(ctx, "z").unwrap_or(0.0),
        ];
        if let Some(predicate) = &emitter.state_predicate
            && !state_matches(predicate, ctx, ParticleLevel::block_at(at))
        {
            continue;
        }
        let volume = field(ctx, "volume").unwrap_or(1.0) as f32;
        let pitch = field(ctx, "pitch").unwrap_or(1.0) as f32;
        let name = ctx.registry.sound_name(&emitter.sound);
        out.push((name, at, volume, pitch, emitter.distance_delay));
    }
}

/// Sets each `keys` field the program map holds on `p` through `apply`.
fn run_fields(
    programs: &BTreeMap<String, Program>,
    ctx: &mut Ctx,
    keys: &[&str],
    mut apply: impl FnMut(&mut Ctx, &str, f64),
) {
    for &key in keys {
        if let Some(v) = programs.get(key).and_then(|p| eval(p, ctx)) {
            apply(ctx, key, v);
        }
    }
}

/// Writes a particle field a ticker or initializer computed.
fn set_field(p: &mut Particle, custom: &mut f64, key: &str, v: f64) {
    let v32 = v as f32;
    match key {
        "x" => p.position[0] = v,
        "y" => p.position[1] = v,
        "z" => p.position[2] = v,
        "dx" => p.velocity[0] = v,
        "dy" => p.velocity[1] = v,
        "dz" => p.velocity[2] = v,
        "red" => p.rgba[0] = v32,
        "green" => p.rgba[1] = v32,
        "blue" => p.rgba[2] = v32,
        "alpha" => p.rgba[3] = v32,
        "size" => p.size = v32,
        "roll" => p.roll = v32,
        "custom" => *custom = v,
        "lifetime" => p.lifetime = v as i32,
        "friction" => p.friction = v32,
        "hitbox_size" => {
            p.bb_width = v32;
            p.bb_height = v32;
        }
        _ => {}
    }
}

const TICKED: [&str; 13] = [
    "dx", "dy", "dz", "x", "y", "z", "red", "green", "blue", "alpha", "size", "roll", "custom",
];
const INITIALIZED: [&str; 13] = [
    "lifetime",
    "size",
    "hitbox_size",
    "red",
    "green",
    "blue",
    "alpha",
    "roll",
    "friction",
    "custom",
    "dx",
    "dy",
    "dz",
];

/// Whether no block's collision shape overlaps `bb` (where the query fails, nothing does).
pub(super) fn unobstructed(level: &ParticleLevel<'_>, bb: horizon_core::geometry::Aabb) -> bool {
    let mut scene = crate::collisions::SceneCollisions {
        world: level.world,
        binding: level.collision,
        subject: None,
    };
    scene.cells_without_collision(bb) || scene.unobstructed(bb).unwrap_or(true)
}

impl ParticleEngine {
    /// Whether a pending Polytone particle joins the world. One with physics whose box starts
    /// inside a block's collision never does, and its type's `limit` is then counted among those
    /// that joined, so one that never joins takes no place (measured on the mod: with `limit` 1,
    /// one born inside a block and one clear of it in the same tick, the clear one shows).
    pub(super) fn polytone_joins(&self, p: &Particle, level: &ParticleLevel<'_>) -> bool {
        if !p.spawn_check {
            return true;
        }
        if !unobstructed(level, p.bb) {
            return false;
        }
        let Some(name) = p.polytone_type.as_deref() else {
            return true;
        };
        let def = self.polytone.as_ref().and_then(|r| r.get(name));
        let limit = def.and_then(|d| d.limit.filter(|_| d.gpu.is_none()));
        limit.is_none_or(|limit| {
            let same = |q: &&Particle| !q.removed && q.polytone_type.as_deref() == Some(name);
            self.particles.iter().filter(same).count() < limit as usize
        })
    }
}

impl super::ParticleController for Controller {
    fn tick(&mut self, p: &mut Particle, level: &ParticleLevel<'_>) -> ParticleUpdate {
        let mut update = ParticleUpdate::default();
        if !p.advance_age() {
            return update;
        }
        let registry = self.registry.clone();
        let def = self.def.clone();
        self.ticks += 1;
        if self.ticks.is_multiple_of(def.tick_interval.max(1)) {
            let Ok(frame) = registry.frame.read() else {
                return update;
            };
            // Expressions see the colour before the colormap.
            if let Some(base) = self.tinting.base {
                p.rgba[..3].copy_from_slice(&base);
            }
            let mut random = std::mem::replace(&mut p.random, LegacyRandom::new(0));
            let mut ctx = Ctx {
                subject: Subject::Particle(p),
                random: &mut random,
                custom: &mut self.custom,
                customs: &mut self.customs,
                frame: &frame,
                registry: &registry,
                level: Some(level),
                blocks: Vec::new(),
            };
            let removed = def
                .ticker
                .get("remove_condition")
                .and_then(|prog| prog.eval(&mut ctx).ok())
                .is_some_and(|v| v.num() != 0.0);
            if !removed {
                run_fields(&def.ticker, &mut ctx, &TICKED, |ctx, key, v| {
                    let custom = &mut *ctx.custom;
                    if let Subject::Particle(p) = &mut ctx.subject {
                        set_field(p, custom, key, v);
                    }
                });
                for (name, prog) in &def.ticker_customs {
                    if let Some(v) = eval(prog, &mut ctx) {
                        ctx.customs.insert(name.to_lowercase(), v);
                    }
                }
                emit(&def.emitters, &mut ctx, &mut update.spawned);
                if !def.sound_emitters.is_empty() {
                    let mut sounds = Vec::new();
                    emit_sounds(&def.sound_emitters, &mut ctx, &mut sounds);
                    registry
                        .sounds
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(sounds);
                }
                if let Some(tint) = self.tint.clone() {
                    self.tinting.apply(&tint, &mut ctx);
                }
            }
            drop(ctx);
            p.random = random;
            if removed {
                p.remove();
                return update;
            }
        }
        // Moves with collision when it has physics, else freely; then the friction.
        if def.has_physics {
            p.move_by(level, p.velocity);
            // `kill_when_still`: a particle a collision stopped (one that landed), not one that
            // was never moving: in Java a resting emitter with physics lives out its life.
            if (def.kill_on_contact && (p.on_ground || p.stopped_by_collision))
                || (def.kill_when_still && p.stopped_by_collision)
            {
                p.remove();
            }
        } else {
            for i in 0..3 {
                p.position[i] += p.velocity[i];
            }
            p.bb = super::aabb_at(p.position, p.bb_width, p.bb_height);
        }
        let f = f64::from(p.friction);
        for c in &mut p.velocity {
            *c *= f;
        }
        if def.liquid_affinity != LiquidAffinity::Any {
            let wet = fluid_at(level, ParticleLevel::block_at(p.position)) != Fluid::None;
            if wet != (def.liquid_affinity == LiquidAffinity::Liquids) {
                p.remove();
            }
        }
        if !def.random_sprite && p.lifetime > 0 {
            // Through the sprite set over the particle's life, as vanilla's setSpriteFromAge.
            let last = self.frames.saturating_sub(1);
            update.frame = Some(p.age.max(0) as usize * last / p.lifetime.max(1) as usize);
        }
        update
    }
}

/// A custom particle at `position` moving at `velocity`, its initializer run (seeded by the
/// emitter that spawned it); `None` without a sprite set for a visible type.
#[allow(clippy::too_many_arguments)]
pub(super) fn make(
    engine: &ParticleEngine,
    registry: &Arc<Registry>,
    name: &str,
    def: &Arc<CustomParticle>,
    options: &ParticleOptions,
    position: [f64; 3],
    velocity: [f64; 3],
    seed: i64,
) -> Option<Particle> {
    // `limit`: at most that many of the type alive at once. A pending one with physics may never
    // join (`polytone_joins`), so it takes no place until it does.
    // (A GPU particle's `limit` is its ring of records instead.)
    if let Some(limit) = def.limit
        && def.gpu.is_none()
    {
        let alive = engine
            .particles
            .iter()
            .chain(
                engine
                    .pending
                    .iter()
                    .filter_map(PendingParticle::ordinary)
                    .filter(|p| !p.spawn_check),
            )
            .filter(|p| !p.removed && p.polytone_type.as_deref() == Some(name))
            .count();
        if alive >= limit as usize {
            return None;
        }
    }
    // `exclusion_radius`: none spawns within that distance of a live one of its type (rain
    // splashes, one per patch of ground from tick to tick). Only those already in the world count: measured on
    // the mod, two spawned in the same tick a block apart both show, with or without physics.
    if def.exclusion_radius > 0.0 && def.gpu.is_none() {
        let r2 = def.exclusion_radius * def.exclusion_radius;
        let crowded = engine.particles.iter().any(|p| {
            !p.removed
                && p.polytone_type.as_deref() == Some(name)
                && (0..3)
                    .map(|i| (p.position[i] - position[i]).powi(2))
                    .sum::<f64>()
                    < r2
        });
        if crowded {
            return None;
        }
    }
    let mut random = LegacyRandom::new(seed);
    let mut particle = Particle::at(&mut random, ParticleType::Smoke, position);
    particle.velocity = velocity;
    particle.gravity = 0.0;
    particle.has_physics = def.has_physics;
    let mut frames = 0;
    // A model particle draws its model's faces; one whose model is missing still ticks and
    // emits. A GPU particle is drawn from its record instead (`polytone_gpu`).
    let model = registry.models.get(name).cloned();
    let hidden = def.render_type == RenderType::Invisible
        || (def.render.contains_key("model") && model.is_none())
        || def.gpu.is_some();
    // A custom sprite set, or the vanilla set when replacing a vanilla type.
    let set = engine
        .mod_sets
        .get(name)
        .map(|entry| entry.set)
        .or_else(|| {
            let vanilla = name.strip_prefix("minecraft:")?;
            engine.sets.get(&ParticleType::from_name(vanilla)?).copied()
        });
    match set {
        Some(set) => {
            let sprites = engine.set_sprites.get(set as usize)?;
            frames = sprites.len();
            particle.set = Some(set);
            let pick = if def.random_sprite {
                random.next_int_bound(frames.max(1) as i32).unwrap_or(0) as usize
            } else {
                0
            };
            particle.sprite = Sprite::Index(*sprites.get(pick).or(sprites.first())?);
        }
        None if hidden => {}
        None if model.is_some() => {}
        None => return None,
    }
    particle.no_render = hidden;
    if let Some(model) = model {
        // The sprite lookup that admits a particle to drawing: its first face's.
        particle.sprite = Sprite::Named(model.quads[0].sprite.clone());
        particle.model = Some(model);
    }
    particle.orientation = super::polytone_gpu::Orientation::parse(&def.rotation_mode);
    particle.additive = def.render_type == RenderType::AdditiveTranslucent;
    // `light_level`: the block light the particle gives itself (wiki "Custom Particle Types"),
    // a floor under the light where it is, as GPU particles have.
    if def.light_level > 0 {
        particle.light = super::Light::MinBlock(def.light_level);
    }
    particle.behaviour = Behaviour::Controlled {
        translucent: def.render_type != RenderType::Opaque,
    };
    let (mut custom, mut customs) = (0.0, BTreeMap::new());
    if let Ok(frame) = registry.frame.read() {
        let mut ctx = Ctx {
            subject: Subject::Particle(&mut particle),
            random: &mut random,
            custom: &mut custom,
            customs: &mut customs,
            frame: &frame,
            registry,
            level: None,
            blocks: Vec::new(),
        };
        run_fields(&def.initializer, &mut ctx, &INITIALIZED, |ctx, key, v| {
            let custom = &mut *ctx.custom;
            if let Subject::Particle(p) = &mut ctx.subject {
                set_field(p, custom, key, v);
            }
        });
        for (name, p) in &def.initializer_customs {
            if let Some(v) = eval(p, &mut ctx) {
                ctx.customs.insert(name.to_lowercase(), v);
            }
        }
    }
    particle.o_roll = particle.roll;
    // The emitter's own values win over the initializer's.
    if let Some(seed) = &options.polytone_seed {
        for (key, v) in &seed.fields {
            set_field(&mut particle, &mut custom, key, *v);
        }
        customs.extend(seed.customs.iter().map(|(k, v)| (k.clone(), *v)));
    }
    particle.random = random;
    particle.polytone_type =
        (def.limit.is_some() || def.exclusion_radius > 0.0).then(|| Arc::from(name));
    particle.bb = super::aabb_at(position, particle.bb_width, particle.bb_height);
    // A particle with physics whose box starts inside a block's collision never joins the world:
    // it is not drawn and none of its emitters run (measured on the mod: one spawned just under a
    // block, where falling leaves start, never shows; one 0.01 clear of it does).
    particle.spawn_check = def.has_physics;
    particle.controller = Some(Box::new(Controller {
        def: def.clone(),
        registry: registry.clone(),
        custom,
        customs,
        ticks: 0,
        frames,
        tint: registry.tints.get(name).cloned(),
        tinting: Tinting::default(),
    }));
    Some(particle)
}

/// A particle modifier's `filter`: the blocks and items (an id or a list each) whose particles
/// it changes.
struct Filter {
    blocks: Vec<String>,
    items: Vec<String>,
}

impl Filter {
    fn read(json: &Json) -> Self {
        let ids = |key: &str| -> Vec<String> {
            let id = |s: &str| {
                if s.contains(':') {
                    s.to_owned()
                } else {
                    format!("minecraft:{s}")
                }
            };
            match &json[key] {
                Json::String(s) => vec![id(s)],
                Json::Array(list) => list.iter().filter_map(Json::as_str).map(id).collect(),
                _ => Vec::new(),
            }
        };
        Self {
            blocks: ids("block"),
            items: ids("item"),
        }
    }

    /// Whether a particle carrying `source` (a block's or an item's id) passes.
    fn passes(&self, source: Option<&str>) -> bool {
        source.is_some_and(|s| self.blocks.iter().chain(&self.items).any(|id| id == s))
    }
}

/// Runs the packs' particle modifiers aimed at `id` on a particle that was just made: each
/// expression once, against the particle as its constructor left it (`size` scales it, `speed`
/// sets its power, as vanilla's `Particle.scale` and `setPower`).
pub(super) fn modify(registry: &Registry, id: &str, source: Option<&str>, p: &mut Particle) {
    let modifiers = registry.modifiers_for(id);
    if modifiers.is_empty() {
        return;
    }
    let Ok(frame) = registry.frame.read() else {
        return;
    };
    let mut random = std::mem::replace(&mut p.random, LegacyRandom::new(0));
    let (mut custom, mut customs) = (0.0, BTreeMap::new());
    for &i in modifiers.iter() {
        let (modifier, tint, filter) = &registry.particle_modifiers[i];
        if filter.as_ref().is_some_and(|f| !f.passes(source)) {
            continue;
        }
        let mut ctx = Ctx {
            subject: Subject::Particle(p),
            random: &mut random,
            custom: &mut custom,
            customs: &mut customs,
            frame: &frame,
            registry,
            level: None,
            blocks: Vec::new(),
        };
        // Every expression reads the particle before this modifier changes it.
        let value =
            |ctx: &mut Ctx, key: &str| modifier.fields.get(key).and_then(|prog| eval(prog, ctx));
        let values = MODIFIED.map(|key| value(&mut ctx, key));
        let tinted = tint.as_ref().and_then(|t| t.sample(&mut ctx));
        drop(ctx);
        let [color, red, green, blue, alpha, life, size, speed] = values;
        if let Some(rgb) = tinted {
            p.rgba[..3].copy_from_slice(&rgb);
        }
        if let Some(c) = color {
            let c = c as i64;
            for (channel, shift) in [16, 8, 0].into_iter().enumerate() {
                p.rgba[channel] = ((c >> shift) & 0xff) as f32 / 255.0;
            }
        }
        for (channel, v) in [red, green, blue, alpha].into_iter().enumerate() {
            if let Some(v) = v {
                p.rgba[channel] = v as f32;
            }
        }
        if let Some(life) = life {
            p.lifetime = life as i32;
        }
        if let Some(size) = size {
            p.scale(size as f32);
        }
        if let Some(speed) = speed {
            p.set_power(speed as f32);
        }
    }
    p.random = random;
}

/// The fields a particle modifier sets, in the order `modify` applies them.
const MODIFIED: [&str; 8] = [
    "color", "red", "green", "blue", "alpha", "life", "size", "speed",
];

impl ParticleEngine {
    /// The emitters of the packs' entity modifiers aimed at `entity` (its type, as
    /// `minecraft:player`) standing at `position`, run once a client tick.
    pub fn polytone_entity(&mut self, level: &ParticleLevel<'_>, entity: &str, position: [f64; 3]) {
        let Some(registry) = self.polytone.clone() else {
            return;
        };
        let modifiers = registry.entity_emitters(entity);
        if modifiers.is_empty() {
            return;
        }
        let Ok(frame) = registry.frame.read() else {
            return;
        };
        let camera = self.camera;
        let distance_sq: f64 = (0..3).map(|i| (position[i] - camera[i]).powi(2)).sum();
        let mut out = Vec::new();
        let (mut custom, mut customs) = (0.0, BTreeMap::new());
        let mut ctx = Ctx {
            subject: Subject::Entity(position),
            random: &mut self.random,
            custom: &mut custom,
            customs: &mut customs,
            frame: &frame,
            registry: &registry,
            level: Some(level),
            blocks: Vec::new(),
        };
        for modifier in modifiers.iter() {
            for emitter in &modifier.emitters {
                // `max_distance`: 32 blocks from the camera unless the emitter says otherwise.
                if distance_sq <= emitter.max_distance.unwrap_or(32.0).powi(2) {
                    emit(std::slice::from_ref(emitter), &mut ctx, &mut out);
                }
            }
        }
        drop(ctx);
        drop(frame);
        for (options, position, velocity) in out {
            self.add(&options, position, velocity);
        }
    }

    /// Whether the packs' block modifiers stop `state`'s own particles (`disable_particles`): its
    /// `Block.animateTick` doesn't run.
    pub fn polytone_disables_particles(
        &self,
        state: horizon_world::registry::BlockStateId,
    ) -> bool {
        self.polytone
            .as_ref()
            .and_then(|r| r.block_emitters.get(state.0 as usize))
            .and_then(Option::as_ref)
            .is_some_and(|modifiers| modifiers.iter().any(|m| m.disable_particles))
    }

    /// The block emitters of the packs' block modifiers aimed at the block at `at`, run from a
    /// block animate tick (`Block.animateTick`, as torches spawn their flames).
    pub fn polytone_animate(&mut self, level: &ParticleLevel<'_>, at: [i32; 3]) {
        let Some(registry) = self.polytone.clone() else {
            return;
        };
        let state = level.world.block(at[0], at[1], at[2]);
        let Some(Some(modifiers)) = registry.block_emitters.get(state.0 as usize) else {
            return;
        };
        let modifiers = modifiers.clone();
        let Ok(frame) = registry.frame.read() else {
            return;
        };
        let mut out = Vec::new();
        let (mut custom, mut customs) = (0.0, BTreeMap::new());
        let mut ctx = Ctx {
            subject: Subject::Block(at),
            random: &mut self.random,
            custom: &mut custom,
            customs: &mut customs,
            frame: &frame,
            registry: &registry,
            level: Some(level),
            blocks: Vec::new(),
        };
        let mut sounds = Vec::new();
        for modifier in modifiers.iter() {
            emit(&modifier.particle_emitters, &mut ctx, &mut out);
            emit_sounds(&modifier.sound_emitters, &mut ctx, &mut sounds);
        }
        drop(ctx);
        drop(frame);
        for (options, position, velocity) in out {
            self.add(&options, position, velocity);
        }
        self.sounds.extend(sounds);
    }
}

impl ParticleEngine {
    /// Sets (or clears) the resource packs' Polytone custom particle types.
    pub fn set_polytone(&mut self, registry: Option<Arc<Registry>>) {
        let registry = registry.filter(|r| !r.is_empty());
        let same = match (&self.polytone, &registry) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.polytone_gpu.clear();
            self.polytone_overrides = registry
                .iter()
                .flat_map(|r| {
                    ParticleType::ALL.iter().filter_map(|&ty| {
                        let id = format!("minecraft:{}", ty.name());
                        r.get(&id).is_some().then_some((ty, id))
                    })
                })
                .collect();
        }
        self.polytone = registry;
    }
}

#[cfg(test)]
mod tests;
