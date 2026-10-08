//! Structure processors (`worldgen/processor_list`) and template placement.
use super::{BlockInfo, BoundingBox, Mirror, Rotation, StructureResources, StructureTemplate};
use crate::{
    fluid::FluidKind,
    heightmap::HeightmapType,
    nbt::{Compound, JavaString, Tag},
    registry::BlockStateId,
    worldgen::{
        DecodeError,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        rule_test::RuleTest,
        value_provider::IntProvider,
    },
};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, PositionalRandom, RandomSource, position_seed},
};
use serde_json::Value;
use std::{collections::BTreeSet, sync::Arc};

/// The draws processors make from StructurePlaceSettings' random.
pub trait ProcessorRandom {
    fn float(&mut self) -> f32;
    fn int(&mut self, bound: i32) -> i32;
    fn long(&mut self) -> i64;
}
impl<R: RandomSource> ProcessorRandom for R {
    fn float(&mut self) -> f32 {
        self.next_float()
    }
    fn int(&mut self, bound: i32) -> i32 {
        self.next_int_bound(bound).unwrap_or(0)
    }
    fn long(&mut self) -> i64 {
        self.next_long()
    }
}

#[derive(Clone, Debug)]
enum PosRule {
    Always,
    Linear {
        min_chance: f32,
        max_chance: f32,
        min_dist: i32,
        max_dist: i32,
        axis: Option<usize>,
    },
}
#[derive(Clone, Debug)]
enum Modifier {
    Passthrough,
    Clear,
    AppendLoot(String),
    AppendStatic(Compound),
}
#[derive(Clone, Debug)]
pub struct Rule {
    input: RuleTest,
    location: RuleTest,
    position: PosRule,
    output: BlockStateId,
    modifier: Modifier,
}
#[derive(Clone, Debug)]
pub enum Processor {
    Nop,
    BlockIgnore(BTreeSet<String>),
    BlockRot {
        rottable: Option<Arc<BTreeSet<BlockStateId>>>,
        integrity: f32,
    },
    Gravity {
        heightmap: HeightmapType,
        offset: i32,
    },
    Rule(Vec<Rule>),
    Protected(Arc<BTreeSet<BlockStateId>>),
    Capped {
        delegate: Box<Processor>,
        limit: IntProvider,
    },
    JigsawReplacement,
    BlockAge(f32),
    LavaSubmerged,
}
#[derive(Clone, Debug, Default)]
pub struct ProcessorList(pub Vec<Processor>);

