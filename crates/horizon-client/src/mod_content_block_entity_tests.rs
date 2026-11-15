use super::*;

fn base(name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "horizon-block-entity-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("assets/minecraft/blockstates")).unwrap();
    for (file, text) in [
        (
            "facts.json",
            r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
        ),
        (
            "gameplay-facts.json",
            r#"{"states":[{"item":"minecraft:stone"}]}"#,
        ),
        (
            "registries.json",
            r#"{"minecraft:item":{"entries":{}},"minecraft:block":{"entries":{}},
                "minecraft:block_entity_type":{"entries":{"minecraft:sign":{"protocol_id":8}}}}"#,
        ),
        ("items.json", "{}"),
        ("assets/minecraft/blockstates/stone.json", "{}"),
        ("horizon-resources.json", "{}"),
    ] {
        fs::write(base.join(file), text).unwrap();
    }
    base
}

fn kind(id: &str, blocks: &[&str]) -> BlockEntityType {
    BlockEntityType {
        id: id.into(),
        blocks: blocks.iter().map(|b| (*b).into()).collect(),
    }
}

fn declared_mod() -> ContentMod {
    let mut content = Content::declared("test");
    content
        .declare(
            &["test:a", "test:b"].map(|id| Block {
                id: id.into(),
                template: "minecraft:stone".into(),
                ..Block::default()
            }),
            &[],
        )
        .unwrap();
    ContentMod {
        id: "test".into(),
        content,
        jars: Vec::new(),
    }
}

