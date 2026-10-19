//! YUNG's API's jigsaw assembler, for the structures of the "YUNG's Better"
//! mods carried over from a modpack (docs/modpacks.md). Carry maps
//! `yungsapi:yung_jigsaw` to `minecraft:jigsaw` with a `"horizon:yungsapi"`
//! key, and its pool elements to vanilla's single element with the piece's
//! rules under the same key; vanilla data never has it.
//!
//! The assembler differs from vanilla's JigsawPlacement: it picks the start
//! piece and every child by weight from the pool's weighted entries (a
//! priority piece first), lays pieces out breadth first, caps pieces by name
//! (`max_count`), limits them by depth, lets some ignore the bounds, checks
//! each piece's condition, and keeps every piece between `min_y` and
//! `max_y`. Pieces fit the same free space as vanilla's. A piece that ends
//! up with no child can be swapped for one of a dead-end pool, and once the
//! layout is done each piece's modifiers can turn it into another template
//! or place it last.
use super::{
    BoundingBox, Rotation, StructureResources, StructureTemplate,
    jigsaw::{
        Free, JigsawConfig, JigsawContext, Junction, PoolElement, PoolPiece, Pools, Projection,
        can_attach, shuffle, step,
    },
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{DecodeError, block_feature::FeatureBlockBindings},
};
use horizon_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::Arc,
};

/// A YUNG structure's own settings (`"horizon:yungsapi"` on the structure):
/// the heights every piece must stay within.
#[derive(Debug, Default)]
pub struct Config {
    pub min_y: Option<i32>,
    pub max_y: Option<i32>,
}
impl Config {
    pub fn decode(v: &Value) -> Self {
        let y = |k: &str| v[k].as_i64().map(|n| n as i32);
        Self {
            min_y: y("min_y"),
            max_y: y("max_y"),
        }
    }
}