fn holder_set(
    value: &Value,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<Arc<BTreeSet<BlockStateId>>, DecodeError> {
    let names: Vec<&str> = match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return Err(DecodeError::Field("block holder set")),
    };
    let mut out = BTreeSet::new();
    for name in names {
        if let Some(tag) = name.strip_prefix('#') {
            let tag = super::super::tags::identifier(tag)?;
            out.extend(
                bindings
                    .tags
                    .get(&tag)
                    .map(|s| s.iter().copied())
                    .into_iter()
                    .flatten(),
            );
        } else {
            let name = super::super::tags::identifier(name)?;
            out.extend(
                bindings
                    .blocks
                    .get(&name)
                    .ok_or_else(|| DecodeError::Registry(format!("unknown block {name}")))?
                    .iter()
                    .copied(),
            );
        }
    }
    Ok(Arc::new(out))
}
fn json_to_nbt(value: &Value) -> Tag {
    match value {
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) if i32::try_from(i).is_ok() => Tag::Int(i as i32),
            Some(i) => Tag::Long(i),
            None => Tag::Double(n.as_f64().unwrap_or(0.).to_bits()),
        },
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(a) => Tag::List(a.iter().map(json_to_nbt).collect()),
        Value::Object(o) => Tag::Compound(
            o.iter()
                .map(|(k, v)| (JavaString::from(k.as_str()), json_to_nbt(v)))
                .collect(),
        ),
        Value::Null => Tag::End,
    }
}
fn float(value: &Value, key: &'static str) -> Result<f32, DecodeError> {
    value[key]
        .as_f64()
        .map(|v| v as f32)
        .ok_or(DecodeError::Field(key))
}
impl Processor {
    fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let kind = value["processor_type"]
            .as_str()
            .ok_or(DecodeError::Field("processor_type"))?;
        let state = |v: &Value| -> Result<BlockStateId, DecodeError> {
            bindings
                .codec
                .decode(&v.to_string(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))
        };
        Ok(match super::super::tags::identifier(kind)?.as_str() {
            "minecraft:nop" => Self::Nop,
            "minecraft:block_ignore" => Self::BlockIgnore(
                value["blocks"]
                    .as_array()
                    .ok_or(DecodeError::Field("block_ignore blocks"))?
                    .iter()
                    .map(|b| state(b).map(|id| resources.registry.state(id).unwrap().name.clone()))
                    .collect::<Result<_, _>>()?,
            ),
            "minecraft:block_rot" => Self::BlockRot {
                rottable: match value.get("rottable_blocks") {
                    Some(v) => Some(holder_set(v, bindings)?),
                    None => None,
                },
                integrity: float(value, "integrity")?,
            },
            "minecraft:gravity" => Self::Gravity {
                heightmap: match value.get("heightmap").and_then(Value::as_str) {
                    Some(name) => HeightmapType::ALL
                        .into_iter()
                        .find(|k| k.name() == name)
                        .ok_or(DecodeError::Field("gravity heightmap"))?,
                    None => HeightmapType::WorldSurfaceWg,
                },
                offset: value.get("offset").and_then(Value::as_i64).unwrap_or(0) as i32,
            },
            "minecraft:protected_blocks" => Self::Protected(holder_set(&value["value"], bindings)?),
            "minecraft:jigsaw_replacement" => Self::JigsawReplacement,
            "minecraft:block_age" => Self::BlockAge(float(value, "mossiness")?),
            "minecraft:lava_submerged_block" => Self::LavaSubmerged,
            "minecraft:capped" => Self::Capped {
                delegate: Box::new(Self::decode(&value["delegate"], resources, bindings)?),
                limit: IntProvider::from_json(&value["limit"].to_string())?,
            },
            "minecraft:rule" => Self::Rule(
                value["rules"]
                    .as_array()
                    .ok_or(DecodeError::Field("rules"))?
                    .iter()
                    .map(|rule| {
                        let test = |key| RuleTest::from_json(&rule[key].to_string(), bindings);
                        let position = match rule.get("position_predicate") {
                            Some(p) => match p["predicate_type"]
                                .as_str()
                                .unwrap_or("minecraft:always_true")
                            {
                                "minecraft:always_true" => PosRule::Always,
                                kind @ ("minecraft:linear_pos"
                                | "minecraft:axis_aligned_linear_pos") => PosRule::Linear {
                                    min_chance: p
                                        .get("min_chance")
                                        .and_then(Value::as_f64)
                                        .unwrap_or(0.)
                                        as f32,
                                    max_chance: p
                                        .get("max_chance")
                                        .and_then(Value::as_f64)
                                        .unwrap_or(0.)
                                        as f32,
                                    min_dist: p.get("min_dist").and_then(Value::as_i64).unwrap_or(0)
                                        as i32,
                                    max_dist: p.get("max_dist").and_then(Value::as_i64).unwrap_or(0)
                                        as i32,
                                    axis: (kind == "minecraft:axis_aligned_linear_pos").then(
                                        || match p.get("axis").and_then(Value::as_str) {
                                            Some("x") => 0,
                                            Some("z") => 2,
                                            _ => 1,
                                        },
                                    ),
                                },
                                _ => return Err(DecodeError::Field("position predicate")),
                            },
                            None => PosRule::Always,
                        };
                        let modifier = match rule.get("block_entity_modifier") {
                            None => Modifier::Passthrough,
                            Some(m) => match m["type"].as_str().unwrap_or("") {
                                "minecraft:passthrough" => Modifier::Passthrough,
                                "minecraft:clear" => Modifier::Clear,
                                "minecraft:append_loot" => Modifier::AppendLoot(
                                    m["loot_table"].as_str().unwrap_or_default().to_owned(),
                                ),
                                "minecraft:append_static" => match json_to_nbt(&m["data"]) {
                                    Tag::Compound(c) => Modifier::AppendStatic(c),
                                    _ => Modifier::AppendStatic(Compound::new()),
                                },
                                _ => return Err(DecodeError::Field("block entity modifier")),
                            },
                        };
                        Ok(Rule {
                            input: test("input_predicate")?,
                            location: test("location_predicate")?,
                            position,
                            output: state(&rule["output_state"])?,
                            modifier,
                        })
                    })
                    .collect::<Result<_, DecodeError>>()?,
            ),
            _ => return Err(DecodeError::Field("structure processor type")),
        })
    }
    fn evaluates_entire_piece(&self) -> bool {
        matches!(self, Self::Capped { .. })
    }
}
impl ProcessorList {
    /// A processor list holder: a registry name, `{"processors": [...]}` or a list.
    pub fn decode(
        json: &str,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let value: Value = serde_json::from_str(json)?;
        let value = match &value {
            Value::String(name) => {
                let name = super::super::tags::identifier(name)?;
                let raw = resources.processor_lists.get(&name).ok_or_else(|| {
                    DecodeError::Registry(format!("unknown processor list {name}"))
                })?;
                serde_json::from_str(raw)?
            }
            other => other.clone(),
        };
        let list = match &value {
            Value::Array(a) => a.clone(),
            Value::Object(_) => value["processors"].as_array().cloned().unwrap_or_default(),
            _ => return Err(DecodeError::Field("processor list")),
        };
        Ok(Self(
            list.iter()
                .map(|p| Processor::decode(p, resources, bindings))
                .collect::<Result<_, _>>()?,
        ))
    }
}