#[test]
fn code_types_join_package_and_code_blocks_before_planning() {
    let base = base("merge");
    let mut mods = vec![declared_mod()];
    let blocks = [Block {
        id: "test:c".into(),
        template: "minecraft:stone".into(),
        ..Block::default()
    }];
    let types = [kind("test:machine", &["test:b", "test:c"])];
    let errors = accept_declared(&mut mods, &blocks, &[], &types, &BTreeSet::new());
    assert!(errors.is_empty(), "{errors:?}");
    let plan = Plan::new(&base, &mods).unwrap();
    assert_eq!(plan.block_entity_types.len(), 1);
    assert_eq!(
        plan.block_entity_factories.get("test:b"),
        Some(&"test:machine")
    );
    assert_eq!(
        plan.block_entity_factories.get("test:c"),
        Some(&"test:machine")
    );
    assert!(!plan.block_entity_factories.contains_key("test:a"));
    drop(plan);
    // A type-only code namespace is not skipped by the content merge.
    assert!(
        accept_declared(
            &mut mods,
            &[],
            &[],
            &[kind("other:empty", &[])],
            &BTreeSet::new()
        )
        .is_empty()
    );
    assert_eq!(mods[1].content.block_entity_types[0].id, "other:empty");
    assert_eq!(Plan::new(&base, &mods).unwrap().block_entity_types.len(), 2);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn validates_declared_type_ids_and_exclusive_known_mod_block_membership() {
    let base = base("validation");
    let mut mods = [declared_mod()];
    mods[0].content.block_entity_types = vec![kind("minecraft:custom", &["test:a"])];
    let error = Plan::new(&base, &mods).err().unwrap();
    assert_eq!(
        error
            .downcast_ref::<ReservedBlockEntityNamespace>()
            .unwrap()
            .kind,
        "minecraft:custom"
    );
    for (types, error) in [
        (
            vec![
                kind("test:machine", &["test:a"]),
                kind("test:machine", &["test:b"]),
            ],
            "block entity type test:machine is already declared",
        ),
        (
            vec![kind("minecraft:sign", &["test:a"])],
            "block entity type minecraft:sign cannot use the minecraft namespace",
        ),
        (
            vec![kind("test:machine", &["test:a", "test:a"])],
            "block test:a belongs to both block entity types test:machine and test:machine",
        ),
        (
            vec![kind("test:one", &["test:a"]), kind("test:two", &["test:a"])],
            "block test:a belongs to both block entity types test:one and test:two",
        ),
        (
            vec![kind("test:machine", &["missing:block"])],
            "block entity type test:machine: missing:block is not a block",
        ),
        (
            vec![kind("test:machine", &["minecraft:stone"])],
            "mod test: block entity type test:machine cannot own vanilla block minecraft:stone",
        ),
    ] {
        mods[0].content.block_entity_types = types;
        assert_eq!(Plan::new(&base, &mods).err().unwrap().to_string(), error);
    }
    mods[0].content.block_entity_types = vec![kind("test:machine", &["test:b", "test:a"])];
    let plan = Plan::new(&base, &mods).unwrap();
    assert_eq!(
        serde_json::to_string(&plan.block_entity_factories).unwrap(),
        r#"{"test:a":"test:machine","test:b":"test:machine"}"#
    );
    drop(plan);
    mods[0].content.block_entity_types.clear();
    let plan = Plan::new(&base, &mods).unwrap();
    let mut registry = read_json(&base.join("registries.json")).unwrap();
    plan.json("registries.json", &mut registry).unwrap();
    let registry: serde_json::Value = serde_json::to_value(registry).unwrap();
    assert_eq!(
        registry["minecraft:block_entity_type"]["entries"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn code_declared_types_build_a_root_and_membership_changes_invalidate_its_cache() {
    let base = base("cache");
    let mut mods = [ContentMod {
        id: "test".into(),
        content: Content::declared("test"),
        jars: Vec::new(),
    }];
    let cache = base.with_extension("cache");
    // Even before a code-only mod declares content, its accepted package ID
    // belongs to a distinct resource generation for load conditions.
    let code_only = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(code_only, base);
    assert_eq!(
        horizon_mod_package::enabled::EnabledMods::read(&code_only)
            .unwrap()
            .server_ids()
            .collect::<Vec<_>>(),
        vec!["test"]
    );
    assert_eq!(
        fs::read(code_only.join("registries.json")).unwrap(),
        fs::read(base.join("registries.json")).unwrap()
    );
    mods[0].content.block_entity_types = vec![kind("test:machine", &[])];
    let root = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(root, base);
    assert_ne!(root, code_only);
    assert!(root.join(crate::content_identity::MARKER).exists());
    assert_eq!(
        fs::read_to_string(root.join(BLOCK_ENTITY_TYPES)).unwrap(),
        "{}"
    );
    mods[0].content.blocks = declared_mod().content.blocks;
    mods[0].content.block_entity_types[0].blocks = vec!["test:a".into()];
    let with_a = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(with_a, root);
    assert_eq!(
        fs::read_to_string(root.join(BLOCK_ENTITY_TYPES)).unwrap(),
        "{}"
    );
    let stamp = fs::read(with_a.join(STAMP)).unwrap();
    assert_eq!(
        fs::read_to_string(with_a.join(BLOCK_ENTITY_TYPES)).unwrap(),
        r#"{"test:a":"test:machine"}"#
    );
    // Same type and wire ID, different valid blocks: the overlay must not be reused.
    mods[0].content.block_entity_types[0].blocks = vec!["test:b".into()];
    let with_b = resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap();
    assert_ne!(with_a, with_b);
    assert_eq!(
        resource_root_fixture(&base, Path::new(""), &mods, &cache).unwrap(),
        with_b
    );
    assert_eq!(fs::read(with_a.join(STAMP)).unwrap(), stamp);
    assert_eq!(
        fs::read_to_string(with_b.join(BLOCK_ENTITY_TYPES)).unwrap(),
        r#"{"test:b":"test:machine"}"#
    );
    let registry =
        horizon_protocol::registry::Builtin::load(&with_b.join("registries.json")).unwrap();
    assert_eq!(
        registry
            .get("minecraft:block_entity_type")
            .unwrap()
            .id("test:machine"),
        Some(9)
    );
    fs::remove_dir_all(cache).unwrap();
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn code_items_can_reference_both_owned_namespaces_in_one_batch() {
    let mut content = Content::declared("porter");
    content.namespaces.push("legacy".into());
    let mut mods = vec![ContentMod {
        id: "porter".into(),
        content,
        jars: vec![],
    }];
    let items = [
        ("legacy:wrench", "porter:spoon"),
        ("porter:spoon", "legacy:wrench"),
    ]
    .map(|(id, remainder)| horizon_mod_package::content::Item {
        id: id.into(),
        template: "minecraft:stick".into(),
        crafting_remainder: Some(remainder.into()),
        ..horizon_mod_package::content::Item::default()
    });
    let errors = accept_declared(&mut mods, &[], &items, &[], &BTreeSet::new());
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].id, "porter");
    assert_eq!(mods[0].content.items, items);
}

#[test]
fn code_declarations_join_the_manifest_owner_of_a_secondary_namespace() {
    let base = base("owned-namespace");
    let mut content = Content::declared("porter");
    content.namespaces.push("legacy".into());
    let mut mods = vec![ContentMod {
        id: "porter".into(),
        content,
        jars: vec![],
    }];
    let blocks = [Block {
        id: "legacy:pot".into(),
        template: "minecraft:stone".into(),
        ..Block::default()
    }];
    let types = [kind("legacy:pot_type", &["legacy:pot"])];
    let errors = accept_declared(&mut mods, &blocks, &[], &types, &BTreeSet::new());
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        mods.len(),
        1,
        "the secondary namespace is not a synthetic second package"
    );
    assert_eq!(mods[0].id, "porter");
    assert_eq!(mods[0].content.blocks[0].id, "legacy:pot");
    let plan = Plan::new(&base, &mods).unwrap();
    assert_eq!(
        plan.block_entity_factories.get("legacy:pot"),
        Some(&"legacy:pot_type")
    );
    drop(plan);
    fs::remove_dir_all(base).unwrap();
}

/// Declared boxes are in block units, one block being 1: a code-declared half slab resolves to
/// the bounds `[0, 0, 0, 1, 0.5, 1]`, as the same box in a JSON declaration does.
#[test]
fn a_code_declared_half_slab_resolves_to_block_unit_bounds() {
    let (base, _) = block_base("code-slab", &[]);
    let mut content = Content::declared("testmod");
    content
        .declare(
            &[Block {
                id: "testmod:slab".into(),
                template: "minecraft:test".into(),
                item: false,
                collision_shapes: Some(BlockShapes(StateValues {
                    property: String::new(),
                    properties: Vec::new(),
                    values: BTreeMap::new(),
                    default: Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
                })),
                ..Block::default()
            }],
            &[],
        )
        .unwrap();
    let mods = vec![ContentMod {
        id: "testmod".into(),
        content,
        jars: Vec::new(),
    }];
    let plan = Plan::new(&base, &mods).unwrap();
    for shape in plan.blocks[0].collision_shapes.as_ref().unwrap() {
        let bounds: Vec<[f64; 6]> = shape
            .boxes()
            .iter()
            .map(|b| {
                [
                    b.min(Axis::X),
                    b.min(Axis::Y),
                    b.min(Axis::Z),
                    b.max(Axis::X),
                    b.max(Axis::Y),
                    b.max(Axis::Z),
                ]
            })
            .collect();
        assert_eq!(bounds, vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]);
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}
