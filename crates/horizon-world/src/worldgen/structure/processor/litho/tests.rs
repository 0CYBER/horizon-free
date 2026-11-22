use super::super::{BlockInfo, PlaceSettings, ProcessorList, process_list};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use {
    crate::{
        heightmap::HeightmapType,
        registry::{BlockRegistry, BlockStateId},
        worldgen::{
            anchor::WorldGenerationContext,
            block_feature::FeatureBlockBindings,
            placement::{PlacementContext, PlacementQueryError},
            structure::{
                Rotation, StateTransforms, StructureResources, template::parse_state_string,
            },
        },
    },
    horizon_core::nbt::{JavaString, Tag},
};

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

#[test]
fn wide_weights_litho_apply_random() {
    use horizon_core::random::{LegacyRandom, RandomSource};
    for seed in -20..20 {
        for total in [1, 2, 7, 100, i32::MAX] {
            let mut old = LegacyRandom::new(seed);
            let mut new = old.clone();
            assert_eq!(
                super::draw(i64::from(total), &mut new),
                i64::from(old.next_int_bound(total).unwrap())
            );
            assert_eq!(new.next_long(), old.next_long());
        }
    }
    let t = Lists::new(&[]);
    let entry = json!({"weight": i32::MAX, "data": []});
    let list = t
        .decode(json!([{"processor_type": "lithostitched:apply_random",
        "processor_lists": [entry, entry, entry]}]))
        .unwrap();
    assert_eq!(
        t.out(&list, "minecraft:cobblestone"),
        "minecraft:cobblestone"
    );
    let bindings = FeatureBlockBindings::new(t.registry, BTreeSet::new()).unwrap();
    let value = json!({"processor_lists": [entry, entry, entry]});
    let picker = super::ApplyRandom::decode(&value, &t.resources, &bindings).unwrap();
    // Every entry remains reachable, including the third (a saturated
    // total would always stop in the first entry).
    for index in 0..3 {
        assert!((0..100).any(|seed| {
            let selected = picker
                .pick(&mut horizon_core::random::LegacyRandom::new(seed))
                .unwrap();
            std::ptr::eq(selected, &picker.lists[index].1)
        }));
    }
}

#[test]
fn wide_weights_litho_set_block() {
    let t = Lists::new(&[]);
    let entry = json!({"weight": i32::MAX, "data": {"id": "minecraft:blackstone"}});
    let list = t
        .decode(json!([set_block(
            json!({"type": "minecraft:weighted_state_provider",
        "entries": [entry, entry, entry]})
        )]))
        .unwrap();
    assert_eq!(
        t.out(&list, "minecraft:cobblestone"),
        "minecraft:blackstone"
    );
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