/// StructurePlaceSettings without entity options.
pub struct PlaceSettings<'a> {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub pivot: [i32; 3],
    pub bounding_box: Option<BoundingBox>,
    pub processors: Vec<&'a Processor>,
    pub waterlogging: bool,
    pub random: Option<&'a mut dyn ProcessorRandom>,
}
impl<'a> PlaceSettings<'a> {
    pub fn new(rotation: Rotation) -> Self {
        Self {
            mirror: Mirror::None,
            rotation,
            pivot: [0; 3],
            bounding_box: None,
            processors: Vec::new(),
            waterlogging: true,
            random: None,
        }
    }
    pub fn with_processors(mut self, list: &'a ProcessorList) -> Self {
        self.processors.extend(list.0.iter());
        self
    }
    /// settings.getRandom(pos): the shared random, else a position-seeded one.
    fn with_random<T>(
        &mut self,
        pos: [i32; 3],
        f: impl FnOnce(&mut dyn ProcessorRandom) -> T,
    ) -> T {
        match self.random.as_deref_mut() {
            Some(random) => f(random),
            None => f(&mut LegacyRandom::new(position_seed(BlockPos::new(
                pos[0], pos[1], pos[2],
            )))),
        }
    }
}

type ProcessResult = Result<Option<BlockInfo>, PlacementQueryError>;
#[allow(clippy::too_many_arguments)]
fn process_block<W: PlacementContext>(
    processor: &Processor,
    world: &mut W,
    resources: &StructureResources,
    reference: [i32; 3],
    relative: [i32; 3],
    info: BlockInfo,
    settings: &mut PlaceSettings<'_>,
) -> ProcessResult {
    let registry = resources.registry;
    let name = |id: BlockStateId| registry.state(id).map(|s| s.name.as_str()).unwrap_or("");
    Ok(match processor {
        Processor::Nop => Some(info),
        Processor::BlockIgnore(blocks) => (!blocks.contains(name(info.state))).then_some(info),
        Processor::BlockRot {
            rottable,
            integrity,
        } => {
            let candidate = rottable
                .as_ref()
                .is_none_or(|set| set.contains(&info.state));
            let keep = settings.with_random(info.pos, |r| !candidate || r.float() <= *integrity);
            keep.then_some(info)
        }
        Processor::Gravity { heightmap, offset } => {
            let height = world.height(*heightmap, info.pos[0], info.pos[2])? + offset;
            Some(BlockInfo {
                pos: [info.pos[0], height + relative[1], info.pos[2]],
                ..info
            })
        }
        Processor::Protected(set) => (!set.contains(&world.block_state(info.pos)?)).then_some(info),
        Processor::JigsawReplacement => {
            if !resources.is_jigsaw(info.state) {
                return Ok(Some(info));
            }
            let Some(nbt) = &info.nbt else {
                return Ok(Some(info));
            };
            let text = match nbt.get(&JavaString::from("final_state")) {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => "minecraft:air".to_owned(),
            };
            let Some(state) = super::template::parse_state_string(registry, &text) else {
                return Ok(None);
            };
            (name(state) != "minecraft:structure_void").then_some(BlockInfo {
                pos: info.pos,
                state,
                nbt: None,
            })
        }
        Processor::LavaSubmerged => {
            let lava = name(world.block_state(info.pos)?) == "minecraft:lava";
            if lava && !resources.transforms.full_block(info.state) {
                Some(BlockInfo {
                    state: registry.default_state("minecraft:lava").unwrap(),
                    ..info
                })
            } else {
                Some(info)
            }
        }
        Processor::BlockAge(mossiness) => block_age(*mossiness, info, settings, resources),
        Processor::Rule(rules) => {
            let mut random = LegacyRandom::new(position_seed(BlockPos::new(
                info.pos[0],
                info.pos[1],
                info.pos[2],
            )));
            for rule in rules {
                if !rule.input.test(info.state, info.pos, &mut random) {
                    continue;
                }
                let located = world.block_state(info.pos)?;
                if !rule.location.test(located, info.pos, &mut random) {
                    continue;
                }
                let position_ok = match &rule.position {
                    PosRule::Always => true,
                    PosRule::Linear {
                        min_chance,
                        max_chance,
                        min_dist,
                        max_dist,
                        axis,
                    } => {
                        let dist = match axis {
                            None => (0..3)
                                .map(|i| (info.pos[i] - reference[i]).abs())
                                .sum::<i32>(),
                            Some(a) => {
                                let d: [f32; 3] = std::array::from_fn(|i| {
                                    if i == *a {
                                        ((info.pos[i] - reference[i]) as f32).abs()
                                    } else {
                                        0.
                                    }
                                });
                                (d[0] + d[1] + d[2]) as i32
                            }
                        };
                        let roll = random.next_float();
                        let t = (dist - min_dist) as f32 / (max_dist - min_dist) as f32;
                        let chance = if t < 0. {
                            *min_chance
                        } else if t > 1. {
                            *max_chance
                        } else {
                            min_chance + t * (max_chance - min_chance)
                        };
                        roll <= chance
                    }
                };
                if !position_ok {
                    continue;
                }
                let nbt = match &rule.modifier {
                    Modifier::Passthrough => info.nbt.clone(),
                    Modifier::Clear => Some(Compound::new()),
                    Modifier::AppendLoot(table) => {
                        let mut nbt = info.nbt.clone().unwrap_or_default();
                        nbt.insert("LootTable".into(), Tag::String(table.as_str().into()));
                        nbt.insert("LootTableSeed".into(), Tag::Long(random.next_long()));
                        Some(nbt)
                    }
                    Modifier::AppendStatic(data) => {
                        let mut nbt = info.nbt.clone().unwrap_or_default();
                        nbt.extend(data.clone());
                        Some(nbt)
                    }
                };
                return Ok(Some(BlockInfo {
                    pos: info.pos,
                    state: rule.output,
                    nbt,
                }));
            }
            Some(info)
        }
        Processor::Capped { .. } => Some(info),
    })
}
fn block_age(
    mossiness: f32,
    info: BlockInfo,
    settings: &mut PlaceSettings<'_>,
    resources: &StructureResources,
) -> Option<BlockInfo> {
    let registry = resources.registry;
    let state = registry.state(info.state)?.clone();
    let default = |n: &str| registry.default_state(n).unwrap();
    let with_props = |n: &str| {
        let mut target = registry.state(default(n)).unwrap().clone();
        for (k, v) in &state.properties {
            if target.properties.contains_key(k) {
                target.properties.insert(k.clone(), v.clone());
            }
        }
        registry.id(&target).unwrap_or(default(n))
    };
    let random_stairs = |r: &mut dyn ProcessorRandom, n: &str| {
        let facing = ["north", "east", "south", "west"][r.int(4) as usize];
        let half = ["top", "bottom"][r.int(2) as usize];
        let mut target = registry.state(default(n)).unwrap().clone();
        target.properties.insert("facing".into(), facing.into());
        target.properties.insert("half".into(), half.into());
        registry.id(&target).unwrap()
    };
    let name = state.name.as_str();
    let is_tag = |tag: &str| {
        // Tags are matched by block name through the template resources' bindings.
        resources
            .block_tags
            .get(tag)
            .is_some_and(|set| set.contains(name))
    };
    let new_state = settings.with_random(info.pos, |r| -> Option<BlockStateId> {
        let pick = |r: &mut dyn ProcessorRandom, non: &[BlockStateId], mossy: &[BlockStateId]| {
            if r.float() < mossiness {
                mossy[r.int(mossy.len() as i32) as usize]
            } else {
                non[r.int(non.len() as i32) as usize]
            }
        };
        if matches!(
            name,
            "minecraft:stone_bricks" | "minecraft:stone" | "minecraft:chiseled_stone_bricks"
        ) {
            if r.float() >= 0.5 {
                return None;
            }
            let non = [
                default("minecraft:cracked_stone_bricks"),
                random_stairs(r, "minecraft:stone_brick_stairs"),
            ];
            let mossy = [
                default("minecraft:mossy_stone_bricks"),
                random_stairs(r, "minecraft:mossy_stone_brick_stairs"),
            ];
            Some(pick(r, &non, &mossy))
        } else if is_tag("minecraft:stairs") {
            if r.float() >= 0.5 {
                return None;
            }
            let non = [
                default("minecraft:stone_slab"),
                default("minecraft:stone_brick_slab"),
            ];
            let mossy = [
                with_props("minecraft:mossy_stone_brick_stairs"),
                default("minecraft:mossy_stone_brick_slab"),
            ];
            Some(pick(r, &non, &mossy))
        } else if is_tag("minecraft:slabs") {
            (r.float() < mossiness).then(|| with_props("minecraft:mossy_stone_brick_slab"))
        } else if is_tag("minecraft:walls") {
            (r.float() < mossiness).then(|| with_props("minecraft:mossy_stone_brick_wall"))
        } else if name == "minecraft:obsidian" {
            (r.float() < 0.15).then(|| default("minecraft:crying_obsidian"))
        } else {
            None
        }
    });
    Some(match new_state {
        Some(state) => BlockInfo { state, ..info },
        None => info,
    })
}

