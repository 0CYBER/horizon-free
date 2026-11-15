//! Block tint sources native mods register (`register_block_tint_sources`, vanilla's
//! `BlockColors.register`): resolved per state when the mesh tables build, so a block without a
//! mod's own source meshes as vanilla's blocks do, and one with it asks the mod once per block and
//! tint index, as `ModelBlockRenderer.computeTintColor` caches it.
use super::{MeshTables, Resolver, Tint, Tints, lighting};
use crate::{biome::BiomeInfo, chunk::Chunk};
use horizon_assets::model::Direction as D;
use horizon_core::geometry::Direction;
use horizon_world::{fluid::FluidState, registry::BlockRegistry};
use mod_api::{
    BiomeColorResolver, BiomeColors, BlockAndTintGetter, BlockTintSource, ColorResolver,
    LightLayer, ResolverBiome, TintLayer, VanillaTint,
};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc, sync::OnceLock};

/// The registrations, by block id: set once at start, read by every table build.
static REGISTERED: OnceLock<BTreeMap<String, Arc<[TintLayer]>>> = OnceLock::new();

/// Sets the native mods' block tint sources, once, before the resources first load; each later
/// [`MeshTables::new`] applies them. False when already set.
pub fn register_block_tint_sources(layers: BTreeMap<String, Arc<[TintLayer]>>) -> bool {
    REGISTERED.set(layers).is_ok()
}

/// The mods' colour resolvers, by registration index: set once at start, read by every mesh.
struct RegisteredResolver {
    callback: Arc<dyn BiomeColorResolver>,
    warned: std::sync::atomic::AtomicBool,
}
static RESOLVERS: OnceLock<Box<[RegisteredResolver]>> = OnceLock::new();

/// Sets the native mods' colour resolvers (`register_color_resolvers`), once, before the block
/// tint sources that read them. False when already set.
pub fn register_color_resolvers(resolvers: Arc<[Arc<dyn BiomeColorResolver>]>) -> bool {
    RESOLVERS
        .set(
            resolvers
                .iter()
                .map(|callback| RegisteredResolver {
                    callback: callback.clone(),
                    warned: std::sync::atomic::AtomicBool::new(false),
                })
                .collect(),
        )
        .is_ok()
}

pub(super) fn resolver_exists(index: u32) -> bool {
    RESOLVERS.get().is_some_and(|r| (index as usize) < r.len())
}

/// Mod resolver `index`'s colour of `biome` at block column `(x, z)`, 0xRRGGBB: vanilla's
/// `ColorResolver.getColor`. White for an index no resolver has.
pub(super) fn resolve(index: u32, biome: &BiomeInfo, x: f64, z: f64) -> u32 {
    let Some(resolver) = RESOLVERS.get().and_then(|r| r.get(index as usize)) else {
        return 0xFF_FFFF;
    };
    // A failed resolver stays disabled, avoiding repeated panic-hook output as
    // subsequent columns and sections are meshed.
    if resolver.warned.load(std::sync::atomic::Ordering::Relaxed) {
        return 0xFF_FFFF;
    }
    let info = ResolverBiome {
        temperature: biome.temperature,
        downfall: biome.downfall,
        colors: BiomeColors {
            grass: biome.grass,
            foliage: biome.foliage,
            dry_foliage: biome.dry_foliage,
            water: biome.water,
        },
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resolver.callback.color(&biome.name, &info, x, z)
    })) {
        Ok(color) => color & 0xFF_FFFF,
        Err(_) => {
            if !resolver
                .warned
                .swap(true, std::sync::atomic::Ordering::Relaxed)
            {
                log::warn!("Horizon: colour resolver {index} panicked; using white");
            }
            0xFF_FFFF
        }
    }
}

/// What [`register_block_tint_sources`] set; none before.
pub(super) fn registered() -> Option<&'static BTreeMap<String, Arc<[TintLayer]>>> {
    REGISTERED.get()
}

