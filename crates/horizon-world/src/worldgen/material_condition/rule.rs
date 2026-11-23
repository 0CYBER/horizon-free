//! Compiled material rule composition, sharing the condition worker's context.
use super::{
    CachedNoise, Compiled, Context, Facts, MaterialCondition, MaterialConditions,
    MaterialSurfaceQueries, PossibleBiomes, density::MaterialDensitySources,
};
use crate::{registry::BlockStateId, worldgen::DecodeError};
use horizon_core::{
    density::DensitySampler,
    noise::Noise,
    position::BlockPos,
    random::{PositionalRandom, RandomSource},
};
use std::{borrow::Cow, sync::Arc};

#[derive(Clone, Debug)]
pub enum MaterialRule {
    Block(BlockStateId),
    Reference(Arc<MaterialRule>),
    /// Density roots must be compiled for the same world and sampler context.
    OreVein {
        ore: BlockStateId,
        raw_ore: BlockStateId,
        filler: BlockStateId,
        raw_ore_chance: f32,
        density: Arc<dyn DensitySampler>,
        richness: Arc<dyn DensitySampler>,
        filler_gap: Arc<dyn DensitySampler>,
        /// Inclusive y bounds outside which `density` is never positive, so
        /// the vein places nothing there (`DensityGraph::positive_y_bounds`);
        /// `[i32::MIN, i32::MAX]` when unknown.
        positive_y: [i32; 2],
    },
    Sequence(Vec<MaterialRule>),
    Condition {
        if_true: MaterialCondition,
        then_run: Box<MaterialRule>,
    },
    /// Vanilla calls this rule "bandlands"; it queries terracotta bands.
    Bandlands,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialRuleError {
    UnknownRule,
    UnknownCondition,
    MissingBiomeProvider,
    BiomeQueryFailed,
    MissingBiome,
    CachedConditionUnavailable,
    MissingBandProvider,
    MissingDensityProvider,
    Density(horizon_core::density::SampleError),
    InvalidBandIndex,
}
impl std::fmt::Display for MaterialRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Self::Density(error) = self {
            return error.fmt(f);
        }
        f.write_str(match self {
            Self::Density(_) => unreachable!(),
            Self::MissingDensityProvider => "missing material density provider",
            Self::UnknownRule => "unknown material rule",
            Self::UnknownCondition => "unknown material condition",
            Self::MissingBiomeProvider => "missing material biome provider",
            Self::BiomeQueryFailed => "material biome lookup failed",
            Self::MissingBiome => "material position has no biome",
            Self::CachedConditionUnavailable => "condition was updated without a cached result",
            Self::MissingBandProvider => "missing material band provider",
            Self::InvalidBandIndex => "negative material band index",
        })
    }
}
impl std::error::Error for MaterialRuleError {}
impl From<horizon_core::density::SampleError> for MaterialRuleError {
    fn from(error: horizon_core::density::SampleError) -> Self {
        Self::Density(error)
    }
}

