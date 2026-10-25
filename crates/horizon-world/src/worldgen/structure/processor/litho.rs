//! Lithostitched's processors (spec-lithostitched §5), read as its data
//! writes them: `lithostitched:block_swap` (in `processor.rs`), `reference`,
//! `apply_random`, `condition`, `set_block` and `discard_input`.
//!
//! The draws of `random_chance`, `apply_random` and `set_block` follow
//! Lithostitched's `random_mode` (per block or per piece), but its seeding is
//! not known (OPEN-L6): a positional random of the world seed, the block's or
//! the piece's position and the processor itself, so the shares match while
//! the blocks chosen do not.
use super::{BlockStateId, DecodeError, Modifier, ProcessorList, ProcessorRandom};
use crate::worldgen::{block_feature::FeatureBlockBindings, structure::StructureResources};
use horizon_core::{
    position::BlockPos,
    random::{LegacyRandom, position_seed},
};
use serde_json::Value;
use std::collections::BTreeSet;

/// `random_mode` (`condition`, `set_block`) or `mode` (`apply_random`).
#[derive(Clone, Copy, Debug)]
pub(super) struct Draws {
    per_piece: bool,
    /// The processor's own salt, so two processors on a block draw apart.
    salt: i64,
}
impl Draws {
    fn decode(mode: Option<&Value>, processor: &Value) -> Result<Self, DecodeError> {
        let per_piece = match mode.and_then(Value::as_str) {
            None | Some("per_block") => false,
            Some("per_piece") => true,
            _ => return Err(DecodeError::Field("random_mode")),
        };
        // FNV-1a over the processor's JSON: the same in every run.
        let salt = (processor.to_string().bytes()).fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        }) as i64;
        Ok(Self { per_piece, salt })
    }
    /// The random for the block at `pos` of the piece at `piece`.
    pub(super) fn random(self, world_seed: i64, piece: [i32; 3], pos: [i32; 3]) -> LegacyRandom {
        let [x, y, z] = if self.per_piece { piece } else { pos };
        LegacyRandom::new(world_seed ^ self.salt ^ position_seed(BlockPos::new(x, y, z)))
    }
}