/// Whether game-prepared values are needed for a mod's own tint callback.
pub fn has_custom_block_tint_sources() -> bool {
    REGISTERED.get().is_some_and(has_custom_sources)
}

fn has_custom_sources(blocks: &BTreeMap<String, Arc<[TintLayer]>>) -> bool {
    blocks
        .values()
        .flat_map(|layers| layers.iter())
        .any(|layer| matches!(layer, TintLayer::Source(_)))
}

/// One tint layer of one state.
#[derive(Clone)]
pub(super) enum ModLayer {
    /// A vanilla source the mesher evaluates as for its own blocks.
    Tint(Tint),
    /// `BlockTintSources.constant(colorInHand, colorInWorld)`.
    Split { hand: u32, world: u32 },
    /// `BlockTintSources.waterParticles()`: untinted, but water-coloured particles.
    WaterParticles,
    /// A mod's own source.
    Source(Arc<dyn BlockTintSource>),
}

impl ModLayer {
    /// `layer` for a state whose property values `value` reads.
    fn of<'a>(layer: &TintLayer, value: impl Fn(&str) -> Option<&'a str>) -> Self {
        let number = |name| value(name).and_then(|v| v.parse().ok()).unwrap_or(0);
        match layer {
            TintLayer::Source(source) => Self::Source(source.clone()),
            TintLayer::Vanilla(tint) => match *tint {
                VanillaTint::Constant(c) => Self::Tint(Tint::Constant(c & 0xFF_FFFF)),
                VanillaTint::ConstantSplit { hand, world } => Self::Split {
                    hand: hand & 0xFF_FFFF,
                    world: world & 0xFF_FFFF,
                },
                VanillaTint::Grass => Self::Tint(Tint::Grass),
                VanillaTint::GrassBlock => Self::Tint(Tint::GrassBlock),
                VanillaTint::DoubleTallGrass if value("half") == Some("upper") => {
                    Self::Tint(Tint::TallGrassUpper)
                }
                VanillaTint::DoubleTallGrass => Self::Tint(Tint::Grass),
                VanillaTint::SugarCane => Self::Tint(Tint::SugarCane),
                VanillaTint::Foliage => Self::Tint(Tint::Foliage),
                VanillaTint::DryFoliage => Self::Tint(Tint::DryFoliage),
                VanillaTint::Water => Self::Tint(Tint::Water),
                VanillaTint::WaterParticles => Self::WaterParticles,
                VanillaTint::Redstone => Self::Tint(Tint::Redstone(number("power"))),
                VanillaTint::Stem => Self::Tint(Tint::Stem(number("age"))),
            },
        }
    }
}

/// The states mods colour: their layers by slot ([`Tint::Mod`]), and every state as a mod reads
/// it, kept while a mod's own source is registered.
#[derive(Default)]
pub(super) struct ModTints {
    pub(super) layers: Vec<Box<[ModLayer]>>,
    pub(super) values: Option<ModTintValues>,
}

/// API values prepared by the owning game before mesh tables are built.
pub struct ModTintValues {
    /// One value for each engine state slot; conversion belongs to the game boundary.
    pub states: Box<[mod_api::BlockState]>,
    /// The immutable definition resource shared by all meshing workers.
    pub metadata: &'static (dyn mod_api::BlockMetadata + Send + Sync),
}
impl ModTints {
    pub(super) fn values(&self) -> &ModTintValues {
        self.values
            .as_ref()
            .expect("source metadata was checked when layers were installed")
    }
}