/// StructureTemplate.processBlockInfos.
fn process_infos<W: PlacementContext>(
    world: &mut W,
    resources: &StructureResources,
    template: &StructureTemplate,
    palette: usize,
    position: [i32; 3],
    reference: [i32; 3],
    settings: &mut PlaceSettings<'_>,
) -> Result<Vec<BlockInfo>, PlacementQueryError> {
    let whole = settings
        .processors
        .iter()
        .any(|p| p.evaluates_entire_piece());
    let mut original = Vec::new();
    let mut processed = Vec::new();
    for info in template.palettes()[palette].blocks() {
        let relative = StructureTemplate::transform(
            info.pos,
            settings.mirror,
            settings.rotation,
            settings.pivot,
        );
        let pos = std::array::from_fn(|i| relative[i] + position[i]);
        if !whole && settings.bounding_box.is_some_and(|b| !b.is_inside(pos)) {
            continue;
        }
        let mut current = Some(BlockInfo {
            pos,
            state: info.state,
            nbt: info.nbt.clone(),
        });
        let processors = settings.processors.clone();
        for processor in processors {
            let Some(value) = current.take() else { break };
            current = process_block(
                processor, world, resources, reference, info.pos, value, settings,
            )?;
        }
        if let Some(value) = current {
            processed.push(value);
            original.push(info.clone());
        }
    }
    let processors = settings.processors.clone();
    for processor in processors {
        let Processor::Capped { delegate, limit } = processor else {
            continue;
        };
        if limit.bounds()[1] == 0 || processed.is_empty() || original.len() != processed.len() {
            continue;
        }
        let seed = world.world_seed()?;
        let mut random = LegacyRandom::new(seed).fork_positional().at(BlockPos::new(
            position[0],
            position[1],
            position[2],
        ));
        let max = limit
            .sample(&mut random)
            .unwrap_or(0)
            .min(processed.len() as i32);
        if max < 1 {
            continue;
        }
        let mut indices: Vec<usize> = (0..processed.len()).collect();
        for i in (2..=indices.len()).rev() {
            let swap = random.next_int_bound(i as i32).unwrap_or(0) as usize;
            indices.swap(i - 1, swap);
        }
        let mut replaced = 0;
        for index in indices {
            if replaced >= max {
                break;
            }
            let before = processed[index].clone();
            let altered = process_block(
                delegate,
                world,
                resources,
                reference,
                original[index].pos,
                before.clone(),
                settings,
            )?;
            if let Some(altered) = altered {
                if altered != before {
                    replaced += 1;
                    processed[index] = altered;
                }
            }
        }
    }
    Ok(processed)
}