/// A processor or a list of them (`then`, `else`, `apply_random`'s data):
/// a list holder (`ProcessorList::decode`), or one processor object.
fn processors(
    value: &Value,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<ProcessorList, DecodeError> {
    let one = value.get("processor_type").is_some();
    let holder = if one {
        Value::Array(vec![value.clone()])
    } else {
        value.clone()
    };
    ProcessorList::decode(&holder.to_string(), resources, bindings)
}

/// `reference`: the named lists, run in turn on the block.
pub(super) fn reference(
    value: &Value,
    resources: &StructureResources,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<Vec<ProcessorList>, DecodeError> {
    let names = match &value["processor_lists"] {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    (names.iter())
        .map(|name| match name.as_str() {
            Some(n) if n.starts_with('#') => Err(DecodeError::Field("processor list tag")),
            Some(_) => ProcessorList::decode(&name.to_string(), resources, bindings),
            None => Err(DecodeError::Field("processor_lists")),
        })
        .collect()
}

/// `apply_random`: one list drawn by weight runs on the block.
#[derive(Clone, Debug)]
pub struct ApplyRandom {
    pub(super) draws: Draws,
    lists: Vec<(i32, ProcessorList)>,
    total: i32,
}
impl ApplyRandom {
    pub(super) fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let entries = value["processor_lists"]
            .as_array()
            .ok_or(DecodeError::Field("apply_random processor_lists"))?;
        let lists = (entries.iter())
            .map(|e| match e.get("data") {
                Some(data) => {
                    let weight = e["weight"]
                        .as_i64()
                        .unwrap_or(1)
                        .clamp(0, i64::from(i32::MAX));
                    Ok((weight as i32, processors(data, resources, bindings)?))
                }
                None => Ok((1, processors(e, resources, bindings)?)),
            })
            .collect::<Result<Vec<_>, DecodeError>>()?;
        let total = lists
            .iter()
            .map(|(w, _)| *w)
            .fold(0i32, i32::saturating_add);
        Ok(Self {
            draws: Draws::decode(value.get("mode"), value)?,
            lists,
            total,
        })
    }
    /// The list drawn: one bounded draw over the total weight.
    pub(super) fn pick(&self, random: &mut impl ProcessorRandom) -> Option<&ProcessorList> {
        if self.total <= 0 {
            return None;
        }
        let mut at = random.int(self.total);
        for (weight, list) in &self.lists {
            at -= weight;
            if at < 0 {
                return Some(list);
            }
        }
        None
    }
}

/// Lithostitched's processor conditions.
#[derive(Clone, Debug)]
pub(super) enum Condition {
    True,
    AllOf(Vec<Condition>),
    AnyOf(Vec<Condition>),
    Not(Box<Condition>),
    Chance(f32),
    /// `matching_blocks`: the template's block (`input`) or the world's
    /// (`location`) is one of `blocks`, with `properties`.
    Matching {
        blocks: BTreeSet<String>,
        properties: Vec<(String, String)>,
        location: bool,
    },
}
impl Condition {
    /// One condition, or a list of them that must all pass (`if_true`).
    pub(super) fn decode(
        value: &Value,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let list = |v: &Value| -> Result<Vec<Self>, DecodeError> {
            (v.as_array().ok_or(DecodeError::Field("conditions"))?.iter())
                .map(|c| Self::decode(c, bindings))
                .collect()
        };
        if value.is_array() {
            return Ok(Self::AllOf(list(value)?));
        }
        let kind = value["type"].as_str().unwrap_or_default();
        Ok(match crate::worldgen::tags::identifier(kind)?.as_str() {
            "lithostitched:true" => Self::True,
            "lithostitched:all_of" => Self::AllOf(list(&value["conditions"])?),
            "lithostitched:any_of" => Self::AnyOf(list(&value["conditions"])?),
            "lithostitched:not" => {
                Self::Not(Box::new(Self::decode(&value["condition"], bindings)?))
            }
            "lithostitched:random_chance" => Self::Chance(
                value["chance"]
                    .as_f64()
                    .ok_or(DecodeError::Field("random_chance chance"))? as f32,
            ),
            "lithostitched:matching_blocks" => Self::Matching {
                blocks: blocks(&value["blocks"], bindings)?,
                properties: (value
                    .get("properties")
                    .and_then(Value::as_object)
                    .into_iter())
                .flatten()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map_or_else(|| v.to_string(), str::to_owned),
                    )
                })
                .collect(),
                location: match value.get("match_type").and_then(Value::as_str) {
                    None | Some("input") => false,
                    Some("location") => true,
                    _ => return Err(DecodeError::Field("matching_blocks match_type")),
                },
            },
            _ => return Err(DecodeError::Field("lithostitched condition type")),
        })
    }
    pub(super) fn test(
        &self,
        input: BlockStateId,
        location: BlockStateId,
        registry: &crate::registry::BlockRegistry,
        random: &mut impl ProcessorRandom,
    ) -> bool {
        match self {
            Self::True => true,
            Self::AllOf(all) => all
                .iter()
                .all(|c| c.test(input, location, registry, random)),
            Self::AnyOf(any) => any
                .iter()
                .any(|c| c.test(input, location, registry, random)),
            Self::Not(c) => !c.test(input, location, registry, random),
            Self::Chance(chance) => random.float() < *chance,
            Self::Matching {
                blocks,
                properties,
                location: at_location,
            } => {
                let Some(state) = registry.state(if *at_location { location } else { input })
                else {
                    return false;
                };
                blocks.contains(&state.name)
                    && (properties.iter()).all(|(k, v)| state.properties.get(k) == Some(v))
            }
        }
    }
}
/// A block set (an id, a list, `#tag`) by block name. Blocks the game does
/// not have are left out: no template block can be one.
fn blocks(
    value: &Value,
    bindings: &FeatureBlockBindings<'_>,
) -> Result<BTreeSet<String>, DecodeError> {
    let names: Vec<&str> = match value {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return Err(DecodeError::Field("matching_blocks blocks")),
    };
    let registry = bindings.registry();
    let mut out = BTreeSet::new();
    for name in names {
        match name.strip_prefix('#') {
            Some(tag) => {
                let tag = crate::worldgen::tags::identifier(tag)?;
                let states = bindings.tags.get(&tag).into_iter().flat_map(|s| s.iter());
                out.extend(states.filter_map(|s| Some(registry.state(*s)?.name.clone())));
            }
            None => {
                out.insert(crate::worldgen::tags::identifier(name)?.to_string());
            }
        }
    }
    Ok(out)
}

