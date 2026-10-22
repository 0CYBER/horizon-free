//! Template pools, pool elements and JigsawPlacement.
use super::{
    BoundingBox, Mirror, Rotation, StructureResources, StructureTemplate,
    processor::{PlaceSettings, Processor, ProcessorList, place_in_world},
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{
        DecodeError,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
    },
};
use horizon_core::{
    geometry::Direction,
    position::BlockPos,
    random::{LegacyRandom, RandomSource, position_seed},
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, VecDeque},
    rc::Rc,
    sync::{Arc, OnceLock, RwLock},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    Rigid,
    TerrainMatching,
}
#[derive(Clone, Debug)]
pub enum PoolElement {
    Empty,
    Single {
        template: Arc<StructureTemplate>,
        processors: ProcessorList,
        legacy: bool,
        projection: Projection,
        waterlogging: Option<bool>,
        /// YUNG's API's rules on the piece (`"horizon:yungsapi"`), read only
        /// by its assembler ([`super::yung`]).
        yung: Option<Box<super::yung::Rules>>,
        /// The piece's own terrain kernel in a structure that has one
        /// (`"horizon:enhanced_terrain_adaptation"`).
        enhanced_terrain: Option<Arc<super::enhanced_terrain::Adaptation>>,
        /// getGroundLevelDelta: 1, or Formations' `ground_level`
        /// (`"horizon:ground_level"`).
        ground_level: i32,
    },
    Feature {
        feature: String,
        projection: Projection,
    },
    List {
        elements: Vec<PoolElement>,
        projection: Projection,
    },
}
#[derive(Clone, Debug)]
pub struct Jigsaw {
    pub pos: [i32; 3],
    pub front: Direction,
    pub top: Direction,
    pub rollable: bool,
    pub name: Option<String>,
    pub pool: String,
    pub target: String,
    pub placement_priority: i32,
    pub selection_priority: i32,
}
fn direction(name: &str) -> Direction {
    match name {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        _ => Direction::East,
    }
}
fn opposite(d: Direction) -> Direction {
    match d {
        Direction::Down => Direction::Up,
        Direction::Up => Direction::Down,
        Direction::North => Direction::South,
        Direction::South => Direction::North,
        Direction::West => Direction::East,
        Direction::East => Direction::West,
    }
}
pub(super) fn step(d: Direction) -> [i32; 3] {
    let n = d.normal();
    [n[0], n[1], n[2]]
}
/// Util.shuffle.
pub(super) fn shuffle<T>(list: &mut [T], random: &mut impl RandomSource) {
    for i in (2..=list.len()).rev() {
        let swap = random.next_int_bound(i as i32).unwrap_or(0) as usize;
        list.swap(i - 1, swap);
    }
}
fn projection(value: &Value) -> Result<Projection, DecodeError> {
    match value["projection"].as_str() {
        Some("rigid") => Ok(Projection::Rigid),
        Some("terrain_matching") => Ok(Projection::TerrainMatching),
        _ => Err(DecodeError::Field("pool element projection")),
    }
}
impl PoolElement {
    fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
        inherited: Option<Projection>,
    ) -> Result<Self, DecodeError> {
        let kind = value["element_type"].as_str().unwrap_or_default();
        let projection = || match inherited {
            Some(p) => Ok(p),
            None => projection(value),
        };
        Ok(match kind {
            "minecraft:empty_pool_element" => Self::Empty,
            "minecraft:single_pool_element" | "minecraft:legacy_single_pool_element" => {
                let location = value["location"]
                    .as_str()
                    .ok_or(DecodeError::Field("location"))?;
                Self::Single {
                    template: resources.template(location).ok_or_else(|| {
                        DecodeError::Registry(format!("missing template {location}"))
                    })?,
                    processors: ProcessorList::decode(
                        &value["processors"].to_string(),
                        resources,
                        bindings,
                    )?,
                    legacy: kind == "minecraft:legacy_single_pool_element",
                    projection: projection()?,
                    waterlogging: match value
                        .get("override_liquid_settings")
                        .and_then(Value::as_str)
                    {
                        Some("apply_waterlogging") => Some(true),
                        Some("ignore_waterlogging") => Some(false),
                        _ => None,
                    },
                    yung: value
                        .get("horizon:yungsapi")
                        .map(|rules| super::yung::Rules::decode(rules, resources).map(Box::new))
                        .transpose()?,
                    enhanced_terrain: value
                        .get("horizon:enhanced_terrain_adaptation")
                        .map(|a| super::enhanced_terrain::Adaptation::decode(a).map(Arc::new))
                        .transpose()?,
                    ground_level: match value.get("horizon:ground_level") {
                        None => 1,
                        Some(v) => v
                            .as_u64()
                            .and_then(|g| i32::try_from(g).ok())
                            .ok_or(DecodeError::Field("horizon:ground_level"))?,
                    },
                }
            }
            "minecraft:feature_pool_element" => Self::Feature {
                feature: value["feature"].as_str().unwrap_or_default().to_owned(),
                projection: projection()?,
            },
            "minecraft:list_pool_element" => {
                let p = projection()?;
                Self::List {
                    elements: value["elements"]
                        .as_array()
                        .ok_or(DecodeError::Field("list elements"))?
                        .iter()
                        .map(|e| Self::decode(e, resources, bindings, Some(p)))
                        .collect::<Result<_, _>>()?,
                    projection: p,
                }
            }
            _ => return Err(DecodeError::Field("pool element type")),
        })
    }
    pub fn projection(&self) -> Projection {
        match self {
            Self::Empty => Projection::TerrainMatching,
            Self::Single { projection, .. }
            | Self::Feature { projection, .. }
            | Self::List { projection, .. } => *projection,
        }
    }
    pub fn size(&self, rotation: Rotation) -> [i32; 3] {
        match self {
            Self::Empty | Self::Feature { .. } => [0; 3],
            Self::Single { template, .. } => template.size(rotation),
            Self::List { elements, .. } => elements.iter().fold([0; 3], |acc, e| {
                let s = e.size(rotation);
                std::array::from_fn(|i| acc[i].max(s[i]))
            }),
        }
    }
    pub fn bounding_box(&self, position: [i32; 3], rotation: Rotation) -> BoundingBox {
        match self {
            Self::Empty => BoundingBox::new(position, position),
            Self::Feature { .. } => BoundingBox::new(position, position),
            Self::Single { template, .. } => {
                template.bounding_box(position, rotation, [0; 3], Mirror::None)
            }
            Self::List { elements, .. } => {
                let mut boxes = elements
                    .iter()
                    .filter(|e| !matches!(e, Self::Empty))
                    .map(|e| e.bounding_box(position, rotation));
                let mut first = boxes.next().unwrap_or(BoundingBox::new(position, position));
                for b in boxes {
                    first.encapsulate(&b);
                }
                first
            }
        }
    }
    /// getShuffledJigsawBlocks.
    pub fn shuffled_jigsaws(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
        random: &mut impl RandomSource,
    ) -> Vec<Jigsaw> {
        match self {
            Self::Empty => Vec::new(),
            Self::Feature { .. } => vec![Jigsaw {
                pos: position,
                front: Direction::Down,
                top: Direction::South,
                rollable: true,
                name: None,
                pool: "minecraft:empty".into(),
                target: "minecraft:empty".into(),
                placement_priority: 0,
                selection_priority: 0,
            }],
            Self::List { elements, .. } => elements
                .first()
                .map(|e| e.shuffled_jigsaws(resources, position, rotation, random))
                .unwrap_or_default(),
            Self::Single { template, yung, .. } => {
                let mut jigsaws = template_jigsaws(resources, template, position, rotation);
                shuffle(&mut jigsaws, random);
                // YUNG's elements keep the shuffled order.
                if yung.is_none() {
                    jigsaws.sort_by_key(|j| std::cmp::Reverse(j.selection_priority));
                }
                jigsaws
            }
        }
    }
    pub fn ground_level_delta(&self) -> i32 {
        match self {
            Self::Single { ground_level, .. } => *ground_level,
            _ => 1,
        }
    }
    /// YUNG's API's rules, on a piece of one of its structures.
    pub fn yung(&self) -> Option<&super::yung::Rules> {
        match self {
            Self::Single { yung, .. } => yung.as_deref(),
            _ => None,
        }
    }
    /// The piece's own terrain kernel, in place of its structure's.
    pub fn enhanced_terrain(&self) -> Option<&super::enhanced_terrain::Adaptation> {
        match self {
            Self::Single {
                enhanced_terrain, ..
            } => enhanced_terrain.as_deref(),
            _ => None,
        }
    }
}
/// StructureTemplate.getJigsaws: palette chosen by a position-seeded random.
fn template_jigsaws(
    resources: &StructureResources,
    template: &StructureTemplate,
    position: [i32; 3],
    rotation: Rotation,
) -> Vec<Jigsaw> {
    if template.palettes().is_empty() {
        return Vec::new();
    }
    let palette = LegacyRandom::new(position_seed(BlockPos::new(
        position[0],
        position[1],
        position[2],
    )))
    .next_int_bound(template.palettes().len() as i32)
    .unwrap_or(0) as usize;
    template
        .jigsaws(palette, &|state| resources.jigsaw_horizontal(state))
        .iter()
        .map(|j| {
            let relative = StructureTemplate::transform(j.info.pos, Mirror::None, rotation, [0; 3]);
            let state = resources.transforms.rotate(j.info.state, rotation);
            let orientation = resources
                .registry
                .state(state)
                .and_then(|s| s.properties.get("orientation").cloned())
                .unwrap_or_default();
            let (front, top) = orientation.split_once('_').unwrap_or(("north", "up"));
            Jigsaw {
                pos: std::array::from_fn(|i| relative[i] + position[i]),
                front: direction(front),
                top: direction(top),
                rollable: j.rollable,
                name: Some(j.name.clone()),
                pool: j.pool.clone(),
                target: j.target.clone(),
                placement_priority: j.placement_priority,
                selection_priority: j.selection_priority,
            }
        })
        .collect()
}
pub(super) fn can_attach(source: &Jigsaw, target: &Jigsaw) -> bool {
    source.front == opposite(target.front)
        && (source.rollable || source.top == target.top)
        && target
            .name
            .as_ref()
            .is_none_or(|name| *name == source.target)
}