pub(super) enum CompiledRule<R: RandomSource> {
    Empty,
    Block(BlockStateId),
    OreVein {
        blocks: [BlockStateId; 3],
        raw_ore_chance: f32,
        densities: [usize; 3],
        random: R::Positional,
        positive_y: [i32; 2],
    },
    Sequence(Vec<Self>),
    Condition {
        condition: Compiled<R>,
        rule: Box<Self>,
    },
    Bandlands,
}
impl<R: RandomSource> CompiledRule<R> {
    fn compile<S: MaterialSurfaceQueries>(
        rule: MaterialRule,
        root: &R::Positional,
        context: &mut Context<S>,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<Self, DecodeError> {
        *nodes += 1;
        if depth >= 256 || *nodes > 65536 {
            return Err(DecodeError::Field("material rule size"));
        }
        Ok(match rule {
            MaterialRule::Block(block) => Self::Block(block),
            MaterialRule::Reference(value) => {
                return Self::compile((*value).clone(), root, context, depth + 1, nodes);
            }
            MaterialRule::OreVein {
                ore,
                raw_ore,
                filler,
                raw_ore_chance,
                density,
                richness,
                filler_gap,
                positive_y,
            } => {
                let surface = &mut context.surface;
                let density = surface.bind_density_rows(density, true, positive_y)?;
                let richness = surface.bind_density_rows(richness, true, positive_y)?;
                let gap = surface.bind_density_rows(filler_gap, false, positive_y)?;
                let random = root.from_hash_of("minecraft:ore").fork_positional();
                Self::OreVein {
                    blocks: [ore, raw_ore, filler],
                    raw_ore_chance,
                    densities: [density, richness, gap],
                    random,
                    positive_y,
                }
            }
            MaterialRule::Bandlands => Self::Bandlands,
            MaterialRule::Condition { if_true, then_run } => {
                // Compile the predicate before the nested rule, regardless of
                // whether the predicate will match any subsequent position.
                let condition = Compiled::compile(if_true, root, context, depth + 1)?;
                let rule = Box::new(Self::compile(*then_run, root, context, depth + 1, nodes)?);
                // Compile both sides first: binding and errors must retain their order.
                match condition.constant_result() {
                    Some(false) => Self::Empty,
                    Some(true) => *rule,
                    None => Self::Condition { condition, rule },
                }
            }
            MaterialRule::Sequence(mut children) => {
                if children.len() == 1 {
                    return Self::compile(children.pop().unwrap(), root, context, depth + 1, nodes);
                }
                let mut result = Vec::with_capacity(children.len());
                for child in children {
                    let child = Self::compile(child, root, context, depth + 1, nodes)?;
                    if !matches!(child, Self::Empty) {
                        result.push(child);
                    }
                }
                match result.len() {
                    0 => Self::Empty,
                    1 => result.pop().unwrap(),
                    _ => Self::Sequence(result),
                }
            }
        })
    }
    fn apply<S: MaterialSurfaceQueries>(
        &mut self,
        position: BlockPos,
        context: &mut Context<S>,
    ) -> Result<Option<BlockStateId>, MaterialRuleError> {
        match self {
            Self::Empty => Ok(None),
            Self::Block(block) => Ok(Some(*block)),
            Self::OreVein {
                blocks,
                raw_ore_chance,
                densities,
                random,
                ..
            } => {
                // Densities use the context; randomness uses application coordinates.
                let density = context.surface.density(densities[0], context.position)?;
                if density <= 0.0 {
                    return Ok(None);
                }
                let mut random = random.at(position);
                if random.next_float() > density {
                    return Ok(None);
                }
                let richness = context.surface.density(densities[1], context.position)?;
                if random.next_float() < richness
                    && context.surface.density(densities[2], context.position)? < 0.0
                {
                    Ok(Some(if random.next_float() < *raw_ore_chance {
                        blocks[1]
                    } else {
                        blocks[0]
                    }))
                } else {
                    Ok(Some(blocks[2]))
                }
            }
            Self::Bandlands => context.surface.band(position).map(Some),
            Self::Condition { condition, rule } => {
                if condition.test(context)? {
                    rule.apply(position, context)
                } else {
                    Ok(None)
                }
            }
            Self::Sequence(children) => {
                for child in children {
                    // A condition's test, in the loop: most children are one
                    // condition, so this saves a call and a dispatch each.
                    let child = match child {
                        Self::Condition { condition, rule } => {
                            if !condition.test(context)? {
                                continue;
                            }
                            &mut **rule
                        }
                        child => child,
                    };
                    if let Some(block) = child.apply(position, context)? {
                        return Ok(Some(block));
                    }
                }
                Ok(None)
            }
        }
    }
    /// Retained cache budget units for this rule tree: AST nodes plus owned
    /// biome-mask / biome-name storage inside every condition.
    fn retained_cost(&self) -> usize {
        match self {
            Self::Empty | Self::Block(_) | Self::OreVein { .. } | Self::Bandlands => 1,
            Self::Condition { condition, rule } => 1usize
                .saturating_add(condition.retained_cost())
                .saturating_add(rule.retained_cost()),
            Self::Sequence(children) => children.iter().fold(1usize, |acc, child| {
                acc.saturating_add(child.retained_cost())
            }),
        }
    }
    /// The first y of each band in which this rule specialises differently
    /// from the band below, unordered, and whether the preliminary surface is
    /// asked for.
    fn inputs(&self, starts: &mut Vec<i32>, surface: &mut bool) {
        match self {
            Self::Condition { condition, rule } => {
                condition.inputs(starts, surface);
                rule.inputs(starts, surface);
            }
            Self::Sequence(children) => {
                for child in children {
                    child.inputs(starts, surface);
                }
            }
            Self::OreVein {
                positive_y: [low, high],
                ..
            } if low <= high => {
                if *low != i32::MIN {
                    starts.push(*low);
                }
                if *high != i32::MAX {
                    starts.push(high + 1);
                }
            }
            _ => {}
        }
    }
}
impl<R: RandomSource> CompiledRule<R>
where
    R::Positional: Clone,
{
    /// This rule where `facts` hold: conditions they decide are folded away,
    /// with what can no longer produce a block, and a sequence stops at its
    /// first child that always produces one. For every position the facts
    /// cover it gives the block the whole rule gives.
    fn specialize(&self, facts: Facts) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Block(block) => Self::Block(*block),
            Self::Bandlands => Self::Bandlands,
            Self::OreVein {
                blocks,
                raw_ore_chance,
                densities,
                random,
                positive_y: [low, high],
            } => {
                // A non-positive density returns before the vein's
                // positional random is drawn: nothing else is consumed.
                if low > high || facts.high < *low || facts.low > *high {
                    return Self::Empty;
                }
                Self::OreVein {
                    blocks: *blocks,
                    raw_ore_chance: *raw_ore_chance,
                    densities: *densities,
                    random: random.clone(),
                    positive_y: [*low, *high],
                }
            }
            Self::Condition { condition, rule } => match condition.decided(facts) {
                Some(false) => Self::Empty,
                Some(true) => rule.specialize(facts),
                None => match rule.specialize(facts) {
                    Self::Empty => Self::Empty,
                    rule => Self::Condition {
                        condition: condition.clone(),
                        rule: Box::new(rule),
                    },
                },
            },
            Self::Sequence(children) => {
                let mut result = Vec::new();
                for child in children {
                    let child = child.specialize(facts);
                    if matches!(child, Self::Empty) {
                        continue;
                    }
                    let last = child.always_produces();
                    result.push(child);
                    if last {
                        break;
                    }
                }
                match result.len() {
                    0 => Self::Empty,
                    1 => result.pop().unwrap(),
                    _ => Self::Sequence(result),
                }
            }
        }
    }
    /// This rule as `compile` makes it for a chunk with `possible` biomes
    /// (a copy without them): the biome conditions they decide are constant,
    /// and folded away as `compile` folds them.
    fn fold_biomes(&self, possible: Option<&PossibleBiomes>) -> Self {
        match self {
            Self::Empty => Self::Empty,
            Self::Block(block) => Self::Block(*block),
            Self::Bandlands => Self::Bandlands,
            Self::OreVein {
                blocks,
                raw_ore_chance,
                densities,
                random,
                positive_y,
            } => Self::OreVein {
                blocks: *blocks,
                raw_ore_chance: *raw_ore_chance,
                densities: *densities,
                random: random.clone(),
                positive_y: *positive_y,
            },
            Self::Condition { condition, rule } => {
                let condition = match possible {
                    Some(possible) => condition.fold_biomes(possible),
                    None => condition.clone(),
                };
                let rule = rule.fold_biomes(possible);
                match condition.constant_result() {
                    Some(false) => Self::Empty,
                    Some(true) => rule,
                    None => Self::Condition {
                        condition,
                        rule: Box::new(rule),
                    },
                }
            }
            Self::Sequence(children) => {
                let mut result: Vec<_> = children
                    .iter()
                    .map(|child| child.fold_biomes(possible))
                    .filter(|child| !matches!(child, Self::Empty))
                    .collect();
                match result.len() {
                    0 => Self::Empty,
                    1 => result.pop().unwrap(),
                    _ => Self::Sequence(result),
                }
            }
        }
    }
    /// `Some` when applying this rule gives `block` at every position where
    /// none of its ore veins places one: the rule is that block alone, or ore
    /// veins and then it. Each vein begins with its first density, and a
    /// density that is not positive returns `None` before any random is drawn
    /// or another density is read, so a position every vein misses there gives
    /// exactly `block`.
    fn fill(&self) -> Option<SurfaceFill> {
        let (block, veins) = match self {
            Self::Block(block) => (*block, &[][..]),
            Self::Sequence(children) => {
                let (last, veins) = children.split_last()?;
                match last {
                    Self::Block(block) => (*block, veins),
                    _ => return None,
                }
            }
            _ => return None,
        };
        if veins.len() > SurfaceFill::MAX_VEINS {
            return None;
        }
        let mut fill = SurfaceFill {
            block,
            veins: [0; SurfaceFill::MAX_VEINS],
            vein_count: veins.len(),
        };
        for (slot, vein) in fill.veins.iter_mut().zip(veins) {
            let Self::OreVein { densities, .. } = vein else {
                return None;
            };
            *slot = densities[0];
        }
        Some(fill)
    }
    /// Whether applying this rule never gives `None` (only an error).
    fn always_produces(&self) -> bool {
        match self {
            Self::Block(_) | Self::Bandlands => true,
            Self::Sequence(children) => children.iter().any(Self::always_produces),
            Self::Empty | Self::OreVein { .. } | Self::Condition { .. } => false,
        }
    }
}
impl MaterialRule {
    /// The highest y at which an ore vein of this rule can place a block
    /// (`MaterialRule::OreVein::positive_y`), `i32::MIN` without veins.
    pub fn ore_vein_top(&self) -> i32 {
        self.ore_vein_top_within(0, &mut 0)
    }
    fn ore_vein_top_within(&self, depth: usize, nodes: &mut usize) -> i32 {
        *nodes += 1;
        // Larger rules fail to compile (`CompiledRule::compile`); do not
        // bound them.
        if depth >= 256 || *nodes > 65536 {
            return i32::MAX;
        }
        match self {
            Self::Block(_) | Self::Bandlands => i32::MIN,
            Self::Reference(rule) => rule.ore_vein_top_within(depth + 1, nodes),
            Self::OreVein {
                positive_y: [low, high],
                ..
            } => {
                if low > high {
                    i32::MIN
                } else {
                    *high
                }
            }
            Self::Sequence(children) => {
                let mut top = i32::MIN;
                for child in children {
                    top = top.max(child.ore_vein_top_within(depth + 1, nodes));
                }
                top
            }
            Self::Condition { then_run, .. } => then_run.ore_vein_top_within(depth + 1, nodes),
        }
    }
}
/// A rule specialised (`CompiledRule::specialize`) for each band of y between
/// its vertical gradients' bounds and its ore veins' `positive_y` bounds, and
/// below and at or above the preliminary surface when the rule asks for it:
/// most blocks of a column, deep stone, then evaluate only the few rules that
/// can apply there. Each variant is made when first asked for.
pub struct SurfaceVariants {
    /// The rule the variants specialise.
    base: usize,
    /// The first y of each band after the first, ascending.
    starts: Box<[i32]>,
    /// Per band (two per band, below then above the surface, when
    /// `surface`): the variant once made, `Some(None)` when it gives no block.
    rules: Box<[Option<Option<BandRule>>]>,
    /// The run fills the variants' `BandRule::Fill` refer to.
    fills: Vec<SurfaceFill>,
    /// Retained budget units across `base` and every materialized band variant
    /// (AST nodes plus owned biome-mask / biome-name storage).
    retained_units: usize,
    pub surface: bool,
}
/// One band's specialised rule, as `build_surface_shared` applies it.
#[derive(Clone, Copy, Debug)]
pub enum BandRule {
    /// Apply this rule at each position.
    Rule(usize),
    /// Apply this rule at each position, except that a whole run of blocks it
    /// gives one block wherever none of its ore veins places one may be
    /// written at once (`MaterialConditions::veins_miss`).
    Fill(usize, usize),
}
impl BandRule {
    /// The rule to apply at one position.
    pub fn rule(self) -> usize {
        match self {
            Self::Rule(rule) | Self::Fill(rule, _) => rule,
        }
    }
}
/// A rule that gives one block wherever none of its ore veins places one, and
/// the veins that can stop it: `BandRule::Fill`'s run fill, tested through
/// `MaterialConditions::veins_miss`.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceFill {
    /// The block of every position no vein places one at.
    pub block: BlockStateId,
    /// The veins' first density bindings, in application order.
    pub veins: [usize; Self::MAX_VEINS],
    pub vein_count: usize,
}
impl SurfaceFill {
    /// Most veins one run fill tests.
    pub const MAX_VEINS: usize = 4;
    /// The veins' density bindings.
    #[inline]
    pub fn veins(&self) -> &[usize] {
        &self.veins[..self.vein_count]
    }
}
impl SurfaceVariants {
    fn from_rule<R: RandomSource>(base: usize, rule: &CompiledRule<R>) -> Self {
        let (mut starts, mut surface) = (Vec::new(), false);
        rule.inputs(&mut starts, &mut surface);
        starts.sort_unstable();
        starts.dedup();
        let starts = starts.into_boxed_slice();
        let sides = if surface { 2 } else { 1 };
        Self {
            base,
            rules: vec![None; (starts.len() + 1) * sides].into_boxed_slice(),
            fills: Vec::new(),
            starts,
            retained_units: rule.retained_cost(),
            surface,
        }
    }
    /// Retained cache budget units across `base`, every materialized band
    /// variant (including owned biome-mask / biome-name storage), band tables,
    /// and the `key_words`-word `possible_ids` cache key.
    fn retained_cost(&self, key_words: usize) -> usize {
        self.retained_units
            .saturating_add(self.starts.len())
            .saturating_add(self.rules.len())
            .saturating_add(self.fills.len())
            .saturating_add(key_words)
    }
    /// The band holding `y`.
    #[inline]
    pub fn band(&self, y: i32) -> usize {
        self.starts.partition_point(|&start| start <= y)
    }
    /// The run fill `BandRule::Fill` index refers to.
    pub fn fill(&self, index: usize) -> &SurfaceFill {
        &self.fills[index]
    }
    /// The inclusive y bounds of `band`.
    pub fn band_bounds(&self, band: usize) -> [i32; 2] {
        let low = if band == 0 {
            i32::MIN
        } else {
            self.starts[band - 1]
        };
        let high = self.starts.get(band).map_or(i32::MAX, |&next| next - 1);
        [low, high]
    }
}
impl<R: RandomSource, S: MaterialSurfaceQueries> MaterialConditions<R, S>
where
    R::Positional: Clone,
{
    /// The bands of the specialised variants of rule `index`
    /// (`SurfaceVariants`); `surface_variant` makes each.
    pub fn surface_variants(&mut self, index: usize) -> Result<SurfaceVariants, MaterialRuleError> {
        let rule = self
            .rules
            .get(index)
            .ok_or(MaterialRuleError::UnknownRule)?;
        Ok(SurfaceVariants::from_rule(index, rule))
    }
    /// The variant for `band`, `above` the preliminary surface (ignored unless
    /// `variants.surface`), or `None` when it gives no block at any position
    /// of the band.
    #[inline]
    pub fn surface_variant(
        &mut self,
        variants: &mut SurfaceVariants,
        band: usize,
        above: bool,
    ) -> Option<BandRule> {
        let slot = if variants.surface {
            band * 2 + usize::from(above)
        } else {
            band
        };
        if let Some(rule) = variants.rules[slot] {
            return rule;
        }
        self.make_surface_variant(variants, band, above, slot)
    }
    #[cold]
    fn make_surface_variant(
        &mut self,
        variants: &mut SurfaceVariants,
        band: usize,
        above: bool,
        slot: usize,
    ) -> Option<BandRule> {
        let [low, high] = variants.band_bounds(band);
        let facts = Facts {
            low,
            high,
            above_surface: variants.surface.then_some(above),
        };
        let rule = match self.rules[variants.base].specialize(facts) {
            CompiledRule::Empty => None,
            rule => {
                variants.retained_units =
                    variants.retained_units.saturating_add(rule.retained_cost());
                let index = self.rules.len();
                let fill = rule.fill().map(|fill| {
                    variants.fills.push(fill);
                    variants.fills.len() - 1
                });
                self.rules.push(rule);
                Some(match fill {
                    Some(fill) => BandRule::Fill(index, fill),
                    None => BandRule::Rule(index),
                })
            }
        };
        variants.rules[slot] = Some(rule);
        rule
    }
    /// Whether every ore vein of `fill` places nothing at `position`: each
    /// begins with a density test, and a density that is not positive returns
    /// `None` before any random is drawn or another density is read.
    pub fn veins_miss(
        &mut self,
        fill: &SurfaceFill,
        position: BlockPos,
    ) -> Result<bool, MaterialRuleError> {
        for &index in fill.veins() {
            if self.context.surface.density(index, position)? > 0.0 {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
/// Cached biome-folded rule tree and its lazily populated `SurfaceVariants`,
/// keyed by the trimmed possible-biome bitset of a chunk's 3x3 region.
struct CachedVariantSet<R: RandomSource> {
    possible_ids: Box<[u64]>,
    rules: Vec<CompiledRule<R>>,
    variants: SurfaceVariants,
}

const VARIANT_CACHE_LIMIT: usize = 16;
/// Maximum retained budget units per `CachedVariantSet`, charging rule and
/// condition AST nodes, owned biome-mask `u64` words (and biome-name storage)
/// across `base` and every materialized variant, band tables, and the
/// `possible_ids` cache key words. Entries that grow past this budget still
/// build band variants on demand for the active chunk (leaving generated
/// blocks unchanged) but are dropped when the chunk finishes instead of
/// remaining cached on the worker.
const VARIANT_ENTRY_BUDGET: usize = 4096;

fn trim_words(words: &[u64]) -> &[u64] {
    let mut len = words.len();
    while len > 0 && words[len - 1] == 0 {
        len -= 1;
    }
    &words[..len]
}

/// A world's surface rule compiled once for every chunk of a generator
/// worker: the rule before any chunk's possible biomes fold it, the noises it
/// bound, and the densities (ore veins) it binds to each chunk's volume, in
/// the order it bound them.
pub struct CompiledSurfaceRule<R: RandomSource> {
    generation: super::WorldGenerationContext,
    rule: CompiledRule<R>,
    noises: Vec<((Arc<str>, bool), Arc<dyn Noise>)>,
    densities: Vec<DensityBinding>,
    /// Monotonic revision watermark across chunks reusing cached variant trees,
    /// ensuring cached `Compiled` condition nodes never match a prior chunk's revision.
    next_revision: i64,
    cached: Vec<CachedVariantSet<R>>,
}
impl<R: RandomSource> CompiledSurfaceRule<R> {
    /// Whether binding this compiled rule to a worker samples any density
    /// eagerly.
    pub(crate) fn has_eager_density_bindings(&self) -> bool {
        self.densities.iter().any(|(_, eager, _)| *eager)
    }
}
/// A density a rule binds: its root, whether it is sampled over the volume,
/// and the rows it is read at.
type DensityBinding = (Arc<dyn DensitySampler>, bool, [i32; 2]);
/// The densities `CompiledRule::compile` binds for `rule`, in its order,
/// with whether it samples each over the volume and the rows it reads.
fn density_bindings(rule: &MaterialRule, out: &mut Vec<DensityBinding>) {
    match rule {
        MaterialRule::Reference(value) => density_bindings(value, out),
        MaterialRule::OreVein {
            density,
            richness,
            filler_gap,
            positive_y,
            ..
        } => out.extend([
            (Arc::clone(density), true, *positive_y),
            (Arc::clone(richness), true, *positive_y),
            (Arc::clone(filler_gap), false, *positive_y),
        ]),
        MaterialRule::Condition { then_run, .. } => density_bindings(then_run, out),
        MaterialRule::Sequence(children) => {
            for child in children {
                density_bindings(child, out);
            }
        }
        MaterialRule::Block(_) | MaterialRule::Bandlands => {}
    }
}
impl<R: RandomSource, S: MaterialSurfaceQueries> MaterialConditions<R, S>
where
    R::Positional: Clone,
{
    fn build_surface_variants(&self, index: usize) -> SurfaceVariants {
        SurfaceVariants::from_rule(index, &self.rules[index])
    }

    fn compile_shared_entry(
        &mut self,
        rule: &MaterialRule,
    ) -> Result<CompiledSurfaceRule<R>, DecodeError> {
        let possible = self.context.possible_biomes.take();
        let prev_fold = self.context.fold_possible;
        self.context.fold_possible = false;
        let compiled =
            CompiledRule::compile(rule.clone(), &self.root, &mut self.context, 0, &mut 0);
        self.context.fold_possible = prev_fold;
        self.context.possible_biomes = possible;
        let compiled = compiled?;
        let noises = self
            .context
            .noises
            .iter()
            .map(|cached| {
                (
                    (Arc::clone(&cached.name), cached.is_3d),
                    Arc::clone(&cached.sampler),
                )
            })
            .collect();
        let mut densities = Vec::new();
        density_bindings(rule, &mut densities);
        Ok(CompiledSurfaceRule {
            generation: self.context.generation,
            rule: compiled,
            noises,
            densities,
            next_revision: 0,
            cached: Vec::new(),
        })
    }

    fn bind_shared_resources(
        &mut self,
        cached: &CompiledSurfaceRule<R>,
    ) -> Result<(), DecodeError> {
        for ((name, is_3d), sampler) in &cached.noises {
            self.context.noises.push(CachedNoise {
                name: Arc::clone(name),
                sampler: Arc::clone(sampler),
                is_3d: *is_3d,
                revision: if *is_3d {
                    self.context.block_revision
                } else {
                    self.context.column_revision
                }
                .wrapping_sub(1),
                value: 0.0,
            });
        }
        self.context.noises_indexed = false;
        for (sampler, eager, rows) in &cached.densities {
            self.context
                .surface
                .bind_density_rows(Arc::clone(sampler), *eager, *rows)?;
        }
        Ok(())
    }

    /// Resolve possible biomes into a trimmed borrowed-or-owned bitset when the
    /// surface numbers biomes (borrowing `possible_biome_ids` directly, or
    /// converting `possible_biomes` via `biome_index`). Returns `Err(())` if
    /// `possible_biomes` contains a biome the surface does not number.
    fn resolve_possible_ids(&self) -> Result<Option<Cow<'_, [u64]>>, ()> {
        if !self.context.fold_possible {
            return Ok(None);
        }
        if let Some(ids) = self.context.surface.possible_biome_ids() {
            return Ok(Some(Cow::Borrowed(trim_words(ids))));
        }
        let Some(names) = &self.context.possible_biomes else {
            return Ok(None);
        };
        let mut ids = Vec::new();
        for name in names {
            let Some(Some(id)) = self.context.surface.biome_index(name) else {
                return Err(());
            };
            let word = id as usize / 64;
            if ids.len() <= word {
                ids.resize(word + 1, 0);
            }
            ids[word] |= 1 << (id % 64);
        }
        while ids.last() == Some(&0) {
            ids.pop();
        }
        Ok(Some(Cow::Owned(ids)))
    }

    /// `add_rule`, compiling `rule` only the first time `shared` is used:
    /// later workers take the compiled rule and its noises, bind its densities
    /// to their own volume in the same order, and fold it with their own
    /// possible biomes as `compile` does. A worker that already holds rules or
    /// noises, or a possible biome the surface doesn't number, compiles as
    /// `add_rule` does.
    pub fn add_shared_rule(
        &mut self,
        rule: &MaterialRule,
        shared: &mut Option<CompiledSurfaceRule<R>>,
    ) -> Result<usize, DecodeError> {
        let fresh = self.rules.is_empty() && self.context.noises.is_empty();
        if !fresh || self.rules.len() >= 8192 {
            return self.add_rule(rule.clone());
        }
        // Only materialize an owned bitset when converting `possible_biomes`;
        // a surface's borrowed `possible_biome_ids()` slice is re-borrowed below
        // after binding shared resources without allocating.
        let owned_ids =
            if !self.context.fold_possible || self.context.surface.possible_biome_ids().is_some() {
                None
            } else {
                match self.resolve_possible_ids() {
                    Ok(Some(Cow::Owned(ids))) => Some(ids),
                    Ok(Some(Cow::Borrowed(_))) | Ok(None) => None,
                    Err(()) => return self.add_rule(rule.clone()),
                }
            };
        if let Some(cached) = shared.as_ref()
            && cached.generation == self.context.generation
        {
            self.bind_shared_resources(cached)?;
        } else {
            *shared = Some(self.compile_shared_entry(rule)?);
        }
        let shared = shared.as_ref().expect("compiled above");
        let ids: Option<Cow<'_, [u64]>> = if !self.context.fold_possible {
            None
        } else {
            match owned_ids {
                Some(ids) => Some(Cow::Owned(ids)),
                None => self
                    .context
                    .surface
                    .possible_biome_ids()
                    .map(|ids| Cow::Borrowed(trim_words(ids))),
            }
        };
        let possible = ids.as_deref().map(|ids| PossibleBiomes {
            names: self.context.possible_biomes.as_ref(),
            ids,
        });
        let folded = shared.rule.fold_biomes(possible.as_ref());
        let index = self.rules.len();
        self.rules.push(folded);
        Ok(index)
    }

    /// Prepare a chunk's folded surface rule and cached `SurfaceVariants`,
    /// reusing a previously folded + band-specialized tree when a chunk's 3x3
    /// neighborhood has the same possible-biome bitset. Must be paired with
    /// [`Self::finish_shared_surface`].
    pub(crate) fn prepare_shared_surface(
        &mut self,
        rule: &MaterialRule,
        shared: &mut Option<CompiledSurfaceRule<R>>,
    ) -> Result<(SurfaceVariants, Option<Box<[u64]>>), DecodeError> {
        let fresh = self.rules.is_empty() && self.context.noises.is_empty();
        if !fresh || self.rules.len() >= 8192 {
            let id = self.add_rule(rule.clone())?;
            let variants = self.build_surface_variants(id);
            return Ok((variants, None));
        }
        if !self.context.fold_possible {
            let id = self.add_shared_rule(rule, shared)?;
            let variants = self.build_surface_variants(id);
            return Ok((variants, None));
        }
        let owned_ids = if self.context.surface.possible_biome_ids().is_some() {
            None
        } else {
            match self.resolve_possible_ids() {
                Ok(Some(Cow::Owned(ids))) => Some(ids),
                Ok(Some(Cow::Borrowed(_))) => None,
                Ok(None) | Err(()) => {
                    let id = self.add_shared_rule(rule, shared)?;
                    let variants = self.build_surface_variants(id);
                    return Ok((variants, None));
                }
            }
        };
        if shared
            .as_ref()
            .is_none_or(|cached| cached.generation != self.context.generation)
        {
            *shared = Some(self.compile_shared_entry(rule)?);
        } else {
            let cached = shared.as_mut().expect("checked above");
            if cached.next_revision > i64::MAX - (1 << 30) {
                cached.cached.clear();
                cached.next_revision = 0;
            }
            let base = cached.next_revision;
            self.context.column_revision = base;
            self.context.block_revision = base;
            self.context.secondary_revision = base.wrapping_sub(1);
            self.context.minimum_revision = base.wrapping_sub(1);
            self.bind_shared_resources(cached)?;
        }
        let cached = shared.as_mut().expect("compiled above");
        let ids: Cow<'_, [u64]> = match owned_ids {
            Some(ids) => Cow::Owned(ids),
            None => Cow::Borrowed(trim_words(
                self.context
                    .surface
                    .possible_biome_ids()
                    .expect("checked above"),
            )),
        };
        if let Some(pos) = cached
            .cached
            .iter()
            .position(|entry| entry.possible_ids.as_ref() == ids.as_ref())
        {
            let entry = cached.cached.remove(pos);
            self.rules = entry.rules;
            return Ok((entry.variants, Some(entry.possible_ids)));
        }
        let possible = PossibleBiomes {
            names: self.context.possible_biomes.as_ref(),
            ids: ids.as_ref(),
        };
        let folded = cached.rule.fold_biomes(Some(&possible));
        let variants = SurfaceVariants::from_rule(0, &folded);
        let cached_slot =
            (variants.retained_cost(ids.len()) <= VARIANT_ENTRY_BUDGET).then(|| match ids {
                Cow::Owned(vec) => vec.into_boxed_slice(),
                Cow::Borrowed(slice) => slice.into(),
            });
        self.rules.push(folded);
        Ok((variants, cached_slot))
    }

    /// Return the chunk's folded rule + specialized `SurfaceVariants` to `shared`
    /// and advance the revision watermark past all revisions used by this chunk.
    /// Entries whose accumulated specialized variants and cache key exceed
    /// [`VARIANT_ENTRY_BUDGET`] are dropped instead of persisting across chunks.
    pub(crate) fn finish_shared_surface(
        &mut self,
        shared: &mut Option<CompiledSurfaceRule<R>>,
        variants: SurfaceVariants,
        cached_slot: Option<Box<[u64]>>,
    ) {
        let Some(cached) = shared.as_mut() else {
            return;
        };
        let used_max = self
            .context
            .column_revision
            .max(self.context.block_revision);
        cached.next_revision = cached.next_revision.max(used_max.wrapping_add(2));
        if let Some(possible_ids) = cached_slot
            && variants.retained_cost(possible_ids.len()) <= VARIANT_ENTRY_BUDGET
        {
            if cached.cached.len() >= VARIANT_CACHE_LIMIT {
                cached.cached.remove(0);
            }
            cached.cached.push(CachedVariantSet {
                possible_ids,
                rules: std::mem::take(&mut self.rules),
                variants,
            });
        }
    }
}
impl<R: RandomSource, T: MaterialSurfaceQueries> MaterialConditions<R, MaterialDensitySources<T>> {
    /// Re-target the per-cell density bindings for a reused worker.
    pub fn retarget_surface_densities(&mut self, position: [i32; 3]) -> Result<(), DecodeError> {
        self.context.surface.retarget(position)
    }
}
impl<R: RandomSource, S: MaterialSurfaceQueries> MaterialConditions<R, S> {
    /// Compile all children in order, even after an unconditional block rule.
    /// Failed compilation never adds a partial rule to this worker.
    pub fn add_rule(&mut self, rule: MaterialRule) -> Result<usize, DecodeError> {
        if self.rules.len() >= 8192 {
            return Err(DecodeError::Field("material rule count"));
        }
        let compiled = CompiledRule::compile(rule, &self.root, &mut self.context, 0, &mut 0)?;
        let index = self.rules.len();
        self.rules.push(compiled);
        Ok(index)
    }
    /// Predicates read the worker's updated context. Position arguments are
    /// forwarded to block-producing rules independently, as in vanilla.
    pub fn try_apply(
        &mut self,
        index: usize,
        position: BlockPos,
    ) -> Result<Option<BlockStateId>, MaterialRuleError> {
        self.rules
            .get_mut(index)
            .ok_or(MaterialRuleError::UnknownRule)?
            .apply(position, &mut self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::anchor::{VerticalAnchor, WorldGenerationContext};
    use horizon_core::random::LegacyRandom;

    struct BitsetSurface {
        possible_ids: Vec<u64>,
        biome_name: &'static str,
        biome_id: u32,
    }
    impl MaterialSurfaceQueries for BitsetSurface {
        fn possible_biome_ids(&self) -> Option<&[u64]> {
            Some(&self.possible_ids)
        }
        fn biome_index(&self, name: &str) -> Option<Option<u32>> {
            Some(match name {
                "minecraft:plains" => Some(0),
                "minecraft:desert" => Some(1),
                _ => None,
            })
        }
        fn biome(&mut self, _: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
            Ok(Some(Arc::from(self.biome_name)))
        }
        fn biome_id(&mut self, _: BlockPos) -> Result<Option<u32>, MaterialRuleError> {
            Ok(Some(self.biome_id))
        }
        fn surface_depth(&mut self, _: i32, _: i32) -> i32 {
            0
        }
        fn surface_secondary(&mut self, _: i32, _: i32) -> f64 {
            0.0
        }
    }

    fn eval_band_rule<S: MaterialSurfaceQueries>(
        worker: &mut MaterialConditions<LegacyRandom, S>,
        variants: &mut SurfaceVariants,
        x: i32,
        y: i32,
        z: i32,
        gradient_x: i32,
    ) -> Option<BlockStateId> {
        worker.update_xz(x, z, gradient_x, 0);
        worker.update_y(1, 0, i32::MIN, y);
        let band = variants.band(y);
        let rule = worker.surface_variant(variants, band, true)?;
        worker
            .try_apply(rule.rule(), BlockPos::new(x, y, z))
            .unwrap()
    }

    #[test]
    fn shared_surface_variants_reuse_borrowed_ids_and_isolate_revisions() {
        let generation = WorldGenerationContext::new(0, 64, 32);
        let root = LegacyRandom::new(7).fork_positional();
        let steep_block = BlockStateId(1);
        let plains_block = BlockStateId(2);
        let desert_block = BlockStateId(3);
        let rule = MaterialRule::Sequence(vec![
            MaterialRule::Condition {
                if_true: MaterialCondition::Steep,
                then_run: Box::new(MaterialRule::Block(steep_block)),
            },
            MaterialRule::Condition {
                if_true: MaterialCondition::Biome(vec!["minecraft:plains".into()]),
                then_run: Box::new(MaterialRule::Block(plains_block)),
            },
            MaterialRule::Condition {
                if_true: MaterialCondition::Biome(vec!["minecraft:desert".into()]),
                then_run: Box::new(MaterialRule::Block(desert_block)),
            },
        ]);

        let mut shared = None;
        // Pass 1: miss, populates cached entry for {plains, desert} with trailing zero word.
        let mut w1 = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: vec![0b11, 0],
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v1, slot1) = w1.prepare_shared_surface(&rule, &mut shared).unwrap();
        // Steep triggers on gradient_x <= -4 || gradient_z >= 4.
        assert_eq!(
            eval_band_rule(&mut w1, &mut v1, 1, 20, 2, -5),
            Some(steep_block)
        );
        // Also evaluate a non-steep plains column so Biome(plains) caches `true`.
        assert_eq!(
            eval_band_rule(&mut w1, &mut v1, 2, 20, 2, 0),
            Some(plains_block)
        );
        // End pass 1 with Steep cached `true` as well.
        assert_eq!(
            eval_band_rule(&mut w1, &mut v1, 1, 20, 2, -5),
            Some(steep_block)
        );
        w1.finish_shared_surface(&mut shared, v1, slot1);
        assert_eq!(shared.as_ref().unwrap().cached.len(), 1);
        assert_eq!(
            shared.as_ref().unwrap().cached[0].possible_ids.as_ref(),
            &[0b11]
        );

        // Pass 2: cache hit with same bitset; non-steep + desert must not see stale steep/plains cache.
        let mut w2 = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: vec![0b11],
                biome_name: "minecraft:desert",
                biome_id: 1,
            },
        );
        let (mut v2, slot2) = w2.prepare_shared_surface(&rule, &mut shared).unwrap();
        assert_eq!(
            eval_band_rule(&mut w2, &mut v2, 1, 20, 2, 0),
            Some(desert_block)
        );
        w2.finish_shared_surface(&mut shared, v2, slot2);
        assert_eq!(shared.as_ref().unwrap().cached.len(), 1);

        // Pass 3: second cache hit after positive `next_revision` advance; non-steep + plains
        // at the same coordinates must not see pass 2's cached desert result.
        let mut w3 = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: vec![0b11],
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v3, slot3) = w3.prepare_shared_surface(&rule, &mut shared).unwrap();
        assert_eq!(
            eval_band_rule(&mut w3, &mut v3, 1, 20, 2, 0),
            Some(plains_block)
        );
        w3.finish_shared_surface(&mut shared, v3, slot3);
        assert_eq!(shared.as_ref().unwrap().cached.len(), 1);
    }

    #[test]
    fn shared_surface_variants_drop_entries_exceeding_size_budget() {
        let generation = WorldGenerationContext::new(0, 256, 64);
        let root = LegacyRandom::new(11).fork_positional();
        // Build many distinct vertical gradient bands combined with undecided
        // conditions so visiting bands pushes retained_cost() over VARIANT_ENTRY_BUDGET.
        let mut children = Vec::new();
        for band in 0..80_i32 {
            let y = band * 2;
            children.push(MaterialRule::Condition {
                if_true: MaterialCondition::VerticalGradient {
                    random_name: format!("minecraft:grad_{band}"),
                    true_at_and_below: VerticalAnchor::Absolute(y),
                    false_at_and_above: VerticalAnchor::Absolute(y + 1),
                },
                then_run: Box::new(MaterialRule::Condition {
                    if_true: MaterialCondition::Steep,
                    then_run: Box::new(MaterialRule::Block(BlockStateId((band + 1) as u32))),
                }),
            });
        }
        // Add undecided trailing conditions so every band specializes to a large tree.
        for extra in 0..40_i32 {
            children.push(MaterialRule::Condition {
                if_true: MaterialCondition::Water {
                    offset: extra,
                    surface_depth_multiplier: 0,
                    add_stone_depth: false,
                },
                then_run: Box::new(MaterialRule::Block(BlockStateId((100 + extra) as u32))),
            });
        }
        let rule = MaterialRule::Sequence(children);

        let mut shared = None;
        let mut w_shared = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: vec![0b1],
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v_shared, slot) = w_shared.prepare_shared_surface(&rule, &mut shared).unwrap();

        let mut w_plain = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: vec![0b1],
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let plain_id = w_plain.add_rule(rule).unwrap();
        let mut v_plain = w_plain.surface_variants(plain_id).unwrap();

        for y in (0..160).step_by(2) {
            let got = eval_band_rule(&mut w_shared, &mut v_shared, 3, y, 5, -4);
            let want = eval_band_rule(&mut w_plain, &mut v_plain, 3, y, 5, -4);
            assert_eq!(got, want, "mismatch at y={y}");
            assert!(got.is_some(), "expected steep band hit at y={y}");
        }
        assert!(
            v_shared.retained_cost(1) > VARIANT_ENTRY_BUDGET,
            "expected test rule to exceed budget: {} <= {}",
            v_shared.retained_cost(1),
            VARIANT_ENTRY_BUDGET
        );
        w_shared.finish_shared_surface(&mut shared, v_shared, slot);
        assert!(
            shared.as_ref().unwrap().cached.is_empty(),
            "oversized variant entry must not remain cached after chunk finishes"
        );
    }

    struct SparseBitsetSurface {
        possible_ids: Vec<u64>,
        far_biome_id: u32,
        biome_name: &'static str,
        biome_id: u32,
    }
    impl MaterialSurfaceQueries for SparseBitsetSurface {
        fn possible_biome_ids(&self) -> Option<&[u64]> {
            Some(&self.possible_ids)
        }
        fn biome_index(&self, name: &str) -> Option<Option<u32>> {
            Some(match name {
                "minecraft:plains" => Some(0),
                "minecraft:desert" => Some(1),
                "minecraft:far_biome" => Some(self.far_biome_id),
                _ => None,
            })
        }
        fn biome(&mut self, _: BlockPos) -> Result<Option<Arc<str>>, MaterialRuleError> {
            Ok(Some(Arc::from(self.biome_name)))
        }
        fn biome_id(&mut self, _: BlockPos) -> Result<Option<u32>, MaterialRuleError> {
            Ok(Some(self.biome_id))
        }
        fn surface_depth(&mut self, _: i32, _: i32) -> i32 {
            0
        }
        fn surface_secondary(&mut self, _: i32, _: i32) -> f64 {
            0.0
        }
    }

    fn dense_mask_rule(bands: i32) -> MaterialRule {
        let mut children = Vec::with_capacity(bands as usize);
        for band in 0..bands {
            let y = band * 2;
            children.push(MaterialRule::Condition {
                if_true: MaterialCondition::VerticalGradient {
                    random_name: format!("minecraft:dense_grad_{band}"),
                    true_at_and_below: VerticalAnchor::Absolute(y),
                    false_at_and_above: VerticalAnchor::Absolute(y + 1),
                },
                then_run: Box::new(MaterialRule::Condition {
                    if_true: MaterialCondition::Biome(vec![
                        "minecraft:plains".into(),
                        "minecraft:far_biome".into(),
                    ]),
                    then_run: Box::new(MaterialRule::Block(BlockStateId((band + 1) as u32))),
                }),
            });
        }
        MaterialRule::Sequence(children)
    }

    #[test]
    fn shared_surface_variants_budget_charges_dense_masks_and_cache_key() {
        let generation = WorldGenerationContext::new(0, 64, 32);
        let root = LegacyRandom::new(19).fork_positional();

        // Part 1: Few AST nodes (~99 nodes across base + all 6 bands), tiny 1-word
        // cache key, but each BiomeIds mask owns 200 u64 words (far_biome_id = 199 * 64).
        // Pass 1 visits a single band and fits in cache; Pass 2 hits cache and
        // materializes the remaining bands, cloning the dense masks past the budget.
        let dense_rule = dense_mask_rule(6);
        let far_dense_id = 199 * 64;
        let mut shared_dense = None;

        let mut w_pass1 = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11],
                far_biome_id: far_dense_id,
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v_pass1, slot_pass1) = w_pass1
            .prepare_shared_surface(&dense_rule, &mut shared_dense)
            .unwrap();
        assert!(slot_pass1.is_some());
        assert_eq!(
            eval_band_rule(&mut w_pass1, &mut v_pass1, 2, 10, 4, 0),
            Some(BlockStateId(6))
        );
        assert!(v_pass1.retained_cost(1) <= VARIANT_ENTRY_BUDGET);
        w_pass1.finish_shared_surface(&mut shared_dense, v_pass1, slot_pass1);
        assert_eq!(shared_dense.as_ref().unwrap().cached.len(), 1);

        let mut w_pass2 = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11],
                far_biome_id: far_dense_id,
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v_pass2, slot_pass2) = w_pass2
            .prepare_shared_surface(&dense_rule, &mut shared_dense)
            .unwrap();
        let mut w_plain = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11],
                far_biome_id: far_dense_id,
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let plain_id = w_plain.add_rule(dense_rule.clone()).unwrap();
        let mut v_plain = w_plain.surface_variants(plain_id).unwrap();
        for y in (0..12).step_by(2) {
            let got = eval_band_rule(&mut w_pass2, &mut v_pass2, 2, y, 4, 0);
            let want = eval_band_rule(&mut w_plain, &mut v_plain, 2, y, 4, 0);
            assert_eq!(got, want, "dense plains mismatch at y={y}");
            assert_eq!(got, Some(BlockStateId((y / 2 + 1) as u32)));
        }
        assert!(
            v_pass2.retained_cost(1) > VARIANT_ENTRY_BUDGET,
            "expected cloned dense masks to exceed budget: {} <= {}",
            v_pass2.retained_cost(1),
            VARIANT_ENTRY_BUDGET
        );
        w_pass2.finish_shared_surface(&mut shared_dense, v_pass2, slot_pass2);
        assert!(
            shared_dense.as_ref().unwrap().cached.is_empty(),
            "dense-mask variant entry must be dropped once cloned masks exceed budget"
        );

        // Non-matching biome on the same oversized dense-mask rule also matches uncached output.
        let mut w_desert = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11],
                far_biome_id: far_dense_id,
                biome_name: "minecraft:desert",
                biome_id: 1,
            },
        );
        let (mut v_desert, slot_desert) = w_desert
            .prepare_shared_surface(&dense_rule, &mut shared_dense)
            .unwrap();
        let mut w_desert_plain = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11],
                far_biome_id: far_dense_id,
                biome_name: "minecraft:desert",
                biome_id: 1,
            },
        );
        let desert_plain_id = w_desert_plain.add_rule(dense_rule.clone()).unwrap();
        let mut v_desert_plain = w_desert_plain.surface_variants(desert_plain_id).unwrap();
        for y in (0..12).step_by(2) {
            let got = eval_band_rule(&mut w_desert, &mut v_desert, 2, y, 4, 0);
            let want = eval_band_rule(&mut w_desert_plain, &mut v_desert_plain, 2, y, 4, 0);
            assert_eq!(got, want, "dense desert mismatch at y={y}");
            assert_eq!(got, None);
        }
        w_desert.finish_shared_surface(&mut shared_dense, v_desert, slot_desert);
        assert!(shared_dense.as_ref().unwrap().cached.is_empty());

        // Part 2: Calibration check — the same 6-band rule with vanilla-scale 2-word
        // masks (~65 biomes, far_biome_id = 64) and a 2-word cache key stays cached.
        let mut shared_small = None;
        let mut w_small = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11, 0b1],
                far_biome_id: 64,
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v_small, slot_small) = w_small
            .prepare_shared_surface(&dense_rule, &mut shared_small)
            .unwrap();
        let mut w_small_plain = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            SparseBitsetSurface {
                possible_ids: vec![0b11, 0b1],
                far_biome_id: 64,
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let small_plain_id = w_small_plain.add_rule(dense_rule).unwrap();
        let mut v_small_plain = w_small_plain.surface_variants(small_plain_id).unwrap();
        for y in (0..12).step_by(2) {
            let got = eval_band_rule(&mut w_small, &mut v_small, 2, y, 4, 0);
            let want = eval_band_rule(&mut w_small_plain, &mut v_small_plain, 2, y, 4, 0);
            assert_eq!(got, want, "small mask mismatch at y={y}");
        }
        assert!(v_small.retained_cost(2) <= VARIANT_ENTRY_BUDGET);
        w_small.finish_shared_surface(&mut shared_small, v_small, slot_small);
        assert_eq!(
            shared_small.as_ref().unwrap().cached.len(),
            1,
            "normal 2-word biome mask entry must stay cached"
        );

        // Part 3: Oversized biome cache key (`possible_ids`) alone rejects caching even
        // for a minimal 3-node rule with a 1-word biome mask, while a normal key caches.
        let tiny_rule = MaterialRule::Condition {
            if_true: MaterialCondition::Biome(vec!["minecraft:plains".into()]),
            then_run: Box::new(MaterialRule::Block(BlockStateId(42))),
        };
        let mut oversized_key = vec![0u64; VARIANT_ENTRY_BUDGET + 1];
        oversized_key[0] = 0b11;
        oversized_key[VARIANT_ENTRY_BUDGET] = 1;
        let oversized_key_len = oversized_key.len();

        let mut shared_key = None;
        let mut w_big_key = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: oversized_key.clone(),
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v_big_key, slot_big_key) = w_big_key
            .prepare_shared_surface(&tiny_rule, &mut shared_key)
            .unwrap();
        assert!(
            slot_big_key.is_none(),
            "oversized biome cache key must skip cache-slot allocation"
        );
        let mut w_big_key_plain = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: oversized_key,
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let big_key_plain_id = w_big_key_plain.add_rule(tiny_rule.clone()).unwrap();
        let mut v_big_key_plain = w_big_key_plain.surface_variants(big_key_plain_id).unwrap();
        let got = eval_band_rule(&mut w_big_key, &mut v_big_key, 1, 10, 1, 0);
        let want = eval_band_rule(&mut w_big_key_plain, &mut v_big_key_plain, 1, 10, 1, 0);
        assert_eq!(got, want);
        assert_eq!(got, Some(BlockStateId(42)));
        assert!(v_big_key.retained_cost(0) <= VARIANT_ENTRY_BUDGET);
        assert!(v_big_key.retained_cost(oversized_key_len) > VARIANT_ENTRY_BUDGET);
        w_big_key.finish_shared_surface(&mut shared_key, v_big_key, slot_big_key);
        assert!(
            shared_key.as_ref().unwrap().cached.is_empty(),
            "oversized biome cache key must not be retained"
        );

        let mut w_small_key = MaterialConditions::<LegacyRandom, _>::new(
            generation,
            root,
            BitsetSurface {
                possible_ids: vec![0b11, 0b1],
                biome_name: "minecraft:plains",
                biome_id: 0,
            },
        );
        let (mut v_small_key, slot_small_key) = w_small_key
            .prepare_shared_surface(&tiny_rule, &mut shared_key)
            .unwrap();
        assert!(slot_small_key.is_some());
        assert_eq!(
            eval_band_rule(&mut w_small_key, &mut v_small_key, 1, 10, 1, 0),
            Some(BlockStateId(42))
        );
        w_small_key.finish_shared_surface(&mut shared_key, v_small_key, slot_small_key);
        assert_eq!(
            shared_key.as_ref().unwrap().cached.len(),
            1,
            "normal 2-word biome cache key must be retained"
        );
    }
}