/// `condition`: `then` when the conditions pass, else `else` (none: the
/// block as it is).
#[derive(Clone, Debug)]
pub struct Conditional {
    pub(super) draws: Draws,
    pub(super) condition: Condition,
    pub(super) then: ProcessorList,
    pub(super) otherwise: ProcessorList,
}
impl Conditional {
    pub(super) fn decode(
        value: &Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Self, DecodeError> {
        let branch = |key| match value.get(key) {
            Some(v) => processors(v, resources, bindings),
            None => Ok(ProcessorList(Vec::new())),
        };
        Ok(Self {
            draws: Draws::decode(value.get("random_mode"), value)?,
            condition: Condition::decode(&value["if_true"], bindings)?,
            then: branch("then")?,
            otherwise: branch("else")?,
        })
    }
}

/// `set_block`: the block from a state provider (simple or weighted).
#[derive(Clone, Debug)]
pub struct SetBlock {
    pub(super) draws: Draws,
    states: Vec<(i32, BlockStateId)>,
    total: i32,
    /// `preserve_state` (true by default): the template block's properties
    /// the new block has too are kept.
    pub(super) preserve: bool,
    pub(super) modifier: Modifier,
}
impl SetBlock {
    pub(super) fn decode(
        value: &Value,
        state: impl Fn(&Value) -> Result<BlockStateId, DecodeError>,
    ) -> Result<Self, DecodeError> {
        let provider = &value["state_provider"];
        let kind = provider["type"].as_str().unwrap_or_default();
        let states = match crate::worldgen::tags::identifier(kind)?.as_str() {
            "minecraft:simple_state_provider" => vec![(1, state(&provider["state"])?)],
            "minecraft:weighted_state_provider" => (provider["entries"].as_array().into_iter())
                .flatten()
                .map(|e| {
                    Ok((
                        e["weight"].as_i64().unwrap_or(1).clamp(0, 1 << 30) as i32,
                        state(&e["data"])?,
                    ))
                })
                .collect::<Result<_, DecodeError>>()?,
            _ => return Err(DecodeError::Field("set_block state_provider")),
        };
        let total = states.iter().map(|(w, _)| *w).sum();
        if total <= 0 {
            return Err(DecodeError::Field("set_block state_provider"));
        }
        Ok(Self {
            draws: Draws::decode(value.get("random_mode"), value)?,
            states,
            total,
            preserve: value
                .get("preserve_state")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            modifier: Modifier::decode(value.get("block_entity_modifier"))?,
        })
    }
    /// The provider's state: WeightedList.getRandom, one bounded draw.
    pub(super) fn state(&self, random: &mut impl ProcessorRandom) -> BlockStateId {
        let mut at = random.int(self.total);
        for (weight, state) in &self.states {
            at -= weight;
            if at < 0 {
                return *state;
            }
        }
        self.states[0].1
    }
}

#[cfg(test)]
mod tests {
    use super::super::{BlockInfo, PlaceSettings, ProcessorList, process_list};
    use crate::{
        heightmap::HeightmapType,
        nbt::{JavaString, Tag},
        registry::{BlockRegistry, BlockStateId},
        worldgen::{
            anchor::WorldGenerationContext,
            block_feature::FeatureBlockBindings,
            placement::{PlacementContext, PlacementQueryError},
            structure::{
                Rotation, StateTransforms, StructureResources, template::parse_state_string,
            },
        },
    };
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, BTreeSet};

    /// Every combination of `properties`' values, the first one the default.
    fn states(name: &str, properties: &[(&str, &[&str])]) -> Vec<Value> {
        let mut out = vec![serde_json::Map::new()];
        for (key, values) in properties {
            out = (out.iter())
                .flat_map(|o| {
                    values.iter().map(move |v| {
                        let mut o = o.clone();
                        o.insert((*key).into(), (*v).into());
                        o
                    })
                })
                .collect();
        }
        (out.into_iter())
            .map(|p| json!([name, name.ends_with("air"), false, "", {"id": name, "properties": p}]))
            .collect()
    }