/// A piece's rules (`"horizon:yungsapi"` on a pool element).
#[derive(Clone, Debug, Default)]
pub struct Rules {
    /// Pieces sharing a name share the cap `max_count`.
    name: Option<String>,
    max_count: Option<i32>,
    min_depth: Option<i32>,
    max_depth: Option<i32>,
    /// Chosen before any other piece of its pool.
    priority: bool,
    /// Placed even where it overlaps other pieces or leaves the bounds.
    ignore_bounds: bool,
    condition: Condition,
    /// The pool a piece that got no child is swapped for.
    deadend_pool: Option<String>,
    /// Applied to the piece once the layout is done.
    modifiers: Vec<Modifier>,
}
impl Rules {
    pub fn decode(v: &Value, resources: &StructureResources) -> Result<Self, DecodeError> {
        let int = |k: &str| v[k].as_i64().map(|n| n as i32);
        Ok(Self {
            name: v["name"].as_str().map(str::to_owned),
            max_count: int("max_count"),
            min_depth: int("min_depth"),
            max_depth: int("max_depth"),
            priority: v["priority"].as_bool().unwrap_or(false),
            ignore_bounds: v["ignore_bounds"].as_bool().unwrap_or(false),
            condition: match v.get("condition") {
                Some(c) => Condition::decode(c, resources)?,
                None => Condition::True,
            },
            deadend_pool: v["deadend_pool"].as_str().map(str::to_owned),
            modifiers: v["modifiers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| Modifier::decode(m, resources))
                .collect::<Result<_, _>>()?,
        })
    }
    /// YungJigsawPoolElement.isAtValidDepth.
    fn at_valid_depth(&self, depth: i32) -> bool {
        self.min_depth.is_none_or(|d| d <= depth) && self.max_depth.is_none_or(|d| d >= depth)
    }
    /// The name a capped piece counts under: `max_count` without a name is
    /// ignored, as YUNG's does.
    fn capped(&self) -> Option<(&str, i32)> {
        Some((self.name.as_deref()?, self.max_count?))
    }
}

/// A StructureModifier: when its condition holds for the piece, its actions
/// apply to the piece (the `self` target, the only one YUNG's API has).
#[derive(Clone, Debug)]
struct Modifier {
    condition: Condition,
    actions: Vec<Action>,
}
impl Modifier {
    fn decode(v: &Value, resources: &StructureResources) -> Result<Self, DecodeError> {
        Ok(Self {
            condition: match v.get("condition") {
                Some(c) => Condition::decode(c, resources)?,
                None => Condition::True,
            },
            actions: v["actions"]
                .as_array()
                .ok_or(DecodeError::Field("modifier actions"))?
                .iter()
                .map(|a| Action::decode(a, resources))
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Clone, Debug)]
enum Action {
    /// TransformAction: the piece becomes one of these templates, moved by
    /// `offset` turned with the piece.
    Transform {
        output: Vec<Arc<StructureTemplate>>,
        offset: [i32; 3],
    },
    /// DelayGenerationAction: the piece is placed after every other.
    DelayGeneration,
}
impl Action {
    fn decode(v: &Value, resources: &StructureResources) -> Result<Self, DecodeError> {
        Ok(match v["type"].as_str().unwrap_or_default() {
            "transform" => {
                let output = v["output"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| {
                        let id = t.as_str().ok_or(DecodeError::Field("transform output"))?;
                        resources
                            .template(id)
                            .ok_or_else(|| DecodeError::Registry(format!("missing template {id}")))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if output.is_empty() {
                    return Err(DecodeError::Field("transform output"));
                }
                let offset = v["offset"].as_array();
                let at = |i: usize| {
                    offset
                        .and_then(|o| o.get(i))
                        .and_then(Value::as_i64)
                        .unwrap_or(0) as i32
                };
                Self::Transform {
                    output,
                    offset: [at(0), at(1), at(2)],
                }
            }
            "delay_generation" => Self::DelayGeneration,
            _ => return Err(DecodeError::Field("YUNG modifier action type")),
        })
    }
}

/// YUNG's structure conditions, as carry writes them.
#[derive(Clone, Debug, Default)]
enum Condition {
    #[default]
    True,
    False,
    Not(Box<Condition>),
    AllOf(Vec<Condition>),
    AnyOf(Vec<Condition>),
    Depth {
        min: Option<i32>,
        max: Option<i32>,
    },
    Altitude {
        bottom: Option<f64>,
        top: Option<f64>,
    },
    RandomChance(f32),
    Rotation(Vec<Rotation>),
    /// Another piece (of these templates, or any) within `range` blocks in
    /// the direction `rotation` turns the piece's own north.
    PieceInDirection {
        templates: Vec<Arc<StructureTemplate>>,
        range: i32,
        rotation: Rotation,
    },
    /// Another piece (of these templates, or any when None) within the
    /// piece's box grown by these ranges, not overlapping the piece.
    PieceInRange {
        templates: Option<Vec<Arc<StructureTemplate>>>,
        horizontal: i32,
        above: i32,
        below: i32,
    },
}
impl Condition {
    fn decode(v: &Value, resources: &StructureResources) -> Result<Self, DecodeError> {
        let list = |k: &'static str| -> Result<Vec<Condition>, DecodeError> {
            v[k].as_array()
                .ok_or(DecodeError::Field(k))?
                .iter()
                .map(|c| Self::decode(c, resources))
                .collect()
        };
        let int = |k: &str| v[k].as_i64().map(|n| n as i32);
        let rotation = |r: &Value| {
            r.as_str()
                .and_then(Rotation::from_name)
                .ok_or(DecodeError::Field("condition rotation"))
        };
        Ok(match v["type"].as_str().unwrap_or_default() {
            "true" => Self::True,
            "false" => Self::False,
            "not" => Self::Not(Box::new(Self::decode(&v["condition"], resources)?)),
            "all_of" => Self::AllOf(list("conditions")?),
            "any_of" => Self::AnyOf(list("conditions")?),
            "depth" => Self::Depth {
                min: int("min_depth"),
                max: int("max_depth"),
            },
            "altitude" => Self::Altitude {
                bottom: v["bottom"].as_f64(),
                top: v["top"].as_f64(),
            },
            "random_chance" => Self::RandomChance(v["chance"].as_f64().unwrap_or(0.) as f32),
            "rotation" => Self::Rotation(
                v["rotations"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(rotation)
                    .collect::<Result<_, _>>()?,
            ),
            "piece_in_horizontal_direction" => Self::PieceInDirection {
                templates: v["pieces"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| resources.template(p.as_str()?))
                    .collect(),
                range: int("range").unwrap_or(0),
                rotation: rotation(&v["rotation"])?,
            },
            "piece_in_range" => {
                let pieces = v["pieces"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let any = pieces.is_empty() || pieces.iter().any(|p| p == "yungsapi:all");
                Self::PieceInRange {
                    // An unknown template matches nothing, as YUNG's API's
                    // empty stand-in does.
                    templates: (!any).then(|| {
                        pieces
                            .iter()
                            .filter_map(|p| resources.template(p.as_str()?))
                            .collect()
                    }),
                    horizontal: int("horizontal_range").unwrap_or(0),
                    above: int("above_range").unwrap_or(0),
                    below: int("below_range").unwrap_or(0),
                }
            }
            _ => return Err(DecodeError::Field("YUNG condition type")),
        })
    }
    /// StructureCondition.passes. `at` is the piece being placed: its box,
    /// rotation and depth; the start piece has no box yet.
    fn passes(&self, at: &At<'_>, random: &mut impl RandomSource) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Not(c) => !c.passes(at, random),
            Self::AllOf(all) => all.iter().all(|c| c.passes(at, random)),
            Self::AnyOf(any) => any.iter().any(|c| c.passes(at, random)),
            Self::Depth { min, max } => {
                min.is_none_or(|d| d <= at.depth) && max.is_none_or(|d| d >= at.depth)
            }
            Self::Altitude { bottom, top } => {
                let [low, high] = at.span;
                bottom.is_none_or(|y| f64::from(low) >= y)
                    && top.is_none_or(|y| f64::from(high) <= y)
            }
            Self::RandomChance(chance) => random.next_float() < *chance,
            Self::Rotation(valid) => valid.contains(&at.rotation),
            Self::PieceInDirection {
                templates,
                range,
                rotation,
            } => {
                let Some(piece) = at.piece else {
                    return false;
                };
                let mut search = piece;
                match at.rotation.get_rotated(*rotation) {
                    Rotation::None => search.min[2] -= range,
                    Rotation::Clockwise90 => search.max[0] += range,
                    Rotation::Clockwise180 => search.max[2] += range,
                    Rotation::Counterclockwise90 => search.min[0] -= range,
                }
                at.near(piece, &search, |template| {
                    templates.is_empty() || templates.iter().any(|t| Arc::ptr_eq(t, template))
                })
            }
            Self::PieceInRange {
                templates,
                horizontal,
                above,
                below,
            } => {
                let Some(piece) = at.piece else {
                    return false;
                };
                let search = BoundingBox::new(
                    [
                        piece.min[0] - horizontal,
                        piece.min[1] - below,
                        piece.min[2] - horizontal,
                    ],
                    [
                        piece.max[0] + horizontal,
                        piece.max[1] + above,
                        piece.max[2] + horizontal,
                    ],
                );
                at.near(piece, &search, |template| {
                    templates
                        .as_ref()
                        .is_none_or(|ts| ts.iter().any(|t| Arc::ptr_eq(t, template)))
                })
            }
        }
    }
}

/// The surface height query, object-safe for the per-call cache.
trait JigsawContextDyn {
    fn surface(&mut self, x: i32, z: i32) -> i32;
}
impl<C: JigsawContext> JigsawContextDyn for C {
    fn surface(&mut self, x: i32, z: i32) -> i32 {
        self.base_height(x, z, HeightmapType::WorldSurfaceWg)
    }
}

/// What a condition sees of the piece being placed (or modified).
struct At<'a> {
    depth: i32,
    rotation: Rotation,
    piece: Option<BoundingBox>,
    /// The piece's lowest and highest Y when the condition was set up.
    span: [i32; 2],
    pieces: &'a [PoolPiece],
    /// Pieces taken out by a dead end, by index; the rest are kept.
    gone: &'a [bool],
}
impl At<'_> {
    /// Whether another single piece whose template matches lies in `search`
    /// without overlapping `piece` (nor being exactly it).
    fn near(
        &self,
        piece: BoundingBox,
        search: &BoundingBox,
        matches: impl Fn(&Arc<StructureTemplate>) -> bool,
    ) -> bool {
        self.pieces.iter().enumerate().any(|(i, other)| {
            let PoolElement::Single { template, .. } = other.element.as_ref() else {
                return false;
            };
            let b = other.bounding_box;
            !self.gone.get(i).copied().unwrap_or(false)
                && b != piece
                && matches(template)
                && b.intersects(search)
                && !b.intersects(&piece)
        })
    }
}

/// The index of the entry to try: the first priority piece, else one drawn
/// by weight (`nextInt(total) + 1`, counted down).
fn pick(
    candidates: &[(Arc<PoolElement>, i32)],
    total: i32,
    random: &mut impl RandomSource,
) -> usize {
    if let Some(i) = candidates
        .iter()
        .position(|(e, _)| e.yung().is_some_and(|r| r.priority))
    {
        return i;
    }
    let mut chosen = random.next_int_bound(total).unwrap_or(0) + 1;
    for (i, (_, weight)) in candidates.iter().enumerate() {
        chosen -= weight;
        if chosen <= 0 {
            return i;
        }
    }
    candidates.len() - 1
}

/// JigsawManager.assembleJigsawStructure: the start position and the
/// pieces, or None when no start piece fits.
#[allow(clippy::too_many_arguments)]
pub fn assemble(
    config: &JigsawConfig<'_>,
    yung: &Config,
    context: &mut impl JigsawContext,
    pools: &Pools,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
    random: &mut impl RandomSource,
    position: [i32; 3],
) -> Option<([i32; 3], Vec<PoolPiece>)> {
    let pool = pools.get(config.start_pool, resources, bindings)?;
    let mut candidates = pool.raw.clone();
    shuffle(&mut candidates, random);
    let rotation = Rotation::random(random);
    let mut total: i32 = candidates.iter().map(|c| c.1).sum();
    let (element, adjusted, local) = loop {
        if candidates.is_empty() || total <= 0 {
            return None;
        }
        let index = pick(&candidates, total, random);
        let (element, weight) = candidates[index].clone();
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
        let at = At {
            depth: 0,
            rotation,
            piece: None,
            span: [0, 0],
            pieces: &[],
            gone: &[],
        };
        if element
            .yung()
            .is_some_and(|r| !r.condition.passes(&at, random))
        {
            total -= weight;
            candidates.remove(index);
            continue;
        }
        break (element, adjusted, local);
    };
    let mut start = PoolPiece {
        bounding_box: element.bounding_box(adjusted, rotation),
        ground_level_delta: element.ground_level_delta(),
        element,
        position: adjusted,
        rotation,
        junctions: Vec::new(),
        waterlogging: config.waterlogging,
    };
    let b = start.bounding_box;
    let (cx, cz) = ((b.max[0] + b.min[0]) / 2, (b.max[2] + b.min[2]) / 2);
    let bottom = match config.project_to {
        Some(kind) => position[1] + context.base_height(cx, cz, kind),
        None => adjusted[1],
    };
    start.moved(bottom - (b.min[1] + start.ground_level_delta));
    let cy = bottom + local[1];
    let d = config.max_distance[0];
    let free = Rc::new(RefCell::new(Free {
        container: BoundingBox::new([cx - d, cy - d, cz - d], [cx + d, cy + d, cz + d]),
        removed: vec![start.bounding_box],
    }));
    let mut assembler = Assembler {
        config,
        yung,
        pools,
        resources,
        bindings,
        pieces: vec![start],
        states: vec![(free, 0)],
        origins: vec![None],
        gone: vec![false],
        counts: HashMap::new(),
    };
    if config.max_depth > 0 {
        let mut queue = VecDeque::from([0]);
        while let Some(piece) = queue.pop_front() {
            assembler.children(piece, &mut queue, context, random);
        }
    }
    Some(([cx, cy, cz], assembler.modified(random)))
}

/// JigsawStructureAssembler.
struct Assembler<'a> {
    config: &'a JigsawConfig<'a>,
    yung: &'a Config,
    pools: &'a Pools,
    resources: &'a StructureResources,
    bindings: &'a FeatureBlockBindings<'a>,
    pieces: Vec<PoolPiece>,
    /// Per piece: the free space it was placed in, and its depth.
    states: Vec<(Rc<RefCell<Free>>, i32)>,
    /// Per piece: where it was placed and the junction it left on its
    /// parent (none for the start piece), for a dead end to undo.
    origins: Vec<Option<(Slot, Junction)>>,
    /// Per piece: taken out by a dead end.
    gone: Vec<bool>,
    /// Pieces placed per capped name.
    counts: HashMap<String, i32>,
}

/// Where a child goes: the parent's jigsaw and what it needs.
#[derive(Clone)]
struct Slot {
    source: usize,
    jigsaw: super::jigsaw::Jigsaw,
    target_pos: [i32; 3],
    free: Rc<RefCell<Free>>,
    depth: i32,
}

impl Assembler<'_> {
    fn pool(&self, name: &str) -> Option<Arc<super::jigsaw::Pool>> {
        let name = self.config.aliases.get(name).map_or(name, String::as_str);
        self.pools.get(name, self.resources, self.bindings)
    }
    /// JigsawStructureAssembler.Settings.isInBounds.
    fn in_bounds(&self, y: i32, context: &impl JigsawContext) -> bool {
        self.yung.max_y.is_none_or(|m| y <= m)
            && self.yung.min_y.is_none_or(|m| y >= m)
            && y >= context.min_y() + self.config.padding[0]
            && y <= context.max_y() + 1 - self.config.padding[1]
    }
    /// addChildrenForPiece: one child per jigsaw where one fits, from the
    /// target pool unless the piece is at the last depth, else its fallback.
    fn children(
        &mut self,
        source: usize,
        queue: &mut VecDeque<usize>,
        context: &mut impl JigsawContext,
        random: &mut impl RandomSource,
    ) {
        let (element, position, rotation, source_box) = {
            let s = &self.pieces[source];
            (s.element.clone(), s.position, s.rotation, s.bounding_box)
        };
        let (context_free, depth) = self.states[source].clone();
        let mut source_free: Option<Rc<RefCell<Free>>> = None;
        let jigsaws = element.shuffled_jigsaws(self.resources, position, rotation, random);
        let count = jigsaws.len();
        let mut any = false;
        for jigsaw in jigsaws {
            let Some(target) = self.pool(&jigsaw.pool) else {
                continue;
            };
            if target.templates.is_empty() && jigsaw.pool != "minecraft:empty" {
                continue;
            }
            let Some(fallback) = self.pool(&target.fallback) else {
                continue;
            };
            if fallback.templates.is_empty() && target.fallback != "minecraft:empty" {
                continue;
            }
            let s = step(jigsaw.front);
            let target_pos: [i32; 3] = std::array::from_fn(|i| jigsaw.pos[i] + s[i]);
            let free = if source_box.is_inside(target_pos) {
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
            let slot = Slot {
                source,
                jigsaw,
                target_pos,
                free,
                depth,
            };
            let placed = depth != self.config.max_depth
                && self.choose(&slot, target.raw.clone(), queue, context, random);
            any |= placed || self.choose(&slot, fallback.raw.clone(), queue, context, random);
        }
        if let Some(pool) = element.yung().and_then(|r| r.deadend_pool.as_deref())
            && !any
            && count > 1
        {
            self.dead_end(source, pool, queue, context, random);
        }
    }
    /// A piece with a dead-end pool that got no child though it has other
    /// jigsaws is taken out, its parent's junction and its room freed and its
    /// count given back, and a piece of that pool is chosen in its place for
    /// the same parent jigsaw. The start piece stays.
    fn dead_end(
        &mut self,
        piece: usize,
        pool: &str,
        queue: &mut VecDeque<usize>,
        context: &mut impl JigsawContext,
        random: &mut impl RandomSource,
    ) {
        let Some(pool) = self.pool(pool) else {
            return;
        };
        let Some((slot, junction)) = self.origins[piece].clone() else {
            return;
        };
        let parent = &mut self.pieces[slot.source].junctions;
        if let Some(i) = parent.iter().position(|j| *j == junction) {
            parent.remove(i);
        }
        let bounds = self.pieces[piece].bounding_box;
        {
            let mut free = slot.free.borrow_mut();
            if let Some(i) = free.removed.iter().position(|b| *b == bounds) {
                free.removed.remove(i);
            }
        }
        self.gone[piece] = true;
        let element = self.pieces[piece].element.clone();
        if let Some((name, _)) = element.yung().and_then(Rules::capped)
            && let Some(count) = self.counts.get_mut(name)
        {
            *count -= 1;
        }
        self.choose(&slot, pool.raw.clone(), queue, context, random);
    }
    /// applyModifications: each piece's modifiers, in piece order, then the
    /// delayed pieces moved last. Returns the pieces kept.
    fn modified(mut self, random: &mut impl RandomSource) -> Vec<PoolPiece> {
        let mut delayed = vec![false; self.pieces.len()];
        for (i, delay) in delayed.iter_mut().enumerate() {
            let element = self.pieces[i].element.clone();
            let Some(rules) = element
                .yung()
                .filter(|r| !self.gone[i] && !r.modifiers.is_empty())
            else {
                continue;
            };
            let b = self.pieces[i].bounding_box;
            let (depth, rotation, span) = (
                self.states[i].1,
                self.pieces[i].rotation,
                [b.min[1], b.max[1]],
            );
            for modifier in &rules.modifiers {
                let at = At {
                    depth,
                    rotation,
                    piece: Some(self.pieces[i].bounding_box),
                    span,
                    pieces: &self.pieces,
                    gone: &self.gone,
                };
                if !modifier.condition.passes(&at, random) {
                    continue;
                }
                for action in &modifier.actions {
                    match action {
                        Action::Transform { output, offset } => {
                            transform(&mut self.pieces[i], output, *offset);
                        }
                        Action::DelayGeneration => *delay = true,
                    }
                }
            }
        }
        let (mut kept, mut last) = (Vec::new(), Vec::new());
        for ((piece, gone), delay) in self.pieces.into_iter().zip(self.gone).zip(delayed) {
            if !gone {
                if delay { &mut last } else { &mut kept }.push(piece);
            }
        }
        kept.extend(last);
        kept
    }
    /// chooseCandidateFromPool: whether a piece was placed.
    fn choose(
        &mut self,
        slot: &Slot,
        mut candidates: Vec<(Arc<PoolElement>, i32)>,
        queue: &mut VecDeque<usize>,
        context: &mut impl JigsawContext,
        random: &mut impl RandomSource,
    ) -> bool {
        let jigsaw = &slot.jigsaw;
        let (source_rigid, source_y, source_ground) = {
            let s = &self.pieces[slot.source];
            (
                s.element.projection() == Projection::Rigid,
                s.bounding_box.min[1],
                s.ground_level_delta,
            )
        };
        let local_y = jigsaw.pos[1] - source_y;
        // The surface at the jigsaw, read once per call.
        let mut surface = None;
        let (x, z) = (jigsaw.pos[0], jigsaw.pos[2]);
        let mut base = |context: &mut dyn JigsawContextDyn| {
            *surface.get_or_insert_with(|| context.surface(x, z))
        };
        shuffle(&mut candidates, random);
        let mut total: i32 = candidates.iter().map(|c| c.1).sum();
        while !candidates.is_empty() && total > 0 {
            let index = pick(&candidates, total, random);
            let (target, weight) = candidates[index].clone();
            if matches!(target.as_ref(), PoolElement::Empty) {
                return false;
            }
            let rules = target.yung();
            let capped = rules.and_then(Rules::capped);
            let full = capped
                .is_some_and(|(name, max)| self.counts.get(name).copied().unwrap_or(0) >= max);
            if full || rules.is_some_and(|r| !r.at_valid_depth(slot.depth)) {
                total -= weight;
                candidates.remove(index);
                continue;
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
                    if !can_attach(jigsaw, target_jigsaw) {
                        continue;
                    }
                    let target_local = target_jigsaw.pos;
                    let raw_position: [i32; 3] =
                        std::array::from_fn(|i| slot.target_pos[i] - target_local[i]);
                    let raw_box = target.bounding_box(raw_position, rotation);
                    let target_rigid = target.projection() == Projection::Rigid;
                    let delta_y = local_y - target_local[1] + step(jigsaw.front)[1];
                    let target_y = if source_rigid && target_rigid {
                        source_y + delta_y
                    } else {
                        base(context) - target_local[1]
                    };
                    let offset = target_y - raw_box.min[1];
                    let mut target_box = raw_box.moved([0, offset, 0]);
                    let target_position =
                        [raw_position[0], raw_position[1] + offset, raw_position[2]];
                    if expand_to > 0 {
                        let size = (expand_to + 1).max(target_box.max[1] - target_box.min[1]);
                        target_box.max[1] = target_box.min[1] + size;
                    }
                    if !self.in_bounds(target_box.max[1], context)
                        || !self.in_bounds(target_box.min[1], context)
                    {
                        continue;
                    }
                    if !rules.is_some_and(|r| r.ignore_bounds)
                        && !slot.free.borrow().fits(&target_box)
                    {
                        continue;
                    }
                    let target_ground = if target_rigid {
                        source_ground - delta_y
                    } else {
                        target.ground_level_delta()
                    };
                    let junction_y = if source_rigid {
                        source_y + local_y
                    } else if target_rigid {
                        target_y + target_local[1]
                    } else {
                        base(context) + delta_y / 2
                    };
                    let at = At {
                        depth: slot.depth + 1,
                        rotation,
                        piece: Some(target_box),
                        span: [target_box.min[1], target_box.max[1]],
                        pieces: &self.pieces,
                        gone: &self.gone,
                    };
                    if rules.is_some_and(|r| !r.condition.passes(&at, random)) {
                        continue;
                    }
                    let on_parent = Junction {
                        source_x: slot.target_pos[0],
                        source_ground_y: junction_y - local_y + source_ground,
                        source_z: slot.target_pos[2],
                    };
                    self.pieces[slot.source].junctions.push(on_parent);
                    slot.free.borrow_mut().removed.push(target_box);
                    self.pieces.push(PoolPiece {
                        element: target.clone(),
                        position: target_position,
                        ground_level_delta: target_ground,
                        rotation,
                        bounding_box: target_box,
                        junctions: vec![Junction {
                            source_x: jigsaw.pos[0],
                            source_ground_y: junction_y - target_local[1] + target_ground,
                            source_z: jigsaw.pos[2],
                        }],
                        waterlogging: self.config.waterlogging,
                    });
                    self.states.push((slot.free.clone(), slot.depth + 1));
                    self.origins.push(Some((slot.clone(), on_parent)));
                    self.gone.push(false);
                    if slot.depth < self.config.max_depth {
                        queue.push_back(self.pieces.len() - 1);
                    }
                    if let Some((name, _)) = capped {
                        *self.counts.entry(name.to_owned()).or_default() += 1;
                    }
                    return true;
                }
            }
            total -= weight;
            candidates.remove(index);
        }
        false
    }
}

/// TransformAction: the piece takes one of `output`'s templates, picked by a
/// random seeded with setFeatureSeed(x, y, x) of its position, and moves by
/// `offset` turned with it. Like YUNG's API's new piece, it loses its
/// junctions and takes its own liquid setting (else waterlogging).
fn transform(piece: &mut PoolPiece, output: &[Arc<StructureTemplate>], offset: [i32; 3]) {
    let [x, y, _] = piece.position;
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_feature_seed(i64::from(x), y, x);
    let chosen = random.next_int_bound(output.len() as i32).unwrap_or(0) as usize;
    let mut element = (*piece.element).clone();
    let PoolElement::Single {
        template,
        waterlogging,
        ..
    } = &mut element
    else {
        return;
    };
    *template = output[chosen].clone();
    piece.waterlogging = waterlogging.unwrap_or(true);
    let [dx, dy, dz] = offset;
    let [dx, dz] = match piece.rotation {
        Rotation::None => [dx, dz],
        Rotation::Clockwise90 => [-dz, dx],
        Rotation::Clockwise180 => [-dx, -dz],
        Rotation::Counterclockwise90 => [dz, -dx],
    };
    piece.position = [x + dx, y + dy, piece.position[2] + dz];
    piece.bounding_box = element.bounding_box(piece.position, piece.rotation);
    piece.element = Arc::new(element);
    piece.junctions.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    fn single(template: &Arc<StructureTemplate>, rules: Option<Rules>) -> Arc<PoolElement> {
        Arc::new(PoolElement::Single {
            template: template.clone(),
            processors: Default::default(),
            legacy: false,
            projection: Projection::Rigid,
            waterlogging: None,
            yung: rules.map(Box::new),
            enhanced_terrain: None,
            ground_level: 1,
        })
    }

    #[test]
    fn picks_priority_pieces_then_by_weight() {
        let t = Arc::new(StructureTemplate::empty());
        let plain = single(&t, None);
        let first = single(
            &t,
            Some(Rules {
                priority: true,
                ..Rules::default()
            }),
        );
        let mut random = LegacyRandom::new(7);
        let list = vec![(plain.clone(), 3), (first, 1)];
        assert_eq!(pick(&list, 4, &mut random), 1);
        // By weight: nextInt(total) + 1, counted down; the first entry
        // takes draws 1..=3 of 4.
        let list = vec![(plain.clone(), 3), (plain, 1)];
        let mut ours = LegacyRandom::new(42);
        let mut theirs = LegacyRandom::new(42);
        for _ in 0..32 {
            let draw = theirs.next_int_bound(4).unwrap() + 1;
            assert_eq!(pick(&list, 4, &mut ours), usize::from(draw > 3));
        }
    }

    #[test]
    fn rules_cap_by_name_and_depth() {
        let rules = Rules {
            name: Some("room".into()),
            max_count: Some(2),
            min_depth: Some(1),
            max_depth: Some(3),
            ..Rules::default()
        };
        assert_eq!(rules.capped(), Some(("room", 2)));
        assert!(!rules.at_valid_depth(0) && rules.at_valid_depth(1) && !rules.at_valid_depth(4));
        // max_count without a name is ignored, as YUNG's does.
        let unnamed = Rules {
            max_count: Some(1),
            ..Rules::default()
        };
        assert_eq!(unnamed.capped(), None);
    }

    #[test]
    fn conditions() {
        let t = Arc::new(StructureTemplate::empty());
        let other = Arc::new(StructureTemplate::empty());
        let neighbour = PoolPiece {
            element: single(&t, None),
            position: [0; 3],
            ground_level_delta: 1,
            rotation: Rotation::None,
            bounding_box: BoundingBox::new([0, 0, -6], [4, 4, -1]),
            junctions: Vec::new(),
            waterlogging: true,
        };
        let pieces = [neighbour];
        let at = |rotation, depth| At {
            depth,
            rotation,
            piece: Some(BoundingBox::new([0, 10, 0], [4, 14, 4])),
            span: [10, 14],
            pieces: &pieces,
            gone: &[],
        };
        let mut random = LegacyRandom::new(1);
        let depth = Condition::Depth {
            min: Some(2),
            max: None,
        };
        assert!(!depth.passes(&at(Rotation::None, 1), &mut random));
        assert!(depth.passes(&at(Rotation::None, 2), &mut random));
        let altitude = |bottom, top| Condition::Altitude { bottom, top };
        assert!(altitude(Some(10.), Some(14.)).passes(&at(Rotation::None, 0), &mut random));
        assert!(!altitude(Some(10.5), None).passes(&at(Rotation::None, 0), &mut random));
        assert!(!altitude(None, Some(13.9)).passes(&at(Rotation::None, 0), &mut random));
        // North (-Z) of an unrotated piece, within 5 blocks; the neighbour
        // stays below it in Y, but only X and Z are searched, and it must
        // not overlap the piece itself.
        let north = |templates, rotation| Condition::PieceInDirection {
            templates,
            range: 5,
            rotation,
        };
        assert!(!north(Vec::new(), Rotation::None).passes(&at(Rotation::None, 1), &mut random));
        let low = At {
            piece: Some(BoundingBox::new([0, 0, 0], [4, 4, 4])),
            span: [0, 4],
            ..at(Rotation::None, 1)
        };
        assert!(north(Vec::new(), Rotation::None).passes(&low, &mut random));
        assert!(north(vec![t.clone()], Rotation::None).passes(&low, &mut random));
        assert!(!north(vec![other], Rotation::None).passes(&low, &mut random));
        // The piece turned 180°: its north is south.
        let turned = At {
            rotation: Rotation::Clockwise180,
            ..low
        };
        assert!(!north(Vec::new(), Rotation::None).passes(&turned, &mut random));
        assert!(north(Vec::new(), Rotation::Clockwise180).passes(&turned, &mut random));
        let all = Condition::AllOf(vec![
            Condition::True,
            Condition::Not(Box::new(Condition::False)),
            Condition::Rotation(vec![Rotation::Clockwise180]),
        ]);
        assert!(all.passes(&turned, &mut random));
        assert!(!Condition::AnyOf(vec![Condition::False]).passes(&turned, &mut random));
    }

    #[test]
    fn piece_in_range_searches_a_grown_box() {
        let t = Arc::new(StructureTemplate::empty());
        let other = Arc::new(StructureTemplate::empty());
        let piece = BoundingBox::new([0, 0, 0], [4, 4, 4]);
        let above = |gone: &'static [bool]| {
            let pieces = vec![PoolPiece {
                element: single(&t, None),
                position: [0, 30, 0],
                ground_level_delta: 1,
                rotation: Rotation::None,
                bounding_box: BoundingBox::new([0, 30, 0], [4, 34, 4]),
                junctions: Vec::new(),
                waterlogging: true,
            }];
            (pieces, gone)
        };
        let range = |templates, above| Condition::PieceInRange {
            templates,
            horizontal: 0,
            above,
            below: 0,
        };
        let mut random = LegacyRandom::new(1);
        let (pieces, gone) = above(&[]);
        let at = At {
            depth: 1,
            rotation: Rotation::None,
            piece: Some(piece),
            span: [0, 4],
            pieces: &pieces,
            gone,
        };
        // 26 blocks above the piece's top: in a 40-block search, not 20.
        assert!(range(None, 40).passes(&at, &mut random));
        assert!(!range(None, 20).passes(&at, &mut random));
        assert!(range(Some(vec![t.clone()]), 40).passes(&at, &mut random));
        assert!(!range(Some(vec![other]), 40).passes(&at, &mut random));
        // An unknown template matches nothing; a piece taken out by a dead
        // end is not seen.
        assert!(!range(Some(Vec::new()), 40).passes(&at, &mut random));
        let (pieces, gone) = above(&[true]);
        let at = At {
            pieces: &pieces,
            gone,
            ..at
        };
        assert!(!range(None, 40).passes(&at, &mut random));
    }

