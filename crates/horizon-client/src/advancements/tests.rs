use super::*;
use serde_json::json;
#[test]
fn conditional_advancements_select_before_filtering_and_cache_by_inventory() {
    let data = std::env::temp_dir().join(format!(
        "horizon-advancement-conditions-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(data.join("demo/advancement")).unwrap();
    let plain = json!({"criteria":{"never":{"trigger":"minecraft:impossible"}}});
    let mut conditional = plain.clone();
    conditional["neoforge:conditions"] = json!([{"type":"neoforge:mod_loaded","modid":"provider"}]);
    std::fs::write(
        data.join("demo/advancement/entry.json"),
        conditional.to_string(),
    )
    .unwrap();
    let enabled = horizon_world::conditions::LoadConditions::new(["provider"]);
    let off = Tree::shared(&data, &horizon_world::loot::PackOverlay::NONE);
    let on_overlay = horizon_world::loot::PackOverlay {
        conditions: &enabled,
        ..horizon_world::loot::PackOverlay::NONE
    };
    let on = Tree::shared(&data, &on_overlay);
    assert!(off.get("demo:entry").is_none());
    assert!(on.get("demo:entry").is_some());
    assert!(!Arc::ptr_eq(&off, &on));
    assert!(Arc::ptr_eq(&on, &Tree::shared(&data, &on_overlay)));
    let files = [(
        "advancement/demo:entry".into(),
        vec![
            plain.to_string().into_bytes(),
            json!({"neoforge:conditions":[{"type":"unsupported"}]})
                .to_string()
                .into_bytes(),
        ],
    )];
    let overridden = Tree::shared(
        &data,
        &horizon_world::loot::PackOverlay {
            files: &files,
            ..on_overlay
        },
    );
    assert!(overridden.get("demo:entry").is_none());
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn advancement_reward_function_is_optional_and_namespaced() {
    for (raw, expected) in [
        ("parity:reward", "parity:reward"),
        ("reward", "minecraft:reward"),
    ] {
        let input = serde_json::json!({
            "criteria": {"never": {"trigger": "minecraft:impossible"}},
            "rewards": {"function": raw, "experience": 7, "recipes": ["minecraft:bread"]}
        });
        let advancement = parse("parity:test", &input).unwrap();
        assert_eq!(advancement.rewards.function.as_deref(), Some(expected));
        assert_eq!(advancement.rewards.experience, 7);
        assert_eq!(advancement.rewards.recipes, ["minecraft:bread"]);
    }
    let without = parse(
        "parity:empty",
        &serde_json::json!({
            "criteria": {"never": {"trigger": "minecraft:impossible"}}
        }),
    )
    .unwrap();
    assert!(without.rewards.function.is_none());
}
/// The criteria an inventory change and a recipe unlock test, on vanilla's tree:
/// `cargo test -p horizon-client --lib -- --ignored --nocapture index_on_vanilla_tree`
/// with HORIZON_TEST_RESOURCES set to the prepared resources.
#[test]
#[ignore = "daily"]
fn index_on_vanilla_tree() {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES"),
    );
    let data = root.join("data");
    let t = std::time::Instant::now();
    let tree = Tree::load(&data);
    let load = t.elapsed();
    let tags = data.join("minecraft/tags/item");
    let tag_items: HashMap<String, Vec<String>> = std::fs::read_dir(&tags)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            let name = format!("minecraft:{}", e.path().file_stem()?.to_string_lossy());
            let values = v["values"]
                .as_array()?
                .iter()
                .filter_map(|x| x.as_str().map(str::to_owned));
            Some((name, values.collect()))
        })
        .collect();
    let t = std::time::Instant::now();
    let index = InventoryIndex::build(&tree, |tag| {
        tag_items
            .get(tag)
            .map(|v| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    });
    let build = t.elapsed();
    let all = tree
        .by_trigger
        .get("minecraft:inventory_changed")
        .map_or(0, Vec::len);
    let held = [
        "minecraft:iron_ingot",
        "minecraft:oak_log",
        "minecraft:cobblestone",
        "minecraft:diamond_pickaxe",
        "minecraft:bread",
        "minecraft:torch",
        "minecraft:oak_planks",
        "minecraft:stick",
        "minecraft:coal",
    ];
    let t = std::time::Instant::now();
    let mut candidates = 0;
    for _ in 0..1000 {
        candidates = index.candidates(held).len();
    }
    let per_change = t.elapsed() / 1000;
    let recipes = tree
        .by_trigger
        .get("minecraft:recipe_unlocked")
        .map_or(0, Vec::len);
    let per_recipe = tree.recipe_criteria("minecraft:iron_pickaxe").len();
    eprintln!(
        "tree {load:?}; inventory_changed: {all} criteria, index built in {build:?}, \
         a 9-stack inventory tests {candidates} (looked up in {per_change:?}); \
         recipe_unlocked: {recipes} criteria, iron_pickaxe tests {per_recipe}"
    );
    assert!(candidates < all / 10);
}
struct Flat;
impl World for Flat {
    fn dimension(&self) -> &str {
        "minecraft:overworld"
    }
    fn weather(&self) -> (bool, bool) {
        (false, false)
    }
    fn biome(&self, _: [i32; 3]) -> String {
        "minecraft:plains".into()
    }
    fn structures(&self, _: [i32; 3]) -> Vec<String> {
        vec!["minecraft:village_plains".into()]
    }
    fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
        ("minecraft:stone".into(), BTreeMap::new())
    }
    fn fluid(&self, _: [i32; 3]) -> Option<String> {
        None
    }
    fn can_see_sky(&self, _: [i32; 3]) -> bool {
        true
    }
    fn light(&self, _: [i32; 3]) -> i32 {
        15
    }
    fn smokey(&self, _: [i32; 3]) -> bool {
        false
    }
    fn in_tag(&self, _: &str, tag: &str, id: &str) -> bool {
        (tag == "minecraft:stone_tool_materials" && id == "minecraft:cobblestone")
            || (tag == "minecraft:sulfur_cube_archetype/explosive" && id == "minecraft:tnt")
            || (tag == "minecraft:is_projectile"
                && matches!(id, "minecraft:fireball" | "minecraft:arrow"))
    }
}
/// `BlockPredicate.matchesBlockEntity`: an `nbt` test needs a block
/// entity whose `saveWithFullMetadata` contains the pattern
/// (`NbtPredicate.matches`), so it never matches a plain block.
#[test]
fn a_block_predicates_nbt_needs_the_block_entitys_data() {
    let w = Flat;
    let stone = serde_json::json!({"blocks": "minecraft:stone", "nbt": "{}"});
    assert!(!block(&stone, [0, 0, 0], &w), "stone has no block entity");
    assert!(block(
        &serde_json::json!({"blocks": "minecraft:stone"}),
        [0, 0, 0],
        &w
    ));
    /// An unpowered beacon: its saved data has `Levels:0`.
    struct Beacon;
    impl World for Beacon {
        fn dimension(&self) -> &str {
            Flat.dimension()
        }
        fn weather(&self) -> (bool, bool) {
            Flat.weather()
        }
        fn biome(&self, p: [i32; 3]) -> String {
            Flat.biome(p)
        }
        fn structures(&self, p: [i32; 3]) -> Vec<String> {
            Flat.structures(p)
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            ("minecraft:beacon".into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            true
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            15
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, _: &str, _: &str, _: &str) -> bool {
            false
        }
        fn block_entity_nbt(&self, _: [i32; 3]) -> Option<horizon_core::nbt::Tag> {
            horizon_core::nbt::parse_snbt(r#"{id:"minecraft:beacon",x:0,y:0,z:0,Levels:0}"#)
                .ok()
                .map(|(t, _)| t)
        }
    }
    let beacon = |nbt: &str| serde_json::json!({"blocks": "minecraft:beacon", "nbt": nbt});
    assert!(block(&beacon("{Levels:0}"), [0, 0, 0], &Beacon));
    assert!(block(&beacon("{}"), [0, 0, 0], &Beacon));
    assert!(!block(&beacon("{Levels:4}"), [0, 0, 0], &Beacon));
    assert!(
        !block(&beacon("{Levels:0b}"), [0, 0, 0], &Beacon),
        "an int, not a byte"
    );
}
#[test]
fn weather_conditions_leave_absent_fields_unconstrained_and_compose() {
    let ctx = Entities {
        this: None,
        origin: [0.0; 3],
        tool: None,
        block_pos: None,
    };
    for (mut predicate, expected) in [
        (serde_json::json!({}), true),
        (serde_json::json!({"raining": false}), true),
        (serde_json::json!({"raining": true}), false),
        (serde_json::json!({"thundering": false}), true),
        (serde_json::json!({"thundering": true}), false),
        (
            serde_json::json!({"raining": false, "thundering": false}),
            true,
        ),
        (
            serde_json::json!({"raining": false, "thundering": true}),
            false,
        ),
    ] {
        predicate["type"] = "minecraft:weather_check".into();
        assert_eq!(conditions(&predicate, &ctx, &Flat), expected, "{predicate}");
    }
    let dry = serde_json::json!({"type": "minecraft:weather_check", "raining": false});
    let wet = serde_json::json!({"type": "minecraft:inverted", "term": dry});
    assert!(!conditions(&wet, &ctx, &Flat));
    assert!(conditions(
        &serde_json::json!({"type": "minecraft:all_of", "terms": [dry]}),
        &ctx,
        &Flat
    ));
    assert!(!conditions(
        &serde_json::json!({"type": "minecraft:any_of", "terms": [wet]}),
        &ctx,
        &Flat
    ));
}
/// A `recipe_crafted` criterion names `minecraft:x`; the craft hands
/// the recipe book and the trigger the same whole id (`award_crafted`
/// qualifies). The bare name the crafting copy uses never matched —
/// the second half of #7186.
#[test]
fn a_recipe_crafted_criterion_matches_the_whole_id_only() {
    let w = Flat;
    let criterion = serde_json::json!({"recipes": "minecraft:folder_recipe"});
    assert!(
        !id_matches(&criterion["recipes"], "recipe", "folder_recipe", &w),
        "the crafting copy's bare name never matched the criterion"
    );
    assert!(
        id_matches(
            &criterion["recipes"],
            "recipe",
            &crate::crafting::qualified("folder_recipe"),
            &w
        ),
        "the whole id award_crafted passes matches"
    );
}
/// `LockCode.canUnlock` for a chest with
/// `lock:{items:"minecraft:gold_ingot"}`: a gold ingot opens it, an empty
/// hand or another item does not, and a spectator always does.
#[test]
fn a_lock_takes_its_item_and_any_spectator() {
    let lock = serde_json::json!({"items": "minecraft:gold_ingot"});
    let gold = ItemView {
        id: "minecraft:gold_ingot".into(),
        count: 1,
        ..Default::default()
    };
    let stick = ItemView {
        id: "minecraft:stick".into(),
        count: 1,
        ..Default::default()
    };
    assert!(can_unlock(&lock, false, Some(&gold), &Flat));
    assert!(!can_unlock(&lock, false, Some(&stick), &Flat));
    assert!(!can_unlock(&lock, false, None, &Flat));
    assert!(can_unlock(&lock, true, None, &Flat));
    assert!(can_unlock(&lock, true, Some(&stick), &Flat));
    // `LockCode.NO_LOCK` is the empty predicate: it takes any stack, and
    // `unlocksWith(EMPTY)` too.
    let none = serde_json::json!({});
    assert!(can_unlock(&none, false, None, &Flat));
}
/// `ItemPredicate`'s `components` and `predicates` are keyed by registry
/// ids, so a key without a namespace is a `minecraft:` one. An unread key
/// used to be skipped, which let every stack pass.
#[test]
fn item_predicate_reads_bare_component_keys() {
    let potion = ItemView {
        id: "minecraft:potion".into(),
        count: 1,
        potion: Some("minecraft:swiftness".into()),
        ..Default::default()
    };
    let sword = ItemView {
        id: "minecraft:iron_sword".into(),
        count: 1,
        damage: 3,
        enchantments: vec![("minecraft:sharpness".into(), 2)],
        ..Default::default()
    };
    let cases = [
        (
            serde_json::json!({"components": {"potion_contents": "minecraft:swiftness"}}),
            serde_json::json!({"components": {"potion_contents": "minecraft:healing"}}),
            &potion,
        ),
        (
            serde_json::json!({"components": {"damage": 3}}),
            serde_json::json!({"components": {"damage": 4}}),
            &sword,
        ),
        (
            serde_json::json!({"predicates": {"enchantments": [
                {"enchantments": "sharpness", "levels": 2}
            ]}}),
            serde_json::json!({"predicates": {"enchantments": [
                {"enchantments": "sharpness", "levels": {"min": 3}}
            ]}}),
            &sword,
        ),
        (
            serde_json::json!({"predicates": {"damage": {"damage": {"max": 3}}}}),
            serde_json::json!({"predicates": {"damage": {"damage": {"min": 4}}}}),
            &sword,
        ),
    ];
    for (yes, no, stack) in cases {
        assert!(item(&yes, stack, &Flat), "{yes} should match");
        assert!(!item(&no, stack, &Flat), "{no} should not match");
    }
    // The potion is a registry id too.
    let bare = serde_json::json!({"components": {"minecraft:potion_contents": "swiftness"}});
    assert!(item(&bare, &potion, &Flat));
}
/// `PassengerPredicate.matches`: ride_a_boat_with_a_goat's vehicle holds a goat.
#[test]
fn passenger_predicate_needs_a_matching_passenger() {
    let want = serde_json::json!({
        "minecraft:entity_type": "minecraft:oak_boat",
        "minecraft:passenger": {"minecraft:entity_type": "minecraft:goat"}
    });
    let entity = |kind: &str| EntityView {
        kind: kind.into(),
        ..Default::default()
    };
    let mut boat = entity("minecraft:oak_boat");
    assert!(!super::entity(&want, &boat, &Flat, [0.0; 3]));
    boat.passengers = vec![entity("minecraft:player")];
    assert!(!super::entity(&want, &boat, &Flat, [0.0; 3]));
    boat.passengers.push(entity("minecraft:goat"));
    assert!(super::entity(&want, &boat, &Flat, [0.0; 3]));
}
/// `EntityNbtPredicate.matches`: the entity's saved compound holds the
/// pattern (`NbtUtils.compareNbt(…, true)`); without one it fails.
#[test]
fn an_entity_predicates_nbt_tests_the_saved_compound() {
    let snbt = |s: &str| horizon_core::nbt::parse_snbt(s).unwrap().0;
    let view = |kind: &str, nbt: &str| EntityView {
        kind: kind.into(),
        nbt: Some(snbt(nbt)),
        ..Default::default()
    };
    let check = |v: serde_json::Value, e: &EntityView| super::entity(&v, e, &Flat, [0.0; 3]);
    let nbt = |s: &str| serde_json::json!({"minecraft:nbt": s});
    let axolotl = view("minecraft:axolotl", "{Variant:1,FromBucket:1b}");
    assert!(check(nbt("{Variant:1}"), &axolotl));
    assert!(check(nbt("{}"), &axolotl));
    assert!(!check(nbt("{Variant:0}"), &axolotl));
    assert!(
        !check(nbt("{FromBucket:1}"), &axolotl),
        "a byte, not an int"
    );
    let pig = serde_json::json!({"minecraft:entity_type": "minecraft:pig",
        "minecraft:nbt": "{Variant:1}"});
    assert!(
        !check(pig, &axolotl),
        "the other sub-predicates still apply"
    );
    let villager = view(
        "minecraft:villager",
        r#"{VillagerData:{level:2,profession:"minecraft:farmer",type:"minecraft:taiga"}}"#,
    );
    assert!(check(
        nbt(r#"{VillagerData:{profession:"minecraft:farmer"}}"#),
        &villager
    ));
    assert!(!check(
        nbt(r#"{VillagerData:{profession:"minecraft:librarian"}}"#),
        &villager
    ));
    let player = view(
        "minecraft:player",
        r#"{Inventory:[{Slot:0b,id:"minecraft:spyglass",count:1},{Slot:3b,id:"minecraft:stick",count:2}],SelectedItem:{id:"minecraft:spyglass",count:1}}"#,
    );
    assert!(check(
        nbt(r#"{Inventory:[{id:"minecraft:stick"}]}"#),
        &player
    ));
    assert!(!check(
        nbt(r#"{Inventory:[{id:"minecraft:stick",count:3}]}"#),
        &player
    ));
    assert!(
        !check(nbt("{Inventory:[]}"), &player),
        "an empty list wants an empty one"
    );
    assert!(check(
        nbt(r#"{SelectedItem:{id:"minecraft:spyglass"}}"#),
        &player
    ));
    let bare = EntityView {
        kind: "minecraft:axolotl".into(),
        ..Default::default()
    };
    assert!(!check(nbt("{}"), &bare), "no compound known");
    assert!(!check(nbt("{Variant:"), &axolotl), "unparsable");
    assert!(!check(serde_json::json!({"minecraft:nbt": 3}), &axolotl));
}
/// Return to Sender's `killing_blow`: only the ghast's own fireball, as a
/// projectile, kills it for the advancement; a sword or an arrow does not.
#[test]
fn killing_blow_needs_the_fireball() {
    let blow = serde_json::json!({
        "direct_entity": {"minecraft:entity_type": "minecraft:fireball"},
        "tags": [{"expected": true, "id": "#minecraft:is_projectile"}]
    });
    let entity = |kind: &str| EntityView {
        kind: kind.into(),
        ..Default::default()
    };
    let player = entity("minecraft:player");
    let fireball = DamageView {
        kind: "minecraft:fireball".into(),
        direct: Some(entity("minecraft:fireball")),
        source: Some(player.clone()),
        ..Default::default()
    };
    assert!(damage_source(&blow, &fireball, &Flat, [0.0; 3]));
    let sword = DamageView {
        kind: "minecraft:player_attack".into(),
        direct: Some(player.clone()),
        source: Some(player.clone()),
        is_direct: true,
        ..Default::default()
    };
    assert!(!damage_source(&blow, &sword, &Flat, [0.0; 3]));
    let arrow = DamageView {
        kind: "minecraft:arrow".into(),
        direct: Some(entity("minecraft:arrow")),
        source: Some(player),
        ..Default::default()
    };
    assert!(!damage_source(&blow, &arrow, &Flat, [0.0; 3]));
    assert!(damage_source(&Value::Null, &sword, &Flat, [0.0; 3]));
}
fn mob(kind: &str) -> EntityView {
    EntityView {
        kind: kind.into(),
        ..Default::default()
    }
}
/// `TeamPredicate.matches`: the effective team's name, exactly.
#[test]
fn team_predicate_needs_the_exact_team_name() {
    let team = |v: &str| serde_json::json!({ v: "audit_team" });
    let member = EntityView {
        team: Some("audit_team".into()),
        ..mob("minecraft:zombie")
    };
    for key in ["team", "minecraft:team"] {
        assert!(entity(&team(key), &member, &Flat, [0.0; 3]));
    }
    let other = EntityView {
        team: Some("Audit_Team".into()),
        ..mob("minecraft:zombie")
    };
    assert!(!entity(&team("team"), &other, &Flat, [0.0; 3]));
    assert!(!entity(
        &team("team"),
        &mob("minecraft:zombie"),
        &Flat,
        [0.0; 3]
    ));
}
/// `entity_killed_player` by a mob's melee hit: the mob is the killer and
/// the blow's causing and direct entity.
#[test]
fn entity_killed_player_by_a_mob_names_the_mob() {
    let c = serde_json::json!({
        "entity": {"minecraft:entity_type": "minecraft:polar_bear"},
        "killing_blow": {"source_entity": {"minecraft:entity_type": "minecraft:polar_bear"}}
    });
    let me = mob("minecraft:player");
    let bear = mob("minecraft:polar_bear");
    let blow = recorded_blow("minecraft:mob_attack".into(), Some(bear.clone()), [0.0; 3]);
    assert!(blow.is_direct);
    assert_eq!(
        blow.direct.as_ref().map(|e| e.kind.as_str()),
        Some("minecraft:polar_bear")
    );
    assert!(killed(&c, &me, &bear, Some(&blow), &Flat));
    let panda = mob("minecraft:panda");
    let blow = recorded_blow("minecraft:mob_attack".into(), Some(panda.clone()), [0.0; 3]);
    assert!(!killed(&c, &me, &panda, Some(&blow), &Flat));
}
/// `entity_killed_player` by a skeleton's arrow: the shooter is the
/// killer and the blow's causing entity, the arrow its direct one.
#[test]
fn entity_killed_player_by_an_arrow_names_the_shooter() {
    let c = serde_json::json!({
        "entity": {"minecraft:entity_type": "minecraft:skeleton"},
        "killing_blow": {
            "direct_entity": {"minecraft:entity_type": "minecraft:arrow"},
            "source_entity": {"minecraft:entity_type": "minecraft:skeleton"},
            "is_direct": false
        }
    });
    let me = mob("minecraft:player");
    let skeleton = mob("minecraft:skeleton");
    let blow = recorded_blow(
        "minecraft:arrow".into(),
        Some(skeleton.clone()),
        [1.0, 2.0, 3.0],
    );
    assert!(!blow.is_direct);
    assert_eq!(
        blow.direct.as_ref().map(|e| e.position),
        Some([1.0, 2.0, 3.0])
    );
    assert!(killed(&c, &me, &skeleton, Some(&blow), &Flat));
    // Shot point-blank, not with an arrow: the direct entity is the skeleton.
    let melee = recorded_blow(
        "minecraft:mob_attack".into(),
        Some(skeleton.clone()),
        [0.0; 3],
    );
    assert!(!killed(&c, &me, &skeleton, Some(&melee), &Flat));
}
/// A blow with no causing entity fails a `source_entity` test, and with
/// no blow at all any `killing_blow` fails.
#[test]
fn a_blow_with_no_killer_fails_its_entity_tests() {
    let c = serde_json::json!({
        "killing_blow": {"source_entity": {"minecraft:entity_type": "minecraft:zombie"}}
    });
    let me = mob("minecraft:player");
    let zombie = mob("minecraft:zombie");
    let fall = recorded_blow("minecraft:fall".into(), None, [0.0; 3]);
    assert!(fall.direct.is_none() && fall.source.is_none());
    assert!(!killed(&c, &me, &zombie, Some(&fall), &Flat));
    assert!(!killed(&c, &me, &zombie, None, &Flat));
    assert!(killed(&serde_json::json!({}), &me, &zombie, None, &Flat));
}
#[test]
fn uh_oh_accepts_either_tnt_event_only_for_adult_sulfur_cubes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client/26.3/data/minecraft/advancement/husbandry/uh_oh.json");
    if !path.exists() {
        return;
    }
    let json = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let advancement = parse("minecraft:husbandry/uh_oh", &json).unwrap();
    assert_eq!(advancement.criteria.len(), 2);
    let player = EntityView {
        kind: "minecraft:player".into(),
        ..Default::default()
    };
    let mut target = EntityView {
        kind: "minecraft:sulfur_cube".into(),
        ..Default::default()
    };
    let mut stack = ItemView {
        id: "minecraft:tnt".into(),
        count: 1,
        ..Default::default()
    };
    for criterion in &advancement.criteria {
        assert!(item_entity(
            &criterion.conditions,
            &player,
            &target,
            &stack,
            &Flat
        ));
        let mut progress = Progress::default();
        progress.grant(&advancement.id, &criterion.name, "test");
        assert!(
            progress.done(&advancement),
            "either criterion must award Uh Oh"
        );
        target.baby = true;
        assert!(!item_entity(
            &criterion.conditions,
            &player,
            &target,
            &stack,
            &Flat
        ));
        target.baby = false;
        target.kind = "minecraft:pig".into();
        assert!(!item_entity(
            &criterion.conditions,
            &player,
            &target,
            &stack,
            &Flat
        ));
        target.kind = "minecraft:sulfur_cube".into();
        stack.id = "minecraft:stone".into();
        assert!(!item_entity(
            &criterion.conditions,
            &player,
            &target,
            &stack,
            &Flat
        ));
        stack.id = "minecraft:tnt".into();
    }
}
/// Only the criteria the held items could pass are tested; those that
/// need no particular item always are.
#[test]
fn inventory_index_keeps_the_criteria_the_items_could_pass() {
    let criteria = [
        serde_json::json!({"items": [{"items": "minecraft:diamond"}]}),
        serde_json::json!({"items": [{"items": "#minecraft:planks"}]}),
        serde_json::json!({"items": [{"count": {"min": 2}}]}),
        serde_json::json!({"items": [{"count": 1}, {"items": ["minecraft:stick", "minecraft:apple"]}]}),
        serde_json::json!({"slots": {"empty": 0}}),
        serde_json::json!({"items": [{"items": "#minecraft:no_such_tag"}]}),
    ];
    let mut tree = Tree::default();
    for (i, conditions) in criteria.into_iter().enumerate() {
        tree.list.push(Advancement {
            id: format!("test:{i}"),
            parent: None,
            display: None,
            criteria: vec![Criterion {
                name: "c".into(),
                trigger: "minecraft:inventory_changed".into(),
                conditions: LazyJson::new(&conditions),
            }],
            requirements: vec![vec!["c".into()]],
            rewards: Rewards::default(),
        });
        tree.by_trigger
            .entry("minecraft:inventory_changed".into())
            .or_default()
            .push((i, 0));
    }
    let index = InventoryIndex::build(&tree, |tag| match tag {
        "minecraft:planks" => vec!["minecraft:oak_planks", "minecraft:birch_planks"],
        _ => Vec::new(),
    });
    let found = |held: &[&str]| -> Vec<usize> {
        index
            .candidates(held.iter().copied())
            .into_iter()
            .map(|(ai, _)| ai)
            .collect()
    };
    assert_eq!(found(&[]), [2, 4]);
    assert_eq!(found(&["minecraft:dirt"]), [2, 4]);
    assert_eq!(
        found(&["minecraft:oak_planks", "minecraft:diamond"]),
        [0, 1, 2, 4]
    );
    assert_eq!(found(&["minecraft:apple", "minecraft:apple"]), [2, 3, 4]);
}
/// A recipe unlocked tests only the criteria that name it.
#[test]
fn recipe_index_keeps_the_criteria_of_the_recipe() {
    let mut tree = Tree::default();
    for (i, conditions) in [
        serde_json::json!({"recipes": "minecraft:oak_planks"}),
        serde_json::json!({"recipes": "minecraft:stick"}),
        serde_json::json!({}),
        serde_json::json!({"recipes": "minecraft:oak_planks"}),
    ]
    .into_iter()
    .enumerate()
    {
        tree.list.push(Advancement {
            id: format!("test:{i}"),
            parent: None,
            display: None,
            criteria: vec![Criterion {
                name: "c".into(),
                trigger: "minecraft:recipe_unlocked".into(),
                conditions: LazyJson::new(&conditions),
            }],
            requirements: vec![vec!["c".into()]],
            rewards: Rewards::default(),
        });
        tree.by_trigger
            .entry("minecraft:recipe_unlocked".into())
            .or_default()
            .push((i, 0));
    }
    tree.index_recipes();
    assert_eq!(
        tree.recipe_criteria("minecraft:oak_planks"),
        [(0, 0), (2, 0), (3, 0)]
    );
    assert_eq!(tree.recipe_criteria("minecraft:stick"), [(1, 0), (2, 0)]);
    assert_eq!(tree.recipe_criteria("minecraft:torch"), [(2, 0)]);
}
#[test]
fn bucket_axolotl_block_advancements_require_the_placed_block() {
    let ctx = Entities {
        this: None,
        origin: [0.0; 3],
        tool: None,
        block_pos: Some([0; 3]),
    };
    let stone = serde_json::json!({"type":"minecraft:match_block","blocks":"minecraft:stone"});
    let wheat = serde_json::json!({"type":"minecraft:match_block","blocks":"minecraft:wheat"});
    assert!(conditions(&stone, &ctx, &Flat));
    assert!(!conditions(&wheat, &ctx, &Flat));
    let absent = Entities {
        block_pos: None,
        ..ctx
    };
    assert!(!conditions(&stone, &absent, &Flat));
}
/// Data pack ids, tags and registry keys written without a namespace are
/// `minecraft:` ones (`Identifier.parse`), wherever a predicate reads one.
#[test]
fn bare_ids_and_tags_mean_minecraft() {
    let w = Flat;
    let player = EntityView {
        kind: "minecraft:player".into(),
        position: [0.5, 64.0, 0.5],
        effects: vec![("minecraft:speed".into(), 0, 100)],
        ..Default::default()
    };
    let stone = ItemView {
        id: "minecraft:cobblestone".into(),
        count: 1,
        ..Default::default()
    };
    let ctx = Entities {
        this: Some(&player),
        origin: player.position,
        tool: None,
        block_pos: None,
    };
    let arrow = DamageView {
        kind: "minecraft:arrow".into(),
        ..Default::default()
    };
    let o = [0.0; 3];
    let j = |s: &str| serde_json::from_str::<Value>(s).unwrap();
    let cases = [
        (
            "item id",
            id_matches(&j(r#""cobblestone""#), "item", &stone.id, &w),
        ),
        (
            "item tag",
            id_matches(&j(r##""#stone_tool_materials""##), "item", &stone.id, &w),
        ),
        (
            "item list",
            id_matches(&j(r#"["dirt", "cobblestone"]"#), "item", &stone.id, &w),
        ),
        (
            "item predicate",
            item(&j(r#"{"items": "cobblestone"}"#), &stone, &w),
        ),
        (
            "dimension",
            location(&j(r#"{"dimension": "overworld"}"#), player.position, &w),
        ),
        (
            "biome",
            location(&j(r#"{"biomes": "plains"}"#), player.position, &w),
        ),
        (
            "entity type",
            entity(&j(r#"{"minecraft:entity_type": "player"}"#), &player, &w, o),
        ),
        (
            "sub-predicate key",
            entity(&j(r#"{"entity_type": "minecraft:player"}"#), &player, &w, o),
        ),
        (
            "effect key",
            entity(
                &j(r#"{"minecraft:effects": {"speed": {}}}"#),
                &player,
                &w,
                o,
            ),
        ),
        (
            "damage tag",
            damage_source(
                &j(r##"{"tags": [{"id": "#is_projectile", "expected": true}]}"##),
                &arrow,
                &w,
                o,
            ),
        ),
        (
            "condition type",
            conditions(
                &j(r#"{"type": "entity_properties", "entity": "this",
                    "predicate": {"minecraft:entity_type": "minecraft:player"}}"#),
                &ctx,
                &w,
            ),
        ),
        // `TagPredicate.id` is a holder set: one damage type is that type.
        (
            "damage type",
            damage_source(&j(r#"{"tags": [{"id": "arrow"}]}"#), &arrow, &w, o),
        ),
        // Near misses stay misses.
        (
            "other item",
            !id_matches(&j(r#"["dirt", "stone"]"#), "item", &stone.id, &w),
        ),
        (
            "other dimension",
            !location(&j(r#"{"dimension": "the_nether"}"#), player.position, &w),
        ),
        (
            "other effect",
            !entity(&j(r#"{"effects": {"slowness": {}}}"#), &player, &w, o),
        ),
        (
            "other damage type",
            !damage_source(&j(r#"{"tags": [{"id": "fireball"}]}"#), &arrow, &w, o),
        ),
        (
            "the type key is no sub-predicate",
            !entity(&j(r#"{"type": "player"}"#), &player, &w, o),
        ),
    ];
    let failed: Vec<&str> = cases.iter().filter(|c| !c.1).map(|c| c.0).collect();
    assert!(
        failed.is_empty(),
        "bare ids not read as minecraft: {failed:?}"
    );
}
/// A bare trigger, item or recipe id reaches the indexes as `minecraft:`.
#[test]
fn bare_ids_reach_the_trigger_and_item_indexes() {
    let a = parse(
        "test:a",
        &serde_json::json!({"criteria": {
            "items": {"trigger": "inventory_changed",
                "conditions": {"items": [{"items": ["cobblestone", "#planks"]}]}},
            "recipe": {"trigger": "recipe_unlocked", "conditions": {"recipes": "stick"}}
        }}),
    )
    .unwrap();
    let mut tree = Tree::default();
    for (ci, c) in a.criteria.iter().enumerate() {
        tree.by_trigger
            .entry(c.trigger.clone())
            .or_default()
            .push((0, ci));
    }
    tree.list.push(a);
    tree.index_recipes();
    let index = InventoryIndex::build(&tree, |tag| match tag {
        "minecraft:planks" => vec!["minecraft:oak_planks"],
        _ => Vec::new(),
    });
    let mut failed = Vec::new();
    if !tree.by_trigger.contains_key("minecraft:inventory_changed") {
        failed.push("trigger");
    }
    if index.candidates(["minecraft:cobblestone"]).is_empty() {
        failed.push("item index id");
    }
    if index.candidates(["minecraft:oak_planks"]).is_empty() {
        failed.push("item index tag");
    }
    if tree.recipe_criteria("minecraft:stick").is_empty() {
        failed.push("recipe index");
    }
    assert!(
        failed.is_empty(),
        "bare ids not read as minecraft: {failed:?}"
    );
}
#[test]
fn predicates_match_tags_ranges_and_locations() {
    let w = Flat;
    let stone = ItemView {
        id: "minecraft:cobblestone".into(),
        count: 3,
        ..Default::default()
    };
    assert!(item(
        &serde_json::json!({"items": "#minecraft:stone_tool_materials"}),
        &stone,
        &w
    ));
    assert!(!item(
        &serde_json::json!({"items": "minecraft:dirt"}),
        &stone,
        &w
    ));
    assert!(item(&serde_json::json!({"count": {"min": 2}}), &stone, &w));
    let player = EntityView {
        kind: "minecraft:player".into(),
        position: [0.5, 64.0, 0.5],
        ..Default::default()
    };
    let cond = serde_json::json!({
        "type": "minecraft:entity_properties",
        "entity": "this",
        "predicate": {"minecraft:location": {"structures": "minecraft:village_plains"}}
    });
    let ctx = Entities {
        this: Some(&player),
        origin: player.position,
        tool: None,
        block_pos: None,
    };
    assert!(conditions(&cond, &ctx, &w));
    let mut p = Progress::default();
    let a = parse(
        "minecraft:t",
        &serde_json::json!({"criteria": {"a": {"trigger": "minecraft:tick"}, "b": {"trigger": "minecraft:tick"}},
            "requirements": [["a", "b"]]}),
    )
    .unwrap();
    assert!(!p.done(&a));
    p.grant("minecraft:t", "b", "now");
    assert!(p.done(&a));
}
/// `DisplayInfo.icon` is an `ItemStackTemplate`: the loaded icon keeps its components.
#[test]
fn a_display_icon_keeps_its_components() {
    let a = parse(
        "minecraft:t",
        &serde_json::json!({
            "display": {
                "icon": {
                    "id": "minecraft:wooden_pickaxe",
                    "components": {"minecraft:enchantment_glint_override": true}
                },
                "title": "t",
                "description": "d"
            },
            "criteria": {"a": {"trigger": "minecraft:tick"}}
        }),
    )
    .unwrap();
    let icon = a.display.unwrap().icon;
    assert_eq!(&*icon.item, "minecraft:wooden_pickaxe");
    assert!(icon.has_foil(None), "the glint override survives the load");
}
/// `AdvancementRequirements.test` answers false with no requirement groups, so an advancement
/// without criteria is never done by `isDone`; `PlayerAdvancements.checkForAutomaticTriggers`
/// still grants its rewards as progress loads, and `Tree::automatic` is the list it walks.
#[test]
fn an_advancement_without_criteria_is_not_done_but_granted_automatically() {
    let empty = parse("minecraft:free", &serde_json::json!({"criteria": {}})).unwrap();
    assert!(!Progress::default().done(&empty), "no groups: not done");
    let waiting = parse(
        "minecraft:waits",
        &serde_json::json!({"criteria": {"a": {"trigger": "minecraft:tick"}}}),
    )
    .unwrap();
    assert!(!Progress::default().done(&waiting));
    let tree = tree_of(&[
        (
            "minecraft:waits",
            serde_json::json!({"criteria": {"a": {"trigger": "minecraft:tick"}}}),
        ),
        ("minecraft:free", serde_json::json!({"criteria": {}})),
    ]);
    assert_eq!(tree.automatic(), vec![1]);
}
#[test]
fn layout_spreads_children_like_tree_node_position() {
    let mut tree = Tree::default();
    let shown = Display {
        icon: crate::items::ItemStack::new("minecraft:air", 1),
        title: Value::Null,
        description: Value::Null,
        frame: Frame::Task,
        background: None,
        show_toast: true,
        announce: true,
        hidden: false,
    };
    // Sorted ids; "b" names a parent that sorts after it.
    for (id, parent) in [
        ("a", Some("r")),
        ("b", Some("e")),
        ("c", Some("a")),
        ("d", Some("a")),
        ("e", Some("r")),
        ("r", None),
    ] {
        tree.by_id.insert(id.into(), tree.list.len());
        tree.list.push(Advancement {
            id: id.into(),
            parent: parent.map(Into::into),
            display: Some(shown.clone()),
            criteria: Vec::new(),
            requirements: Vec::new(),
            rewards: Rewards::default(),
        });
    }
    let kids = tree.children();
    assert_eq!(kids[5], vec![0, 4]);
    let l = tree.layout();
    assert_eq!(l[5], Some((0, 1.25)));
    assert_eq!(l[0], Some((1, 0.5)));
    assert_eq!(l[2], Some((2, 0.0)));
    assert_eq!(l[3], Some((2, 1.0)));
    assert_eq!(l[4], Some((1, 2.0)));
    assert_eq!(l[1], Some((2, 2.0)));
}
/// A tree of the given advancements, indexed as `load_with` does.
fn tree_of(list: &[(&str, Value)]) -> Tree {
    let mut tree = Tree::default();
    for (id, v) in list {
        let a = parse(id, v).expect("an advancement");
        for (ci, c) in a.criteria.iter().enumerate() {
            (tree.by_trigger.entry(c.trigger.clone()).or_default()).push((tree.list.len(), ci));
        }
        tree.by_id.insert((*id).into(), tree.list.len());
        tree.list.push(a);
    }
    tree.index_player_facts();
    tree
}
/// Whether the player `facts` match `{"type_specific/player": p}`.
fn player_matches(p: Value, facts: &PlayerFacts) -> bool {
    let me = EntityView {
        kind: "minecraft:player".into(),
        player_facts: facts.clone(),
        ..EntityView::default()
    };
    entity(&json!({"type_specific/player": p}), &me, &Flat, [0.0; 3])
}
#[test]
fn player_level_is_tested_in_its_bounds() {
    let five = PlayerFacts {
        level: Some(5),
        ..PlayerFacts::default()
    };
    for (level, expected) in [
        (json!({"min": 5}), true),
        (json!({"min": 6}), false),
        (json!({"max": 4}), false),
        (json!({"min": 1, "max": 5}), true),
        (json!(5), true),
        (json!(6), false),
    ] {
        assert_eq!(
            player_matches(json!({"level": level}), &five),
            expected,
            "{level}"
        );
    }
    // Not filled: a level asked fails, none asked passes.
    assert!(!player_matches(
        json!({"level": {"min": 0}}),
        &PlayerFacts::default()
    ));
    assert!(player_matches(json!({}), &PlayerFacts::default()));
}
#[test]
fn player_food_tests_level_and_saturation_in_their_bounds() {
    let fed = PlayerFacts {
        food: Some((17, 3.0)),
        ..PlayerFacts::default()
    };
    for (food, expected) in [
        (json!({}), true),
        (json!({"level": {"min": 17}}), true),
        (json!({"level": {"min": 18}}), false),
        (json!({"level": {"max": 16}}), false),
        (json!({"level": 17}), true),
        (json!({"saturation": {"min": 3.0}}), true),
        (json!({"saturation": {"min": 3.5}}), false),
        (json!({"saturation": {"max": 2.9}}), false),
        (
            json!({"level": {"min": 17}, "saturation": {"max": 3.0}}),
            true,
        ),
        (
            json!({"level": {"min": 17}, "saturation": {"min": 3.5}}),
            false,
        ),
        (
            json!({"level": {"min": 18}, "saturation": {"min": 3.0}}),
            false,
        ),
    ] {
        assert_eq!(
            player_matches(json!({"food": food}), &fed),
            expected,
            "{food}"
        );
    }
    // The saturation float widens to a double: 1.2f is 1.2000000476837158.
    let widened = PlayerFacts {
        food: Some((20, 1.2)),
        ..PlayerFacts::default()
    };
    assert!(player_matches(
        json!({"food": {"saturation": {"min": 1.2}}}),
        &widened
    ));
    assert!(!player_matches(
        json!({"food": {"saturation": {"max": 1.2}}}),
        &widened
    ));
    // No food known: food asked fails, even unbounded.
    assert!(!player_matches(
        json!({"food": {}}),
        &PlayerFacts::default()
    ));
}
#[test]
fn player_stats_match_each_counter_by_type_and_id() {
    let mut counts: BTreeMap<String, BTreeMap<String, i32>> = BTreeMap::new();
    (counts.entry("minecraft:custom".into()).or_default())
        .insert("minecraft:walk_one_cm".into(), 1000);
    (counts.entry("minecraft:mined".into()).or_default()).insert("minecraft:stone".into(), 3);
    let facts = PlayerFacts {
        stats: Some(counts),
        ..PlayerFacts::default()
    };
    let stat = |ty: &str, id: &str, value: Value| json!({"type": ty, "stat": id, "value": value});
    for (stats, expected) in [
        (
            vec![stat(
                "minecraft:custom",
                "minecraft:walk_one_cm",
                json!({"min": 1000}),
            )],
            true,
        ),
        (
            vec![stat(
                "minecraft:custom",
                "minecraft:walk_one_cm",
                json!({"min": 1001}),
            )],
            false,
        ),
        (
            vec![stat(
                "minecraft:custom",
                "minecraft:walk_one_cm",
                json!({"max": 999}),
            )],
            false,
        ),
        // Bare ids are `minecraft:` ones.
        (
            vec![stat("custom", "walk_one_cm", json!({"min": 1000}))],
            true,
        ),
        // The same id under another type is another counter: never counted, 0.
        (
            vec![stat(
                "minecraft:mined",
                "minecraft:walk_one_cm",
                json!({"min": 1}),
            )],
            false,
        ),
        (
            vec![stat(
                "minecraft:mined",
                "minecraft:walk_one_cm",
                json!({"max": 0}),
            )],
            true,
        ),
        (
            vec![stat("minecraft:killed", "minecraft:zombie", json!(0))],
            true,
        ),
        // Every entry must match.
        (
            vec![
                stat("minecraft:mined", "minecraft:stone", json!({"min": 3})),
                stat(
                    "minecraft:custom",
                    "minecraft:walk_one_cm",
                    json!({"min": 5000}),
                ),
            ],
            false,
        ),
        (
            vec![
                stat("minecraft:mined", "minecraft:stone", json!({"min": 3})),
                stat(
                    "minecraft:custom",
                    "minecraft:walk_one_cm",
                    json!({"min": 500}),
                ),
            ],
            true,
        ),
    ] {
        let p = json!({"stats": stats});
        assert_eq!(player_matches(p.clone(), &facts), expected, "{p}");
    }
    let walked = json!({"stats": [stat("custom", "walk_one_cm", json!({"min": 0}))]});
    assert!(!player_matches(walked, &PlayerFacts::default()));
}
#[test]
fn player_recipes_are_known_or_not_as_asked() {
    let facts = PlayerFacts {
        recipes: Some(["minecraft:iron_pickaxe".to_owned()].into()),
        ..PlayerFacts::default()
    };
    for (recipes, expected) in [
        (json!({"minecraft:iron_pickaxe": true}), true),
        (json!({"iron_pickaxe": true}), true),
        (json!({"minecraft:iron_pickaxe": false}), false),
        (json!({"minecraft:diamond_pickaxe": false}), true),
        (json!({"minecraft:diamond_pickaxe": true}), false),
        (
            json!({"minecraft:iron_pickaxe": true, "minecraft:diamond_pickaxe": true}),
            false,
        ),
    ] {
        assert_eq!(
            player_matches(json!({"recipes": recipes}), &facts),
            expected,
            "{recipes}"
        );
    }
    let unknown = json!({"recipes": {"minecraft:diamond_pickaxe": false}});
    assert!(!player_matches(unknown, &PlayerFacts::default()));
}
#[test]
fn player_advancements_are_done_or_have_their_criteria_as_asked() {
    let tick = json!({"trigger": "minecraft:tick"});
    let tree = Arc::new(tree_of(&[(
        "pack:a",
        json!({"criteria": {"x": tick, "y": tick}}),
    )]));
    let mut progress = Progress::default();
    progress.grant("pack:a", "x", "2026-11-04 12:00:00 +0000");
    let half = PlayerFacts {
        advancements: Some((tree.clone(), progress.clone())),
        ..PlayerFacts::default()
    };
    for (advancements, expected) in [
        (json!({"pack:a": true}), false),
        (json!({"pack:a": false}), true),
        (json!({"pack:a": {"x": true}}), true),
        (json!({"pack:a": {"x": false}}), false),
        (json!({"pack:a": {"y": true}}), false),
        (json!({"pack:a": {"y": false}}), true),
        (json!({"pack:a": {"x": true, "y": true}}), false),
        // A criterion the advancement doesn't have, or an advancement
        // that isn't loaded, matches nothing.
        (json!({"pack:a": {"z": false}}), false),
        (json!({"pack:missing": false}), false),
    ] {
        assert_eq!(
            player_matches(json!({"advancements": advancements}), &half),
            expected,
            "{advancements}"
        );
    }
    progress.grant("pack:a", "y", "2026-11-04 12:00:01 +0000");
    let done = PlayerFacts {
        advancements: Some((tree, progress)),
        ..PlayerFacts::default()
    };
    assert!(player_matches(
        json!({"advancements": {"pack:a": true}}),
        &done
    ));
    assert!(!player_matches(
        json!({"advancements": {"pack:a": false}}),
        &done
    ));
    let asked = json!({"advancements": {"pack:a": false}});
    assert!(!player_matches(asked, &PlayerFacts::default()));
}
#[test]
fn player_facts_are_asked_by_the_triggers_that_name_them() {
    let player = |p: Value| {
        json!({"player": [{"condition": "minecraft:entity_properties", "entity": "this",
            "predicate": {"minecraft:type_specific/player": p}}]})
    };
    let tree = tree_of(&[
        (
            "pack:walk",
            json!({"criteria": {"c": {"trigger": "minecraft:tick",
                "conditions": player(json!({"stats": [{"type": "minecraft:custom",
                    "stat": "minecraft:walk_one_cm", "value": {"min": 1}}]}))}}}),
        ),
        (
            "pack:milestone",
            json!({"criteria": {"c": {"trigger": "minecraft:location",
                "conditions": player(json!({"advancements": {"pack:walk": true},
                    "level": {"min": 30}}))}}}),
        ),
        (
            "pack:ref",
            json!({"criteria": {"c": {"trigger": "minecraft:inventory_changed",
                "conditions": {"player": [{"condition": "minecraft:reference",
                    "name": "pack:p"}]}}}}),
        ),
        (
            "pack:recipe",
            json!({"criteria": {"c": {"trigger": "minecraft:recipe_unlocked",
                "conditions": {"recipe": "minecraft:iron_pickaxe"}},
                "d": {"trigger": "minecraft:using_item",
                "conditions": player(json!({"gamemode": ["survival"]}))}}}),
        ),
    ]);
    let none = FactsAsked::default();
    assert_eq!(
        tree.player_facts_asked("minecraft:tick"),
        FactsAsked {
            stats: true,
            ..none
        }
    );
    assert_eq!(
        tree.player_facts_asked("minecraft:location"),
        FactsAsked {
            advancements: true,
            level: true,
            ..none
        }
    );
    assert_eq!(
        tree.player_facts_asked("minecraft:inventory_changed"),
        FactsAsked::ALL
    );
    assert_eq!(tree.player_facts_asked("minecraft:recipe_unlocked"), none);
    assert_eq!(tree.player_facts_asked("minecraft:using_item"), none);
}
#[test]
fn a_command_predicate_asks_only_the_player_facts_it_names() {
    let player = |p: Value| {
        json!({"condition": "minecraft:entity_properties", "entity": "this",
            "predicate": {"type_specific/player": p}})
    };
    let none = FactsAsked::default();
    for (v, expected) in [
        // Nothing of the player's records: nothing copied.
        (
            json!({"condition": "minecraft:entity_properties", "entity": "this",
            "predicate": {"flags": {"is_sneaking": true}}}),
            none,
        ),
        (player(json!({"gamemode": ["survival"]})), none),
        (
            json!({"condition": "minecraft:random_chance", "chance": 0.5}),
            none,
        ),
        // A field named elsewhere than in a player predicate.
        (
            json!({"function": "minecraft:enchant_with_levels", "levels": 30,
            "conditions": [{"condition": "minecraft:inverted",
            "term": {"condition": "minecraft:entity_properties", "entity": "this",
            "predicate": {"minecraft:effects": {"minecraft:speed": {}}}}}]}),
            none,
        ),
        (
            player(json!({"level": {"min": 3}})),
            FactsAsked {
                level: true,
                ..none
            },
        ),
        // Nested in a list, under `inverted`, in an item modifier.
        (
            json!([{"function": "minecraft:set_count", "count": 2, "conditions": [
                {"condition": "minecraft:inverted", "term": player(json!({
                    "stats": [], "recipes": {"minecraft:bread": true}}))}]}]),
            FactsAsked {
                stats: true,
                recipes: true,
                ..none
            },
        ),
        (
            json!({"condition": "minecraft:all_of", "terms": [player(json!(
                {"advancements": {"minecraft:story/root": true}}))]}),
            FactsAsked {
                advancements: true,
                ..none
            },
        ),
        // A predicate file could ask anything.
        (
            json!({"condition": "reference", "name": "pack:p"}),
            FactsAsked::ALL,
        ),
        (
            json!([{"function": "minecraft:reference", "name": "pack:m"}]),
            FactsAsked::ALL,
        ),
    ] {
        assert_eq!(facts_asked_in(&v), expected, "{v}");
    }
}
#[test]
fn saved_dates_read_back_to_their_instant() {
    for secs in [0, 951_782_400, 1_792_216_800, 4_107_542_399] {
        assert_eq!(date_millis(&date_text(secs)), Some(secs * 1000));
    }
    assert_eq!(
        date_millis("2026-10-17 07:30:00 +0200"),
        date_millis("2026-10-17 05:30:00 +0000")
    );
    assert_eq!(date_millis("not a date"), None);
}
#[test]
fn a_date_outside_the_range_is_not_a_date() {
    // the year would overflow the arithmetic; a month or day that does not exist is no date either
    assert_eq!(date_millis("999999999999999 01-01 10:00:00 +0000"), None);
    assert_eq!(
        date_millis("9223372036854775807-05-05 10:00:00 +0000"),
        None
    );
    assert_eq!(date_millis("2026-13-01 10:00:00 +0000"), None);
    assert_eq!(date_millis("2026-05-00 10:00:00 +0000"), None);
    assert_eq!(date_millis(""), None);
}
#[test]
fn simulate_progression_advancements() {
    let mut tree = Tree::default();
    let advs = [
        ("minecraft:story/root", None),
        ("minecraft:story/mine_stone", Some("story/root")),
        (
            "minecraft:story/upgrade_tools",
            Some("minecraft:story/mine_stone"),
        ),
        ("minecraft:story/smelt_iron", Some("story/upgrade_tools")),
        (
            "minecraft:story/obtain_armor",
            Some("minecraft:story/smelt_iron"),
        ),
        (
            "minecraft:story/lava_bucket",
            Some("minecraft:story/smelt_iron"),
        ),
        (
            "minecraft:story/iron_tools",
            Some("minecraft:story/smelt_iron"),
        ),
        (
            "minecraft:story/deflect_arrow",
            Some("minecraft:story/obtain_armor"),
        ),
        (
            "minecraft:story/form_obsidian",
            Some("minecraft:story/lava_bucket"),
        ),
        (
            "minecraft:story/mine_diamond",
            Some("minecraft:story/iron_tools"),
        ),
        (
            "minecraft:story/enter_the_nether",
            Some("minecraft:story/form_obsidian"),
        ),
        (
            "minecraft:story/shiny_gear",
            Some("minecraft:story/mine_diamond"),
        ),
        (
            "minecraft:story/enchant_item",
            Some("minecraft:story/mine_diamond"),
        ),
        (
            "minecraft:story/cure_zombie_villager",
            Some("minecraft:story/enter_the_nether"),
        ),
        (
            "minecraft:story/follow_ender_eye",
            Some("minecraft:story/enter_the_nether"),
        ),
        (
            "minecraft:story/enter_the_end",
            Some("minecraft:story/follow_ender_eye"),
        ),
    ];
    for (id, parent) in advs {
        let a = parse(
            id,
            &serde_json::json!({
                "parent": parent,
                "criteria": {"crit": {"trigger": "minecraft:tick"}},
                "display": {"icon": {"id": "minecraft:stone"}, "title": "t", "description": "d"}
            }),
        )
        .unwrap();
        tree.by_id.insert(id.to_string(), tree.list.len());
        tree.list.push(a);
    }
    for i in 16..124 {
        let id = format!("minecraft:custom/{i}");
        let a = parse(
            &id,
            &serde_json::json!({
                "criteria": {"crit": {"trigger": "minecraft:tick"}},
                "display": {"icon": {"id": "minecraft:stone"}, "title": "t", "description": "d"}
            }),
        )
        .unwrap();
        tree.by_id.insert(id, tree.list.len());
        tree.list.push(a);
    }
    fn get_list(tree: &Tree, target: Option<&str>, mode: &str) -> Vec<usize> {
        match target {
            None => (0..tree.list.len()).collect(),
            Some(id) => {
                let i = *tree.by_id.get(id).unwrap();
                let mut out = Vec::new();
                if mode == "until" || mode == "through" {
                    let mut p = tree.list[i].parent.as_ref();
                    while let Some(&pi) = p.and_then(|pid| tree.by_id.get(pid)) {
                        out.push(pi);
                        p = tree.list[pi].parent.as_ref();
                    }
                }
                out.push(i);
                if mode == "from" || mode == "through" {
                    let kids = tree.children();
                    fn add(kids: &[Vec<usize>], i: usize, out: &mut Vec<usize>) {
                        for &c in &kids[i] {
                            out.push(c);
                            add(kids, c, out);
                        }
                    }
                    add(&kids, i, &mut out);
                }
                out
            }
        }
    }
    fn grant(tree: &Tree, prog: &mut Progress, list: &[usize]) -> i32 {
        let mut count = 0;
        for &i in list {
            let a = &tree.list[i];
            if prog.done(a) {
                continue;
            }
            for c in &a.criteria {
                prog.grant(&a.id, &c.name, "now");
            }
            count += 1;
        }
        count
    }
    fn revoke(tree: &Tree, prog: &mut Progress, list: &[usize]) -> i32 {
        let mut count = 0;
        for &i in list {
            let a = &tree.list[i];
            let mut any = false;
            for c in &a.criteria {
                any |= prog.revoke(&a.id, &c.name);
            }
            if any {
                count += 1;
            }
        }
        count
    }

    let mut p = Progress::default();
    let until = get_list(&tree, Some("minecraft:story/smelt_iron"), "until");
    assert_eq!(grant(&tree, &mut p, &until), 4);

    let from = get_list(&tree, Some("minecraft:story/root"), "from");
    assert_eq!(grant(&tree, &mut p, &from), 12);

    let through = get_list(&tree, Some("minecraft:story/iron_tools"), "through");
    assert_eq!(revoke(&tree, &mut p, &through), 8);

    let all = get_list(&tree, None, "everything");
    let granted_everything = grant(&tree, &mut p, &all);
    assert_eq!(granted_everything, 116);
}
#[test]
fn random_chance_literal_matches_official_seeded_float_and_continuation() {
    use horizon_core::random::{LegacyRandom, RandomSource};
    let ctx = Entities {
        this: None,
        origin: [0.0; 3],
        tool: None,
        block_pos: None,
    };
    for line in include_str!("../../../../docs/parity/random-chance-seeded.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let mut random = LegacyRandom::new(row["seed"].as_i64().unwrap());
        let result = conditions_with(&row["input"], &ctx, &mut |v, ctx| {
            literal_random_chance(v, &mut || random.next_float())
                .unwrap_or_else(|| conditions(v, ctx, &Flat))
        });
        assert_eq!(result, row["result"].as_bool().unwrap(), "{row}");
        let next: Vec<i64> = (0..4).map(|_| random.next_long()).collect();
        let expected: Vec<i64> = row["continuation"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect();
        assert_eq!(next, expected, "{row}");
    }
}
#[test]
fn existing_no_random_context_and_traversal_keep_their_behavior() {
    let ctx = Entities {
        this: None,
        origin: [0.0; 3],
        tool: None,
        block_pos: None,
    };
    let z = json!({"type":"minecraft:random_chance","chance":0});
    let o = json!({"type":"minecraft:random_chance","chance":1});
    for (v, expected) in [
        (Value::Null, true),
        (json!([]), true),
        (z.clone(), false),
        (o.clone(), false),
        (json!({"type":"inverted","term":o}), true),
        (
            json!({"type":"all_of","terms":[z.clone(),Value::Null]}),
            false,
        ),
        (
            json!({"type":"any_of","terms":[z.clone(),Value::Null]}),
            true,
        ),
        (json!("h122:unsupported_reference"), false),
    ] {
        assert_eq!(conditions(&v, &ctx, &Flat), expected, "{v}");
    }
    let mut seen = vec![];
    let v = json!({"type":"any_of","terms":[{"type":"all_of","terms":[false,true]},true,false]});
    assert!(conditions_with(&v, &ctx, &mut |v, _| {
        seen.push(v.as_bool().unwrap());
        v.as_bool().unwrap()
    }));
    assert_eq!(seen, vec![false, true]);
}
#[test]
fn unsupported_chance_shapes_keep_prior_false_without_a_draw() {
    let ctx = Entities {
        this: None,
        origin: [0.0; 3],
        tool: None,
        block_pos: None,
    };
    let mut draws = 0;
    for v in [
        json!({"type":"random_chance","chance":{"type":"minecraft:uniform","min":0,"max":1}}),
        json!({"type":"random_chance"}),
        json!({"type":"random_chance","chance":"1"}),
        json!({"type":"random_chance","chance":1,"unsupported":true}),
    ] {
        assert_eq!(
            literal_random_chance(&v, &mut || {
                draws += 1;
                0.0
            }),
            None
        );
        assert!(!conditions(&v, &ctx, &Flat));
    }
    assert_eq!(draws, 0);
}

#[cfg(test)]
mod movement_model_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn added_model_movement_units_exact_bounds_and_squared_bounds() {
        // Source-derived model proof; no moving Java/runtime claim.
        let m = MovementView {
            per_tick: [3.0 / 20.0, -4.0 / 20.0, 0.0],
            fall_distance: 7.25,
        };
        assert_eq!(m.per_tick.map(|n| n * 20.0), [3.0, -4.0, 0.0]);
        for p in [
            json!({"x":3.0,"y":-4.0,"z":0.0}),
            json!({"x":{"min":3.0,"max":3.0}}),
            json!({"speed":5.0,"horizontal_speed":3.0,"vertical_speed":4.0,"fall_distance":7.25}),
            json!({"speed":{"min":5.0,"max":5.0}}),
            // Doubles boundsSqr maps each bound through Mth.square, including its sign.
            json!({"speed":-5.0,"horizontal_speed":{"max":-3.0}}),
            json!({}),
        ] {
            assert!(movement(&p, m), "{p}");
        }
        for p in [
            json!({"x":3.0000000001}),
            json!({"x":{"min":3.0000000001}}),
            json!({"y":{"max":-4.0000000001}}),
            json!({"speed":{"max":4.9999999999}}),
            json!({"horizontal_speed":{"min":-4.0}}),
            json!({"vertical_speed":-4.0}),
            json!({"fall_distance":{"max":7.2499999999}}),
        ] {
            assert!(!movement(&p, m), "{p}");
        }
        assert!(!movement_range(&json!(3.0000000001), 3.0, false));
        assert!(
            range_f(&json!(3.0000000001), 3.0),
            "existing global tolerance is untouched"
        );
    }

    #[test]
    fn added_model_unprojected_is_distinct_from_actual_stationary() {
        let p = json!({"minecraft:movement":{}});
        let view = EntityView {
            kind: "minecraft:pig".into(),
            ..Default::default()
        };
        // Unprojected is explicitly unsupported, not claimed Java-correct.
        assert!(!entity(&p, &view, &super::Flat, [0.0; 3]));
        let view = EntityView {
            movement: Some(MovementView {
                per_tick: [0.0; 3],
                fall_distance: 0.0,
            }),
            ..view
        };
        assert!(entity(&p, &view, &super::Flat, [0.0; 3]));
    }
}

#[test]
fn tick_filter_never_rejects_matching_entity_views() {
    use serde_json::json;

    struct BlockWorld {
        block_name: &'static str,
    }
    impl World for BlockWorld {
        fn dimension(&self) -> &str {
            "minecraft:overworld"
        }
        fn weather(&self) -> (bool, bool) {
            (false, false)
        }
        fn biome(&self, _: [i32; 3]) -> String {
            "minecraft:plains".into()
        }
        fn structures(&self, _: [i32; 3]) -> Vec<String> {
            Vec::new()
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            (self.block_name.into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            true
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            15
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, _: &str, _: &str, _: &str) -> bool {
            false
        }
    }

    let filter_matches_view = |f: &TickFilter, e: &EntityView, w: &dyn World| -> bool {
        if f.requires_sneaking && !e.sneaking {
            return false;
        }
        if f.requires_sprinting && !e.sprinting {
            return false;
        }
        if f.requires_swimming && !e.swimming {
            return false;
        }
        if f.requires_on_fire && !e.on_fire {
            return false;
        }
        if f.requires_vehicle && e.vehicle.is_none() {
            return false;
        }
        if f.requires_effects && e.effects.is_empty() {
            return false;
        }
        for bit in 0..6 {
            if (f.requires_equipment_mask & (1 << bit)) != 0 && e.equipment[bit].is_none() {
                return false;
            }
        }
        if let Some(wanted) = &f.requires_location_block {
            let pos = [
                e.position[0].floor() as i32,
                e.position[1].floor() as i32,
                e.position[2].floor() as i32,
            ];
            if &w.block(pos).0 != wanted {
                return false;
            }
        }
        true
    };

    let predicates = [
        json!({"condition": "minecraft:entity_properties", "entity": "this", "predicate": {"flags": {"is_sneaking": true}, "equipment": {"feet": {"items": "minecraft:leather_boots"}}}}),
        json!({"condition": "minecraft:all_of", "terms": [
            {"condition": "minecraft:entity_properties", "entity": "this", "predicate": {"flags": {"is_sprinting": true, "is_on_fire": true}}},
            {"condition": "minecraft:entity_properties", "entity": "this", "predicate": {"location": {"block": {"blocks": "minecraft:water"}}}}
        ]}),
        json!({"flags": {"is_swimming": true, "is_flying": true}, "stepping_on": {"block": {"blocks": "minecraft:stone"}}}),
        json!({"vehicle": {"entity_type": "minecraft:horse"}, "effects": {"minecraft:speed": {}}}),
        json!({"condition": "minecraft:inverted", "term": {"condition": "minecraft:entity_properties", "entity": "this", "predicate": {"flags": {"is_sneaking": true}}}}),
        json!({"condition": "minecraft:any_of", "terms": [{"condition": "minecraft:entity_properties", "entity": "this", "predicate": {"flags": {"is_sneaking": true}}}, {"flags": {"is_sprinting": true}}]}),
    ];

    let mut views = Vec::new();
    for sneaking in [false, true] {
        for sprinting in [false, true] {
            for swimming in [false, true] {
                for on_fire in [false, true] {
                    for flying in [false, true] {
                        for has_feet in [false, true] {
                            let mut eq: [Option<ItemView>; 7] = Default::default();
                            if has_feet {
                                eq[5] = Some(ItemView {
                                    id: "minecraft:leather_boots".into(),
                                    count: 1,
                                    ..Default::default()
                                });
                            }
                            views.push(EntityView {
                                kind: "minecraft:player".into(),
                                position: [0.5, 64.0, 0.5],
                                on_fire,
                                sneaking,
                                sprinting,
                                swimming,
                                flying,
                                equipment: eq,
                                vehicle: Some(Box::new(EntityView {
                                    kind: "minecraft:horse".into(),
                                    ..Default::default()
                                })),
                                effects: vec![("minecraft:speed".into(), 0, 100)],
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }
    }
    views.push(EntityView {
        kind: "minecraft:player".into(),
        ..Default::default()
    });

    for block_name in ["minecraft:air", "minecraft:water", "minecraft:stone"] {
        let w = BlockWorld { block_name };
        for p in &predicates {
            let mut filter = TickFilter::default();
            collect_tick_player_filter(p, &mut filter);
            for v in &views {
                let ctx = Entities {
                    this: Some(v),
                    origin: v.position,
                    tool: None,
                    block_pos: None,
                };
                if conditions(p, &ctx, &w) {
                    assert!(
                        filter_matches_view(&filter, v, &w),
                        "TickFilter rejected view accepted by conditions! pred={p} filter={filter:?} view={v:?} block={block_name}"
                    );
                }
            }
        }
    }

    // Gliding/stepping predicates must not constrain the pre-filter (safe wrt #17473).
    let mut fly_step_filter = TickFilter::default();
    collect_tick_player_filter(&predicates[2], &mut fly_step_filter);
    assert!(fly_step_filter.requires_swimming);
    assert!(filter_matches_view(
        &fly_step_filter,
        &EntityView {
            swimming: true,
            flying: false,
            ..Default::default()
        },
        &BlockWorld {
            block_name: "minecraft:air",
        }
    ));
}

#[test]
fn references_valid_fast_path_matches_full_scan() {
    let w = Flat;
    for v in [
        serde_json::json!(null),
        serde_json::json!([{"condition": "minecraft:entity_properties", "entity": "this", "predicate": {"flags": {"is_sneaking": true}}}]),
        serde_json::json!({"condition": "minecraft:all_of", "terms": [{"condition": "minecraft:entity_properties", "predicate": {}}]}),
        serde_json::json!({"condition": "minecraft:inverted", "term": "missing:ref"}),
        serde_json::json!({"condition": "minecraft:any_of", "terms": ["missing:ref"]}),
    ] {
        assert_eq!(
            references_valid(&v, &w),
            predicate_references(&v, &w, &mut BTreeSet::new()).is_ok(),
            "{v}"
        );
    }
}

fn tree_from_entries<I>(entries: I) -> Tree
where
    I: IntoIterator<Item = (String, Value)>,
{
    let mut tree = Tree::default();
    for (id, v) in entries {
        if let Some(a) = parse(&id, &v) {
            let index = tree.list.len();
            for (ci, c) in a.criteria.iter().enumerate() {
                tree.by_trigger
                    .entry(c.trigger.clone())
                    .or_default()
                    .push((index, ci));
            }
            tree.by_id.insert(id, index);
            tree.list.push(a);
        }
    }
    tree.index_tick_filters();
    tree
}

#[test]
fn progress_done_and_counts_match_empty_and_partial_states() {
    let tree = tree_from_entries([(
        "minecraft:test/a".to_owned(),
        serde_json::json!({
            "criteria": {
                "c1": {"trigger": "minecraft:tick", "conditions": {"player": {"flags": {"is_sneaking": true}}}},
                "c2": {"trigger": "minecraft:tick", "conditions": {}}
            },
            "requirements": [["c1", "c2"]]
        }),
    )]);
    assert_eq!(tree.tick_filters().len(), 2);
    let a = &tree.list[0];
    let mut progress = Progress::default();
    assert!(!progress.done(a));
    assert_eq!(progress.counts(a), (0, 1));
    assert!(progress.grant(&a.id, "c1", "2026-01-01"));
    assert!(progress.done(a));
    assert_eq!(progress.counts(a), (1, 1));
}