    /// A registry of the blocks CTOV's lists and the probe pack use.
    fn registry() -> &'static BlockRegistry {
        let ages: Vec<String> = (0..8).map(|a| a.to_string()).collect();
        let ages: Vec<&str> = ages.iter().map(String::as_str).collect();
        let levels: Vec<String> = (0..16).map(|a| a.to_string()).collect();
        let levels: Vec<&str> = levels.iter().map(String::as_str).collect();
        let water = [("level", &levels[..])];
        let stairs: &[(&str, &[&str])] =
            &[("facing", &["north", "east"]), ("half", &["bottom", "top"])];
        let blocks: Vec<(&str, &[(&str, &[&str])])> = vec![
            ("minecraft:air", &[]),
            ("minecraft:cave_air", &[]),
            ("minecraft:void_air", &[]),
            ("minecraft:lava", &water),
            ("minecraft:cobblestone", &[]),
            ("minecraft:blackstone", &[]),
            ("minecraft:deepslate", &[]),
            ("minecraft:water", &water),
            ("minecraft:spruce_planks", &[]),
            ("minecraft:chest", &[]),
            ("minecraft:oak_log", &[("axis", &["y", "x"])]),
            ("minecraft:stripped_cherry_log", &[("axis", &["y", "x"])]),
            ("minecraft:oak_stairs", stairs),
            ("minecraft:warped_stairs", stairs),
        ];
        let wheat = [("age", &ages[..])];
        let beetroots = [("age", &ages[..4])];
        let mut rows = Vec::new();
        let mut defaults = BTreeMap::new();
        let crops: [(&str, &[(&str, &[&str])]); 3] = [
            ("minecraft:wheat", &wheat),
            ("minecraft:carrots", &wheat),
            ("minecraft:beetroots", &beetroots),
        ];
        for (name, properties) in blocks.into_iter().chain(crops) {
            defaults.insert(name, rows.len());
            rows.extend(states(name, properties));
        }
        let facts = json!({"states": rows, "defaults": defaults});
        let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
        Box::leak(Box::new(registry))
    }

    /// The world a piece is placed in: one block everywhere.
    struct World {
        seed: i64,
        block: BlockStateId,
    }
    impl PlacementContext for World {
        fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
            Ok(self.seed)
        }
        fn block_state(&mut self, _: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
            Ok(self.block)
        }
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, 384, 63)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64)
        }
        fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
            Err(PlacementQueryError::Unavailable)
        }
    }

    struct Lists {
        registry: &'static BlockRegistry,
        resources: StructureResources,
    }
    impl Lists {
        fn new(lists: &[(&str, Value)]) -> Self {
            let registry = registry();
            let lists = (lists.iter())
                .map(|(id, list)| ((*id).to_owned(), list.to_string()))
                .collect();
            let resources = StructureResources::new(
                registry,
                StateTransforms::default(),
                std::path::Path::new("/nonexistent"),
                lists,
                &BTreeMap::new(),
            );
            Self {
                registry,
                resources,
            }
        }
        fn id(&self, state: &str) -> BlockStateId {
            parse_state_string(self.registry, state).unwrap()
        }
        fn decode(&self, processors: Value) -> Result<ProcessorList, crate::worldgen::DecodeError> {
            let bindings = FeatureBlockBindings::new(self.registry, BTreeSet::new()).unwrap();
            ProcessorList::decode(&processors.to_string(), &self.resources, &bindings)
        }
        /// The block the list makes of template block `state` at `pos` of the
        /// piece at `piece`, in a world of `world`: None when it is not placed.
        fn run(
            &self,
            list: &ProcessorList,
            world: (i64, &str),
            piece: [i32; 3],
            pos: [i32; 3],
            state: &str,
        ) -> Option<BlockInfo> {
            let mut world = World {
                seed: world.0,
                block: self.id(world.1),
            };
            let info = BlockInfo {
                pos,
                state: self.id(state),
                nbt: None,
            };
            let mut settings = PlaceSettings::new(Rotation::None);
            process_list(
                list,
                &mut world,
                &self.resources,
                piece,
                piece,
                [0; 3],
                info,
                &mut settings,
            )
            .unwrap()
        }
        fn name(&self, info: Option<BlockInfo>) -> String {
            match info {
                None => "none".into(),
                Some(info) => {
                    let s = self.registry.state(info.state).unwrap();
                    let props: Vec<String> = s
                        .properties
                        .iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect();
                    match props.is_empty() {
                        true => s.name.clone(),
                        false => format!("{}[{}]", s.name, props.join(",")),
                    }
                }
            }
        }
        /// `name(run(...))` for one template block in an air world of seed 0.
        fn out(&self, list: &ProcessorList, state: &str) -> String {
            self.name(self.run(list, (0, "minecraft:air"), [0; 3], [0; 3], state))
        }
    }

    fn swap(blocks: Value) -> Value {
        json!({"processor_type": "lithostitched:block_swap", "blocks": blocks})
    }
    fn set_block(provider: Value) -> Value {
        json!({"processor_type": "lithostitched:set_block", "state_provider": provider})
    }
    fn simple(name: &str) -> Value {
        json!({"type": "minecraft:simple_state_provider", "state": {"id": name}})
    }

    #[test]
    fn block_swap_keeps_the_properties_the_new_block_can_have() {
        let t = Lists::new(&[]);
        let list = t
            .decode(json!([swap(json!({
                "minecraft:wheat": "minecraft:carrots",
                "oak_stairs": "minecraft:warped_stairs",
                "minecraft:cobblestone": "natures_spirit:missing"}))]))
            .unwrap();
        assert_eq!(
            t.out(&list, "minecraft:wheat[age=7]"),
            "minecraft:carrots[age=7]"
        );
        assert_eq!(
            t.out(&list, "minecraft:oak_stairs[facing=east,half=top]"),
            "minecraft:warped_stairs[facing=east,half=top]"
        );
        // A block the game lacks leaves the template's; others pass.
        assert_eq!(
            t.out(&list, "minecraft:cobblestone"),
            "minecraft:cobblestone"
        );
        assert_eq!(
            t.out(&list, "minecraft:oak_log[axis=x]"),
            "minecraft:oak_log[axis=x]"
        );
        // Beetroots' age stops at 3: ripe wheat gives the default (spec-ctov §4).
        let beet = t
            .decode(json!([swap(
                json!({"minecraft:wheat": "minecraft:beetroots"})
            )]))
            .unwrap();
        assert_eq!(
            t.out(&beet, "minecraft:wheat[age=7]"),
            "minecraft:beetroots[age=0]"
        );
        assert_eq!(
            t.out(&beet, "minecraft:wheat[age=2]"),
            "minecraft:beetroots[age=2]"
        );
    }

    #[test]
    fn references_run_the_named_lists_in_turn() {
        let t = Lists::new(&[
            (
                "t:logs",
                json!({"processors": [swap(json!({"minecraft:oak_log": "minecraft:stripped_cherry_log"}))]}),
            ),
            (
                "t:stone",
                json!([swap(
                    json!({"minecraft:stripped_cherry_log": "minecraft:cobblestone"})
                )]),
            ),
            (
                "t:discard",
                json!([{"processor_type": "lithostitched:discard_input"}]),
            ),
        ]);
        let reference = |lists: Value| json!([{"processor_type": "lithostitched:reference", "processor_lists": lists}]);
        let one = t.decode(reference(json!("t:logs"))).unwrap();
        assert_eq!(
            t.out(&one, "minecraft:oak_log[axis=x]"),
            "minecraft:stripped_cherry_log[axis=x]"
        );
        let both = t.decode(reference(json!(["t:logs", "t:stone"]))).unwrap();
        assert_eq!(t.out(&both, "minecraft:oak_log"), "minecraft:cobblestone");
        let reversed = t.decode(reference(json!(["t:stone", "t:logs"]))).unwrap();
        assert_eq!(
            t.out(&reversed, "minecraft:oak_log"),
            "minecraft:stripped_cherry_log[axis=y]"
        );
        // discard_input: the template block is not placed, and the lists after it do not run.
        let discard = t.decode(reference(json!(["t:discard", "t:logs"]))).unwrap();
        assert_eq!(t.out(&discard, "minecraft:oak_log"), "none");
        // The game has no processor list tags; unknown lists are errors.
        assert!(t.decode(reference(json!(["#t:palettes"]))).is_err());
        assert!(t.decode(reference(json!("t:missing"))).is_err());
    }

    /// The probe pack's nested condition (spec-lithostitched §7.3): template
    /// cobblestone becomes blackstone or deepslate, half each.
    #[test]
    fn conditions_run_then_or_else() {
        let t = Lists::new(&[]);
        let inner = json!({"processor_type": "lithostitched:condition", "random_mode": "per_block",
            "if_true": {"type": "lithostitched:random_chance", "chance": 0.5},
            "then": set_block(simple("minecraft:blackstone")),
            "else": [set_block(simple("minecraft:deepslate"))]});
        let list = t
            .decode(json!([{"processor_type": "lithostitched:condition",
                "if_true": [{"type": "lithostitched:matching_blocks", "blocks": "minecraft:cobblestone"}],
                "then": [inner]}]))
            .unwrap();
        let mut counts = BTreeMap::<String, u32>::new();
        for i in 0..4000 {
            let out = t.run(
                &list,
                (1234, "minecraft:air"),
                [0; 3],
                [i % 64, i / 64, 7],
                "minecraft:cobblestone",
            );
            *counts.entry(t.name(out)).or_default() += 1;
        }
        assert_eq!(counts.len(), 2, "{counts:?}");
        assert!(
            (1850..2150).contains(&counts["minecraft:blackstone"]),
            "{counts:?}"
        );
        // Not cobblestone, and no `else`: the block as it is.
        assert_eq!(
            t.out(&list, "minecraft:oak_log"),
            "minecraft:oak_log[axis=y]"
        );

        // `location` tests the world's block, with properties on the input.
        let water = t
            .decode(
                json!([{"processor_type": "lithostitched:condition", "random_mode": "per_piece",
                "if_true": {"type": "lithostitched:matching_blocks", "blocks": ["minecraft:water"],
                    "match_type": "location"},
                "then": set_block(simple("minecraft:spruce_planks"))}]),
            )
            .unwrap();
        let at = |world| {
            t.name(t.run(
                &water,
                (0, world),
                [0; 3],
                [1, 2, 3],
                "minecraft:cobblestone",
            ))
        };
        assert_eq!(at("minecraft:water"), "minecraft:spruce_planks");
        assert_eq!(at("minecraft:air"), "minecraft:cobblestone");
        let ripe = t
            .decode(json!([{"processor_type": "lithostitched:condition",
                "if_true": {"type": "lithostitched:all_of", "conditions": [
                    {"type": "lithostitched:true"},
                    {"type": "lithostitched:matching_blocks", "blocks": "minecraft:wheat", "properties": {"age": "7"}},
                    {"type": "lithostitched:not", "condition": {"type": "lithostitched:any_of", "conditions": [
                        {"type": "lithostitched:random_chance", "chance": 0.0},
                        {"type": "lithostitched:matching_blocks", "blocks": "minecraft:water", "match_type": "location"}]}}]},
                "then": swap(json!({"minecraft:wheat": "minecraft:carrots"}))}]))
            .unwrap();
        assert_eq!(
            t.out(&ripe, "minecraft:wheat[age=7]"),
            "minecraft:carrots[age=7]"
        );
        assert_eq!(
            t.out(&ripe, "minecraft:wheat[age=6]"),
            "minecraft:wheat[age=6]"
        );
        let bad = json!([{"processor_type": "lithostitched:condition", "random_mode": "per_chunk",
            "if_true": {"type": "lithostitched:true"}}]);
        assert!(t.decode(bad).is_err());
    }

    /// `apply_random` (spec-lithostitched §7.3: crimson 1 : birch 3, one
    /// choice per piece): here blackstone 1 : deepslate 3.
    #[test]
    fn apply_random_draws_per_piece_or_per_block() {
        let t = Lists::new(&[(
            "t:deepslate",
            json!([swap(
                json!({"minecraft:cobblestone": "minecraft:deepslate"})
            )]),
        )]);
        let random = |mode: &str| {
            t.decode(json!([{"processor_type": "lithostitched:apply_random", "mode": mode,
                "processor_lists": [
                    {"weight": 1, "data": [swap(json!({"minecraft:cobblestone": "minecraft:blackstone"}))]},
                    {"weight": 3, "data": "t:deepslate"}]}]))
            .unwrap()
        };
        let (per_piece, per_block) = (random("per_piece"), random("per_block"));
        let outcomes = |list: &ProcessorList, seed: i64, piece: [i32; 3]| -> Vec<String> {
            (0..20)
                .map(|i| {
                    let pos = [piece[0] + i, piece[1], piece[2] + 3 * i];
                    t.name(t.run(
                        list,
                        (seed, "minecraft:air"),
                        piece,
                        pos,
                        "minecraft:cobblestone",
                    ))
                })
                .collect()
        };
        let mut blackstone_pieces = 0;
        for p in 0..400 {
            let piece = [16 * p, 70, -40 * p];
            let blocks = outcomes(&per_piece, 1234, piece);
            assert!(blocks.iter().all(|b| *b == blocks[0]), "{blocks:?}");
            blackstone_pieces += u32::from(blocks[0] == "minecraft:blackstone");
            // The same piece and seed give the same blocks.
            assert_eq!(outcomes(&per_piece, 1234, piece), blocks);
        }
        assert!(
            (70..130).contains(&blackstone_pieces),
            "{blackstone_pieces}"
        );
        // Per block, one piece mixes both.
        let mixed: BTreeSet<String> = (0..10)
            .flat_map(|p| outcomes(&per_block, 1234, [p, 0, 0]))
            .collect();
        assert_eq!(mixed.len(), 2, "{mixed:?}");
        // A plain list: every entry weight 1. An entry list holding nothing is a no-op.
        let plain = t
            .decode(
                json!([{"processor_type": "lithostitched:apply_random", "mode": "per_block",
                "processor_lists": ["t:deepslate", []]}]),
            )
            .unwrap();
        let seen: BTreeSet<String> = (0..10)
            .flat_map(|p| outcomes(&plain, 9, [p, 0, 0]))
            .collect();
        assert_eq!(
            seen,
            BTreeSet::from(["minecraft:cobblestone".into(), "minecraft:deepslate".into()])
        );
    }

    #[test]
    fn set_block_runs_its_provider() {
        let t = Lists::new(&[]);
        let stairs = "minecraft:oak_stairs[facing=east,half=top]";
        let warped = |preserve: Value| {
            let mut p = set_block(simple("minecraft:warped_stairs"));
            if !preserve.is_null() {
                p["preserve_state"] = preserve;
            }
            t.decode(json!([p])).unwrap()
        };
        // preserve_state (true by default) keeps the properties both blocks have.
        assert_eq!(
            t.out(&warped(Value::Null), stairs),
            "minecraft:warped_stairs[facing=east,half=top]"
        );
        assert_eq!(
            t.out(&warped(json!(false)), stairs),
            "minecraft:warped_stairs[facing=north,half=bottom]"
        );

        let weighted = t
            .decode(json!([set_block(
                json!({"type": "minecraft:weighted_state_provider", "entries": [
                {"weight": 1, "data": {"id": "minecraft:blackstone"}},
                {"weight": 3, "data": {"id": "minecraft:deepslate"}}]})
            )]))
            .unwrap();
        let blackstone = (0..2000)
            .filter(|i| {
                let out = t.run(
                    &weighted,
                    (42, "minecraft:air"),
                    [0; 3],
                    [*i, 0, -*i],
                    "minecraft:cobblestone",
                );
                t.name(out) == "minecraft:blackstone"
            })
            .count();
        assert!((420..580).contains(&blackstone), "{blackstone}");

        // block_entity_modifier gives the output its block entity.
        let mut loot = set_block(simple("minecraft:chest"));
        loot["block_entity_modifier"] =
            json!({"type": "minecraft:append_loot", "loot_table": "t:chests/house"});
        let out = t
            .run(
                &t.decode(json!([loot])).unwrap(),
                (0, "minecraft:air"),
                [0; 3],
                [0; 3],
                "minecraft:air",
            )
            .unwrap();
        let table = out
            .nbt
            .as_ref()
            .and_then(|n| n.get(&JavaString::from("LootTable")));
        assert_eq!(table, Some(&Tag::String("t:chests/house".into())));

        // Providers the game does not run are refused.
        assert!(
            t.decode(json!([set_block(
                json!({"type": "minecraft:noise_provider"})
            )]))
            .is_err()
        );
    }
}
