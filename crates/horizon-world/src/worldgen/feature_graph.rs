//! Inline feature composition. Named holders, tags and registry reloads remain
//! separate from this bounded executable tree.
use super::{
    DecodeError, Object,
    block_feature::{
        BlockFeatureError, FeatureBlockBindings, FillLayerFeature, ReplaceBlockFeature,
    },
    density::identifier,
    feature::{BasicFeature, FeatureBlocks, FeatureError, FeatureWorld},
    number_text,
    ore_feature::ScatteredOreFeature,
    placement::{
        FeaturePlacementError, FeaturePlacer, PlacementContext, PlacementModifier,
        PlacementQueryError,
    },
    required,
    weighted::OptionalWeightedList,
};
use horizon_core::random::{InvalidBound, RandomSource};
use serde_json::value::RawValue;
use std::{cell::Cell, collections::BTreeSet};

#[derive(Clone, Debug)]
pub struct PlacedFeatureDefinition {
    pub feature: Box<FeatureDefinition>,
    pub placement: Vec<PlacementModifier>,
}
#[derive(Clone, Debug)]
pub enum FeatureDefinition {
    Basic(BasicFeature),
    BlockColumn(super::block_column::BlockColumnFeature),
    SimpleBlock(super::simple_block::SimpleBlockFeature),
    FillLayer(FillLayerFeature),
    ReplaceBlock(ReplaceBlockFeature),
    ScatteredOre(ScatteredOreFeature),
    NoOp,
    WeightedChoice(OptionalWeightedList<PlacedFeatureDefinition>),
    Sequence(Vec<PlacedFeatureDefinition>),
    BooleanChoice(Box<[PlacedFeatureDefinition; 2]>),
    RandomChoice(Vec<PlacedFeatureDefinition>),
    ChanceChoice {
        features: Vec<(f32, PlacedFeatureDefinition)>,
        fallback: Box<PlacedFeatureDefinition>,
    },
}
#[derive(Debug)]
pub enum FeatureExecutionError<E> {
    Basic(FeatureError<E>),
    BlockColumn(super::block_column::BlockColumnError<E>),
    SimpleBlock(super::simple_block::SimpleBlockError<E>),
    Random(InvalidBound),
    Query(PlacementQueryError),
    WorkLimit,
    InvalidPendingIndex,
}
impl<E: std::fmt::Display> std::fmt::Display for FeatureExecutionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic(e) => e.fmt(f),
            Self::BlockColumn(e) => e.fmt(f),
            Self::SimpleBlock(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("feature graph work limit exceeded"),
            Self::InvalidPendingIndex => f.write_str("invalid pending modifier index"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for FeatureExecutionError<E> {}
impl<E> From<InvalidBound> for FeatureExecutionError<E> {
    fn from(error: InvalidBound) -> Self {
        Self::Random(error)
    }
}
impl<E> From<FeaturePlacementError<Self>> for FeatureExecutionError<E> {
    fn from(error: FeaturePlacementError<Self>) -> Self {
        match error {
            FeaturePlacementError::Feature(e) => e,
            FeaturePlacementError::Random(e) => Self::Random(e),
            FeaturePlacementError::World(e) => Self::Query(e),
            FeaturePlacementError::WorkLimit => Self::WorkLimit,
            FeaturePlacementError::InvalidPendingIndex => Self::InvalidPendingIndex,
        }
    }
}
struct Run {
    blocks: FeatureBlocks,
    depth: Cell<usize>,
    remaining: Cell<usize>,
}
fn visit(depth: usize, remaining: &mut usize) -> Result<(), DecodeError> {
    if depth > 128 || *remaining == 0 {
        return Err(DecodeError::Field("feature graph depth or node count"));
    }
    *remaining -= 1;
    Ok(())
}
fn list(raw: &RawValue) -> Result<Vec<Box<RawValue>>, DecodeError> {
    let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
    if values.len() > 8192 {
        return Err(DecodeError::Field("feature list"));
    }
    Ok(values)
}
impl PlacedFeatureDefinition {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536, &BTreeSet::new(), None)
    }
    pub fn from_json_with_blocks(
        json: &str,
        blocks: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536, &BTreeSet::new(), Some(blocks))
    }
    pub(crate) fn from_catalog_json(
        json: &str,
        empty_tags: &BTreeSet<String>,
        blocks: Option<&FeatureBlockBindings<'_>>,
    ) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536, empty_tags, blocks)
    }
    fn decode(
        json: &str,
        depth: usize,
        remaining: &mut usize,
        empty_tags: &BTreeSet<String>,
        blocks: Option<&FeatureBlockBindings<'_>>,
    ) -> Result<Self, DecodeError> {
        visit(depth, remaining)?;
        let object: Object = serde_json::from_str(json)?;
        let feature = Box::new(FeatureDefinition::decode(
            required(&object, "feature")?.get(),
            depth + 1,
            remaining,
            empty_tags,
            blocks,
        )?);
        let values = list(required(&object, "placement")?)?;
        if values.len() > 1024 {
            return Err(DecodeError::Field("feature modifier chain"));
        }
        let placement = values
            .iter()
            .map(|raw| {
                PlacementModifier::decode_with_blocks(
                    raw.get(),
                    depth + 1,
                    remaining,
                    blocks.map(|b| b as &dyn super::block_predicate::PredicateBlockResolver),
                )
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { feature, placement })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        blocks: FeatureBlocks,
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        self.place_in(
            &mut FeaturePlacer::new(),
            world,
            random,
            origin,
            &Run {
                blocks,
                depth: Cell::new(0),
                remaining: Cell::new(65536),
            },
        )
    }
    fn place_in<W: FeatureWorld + PlacementContext>(
        &self,
        placer: &mut FeaturePlacer,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        run: &Run,
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        if run.remaining.get() == 0 {
            return Err(FeatureExecutionError::WorkLimit);
        }
        run.remaining.set(run.remaining.get() - 1);
        Ok(placer.place_with_budget(
            &self.placement,
            world,
            random,
            origin,
            &run.remaining,
            |world, random, p| self.feature.place_inner(world, random, p, run),
        )?)
    }
}
impl FeatureDefinition {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536, &BTreeSet::new(), None)
    }
    pub fn from_json_with_blocks(
        json: &str,
        blocks: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536, &BTreeSet::new(), Some(blocks))
    }
    pub(crate) fn from_catalog_json(
        json: &str,
        empty_tags: &BTreeSet<String>,
        blocks: Option<&FeatureBlockBindings<'_>>,
    ) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536, empty_tags, blocks)
    }
    fn decode(
        json: &str,
        depth: usize,
        remaining: &mut usize,
        empty_tags: &BTreeSet<String>,
        blocks: Option<&FeatureBlockBindings<'_>>,
    ) -> Result<Self, DecodeError> {
        visit(depth, remaining)?;
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        match kind.as_str() {
            "minecraft:no_op" => Ok(Self::NoOp),
            "minecraft:block_column" => super::block_column::BlockColumnFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("block column bindings unavailable"))?,
                depth,
                remaining,
            )
            .map(Self::BlockColumn),
            "minecraft:simple_block" => {
                let (providers, catalog, placement) = blocks
                    .and_then(|blocks| blocks.simple_blocks.as_ref())
                    .ok_or(DecodeError::Field("simple block bindings unavailable"))?;
                super::simple_block::SimpleBlockFeature::decode(
                    json, providers, catalog, placement, depth, remaining,
                )
                .map(Self::SimpleBlock)
            }
            "minecraft:fill_layer" => FillLayerFeature::from_json(
                json,
                blocks.ok_or_else(|| {
                    DecodeError::Registry("feature block bindings unavailable".into())
                })?,
            )
            .map(Self::FillLayer),
            "minecraft:replace_single_block" => ReplaceBlockFeature::decode(
                json,
                blocks.ok_or_else(|| {
                    DecodeError::Registry("feature block bindings unavailable".into())
                })?,
                depth,
                remaining,
            )
            .map(Self::ReplaceBlock),
            "minecraft:scattered_ore" => ScatteredOreFeature::decode(
                json,
                blocks.ok_or_else(|| {
                    DecodeError::Registry("feature block bindings unavailable".into())
                })?,
                depth,
                remaining,
            )
            .map(Self::ScatteredOre),
            "minecraft:weighted_random_selector" => Ok(Self::WeightedChoice(
                OptionalWeightedList::decode(required(&object, "features")?, |raw| {
                    PlacedFeatureDefinition::decode(
                        raw.get(),
                        depth + 1,
                        remaining,
                        empty_tags,
                        blocks,
                    )
                })?,
            )),
            "minecraft:sequence" | "minecraft:simple_random_selector" => {
                let raw = required(&object, "features")?;
                let name = serde_json::from_str::<String>(raw.get()).ok();
                let named_empty = name
                    .as_deref()
                    .and_then(|s| s.strip_prefix('#'))
                    .map(identifier)
                    .transpose()?
                    .is_some_and(|name| empty_tags.contains(&name));
                if named_empty {
                    return Ok(if kind == "minecraft:sequence" {
                        Self::Sequence(vec![])
                    } else {
                        Self::RandomChoice(vec![])
                    });
                }
                let values = if raw.get().trim_start().starts_with('[') {
                    list(raw)?
                } else {
                    vec![raw.to_owned()]
                };
                if values.is_empty() {
                    return Err(DecodeError::Field("empty feature list"));
                }
                let features = values
                    .iter()
                    .map(|raw| {
                        PlacedFeatureDefinition::decode(
                            raw.get(),
                            depth + 1,
                            remaining,
                            empty_tags,
                            blocks,
                        )
                    })
                    .collect::<Result<_, _>>()?;
                Ok(if kind == "minecraft:sequence" {
                    Self::Sequence(features)
                } else {
                    Self::RandomChoice(features)
                })
            }
            "minecraft:random_boolean_selector" => Ok(Self::BooleanChoice(Box::new([
                PlacedFeatureDefinition::decode(
                    required(&object, "feature_true")?.get(),
                    depth + 1,
                    remaining,
                    empty_tags,
                    blocks,
                )?,
                PlacedFeatureDefinition::decode(
                    required(&object, "feature_false")?.get(),
                    depth + 1,
                    remaining,
                    empty_tags,
                    blocks,
                )?,
            ]))),
            "minecraft:random_selector" => {
                let values = list(required(&object, "features")?)?;
                let mut features = Vec::with_capacity(values.len());
                for raw in values {
                    let entry: Object = serde_json::from_str(raw.get())?;
                    let feature = PlacedFeatureDefinition::decode(
                        required(&entry, "feature")?.get(),
                        depth + 1,
                        remaining,
                        empty_tags,
                        blocks,
                    )?;
                    let chance: f32 = number_text(required(&entry, "chance")?)?
                        .parse()
                        .map_err(|_| DecodeError::Field("feature chance"))?;
                    if !(0. ..=1.).contains(&chance) || chance.is_sign_negative() {
                        return Err(DecodeError::Field("feature chance"));
                    }
                    features.push((chance, feature));
                }
                Ok(Self::ChanceChoice {
                    features,
                    fallback: Box::new(PlacedFeatureDefinition::decode(
                        required(&object, "default")?.get(),
                        depth + 1,
                        remaining,
                        empty_tags,
                        blocks,
                    )?),
                })
            }
            _ => BasicFeature::from_json(json).map(Self::Basic),
        }
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        blocks: FeatureBlocks,
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        self.place_inner(
            world,
            random,
            origin,
            &Run {
                blocks,
                depth: Cell::new(0),
                remaining: Cell::new(65536),
            },
        )
    }
    fn place_inner<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        run: &Run,
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        if run.depth.get() >= 128 || run.remaining.get() == 0 {
            return Err(FeatureExecutionError::WorkLimit);
        }
        run.depth.set(run.depth.get() + 1);
        run.remaining.set(run.remaining.get() - 1);
        let result = self.evaluate(world, random, origin, run);
        run.depth.set(run.depth.get() - 1);
        result
    }
    fn evaluate<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
        run: &Run,
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        let mut placer = FeaturePlacer::new();
        match self {
            Self::Basic(feature) => feature
                .place(world, random, origin, run.blocks)
                .map_err(FeatureExecutionError::Basic),
            Self::NoOp => Ok(true),
            Self::BlockColumn(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::BlockColumn),
            Self::SimpleBlock(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::SimpleBlock),
            Self::ScatteredOre(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Basic),
            Self::FillLayer(feature) => feature.place(world, origin).map_err(|e| match e {
                BlockFeatureError::World(e) => FeatureExecutionError::Basic(FeatureError::World(e)),
                BlockFeatureError::Query(e) => FeatureExecutionError::Query(e),
            }),
            Self::ReplaceBlock(feature) => {
                feature.place(world, random, origin).map_err(|e| match e {
                    BlockFeatureError::World(e) => {
                        FeatureExecutionError::Basic(FeatureError::World(e))
                    }
                    BlockFeatureError::Query(e) => FeatureExecutionError::Query(e),
                })
            }
            Self::WeightedChoice(features) => match features.sample(random)? {
                Some(feature) => feature.place_in(&mut placer, world, random, origin, run),
                None => Ok(false),
            },
            Self::Sequence(features) => {
                for feature in features {
                    if !feature.place_in(&mut placer, world, random, origin, run)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::BooleanChoice(features) => {
                let index = if random.next_boolean() { 0 } else { 1 };
                features[index].place_in(&mut placer, world, random, origin, run)
            }
            Self::RandomChoice(features) => {
                let index = random.next_int_bound(features.len() as i32)? as usize;
                features[index].place_in(&mut placer, world, random, origin, run)
            }
            Self::ChanceChoice { features, fallback } => {
                for (chance, feature) in features {
                    if random.next_float() < *chance {
                        return feature.place_in(&mut placer, world, random, origin, run);
                    }
                }
                fallback.place_in(&mut placer, world, random, origin, run)
            }
        }
    }
}