#[derive(Debug)]
pub struct Pool {
    /// Each element repeated by its weight.
    pub(super) templates: Vec<Arc<PoolElement>>,
    /// The elements with their weights, in file order (`rawTemplates`).
    pub(super) raw: Vec<(Arc<PoolElement>, i32)>,
    pub(super) fallback: String,
    max_size: OnceLock<i32>,
}
impl Pool {
    pub(super) fn random_template(&self, random: &mut impl RandomSource) -> Arc<PoolElement> {
        if self.templates.is_empty() {
            return Arc::new(PoolElement::Empty);
        }
        self.templates[random
            .next_int_bound(self.templates.len() as i32)
            .unwrap_or(0) as usize]
            .clone()
    }
    fn shuffled(&self, random: &mut impl RandomSource) -> Vec<Arc<PoolElement>> {
        let mut copy = self.templates.clone();
        shuffle(&mut copy, random);
        copy
    }
    pub(super) fn max_size(&self) -> i32 {
        *self.max_size.get_or_init(|| {
            self.templates
                .iter()
                .filter(|t| !matches!(t.as_ref(), PoolElement::Empty))
                .map(|t| t.bounding_box([0; 3], Rotation::None).span()[1])
                .max()
                .unwrap_or(0)
        })
    }
}
/// Lazily decoded template pools.
pub struct Pools {
    raw: BTreeMap<String, String>,
    decoded: RwLock<HashMap<String, Option<Arc<Pool>>>>,
}
impl std::fmt::Debug for Pools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Pools")
    }
}
impl Pools {
    pub fn new(raw: BTreeMap<String, String>) -> Self {
        Self {
            raw,
            decoded: RwLock::new(HashMap::new()),
        }
    }
    pub fn get(
        &self,
        name: &str,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Option<Arc<Pool>> {
        if let Some(found) = self.decoded.read().unwrap().get(name) {
            return found.clone();
        }
        let pool = self.raw.get(name).and_then(|json| {
            let value: Value = serde_json::from_str(json).ok()?;
            let (mut templates, mut raw) = (Vec::new(), Vec::new());
            for entry in value["elements"].as_array()? {
                let element =
                    match PoolElement::decode(&entry["element"], resources, bindings, None) {
                        Ok(element) => Arc::new(element),
                        Err(error) => {
                            if std::env::var_os("HORIZON_JIGSAW_DEBUG").is_some() {
                                eprintln!("pool {name}: {error}");
                            }
                            return None;
                        }
                    };
                let weight = entry["weight"].as_i64().unwrap_or(1);
                for _ in 0..weight {
                    templates.push(element.clone());
                }
                raw.push((element, weight as i32));
            }
            Some(Arc::new(Pool {
                templates,
                raw,
                fallback: value["fallback"]
                    .as_str()
                    .unwrap_or("minecraft:empty")
                    .to_owned(),
                max_size: OnceLock::new(),
            }))
        });
        self.decoded
            .write()
            .unwrap()
            .insert(name.to_owned(), pool.clone());
        pool
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Junction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
}
#[derive(Debug)]
pub struct PoolPiece {
    pub element: Arc<PoolElement>,
    pub position: [i32; 3],
    pub ground_level_delta: i32,
    pub rotation: Rotation,
    pub bounding_box: BoundingBox,
    pub junctions: Vec<Junction>,
    pub waterlogging: bool,
}
impl PoolPiece {
    pub(super) fn moved(&mut self, dy: i32) {
        self.position[1] += dy;
        self.bounding_box = self.bounding_box.moved([0, dy, 0]);
    }
    /// PoolElementStructurePiece.postProcess.
    #[allow(clippy::too_many_arguments)]
    pub fn place<W: FeatureWorld + PlacementContext, R: RandomSource>(
        &self,
        world: &mut W,
        resources: &StructureResources,
        random: &mut R,
        chunk_box: BoundingBox,
        reference: [i32; 3],
        place_feature: &mut PlaceFeature<'_, W, R>,
    ) -> Result<bool, PlacementQueryError> {
        place_element(
            &self.element,
            world,
            resources,
            random,
            chunk_box,
            self.position,
            reference,
            self.rotation,
            self.waterlogging,
            place_feature,
        )
    }
}
static GRAVITY: OnceLock<Processor> = OnceLock::new();
static IGNORE_STRUCTURE_BLOCK: OnceLock<Processor> = OnceLock::new();
static IGNORE_STRUCTURE_AND_AIR: OnceLock<Processor> = OnceLock::new();
#[allow(clippy::too_many_arguments)]
fn place_element<W: FeatureWorld + PlacementContext, R: RandomSource>(
    element: &PoolElement,
    world: &mut W,
    resources: &StructureResources,
    random: &mut R,
    chunk_box: BoundingBox,
    position: [i32; 3],
    reference: [i32; 3],
    rotation: Rotation,
    waterlogging: bool,
    place_feature: &mut PlaceFeature<'_, W, R>,
) -> Result<bool, PlacementQueryError> {
    match element {
        PoolElement::Empty => Ok(true),
        PoolElement::Feature { feature, .. } => place_feature(world, random, feature, position),
        PoolElement::List { elements, .. } => {
            for e in elements {
                if !place_element(
                    e,
                    world,
                    resources,
                    random,
                    chunk_box,
                    position,
                    reference,
                    rotation,
                    waterlogging,
                    place_feature,
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        PoolElement::Single {
            template,
            processors,
            legacy,
            projection,
            waterlogging: override_liquid,
            ..
        } => {
            let ignore = if *legacy {
                IGNORE_STRUCTURE_AND_AIR.get_or_init(|| {
                    Processor::BlockIgnore(
                        [
                            "minecraft:structure_block".to_owned(),
                            "minecraft:air".to_owned(),
                        ]
                        .into(),
                    )
                })
            } else {
                IGNORE_STRUCTURE_BLOCK.get_or_init(|| {
                    Processor::BlockIgnore(["minecraft:structure_block".to_owned()].into())
                })
            };
            let mut settings = PlaceSettings::new(rotation);
            settings.known_shape = true;
            // SinglePoolElement.getSettings.
            settings.finalize_entities = true;
            settings.bounding_box = Some(chunk_box);
            settings.waterlogging = override_liquid.unwrap_or(waterlogging);
            if !*legacy {
                settings.processors.push(ignore);
            }
            settings.processors.push(&Processor::JigsawReplacement);
            settings.processors.extend(processors.0.iter());
            if *projection == Projection::TerrainMatching {
                settings
                    .processors
                    .push(GRAVITY.get_or_init(|| Processor::Gravity {
                        heightmap: HeightmapType::WorldSurfaceWg,
                        offset: -1,
                    }));
            }
            // LegacySinglePoolElement pops the structure-block ignore and
            // appends STRUCTURE_AND_AIR last, after jigsaw replacement.
            if *legacy {
                settings.processors.push(ignore);
            }
            place_in_world(
                world,
                resources,
                template,
                position,
                reference,
                &mut settings,
                Some(random),
                18,
            )
        }
    }
}

/// Places a named placed feature (feature pool elements) with the piece random.
pub type PlaceFeature<'a, W, R> =
    dyn FnMut(&mut W, &mut R, &str, [i32; 3]) -> Result<bool, PlacementQueryError> + 'a;

/// The world queries jigsaw assembly needs from the chunk generator.
pub trait JigsawContext {
    fn min_y(&self) -> i32;
    fn max_y(&self) -> i32;
    /// ChunkGenerator.getBaseHeight (getFirstFreeHeight).
    fn base_height(&mut self, x: i32, z: i32, kind: HeightmapType) -> i32;
    /// couldStructureExistInColumn over inclusive block Y bounds.
    fn column_has_valid_biome(&mut self, x: i32, z: i32, min_y: i32, max_y: i32) -> bool;
}
pub struct JigsawConfig<'a> {
    pub start_pool: &'a str,
    pub start_jigsaw: Option<&'a str>,
    pub max_depth: i32,
    pub use_expansion_hack: bool,
    pub project_to: Option<HeightmapType>,
    pub max_distance: [i32; 2],
    pub aliases: &'a BTreeMap<String, String>,
    pub padding: [i32; 2],
    pub waterlogging: bool,
}

/// Free space for children: an inclusive container box minus placed boxes.
#[derive(Debug)]
pub(super) struct Free {
    pub(super) container: BoundingBox,
    pub(super) removed: Vec<BoundingBox>,
}
impl Free {
    /// Whether every point of the (0.25-deflated) box lies in free space.
    pub(super) fn fits(&self, b: &BoundingBox) -> bool {
        (0..3).all(|i| b.min[i] >= self.container.min[i] && b.max[i] <= self.container.max[i])
            && !self.removed.iter().any(|r| r.intersects(b))
    }
}
struct PieceState {
    piece: usize,
    free: Rc<RefCell<Free>>,
    depth: i32,
}

/// JigsawPlacement.addPieces. Returns the structure start position and the
/// pieces, or None when the start is invalid.
pub fn assemble(
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    position: [i32; 3],
) -> Option<([i32; 3], Vec<PoolPiece>)> {
    let alias = |name: &str| {
        config
            .aliases
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_owned())
    };
    let rotation = Rotation::random(random);
    let center_pool = pools.get(&alias(config.start_pool), resources, bindings)?;
    let element = center_pool.random_template(random);
    if matches!(element.as_ref(), PoolElement::Empty) {
        return None;
    }
    let anchored = match config.start_jigsaw {
        Some(name) => {
            element
                .shuffled_jigsaws(resources, position, rotation, random)
                .into_iter()
                .find(|j| j.name.as_deref() == Some(name))?
                .pos
        }
        None => position,
    };
    let local: [i32; 3] = std::array::from_fn(|i| anchored[i] - position[i]);
    let adjusted: [i32; 3] = std::array::from_fn(|i| position[i] - local[i]);
    let mut center = PoolPiece {
        bounding_box: element.bounding_box(adjusted, rotation),
        ground_level_delta: element.ground_level_delta(),
        element: element.clone(),
        position: adjusted,
        rotation,
        junctions: Vec::new(),
        waterlogging: config.waterlogging,
    };
    let b = center.bounding_box;
    let cx = (b.max[0] + b.min[0]) / 2;
    let cz = (b.max[2] + b.min[2]) / 2;
    let bottom = match config.project_to {
        None => adjusted[1],
        Some(kind) => {
            if !context.column_has_valid_biome(cx, cz, context.min_y(), context.max_y()) {
                return None;
            }
            position[1] + context.base_height(cx, cz, kind)
        }
    };
    let old_ground = b.min[1] + center.ground_level_delta;
    center.moved(bottom - old_ground);
    let b = center.bounding_box;
    if config.padding != [0, 0]
        && (b.min[1] < context.min_y() + config.padding[0]
            || b.max[1] > context.max_y() - config.padding[1])
    {
        return None;
    }
    let cy = bottom + local[1];
    let mut pieces = vec![center];
    if config.max_depth > 0 {
        let [h, v] = config.max_distance;
        let low_y = (cy - v).max(context.min_y() + config.padding[0]);
        let high_y = (cy + v + 1).min(context.max_y() + 1 - config.padding[1]);
        let container = BoundingBox::new([cx - h, low_y, cz - h], [cx + h, high_y - 1, cz + h]);
        add_children(
            config,
            context,
            pools,
            resources,
            bindings,
            random,
            &mut pieces,
            container,
        );
    }
    Some(([cx, cy, cz], pieces))
}

/// The inner JigsawPlacement.addPieces: lays out the children of
/// `pieces[0]`, a start piece already placed, to `config.max_depth`,
/// within `container` (inclusive) less the start's box. A structure with
/// its own start (Formations' pieced one) reaches the jigsaw layout here.
#[allow(clippy::too_many_arguments)]
pub(super) fn add_children(
    config: &JigsawConfig<'_>,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    pieces: &mut Vec<PoolPiece>,
    container: BoundingBox,
) {
    let free = Rc::new(RefCell::new(Free {
        container,
        removed: vec![pieces[0].bounding_box],
    }));
    let mut placer = Placer {
        config,
        pools,
        resources,
        bindings,
        pieces,
        queue: BTreeMap::new(),
    };
    placer.children(0, free, 0, context, random);
    while let Some(state) = placer.next() {
        placer.children(state.piece, state.free, state.depth, context, random);
    }
}
struct Placer<'a, 'p> {
    config: &'a JigsawConfig<'a>,
    pools: &'a Pools,
    resources: &'a StructureResources,
    bindings: &'a FeatureBlockBindings<'a>,
    pieces: &'p mut Vec<PoolPiece>,
    /// SequencedPriorityIterator: highest priority first, FIFO within one.
    queue: BTreeMap<i32, VecDeque<PieceState>>,
}
impl Placer<'_, '_> {
    fn next(&mut self) -> Option<PieceState> {
        let mut entry = self.queue.last_entry()?;
        let state = entry.get_mut().pop_front();
        if entry.get().is_empty() {
            entry.remove();
        }
        state
    }
    fn pool(&self, name: &str) -> Option<Arc<Pool>> {
        let name = self.config.aliases.get(name).map_or(name, String::as_str);
        self.pools.get(name, self.resources, self.bindings)
    }
    fn children(
        &mut self,
        source_index: usize,
        context_free: Rc<RefCell<Free>>,
        depth: i32,
        context: &mut impl JigsawContext,
        random: &mut impl RandomSource,
    ) {
        let (source_element, source_position, source_rotation, source_box, source_ground) = {
            let s = &self.pieces[source_index];
            (
                s.element.clone(),
                s.position,
                s.rotation,
                s.bounding_box,
                s.ground_level_delta,
            )
        };
        let source_rigid = source_element.projection() == Projection::Rigid;
        let mut source_free: Option<Rc<RefCell<Free>>> = None;
        let source_y = source_box.min[1];
        let jigsaws = source_element.shuffled_jigsaws(
            self.resources,
            source_position,
            source_rotation,
            random,
        );
        'jigsaws: for jigsaw in jigsaws {
            let direction = jigsaw.front;
            let s = step(direction);
            let target_pos: [i32; 3] = std::array::from_fn(|i| jigsaw.pos[i] + s[i]);
            let local_y = jigsaw.pos[1] - source_y;
            let mut base_height = i32::MIN;
            let Some(target_pool) = self.pool(&jigsaw.pool) else {
                continue;
            };
            if target_pool.templates.is_empty() && jigsaw.pool != "minecraft:empty" {
                continue;
            }
            let Some(fallback) = self.pool(&target_pool.fallback) else {
                continue;
            };
            if fallback.templates.is_empty() && target_pool.fallback != "minecraft:empty" {
                continue;
            }
            let children_free = if source_box.is_inside(target_pos) {
                source_free
                    .get_or_insert_with(|| {
                        Rc::new(RefCell::new(Free {
                            container: source_box,
                            removed: Vec::new(),
                        }))
                    })
                    .clone()
            } else {
                context_free.clone()
            };
            let mut targets = Vec::new();
            if depth != self.config.max_depth {
                targets.extend(target_pool.shuffled(random));
            }
            targets.extend(fallback.shuffled(random));
            for target in targets {
                if matches!(target.as_ref(), PoolElement::Empty) {
                    break;
                }
                let mut rotations = Rotation::ALL;
                shuffle(&mut rotations, random);
                for rotation in rotations {
                    let target_jigsaws =
                        target.shuffled_jigsaws(self.resources, [0; 3], rotation, random);
                    let hack_box = target.bounding_box([0; 3], rotation);
                    let expand_to = if self.config.use_expansion_hack && hack_box.span()[1] <= 16 {
                        target_jigsaws
                            .iter()
                            .map(|tj| {
                                let st = step(tj.front);
                                if !hack_box.is_inside(std::array::from_fn(|i| tj.pos[i] + st[i])) {
                                    return 0;
                                }
                                let child = self.pool(&tj.pool);
                                let child_size = child.as_ref().map_or(0, |p| p.max_size());
                                let fallback_size = child
                                    .and_then(|p| self.pool(&p.fallback))
                                    .map_or(0, |p| p.max_size());
                                child_size.max(fallback_size)
                            })
                            .max()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for target_jigsaw in &target_jigsaws {
                        if !can_attach(&jigsaw, target_jigsaw) {
                            continue;
                        }
                        let target_local = target_jigsaw.pos;
                        let raw_position: [i32; 3] =
                            std::array::from_fn(|i| target_pos[i] - target_local[i]);
                        let raw_box = target.bounding_box(raw_position, rotation);
                        let raw_y = raw_box.min[1];
                        let target_rigid = target.projection() == Projection::Rigid;
                        let target_local_y = target_local[1];
                        let delta_y = local_y - target_local_y + step(jigsaw.front)[1];
                        let target_y = if source_rigid && target_rigid {
                            source_y + delta_y
                        } else {
                            if base_height == i32::MIN {
                                base_height = context.base_height(
                                    jigsaw.pos[0],
                                    jigsaw.pos[2],
                                    HeightmapType::WorldSurfaceWg,
                                );
                            }
                            base_height - target_local_y
                        };
                        let offset = target_y - raw_y;
                        let mut target_box = raw_box.moved([0, offset, 0]);
                        let target_position =
                            [raw_position[0], raw_position[1] + offset, raw_position[2]];
                        if expand_to > 0 {
                            let size = (expand_to + 1).max(target_box.max[1] - target_box.min[1]);
                            target_box.max[1] = target_box.max[1].max(target_box.min[1] + size);
                        }
                        if !children_free.borrow().fits(&target_box) {
                            continue;
                        }
                        children_free.borrow_mut().removed.push(target_box);
                        let target_ground = if target_rigid {
                            source_ground - delta_y
                        } else {
                            target.ground_level_delta()
                        };
                        let junction_y = if source_rigid {
                            source_y + local_y
                        } else if target_rigid {
                            target_y + target_local_y
                        } else {
                            if base_height == i32::MIN {
                                base_height = context.base_height(
                                    jigsaw.pos[0],
                                    jigsaw.pos[2],
                                    HeightmapType::WorldSurfaceWg,
                                );
                            }
                            base_height + delta_y / 2
                        };
                        self.pieces[source_index].junctions.push(Junction {
                            source_x: target_pos[0],
                            source_ground_y: junction_y - local_y + source_ground,
                            source_z: target_pos[2],
                        });
                        self.pieces.push(PoolPiece {
                            element: target.clone(),
                            position: target_position,
                            ground_level_delta: target_ground,
                            rotation,
                            bounding_box: target_box,
                            junctions: vec![Junction {
                                source_x: jigsaw.pos[0],
                                source_ground_y: junction_y - target_local_y + target_ground,
                                source_z: jigsaw.pos[2],
                            }],
                            waterlogging: self.config.waterlogging,
                        });
                        if depth < self.config.max_depth {
                            self.queue
                                .entry(jigsaw.placement_priority)
                                .or_default()
                                .push_back(PieceState {
                                    piece: self.pieces.len() - 1,
                                    free: children_free.clone(),
                                    depth: depth + 1,
                                });
                        }
                        continue 'jigsaws;
                    }
                }
            }
        }
    }
}