impl MeshTables {
    /// Gives the states of each block in `by_block` their layers, over vanilla's: a state's
    /// faces with `tintindex` i take layer i. A block no state has is logged and skipped.
    pub fn apply_tint_layers(
        &mut self,
        registry: &BlockRegistry,
        by_block: &BTreeMap<String, Arc<[TintLayer]>>,
        values: Option<ModTintValues>,
    ) -> crate::Result<()> {
        let sources = has_custom_sources(by_block);
        // Section geometry reads the same values through its `level`.
        let values =
            if sources || !self.section_geometry.is_empty() {
                Some(values.ok_or(
                    "a block tint source or section geometry needs game-prepared metadata",
                )?)
            } else {
                None
            };
        let mut mod_tints = ModTints {
            values,
            ..Default::default()
        };
        let mut found = std::collections::BTreeSet::new();
        for (id, info) in self.info.iter_mut().enumerate() {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(id as u32))
            else {
                continue;
            };
            let Some(layers) = by_block.get(&state.name) else {
                continue;
            };
            found.insert(state.name.as_str());
            let value = |name: &str| state.properties.get(name).map(String::as_str);
            let slot = mod_tints.layers.len() as u32;
            mod_tints
                .layers
                .push(layers.iter().map(|l| ModLayer::of(l, value)).collect());
            info.tints = [Tint::Mod(slot); 2];
            info.tint_count = layers.len().min(usize::from(u8::MAX)) as u8;
        }
        for block in by_block.keys().filter(|b| !found.contains(b.as_str())) {
            log::warn!("Horizon: a block tint source names {block}, which no block is");
        }
        self.mod_tints = mod_tints;
        Ok(())
    }
}

/// Asks a mod's `source` for tint `index` of the block of state `id` at section-local `pos` of
/// `tints`' section: its `colorInWorld`, once per block, state and index while the section meshes,
/// or with `particle` its `colorAsTerrainParticle`.
pub(super) fn ask(
    tables: &MeshTables,
    source: &dyn BlockTintSource,
    (id, index): (u32, i32),
    tints: &mut Tints,
    pos @ [x, y, z]: [i32; 3],
    nether: bool,
    particle: bool,
) -> u32 {
    let block = Some((pos, id));
    if !particle
        && tints.asked == block
        && let Some(&(_, color)) = tints.asked_colors.iter().find(|(i, _)| *i == index)
    {
        return color;
    }
    let [cx, cz, base_y] = tints.origin;
    let world = [cx * 16 + x, base_y + y, cz * 16 + z];
    let values = tables.mod_tints.values();
    let state = &values.states[id as usize];
    let color = {
        let region = Region {
            tables,
            tints: RefCell::new(&mut *tints),
            nether,
        };
        if particle {
            source.color_as_terrain_particle(state, values.metadata, &region, world)
        } else {
            source.color_in_world(state, values.metadata, &region, world)
        }
    } & 0xFF_FFFF;
    if !particle {
        if tints.asked != block {
            tints.asked = block;
            tints.asked_colors.clear();
        }
        tints.asked_colors.push((index, color));
    }
    color
}

/// Vanilla's `BlockAndTintGetter` over the region a section is meshed from: the chunk of
/// `tints`' origin and its eight neighbours, its biome tints cached in `tints`.
pub(super) struct Region<'r, 't, 'c> {
    pub(super) tables: &'r MeshTables,
    pub(super) tints: RefCell<&'t mut Tints<'c>>,
    /// `CardinalLighting.NETHER`.
    pub(super) nether: bool,
}

impl Region<'_, '_, '_> {
    /// The loaded chunk holding `pos`, if the region has it.
    fn chunk(&self, [x, _, z]: [i32; 3]) -> Option<&Chunk> {
        let tints = self.tints.borrow();
        let [cx, cz, _] = tints.origin;
        let (rx, rz) = ((x >> 4) - cx + 1, (z >> 4) - cz + 1);
        if (0..3).contains(&rx) && (0..3).contains(&rz) {
            tints.region[rz as usize][rx as usize]
        } else {
            None
        }
    }
    /// The stored light at `pos`, `sky << 4 | block`: full sky and no block light where the
    /// region has no chunk, as an unloaded chunk reads.
    fn light(&self, pos @ [x, y, z]: [i32; 3]) -> u8 {
        self.chunk(pos).map_or(0xF0, |c| c.light(x & 15, y, z & 15))
    }
    fn state_id(&self, pos @ [x, y, z]: [i32; 3]) -> u32 {
        self.chunk(pos).map_or(0, |c| c.get(x & 15, y, z & 15))
    }
}