    #[test]
    fn transform_picks_by_position_and_turns_its_offset() {
        let (a, b) = (
            Arc::new(StructureTemplate::empty()),
            Arc::new(StructureTemplate::empty()),
        );
        let rules = Rules {
            name: Some("pillar".into()),
            ..Rules::default()
        };
        let mut piece = PoolPiece {
            element: single(&a, Some(rules)),
            position: [100, 40, -7],
            ground_level_delta: 1,
            rotation: Rotation::Clockwise90,
            bounding_box: BoundingBox::new([100, 40, -7], [104, 44, -3]),
            junctions: vec![Junction {
                source_x: 0,
                source_ground_y: 0,
                source_z: 0,
            }],
            waterlogging: false,
        };
        transform(&mut piece, &[a.clone(), b.clone()], [-1, 2, 0]);
        // setFeatureSeed(x, y, x): seed x + y + 10000 * x, then nextInt(2).
        let mut java = LegacyRandom::new(100 + 40 + 10000 * 100);
        let chosen = [&a, &b][java.next_int_bound(2).unwrap() as usize];
        let PoolElement::Single { template, yung, .. } = piece.element.as_ref() else {
            panic!("not a single element");
        };
        assert!(Arc::ptr_eq(template, chosen));
        assert_eq!(
            yung.as_ref().and_then(|r| r.name.as_deref()),
            Some("pillar")
        );
        // (-1, 2, 0) turned 90° clockwise is (0, 2, -1).
        assert_eq!(piece.position, [100, 42, -8]);
        assert!(piece.junctions.is_empty() && piece.waterlogging);
    }
}