/// StructureTemplate.placeInWorld without entities or neighbor shape updates.
/// Returns false for empty templates.
#[allow(clippy::too_many_arguments)]
pub fn place_in_world<W: FeatureWorld + PlacementContext>(
    world: &mut W,
    resources: &StructureResources,
    template: &StructureTemplate,
    position: [i32; 3],
    reference: [i32; 3],
    settings: &mut PlaceSettings<'_>,
    mut loot_random: Option<&mut dyn ProcessorRandom>,
    flags: i32,
) -> Result<bool, PlacementQueryError> {
    if template.palettes().is_empty() {
        return Ok(false);
    }
    let palette = match settings.random.as_deref_mut() {
        Some(r) => r.int(template.palettes().len() as i32) as usize,
        None => LegacyRandom::new(position_seed(BlockPos::new(
            position[0],
            position[1],
            position[2],
        )))
        .next_int_bound(template.palettes().len() as i32)
        .unwrap_or(0) as usize,
    };
    if template.palettes()[palette].blocks().is_empty() || template.size.iter().any(|s| *s < 1) {
        return Ok(false);
    }
    let infos = process_infos(
        world, resources, template, palette, position, reference, settings,
    )?;
    let barrier = resources
        .registry
        .default_state("minecraft:barrier")
        .unwrap();
    let mut to_fill = Vec::new();
    let mut locked = Vec::new();
    let world_err = |_| PlacementQueryError::Failed("template write".into());
    for info in infos {
        if settings
            .bounding_box
            .is_some_and(|b| !b.is_inside(info.pos))
        {
            continue;
        }
        let previous = if settings.waterlogging {
            Some(world.fluid_state(info.pos)?)
        } else {
            None
        };
        let state = resources.transforms.rotate(
            resources.transforms.mirror(info.state, settings.mirror),
            settings.rotation,
        );
        if info.nbt.is_some() {
            world.set_block(info.pos, barrier, 820).map_err(world_err)?;
        }
        if !world.set_block(info.pos, state, flags).map_err(world_err)? {
            continue;
        }
        if let Some(mut nbt) = info.nbt {
            if resources.transforms.loot_container(state) {
                let seed = match loot_random.as_deref_mut() {
                    Some(random) => random.long(),
                    None => settings.random.as_deref_mut().map_or(0, |r| r.long()),
                };
                nbt.insert("LootTableSeed".into(), Tag::Long(seed));
            }
            world.set_block_entity(info.pos, nbt)?;
        }
        if let Some(previous) = previous {
            if resources.source_fluid(state) {
                locked.push(info.pos);
            } else if let Some(wet) = resources.waterlogged.get(&state) {
                if previous.kind() == FluidKind::Water {
                    world.set_block(info.pos, *wet, 3).map_err(world_err)?;
                    world.schedule_fluid_tick(info.pos, previous.kind(), 5).ok();
                } else if !previous.is_source() {
                    to_fill.push(info.pos);
                }
            }
        }
    }
    let mut filled = true;
    while filled && !to_fill.is_empty() {
        filled = false;
        let mut remaining = Vec::new();
        for pos in std::mem::take(&mut to_fill) {
            let mut fluid = world.fluid_state(pos)?;
            for d in [[0, 1, 0], [0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]] {
                if fluid.is_source() {
                    break;
                }
                let n = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                let neighbor = world.fluid_state(n)?;
                if neighbor.is_source() && !locked.contains(&n) {
                    fluid = neighbor;
                }
            }
            let state = world.block_state(pos)?;
            match resources.waterlogged.get(&state) {
                Some(wet) if fluid.is_source() && fluid.kind() == FluidKind::Water => {
                    world.set_block(pos, *wet, 3).map_err(world_err)?;
                    world.schedule_fluid_tick(pos, fluid.kind(), 5).ok();
                    filled = true;
                }
                _ => remaining.push(pos),
            }
        }
        to_fill = remaining;
    }
    Ok(true)
}