impl BlockAndTintGetter for Region<'_, '_, '_> {
    fn block_state(&self, pos: [i32; 3]) -> mod_api::BlockState {
        let states = &self.tables.mod_tints.values().states;
        *states
            .get(self.state_id(pos) as usize)
            .unwrap_or(&states[0])
    }
    fn fluid_state(&self, pos: [i32; 3]) -> Option<mod_api::FluidState> {
        let id = self
            .chunk(pos)
            .map(|c| c.get(pos[0] & 15, pos[1], pos[2] & 15))?;
        let state = self
            .tables
            .info
            .get(id as usize)
            .map_or(FluidState::EMPTY, |i| i.fluid);
        let key = match state.kind().vanilla_index() {
            Some(index) => mod_api::__private::world_fluid_key(usize::from(index))?,
            None => mod_api::__private::fluid_key(state.kind().name())?,
        };
        Some(mod_api::__private::fluid_state(
            key,
            state.amount(),
            state.falling(),
            state.is_source(),
        ))
    }
    fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> i32 {
        let light = self.light(pos);
        i32::from(match layer {
            LightLayer::Block => light & 15,
            LightLayer::Sky => light >> 4,
        })
    }
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32 {
        let light = self.light(pos);
        (i32::from(light >> 4) - sky_darken).max(i32::from(light & 15))
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> bool {
        self.light(pos) >> 4 >= 15
    }
    fn shade(&self, face: Direction) -> f32 {
        let face = match face {
            Direction::Down => D::Down,
            Direction::Up => D::Up,
            Direction::North => D::North,
            Direction::South => D::South,
            Direction::West => D::West,
            Direction::East => D::East,
        };
        lighting::cardinal(self.nether, face)
    }
    fn block_tint(&self, [x, y, z]: [i32; 3], resolver: ColorResolver) -> u32 {
        let resolver = match resolver {
            ColorResolver::Grass => Resolver::Grass,
            ColorResolver::Foliage => Resolver::Foliage,
            ColorResolver::DryFoliage => Resolver::Dry,
            ColorResolver::Water => Resolver::Water,
            ColorResolver::Mod(resolver) => {
                Resolver::Mod(mod_api::__private::mod_color_resolver_index(resolver))
            }
        };
        let mut tints = self.tints.borrow_mut();
        let [cx, cz, base_y] = tints.origin;
        let section = [x >> 4, (y - base_y) >> 4, z >> 4];
        if section == [cx, 0, cz] {
            return 0xFF00_0000 | tints.get(resolver, x & 15, y - base_y, z & 15);
        }
        // Another section: the same blend over a region centred on its chunk, plains where
        // this region has no chunk.
        let (dx, dz) = (section[0] - cx, section[2] - cz);
        let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|j| {
            std::array::from_fn(|i| {
                let (rx, rz) = (dx + i as i32, dz + j as i32);
                if (0..3).contains(&rx) && (0..3).contains(&rz) {
                    tints.region[rz as usize][rx as usize]
                } else {
                    None
                }
            })
        });
        let rgb = Tints::new(
            &region,
            tints.biomes,
            tints.zoom,
            tints.radius,
            [section[0], section[2], y & !15],
        )
        .get(resolver, x & 15, y & 15, z & 15);
        0xFF00_0000 | rgb
    }
    fn min_y(&self) -> i32 {
        self.tints.borrow().region[1][1].map_or(crate::chunk::MIN_Y, |c| c.min_y())
    }
    fn height(&self) -> i32 {
        self.tints.borrow().region[1][1].map_or(0, |c| c.layout().height())
    }
}
