//! Inline feature composition. Named holders, tags and registry reloads remain
//! separate from this bounded executable tree.
use super::block_predicate::PredicateBlockResolver;
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
    NeighborSpread {
        provider: super::state_provider::BlockStateProvider,
        neighbors: std::sync::Arc<std::collections::BTreeSet<crate::registry::BlockStateId>>,
        can_replace: super::block_predicate::BlockPredicate,
        attempts: super::value_provider::IntProvider,
        xz_offset: super::value_provider::IntProvider,
        y_offset: super::value_provider::IntProvider,
    },
    Pillar {
        provider: super::state_provider::BlockStateProvider,
        can_replace: super::block_predicate::BlockPredicate,
        direction: horizon_core::geometry::Direction,
        chance: f32,
        cap: Option<Box<PlacedFeatureDefinition>>,
    },
    BlockColumn(super::block_column::BlockColumnFeature),
    BlockPile(super::block_pile::BlockPileFeature),
    Disk(super::disk::DiskFeature),
    Spring(super::spring::SpringFeature),
    Vines(super::vines::VinesFeature),
    SnowAndFreeze(super::snow_freeze::SnowAndFreezeFeature),
    Delta(super::delta::DeltaFeature),
    Bamboo(super::bamboo::BambooFeature),
    ReplaceBlobs(super::replace_blobs::ReplaceBlobsFeature),
    EndPodium(super::end_podium::EndPodiumFeature),
    EndGateway(super::end_gateway::EndGatewayFeature),
    BonusChest(super::bonus_chest::BonusChestFeature),
    ChorusPlant(super::chorus_plant::ChorusPlantFeature),
    Speleothem(super::speleothem::SpeleothemFeature),
    Spike(super::spike::SpikeFeature),
    BlueIce(super::blue_ice::BlueIceFeature),
    UnderwaterMagma(super::underwater_magma::UnderwaterMagmaFeature),
    BlockBlob(super::block_blob::BlockBlobFeature),
    SimpleBlock(super::simple_block::SimpleBlockFeature),
    FillLayer(FillLayerFeature),
    ReplaceBlock(ReplaceBlockFeature),
    ScatteredOre(ScatteredOreFeature),
    Ore(super::ore_feature::OreFeature),
    Tree(Box<super::tree::TreeFeature>),
    VegetationPatch(Box<super::vegetation::VegetationPatchFeature>),
    Coral(Box<super::vegetation::CoralFeature>),
    HugeMushroom(Box<super::vegetation::HugeMushroomFeature>),
    MultifaceGrowth(Box<super::vegetation::MultifaceGrowthFeature>),
    Lake(Box<super::cave::LakeFeature>),
    MonsterRoom(Box<super::cave::MonsterRoomFeature>),
    Geode(Box<super::cave::GeodeFeature>),
    Iceberg(Box<super::iceberg::IcebergFeature>),
    RootSystem(Box<super::root_system::RootSystemFeature>),
    SpeleothemCluster(Box<super::dripstone::SpeleothemClusterFeature>),
    LargeDripstone(Box<super::dripstone::LargeDripstoneFeature>),
    HugeFungus(Box<super::fungus::HugeFungusFeature>),
    SculkPatch(Box<super::sculk::SculkPatchFeature>),
    EndSpike(Box<super::end_spike::EndSpikeFeature>),
    ProjectedPatchySquare(Box<super::projected::ProjectedPatchySquareFeature>),
    SteppedColumnCluster(Box<super::projected::SteppedColumnClusterFeature>),
    FallenTree(Box<super::tree::FallenTreeFeature>),
    NoOp,
    Template(Box<super::structure::features::TemplateFeature>),
    Fossil(Box<super::structure::features::FossilFeature>),
    WeightedChoice(OptionalWeightedList<PlacedFeatureDefinition>),
    Sequence(Vec<PlacedFeatureDefinition>),
    Overlay(Vec<PlacedFeatureDefinition>),
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
    Provider(super::state_provider::StateProviderError<E>),
    BlockColumn(super::block_column::BlockColumnError<E>),
    BlockPile(super::block_pile::BlockPileError<E>),
    Disk(super::disk::DiskError<E>),
    BlockBlob(super::block_blob::BlockBlobError<E>),
    SnowAndFreeze(super::snow_freeze::SnowAndFreezeError<E>),
    Delta(super::delta::DeltaError<E>),
    Bamboo(super::bamboo::BambooError<E>),
    ReplaceBlobs(super::replace_blobs::ReplaceBlobsError<E>),
    EndPodium(super::end_podium::EndPodiumError<E>),
    EndGateway(super::end_gateway::EndGatewayError<E>),
    BonusChest(super::bonus_chest::BonusChestError<E>),
    ChorusPlant(super::chorus_plant::ChorusPlantError<E>),
    Speleothem(super::speleothem::SpeleothemError<E>),
    Spike(super::spike::SpikeError<E>),
    BlueIce(super::blue_ice::BlueIceError<E>),
    UnderwaterMagma(super::underwater_magma::UnderwaterMagmaError<E>),
    SimpleBlock(super::simple_block::SimpleBlockError<E>),
    Random(InvalidBound),
    Query(PlacementQueryError),
    WorkLimit,
    InvalidPendingIndex,
    /// A block-state transition required by a feature is absent from the registry.
    InvalidState,
}
impl<E: std::fmt::Display> std::fmt::Display for FeatureExecutionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic(e) => e.fmt(f),
            Self::Provider(e) => e.fmt(f),
            Self::BlockColumn(e) => e.fmt(f),
            Self::BlockPile(e) => e.fmt(f),
            Self::Disk(e) => e.fmt(f),
            Self::BlockBlob(e) => e.fmt(f),
            Self::SnowAndFreeze(e) => e.fmt(f),
            Self::Delta(e) => e.fmt(f),
            Self::Bamboo(e) => e.fmt(f),
            Self::ReplaceBlobs(e) => e.fmt(f),
            Self::EndPodium(e) => e.fmt(f),
            Self::EndGateway(e) => e.fmt(f),
            Self::BonusChest(e) => e.fmt(f),
            Self::ChorusPlant(e) => e.fmt(f),
            Self::Speleothem(e) => e.fmt(f),
            Self::Spike(e) => e.fmt(f),
            Self::BlueIce(e) => e.fmt(f),
            Self::UnderwaterMagma(e) => e.fmt(f),
            Self::SimpleBlock(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("feature graph work limit exceeded"),
            Self::InvalidPendingIndex => f.write_str("invalid pending modifier index"),
            Self::InvalidState => f.write_str("feature block state outside registry"),
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
            "minecraft:template" => super::structure::features::TemplateFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("template bindings"))?,
            )
            .map(|f| Self::Template(Box::new(f))),
            "minecraft:fossil" => super::structure::features::FossilFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("fossil bindings"))?,
            )
            .map(|f| Self::Fossil(Box::new(f))),
            "minecraft:freeze_top_layer" => super::snow_freeze::SnowAndFreezeFeature::new(
                blocks.ok_or(DecodeError::Field("snow and freeze bindings"))?,
            )
            .map(Self::SnowAndFreeze),
            "minecraft:delta_feature" => super::delta::DeltaFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("delta bindings"))?,
                depth + 1,
                remaining,
            )
            .map(Self::Delta),
            "minecraft:bamboo" => super::bamboo::BambooFeature::from_json(
                json,
                blocks.ok_or(DecodeError::Field("bamboo bindings"))?,
            )
            .map(Self::Bamboo),
            "minecraft:netherrack_replace_blobs" => {
                super::replace_blobs::ReplaceBlobsFeature::decode(
                    json,
                    blocks.ok_or(DecodeError::Field("replace blobs bindings"))?,
                    depth + 1,
                    remaining,
                )
                .map(Self::ReplaceBlobs)
            }
            "minecraft:speleothem" => super::speleothem::SpeleothemFeature::from_json(
                json,
                blocks.ok_or(DecodeError::Field("speleothem bindings"))?,
            )
            .map(Self::Speleothem),
            "minecraft:chorus_plant" => super::chorus_plant::ChorusPlantFeature::new(
                blocks.ok_or(DecodeError::Field("chorus plant bindings"))?,
            )
            .map(Self::ChorusPlant),
            "minecraft:bonus_chest" => super::bonus_chest::BonusChestFeature::new(
                blocks.ok_or(DecodeError::Field("bonus chest bindings"))?,
            )
            .map(Self::BonusChest),
            "minecraft:end_gateway" => super::end_gateway::EndGatewayFeature::from_json(
                json,
                blocks.ok_or(DecodeError::Field("end gateway bindings"))?,
            )
            .map(Self::EndGateway),
            "minecraft:end_podium" => super::end_podium::EndPodiumFeature::from_json(
                json,
                blocks.ok_or(DecodeError::Field("end podium bindings"))?,
            )
            .map(Self::EndPodium),
            "minecraft:spike" => super::spike::SpikeFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("spike bindings"))?,
                depth + 1,
                remaining,
            )
            .map(Self::Spike),
            "minecraft:blue_ice" => super::blue_ice::BlueIceFeature::new(
                blocks.ok_or(DecodeError::Field("blue ice bindings"))?,
            )
            .map(Self::BlueIce),
            "minecraft:underwater_magma" => {
                super::underwater_magma::UnderwaterMagmaFeature::from_json(
                    json,
                    blocks.ok_or(DecodeError::Field("underwater magma bindings"))?,
                )
                .map(Self::UnderwaterMagma)
            }
            "minecraft:vines" => super::vines::VinesFeature::new(
                blocks.ok_or(DecodeError::Field("vine feature bindings"))?,
            )
            .map(Self::Vines),
            "minecraft:spring_feature" => super::spring::SpringFeature::from_json(
                json,
                blocks.ok_or(DecodeError::Field("spring bindings"))?,
            )
            .map(Self::Spring),
            "minecraft:block_blob" => super::block_blob::BlockBlobFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("block blob bindings"))?,
                depth,
                remaining,
            )
            .map(Self::BlockBlob),
            "minecraft:disk" => super::disk::DiskFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("disk bindings"))?,
                depth,
                remaining,
            )
            .map(Self::Disk),
            "minecraft:block_pile" => super::block_pile::BlockPileFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("block pile bindings"))?,
                depth,
                remaining,
            )
            .map(Self::BlockPile),
            "minecraft:random_neighbor_spread" => {
                let bindings =
                    blocks.ok_or(DecodeError::Field("neighbor spread block bindings"))?;
                let (providers, catalog, _) = bindings
                    .simple_blocks
                    .as_ref()
                    .ok_or(DecodeError::Field("neighbor spread provider bindings"))?;
                let provider = catalog.provider_with_budget(
                    required(&object, "block")?.get(),
                    providers,
                    depth + 1,
                    remaining,
                )?;
                let neighbors =
                    bindings.predicate_block_set(required(&object, "accepted_neighbors")?.get())?;
                let can_replace = super::block_predicate::BlockPredicate::decode_with_blocks(
                    required(&object, "can_replace")?.get(),
                    depth + 1,
                    remaining,
                    Some(bindings),
                )?;
                let mut integer = |name, low, high| {
                    let value = super::value_provider::IntProvider::decode(
                        required(&object, name)?,
                        depth + 1,
                        remaining,
                    )?;
                    let bounds = value.bounds();
                    if bounds[0] < low || bounds[1] > high {
                        return Err(DecodeError::Field("neighbor spread integer range"));
                    }
                    Ok(value)
                };
                let attempts = integer("attempts", 1, 3000)?;
                let xz_offset = integer("xz_offset", -16, 16)?;
                let y_offset = integer("y_offset", -16, 16)?;
                Ok(Self::NeighborSpread {
                    provider,
                    neighbors,
                    can_replace,
                    attempts,
                    xz_offset,
                    y_offset,
                })
            }
            "minecraft:single_block_pillar" => {
                let bindings =
                    blocks.ok_or(DecodeError::Field("pillar block bindings unavailable"))?;
                let (providers, catalog, _) = bindings
                    .simple_blocks
                    .as_ref()
                    .ok_or(DecodeError::Field("pillar provider bindings unavailable"))?;
                let provider = catalog.provider_with_budget(
                    required(&object, "block")?.get(),
                    providers,
                    depth + 1,
                    remaining,
                )?;
                let can_replace = match super::optional(&object, "can_replace") {
                    Some(raw) => super::block_predicate::BlockPredicate::decode_with_blocks(
                        raw.get(),
                        depth + 1,
                        remaining,
                        Some(bindings),
                    )?,
                    None => {
                        super::block_predicate::BlockPredicate::from_json(r#"{"type":"true"}"#)?
                    }
                };
                let direction =
                    match serde_json::from_str::<String>(required(&object, "direction")?.get())?
                        .as_str()
                    {
                        "up" => horizon_core::geometry::Direction::Up,
                        "down" => horizon_core::geometry::Direction::Down,
                        _ => return Err(DecodeError::Field("pillar vertical direction")),
                    };
                let chance = match super::optional(&object, "chance_to_continue") {
                    Some(raw) => number_text(raw)?
                        .parse::<f64>()
                        .map_err(|_| DecodeError::Field("pillar chance"))?
                        as f32,
                    None => 1.0,
                };
                if !(0.0..=1.0).contains(&chance) || chance.is_sign_negative() {
                    return Err(DecodeError::Field("pillar chance"));
                }
                let cap = super::optional(&object, "cap_feature")
                    .map(|raw| {
                        PlacedFeatureDefinition::decode(
                            raw.get(),
                            depth + 1,
                            remaining,
                            empty_tags,
                            blocks,
                        )
                        .map(Box::new)
                    })
                    .transpose()?;
                Ok(Self::Pillar {
                    provider,
                    can_replace,
                    direction,
                    chance,
                    cap,
                })
            }
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
            "minecraft:ore" => super::ore_feature::OreFeature::decode(
                json,
                blocks.ok_or_else(|| {
                    DecodeError::Registry("feature block bindings unavailable".into())
                })?,
                depth,
                remaining,
            )
            .map(Self::Ore),
            "minecraft:scattered_ore" => ScatteredOreFeature::decode(
                json,
                blocks.ok_or_else(|| {
                    DecodeError::Registry("feature block bindings unavailable".into())
                })?,
                depth,
                remaining,
            )
            .map(Self::ScatteredOre),
            "minecraft:vegetation_patch" | "minecraft:waterlogged_vegetation_patch" => {
                let bindings = blocks.ok_or(DecodeError::Field("vegetation patch bindings"))?;
                let nested = |json: &str, depth: usize, remaining: &mut usize| {
                    PlacedFeatureDefinition::decode(json, depth, remaining, empty_tags, blocks)
                };
                super::vegetation::VegetationPatchFeature::decode(
                    json, bindings, depth, remaining, &nested,
                )
                .map(|f| Self::VegetationPatch(Box::new(f)))
            }
            "minecraft:root_system" => {
                let bindings = blocks.ok_or(DecodeError::Field("root system bindings"))?;
                let nested = |json: &str, depth: usize, remaining: &mut usize| {
                    PlacedFeatureDefinition::decode(json, depth, remaining, empty_tags, blocks)
                };
                super::root_system::RootSystemFeature::decode(
                    json, bindings, depth, remaining, &nested,
                )
                .map(|f| Self::RootSystem(Box::new(f)))
            }
            "minecraft:coral_tree" | "minecraft:coral_claw" => {
                let nested = |json: &str, depth: usize, remaining: &mut usize| {
                    PlacedFeatureDefinition::decode(json, depth, remaining, empty_tags, blocks)
                };
                super::vegetation::CoralFeature::decode(json, depth, remaining, &nested)
                    .map(|f| Self::Coral(Box::new(f)))
            }
            "minecraft:huge_red_mushroom" | "minecraft:huge_brown_mushroom" => {
                super::vegetation::HugeMushroomFeature::decode(
                    json,
                    blocks.ok_or(DecodeError::Field("huge mushroom bindings"))?,
                    depth,
                    remaining,
                )
                .map(|f| Self::HugeMushroom(Box::new(f)))
            }
            "minecraft:lake" => super::cave::LakeFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("lake bindings"))?,
                depth,
                remaining,
            )
            .map(|f| Self::Lake(Box::new(f))),
            "minecraft:monster_room" => super::cave::MonsterRoomFeature::new(
                blocks.ok_or(DecodeError::Field("monster room bindings"))?,
            )
            .map(|f| Self::MonsterRoom(Box::new(f))),
            "minecraft:geode" => super::cave::GeodeFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("geode bindings"))?,
                depth,
                remaining,
            )
            .map(|f| Self::Geode(Box::new(f))),
            "minecraft:speleothem_cluster" => super::dripstone::SpeleothemClusterFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("speleothem cluster bindings"))?,
                depth,
                remaining,
            )
            .map(|f| Self::SpeleothemCluster(Box::new(f))),
            "minecraft:large_dripstone" => super::dripstone::LargeDripstoneFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("large dripstone bindings"))?,
                depth,
                remaining,
            )
            .map(|f| Self::LargeDripstone(Box::new(f))),
            "minecraft:projected_random_patchy_square" => {
                super::projected::ProjectedPatchySquareFeature::decode(
                    json,
                    blocks.ok_or(DecodeError::Field("patchy square bindings"))?,
                    depth,
                    remaining,
                )
                .map(|f| Self::ProjectedPatchySquare(Box::new(f)))
            }
            "minecraft:stepped_column_cluster" => {
                super::projected::SteppedColumnClusterFeature::decode(
                    json,
                    blocks.ok_or(DecodeError::Field("column cluster bindings"))?,
                    depth,
                    remaining,
                )
                .map(|f| Self::SteppedColumnCluster(Box::new(f)))
            }
            "minecraft:end_spike" => super::end_spike::EndSpikeFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("end spike bindings"))?,
            )
            .map(|f| Self::EndSpike(Box::new(f))),
            "minecraft:sculk_patch" => super::sculk::SculkPatchFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("sculk patch bindings"))?,
            )
            .map(|f| Self::SculkPatch(Box::new(f))),
            "minecraft:huge_fungus" => super::fungus::HugeFungusFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("huge fungus bindings"))?,
                depth,
                remaining,
            )
            .map(|f| Self::HugeFungus(Box::new(f))),
            "minecraft:iceberg" => super::iceberg::IcebergFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("iceberg bindings"))?,
            )
            .map(|f| Self::Iceberg(Box::new(f))),
            "minecraft:multiface_growth" => super::vegetation::MultifaceGrowthFeature::decode(
                json,
                blocks.ok_or(DecodeError::Field("multiface bindings"))?,
            )
            .map(|f| Self::MultifaceGrowth(Box::new(f))),
            "minecraft:tree" | "minecraft:fallen_tree" => {
                let bindings = blocks.ok_or(DecodeError::Field("tree bindings"))?;
                let nested = |json: &str, depth: usize, remaining: &mut usize| {
                    Self::decode(json, depth, remaining, empty_tags, blocks)
                };
                if kind == "minecraft:tree" {
                    super::tree::TreeFeature::decode(json, bindings, depth, remaining, &nested)
                        .map(|t| Self::Tree(Box::new(t)))
                } else {
                    super::tree::FallenTreeFeature::decode(
                        json, bindings, depth, remaining, &nested,
                    )
                    .map(|t| Self::FallenTree(Box::new(t)))
                }
            }
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
            "minecraft:sequence" | "minecraft:overlay" | "minecraft:simple_random_selector" => {
                let raw = required(&object, "features")?;
                let name = serde_json::from_str::<String>(raw.get()).ok();
                let named_empty = name
                    .as_deref()
                    .and_then(|s| s.strip_prefix('#'))
                    .map(identifier)
                    .transpose()?
                    .is_some_and(|name| empty_tags.contains(&name));
                if named_empty {
                    return Ok(match kind.as_str() {
                        "minecraft:sequence" => Self::Sequence(vec![]),
                        "minecraft:overlay" => Self::Overlay(vec![]),
                        _ => Self::RandomChoice(vec![]),
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
                Ok(match kind.as_str() {
                    "minecraft:sequence" => Self::Sequence(features),
                    "minecraft:overlay" => Self::Overlay(features),
                    _ => Self::RandomChoice(features),
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
            Self::Template(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Query),
            Self::Fossil(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Query),
            Self::SnowAndFreeze(feature) => feature
                .place(world, origin)
                .map_err(FeatureExecutionError::SnowAndFreeze),
            Self::Delta(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Delta),
            Self::Bamboo(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Bamboo),
            Self::ReplaceBlobs(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::ReplaceBlobs),
            Self::Speleothem(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Speleothem),
            Self::ChorusPlant(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::ChorusPlant),
            Self::BonusChest(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::BonusChest),
            Self::EndGateway(feature) => feature
                .place(world, origin)
                .map_err(FeatureExecutionError::EndGateway),
            Self::EndPodium(feature) => feature
                .place(world, origin)
                .map_err(FeatureExecutionError::EndPodium),
            Self::Spike(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Spike),
            Self::BlueIce(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::BlueIce),
            Self::UnderwaterMagma(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::UnderwaterMagma),
            Self::Vines(feature) => feature.place(world, origin).map_err(|e| match e {
                BlockFeatureError::World(e) => FeatureExecutionError::Basic(FeatureError::World(e)),
                BlockFeatureError::Query(e) => FeatureExecutionError::Query(e),
            }),
            Self::Spring(feature) => feature.place(world, origin).map_err(|e| match e {
                BlockFeatureError::World(e) => FeatureExecutionError::Basic(FeatureError::World(e)),
                BlockFeatureError::Query(e) => FeatureExecutionError::Query(e),
            }),
            Self::BlockBlob(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::BlockBlob),
            Self::Disk(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Disk),
            Self::BlockPile(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::BlockPile),
            Self::NeighborSpread {
                provider,
                neighbors,
                can_replace,
                attempts,
                xz_offset,
                y_offset,
            } => {
                let state = provider
                    .get_state(world, random, origin)
                    .map_err(FeatureExecutionError::Provider)?;
                world
                    .set_block(origin, state, 2)
                    .map_err(|e| FeatureExecutionError::Basic(FeatureError::World(e)))?;
                let count = attempts.sample(random)?;
                for _ in 0..count {
                    if run.remaining.get() == 0 {
                        return Err(FeatureExecutionError::WorkLimit);
                    }
                    run.remaining.set(run.remaining.get() - 1);
                    let dx = xz_offset.sample(random)?;
                    let dy = y_offset.sample(random)?;
                    let dz = xz_offset.sample(random)?;
                    let position = [
                        origin[0].wrapping_add(dx),
                        origin[1].wrapping_add(dy),
                        origin[2].wrapping_add(dz),
                    ];
                    if !can_replace
                        .test(world, position)
                        .map_err(FeatureExecutionError::Query)?
                    {
                        continue;
                    }
                    let mut found = 0;
                    for direction in horizon_core::geometry::Direction::ALL {
                        let step = direction.normal();
                        let neighbor = std::array::from_fn(|i| position[i].wrapping_add(step[i]));
                        let state = world
                            .block(neighbor)
                            .map_err(|e| FeatureExecutionError::Basic(FeatureError::World(e)))?;
                        found += i32::from(neighbors.contains(&state));
                        if found > 1 {
                            break;
                        }
                    }
                    if found == 1 {
                        let state = provider
                            .get_state(world, random, position)
                            .map_err(FeatureExecutionError::Provider)?;
                        world
                            .set_block(position, state, 2)
                            .map_err(|e| FeatureExecutionError::Basic(FeatureError::World(e)))?;
                    }
                }
                Ok(true)
            }
            Self::Pillar {
                provider,
                can_replace,
                direction,
                chance,
                cap,
            } => {
                let mut position = origin;
                let step = direction.normal();
                loop {
                    if run.remaining.get() == 0 {
                        return Err(FeatureExecutionError::WorkLimit);
                    }
                    run.remaining.set(run.remaining.get() - 1);
                    if !can_replace
                        .test(world, position)
                        .map_err(FeatureExecutionError::Query)?
                    {
                        break;
                    }
                    if random.next_float() >= *chance {
                        break;
                    }
                    if world
                        .outside_build_height(position[1])
                        .map_err(FeatureExecutionError::Query)?
                    {
                        break;
                    }
                    let state = provider
                        .get_state(world, random, position)
                        .map_err(FeatureExecutionError::Provider)?;
                    world
                        .set_block(position, state, 2)
                        .map_err(|e| FeatureExecutionError::Basic(FeatureError::World(e)))?;
                    position = std::array::from_fn(|i| position[i].wrapping_add(step[i]));
                }
                position = std::array::from_fn(|i| position[i].wrapping_sub(step[i]));
                if let Some(cap) = cap {
                    cap.place_in(&mut placer, world, random, position, run)?;
                }
                Ok(true)
            }
            Self::BlockColumn(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::BlockColumn),
            Self::SimpleBlock(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::SimpleBlock),
            Self::ScatteredOre(feature) => feature
                .place(world, random, origin)
                .map_err(FeatureExecutionError::Basic),
            Self::Ore(feature) => feature.place(world, random, origin),
            Self::VegetationPatch(feature) => {
                feature.place(world, random, origin, &mut |f, w, r, p| {
                    f.place_in(&mut placer, w, r, p, run)
                })
            }
            Self::Coral(feature) => feature.place(world, random, origin, &mut |f, w, r, p| {
                f.place_in(&mut placer, w, r, p, run)
            }),
            Self::HugeMushroom(feature) => feature.place(world, random, origin),
            Self::MultifaceGrowth(feature) => feature.place(world, random, origin),
            Self::Lake(feature) => feature.place(world, random, origin),
            Self::MonsterRoom(feature) => feature.place(world, random, origin),
            Self::Geode(feature) => feature.place(world, random, origin),
            Self::Iceberg(feature) => feature.place(world, random, origin),
            Self::SpeleothemCluster(feature) => feature.place(world, random, origin),
            Self::LargeDripstone(feature) => feature.place(world, random, origin),
            Self::HugeFungus(feature) => feature.place(world, random, origin),
            Self::SculkPatch(feature) => feature.place(world, random, origin),
            Self::EndSpike(feature) => feature.place(world, random, origin),
            Self::ProjectedPatchySquare(feature) => feature.place(world, random, origin),
            Self::SteppedColumnCluster(feature) => feature.place(world, random, origin),
            Self::RootSystem(feature) => feature.place(world, random, origin, &mut |f, w, r, p| {
                f.place_in(&mut placer, w, r, p, run)
            }),
            Self::Tree(feature) => feature.place(world, random, origin, &mut |f, w, r, p| {
                f.place_inner(w, r, p, run)
            }),
            Self::FallenTree(feature) => feature.place(world, random, origin, &mut |f, w, r, p| {
                f.place_inner(w, r, p, run)
            }),
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
            Self::Overlay(features) => {
                let mut any = false;
                for feature in features {
                    any |= feature.place_in(&mut placer, world, random, origin, run)?;
                }
                Ok(any)
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
