use super::*;
use serde_json::json;

fn predicate(graph: &mut Graph, id: &str, value: Value) {
    graph
        .values
        .insert(Key::element(Registry::Predicate, id.into()), value);
}

fn truth() -> Value {
    json!({"type": "minecraft:all_of", "terms": []})
}

#[test]
fn shared_targets_and_unknown_fields_are_not_cycles_or_references() {
    let mut graph = Graph::default();
    predicate(&mut graph, "minecraft:target", truth());
    predicate(
        &mut graph,
        "p:left",
        json!({"type":"any_of", "terms":"target"}),
    );
    predicate(
        &mut graph,
        "p:right",
        json!({"type":"inverted", "term":"target"}),
    );
    predicate(
        &mut graph,
        "p:diamond",
        json!({"type":"all_of", "terms":["p:left", "p:right"],
        "unused": {"condition":"p:missing"}}),
    );
    assert!(graph.validate().is_ok());
}

#[test]
fn binding_errors_precede_cycles_and_include_inline_holders() {
    let mut graph = Graph::default();
    predicate(
        &mut graph,
        "p:bad",
        json!({"type":"all_of", "terms":[
        {"type":"inverted", "term":"p:missing"}, "p:absent"]}),
    );
    predicate(
        &mut graph,
        "p:loop",
        json!({"type":"inverted", "term":"p:loop"}),
    );
    let error = graph.validate().err().unwrap();
    assert!(error.binding);
    assert_eq!(
        error.lines,
        [
            "Unbound values in registry ResourceKey[minecraft:root / minecraft:predicate]: [p:absent, p:missing]"
        ]
    );
}

#[test]
fn cycles_keep_the_vanilla_field_paths_from_each_root() {
    let mut graph = Graph::default();
    predicate(
        &mut graph,
        "prg:ref",
        json!({"type":"all_of", "terms":"prg:loop"}),
    );
    predicate(
        &mut graph,
        "prg:loop",
        json!({"type":"inverted", "term":"prg:ref"}),
    );
    let error = graph.validate().err().unwrap();
    assert!(!error.binding);
    assert_eq!(
        error.lines,
        [
            "Found loot table element validation problem in {prg:loop@minecraft:predicate}.term->{prg:ref@minecraft:predicate}.terms[0]: prg:loop of type minecraft:predicate is recursively called",
            "Found loot table element validation problem in {prg:ref@minecraft:predicate}.terms[0]->{prg:loop@minecraft:predicate}.term: prg:ref of type minecraft:predicate is recursively called",
        ]
    );
}

#[test]
fn tags_expand_nested_optional_entries_and_detect_predicate_recursion() {
    let mut graph = Graph::default();
    predicate(&mut graph, "p:target", truth());
    predicate(
        &mut graph,
        "p:ref",
        json!({"type":"any_of", "terms":"#p:yes"}),
    );
    let yes = Key {
        registry: Registry::Predicate,
        id: "p:yes".into(),
        tag: true,
    };
    let nested = Key {
        id: "p:nested".into(),
        ..yes.clone()
    };
    graph.tags.insert(
        yes.clone(),
        vec![("#p:nested".into(), true), ("p:absent".into(), false)],
    );
    graph
        .tags
        .insert(nested.clone(), vec![("p:target".into(), true)]);
    let tags = graph.validate().ok().unwrap();
    let mut predicates = BTreeMap::from([(
        "p:ref".into(),
        graph.values[&Key::element(Registry::Predicate, "p:ref".into())].clone(),
    )]);
    tags.expand_predicates(&mut predicates);
    assert_eq!(predicates["p:ref"]["terms"], json!(["p:target"]));
    graph.tags.insert(nested, vec![("p:ref".into(), true)]);
    let error = graph.validate().err().unwrap();
    assert!(!error.binding);
    assert!(
        error
            .lines
            .iter()
            .any(|line| line.ends_with("p:ref of type minecraft:predicate is recursively called"))
    );
}

#[test]
fn required_tag_entries_bind_even_when_no_predicate_uses_the_tag() {
    let mut graph = Graph::default();
    let tag = Key {
        registry: Registry::Predicate,
        id: "p:tag".into(),
        tag: true,
    };
    graph.tags.insert(tag, vec![("p:missing".into(), true)]);
    let error = graph.validate().err().unwrap();
    assert_eq!(
        error.lines,
        [
            "Unbound values in registry ResourceKey[minecraft:root / minecraft:predicate]: [p:missing]"
        ]
    );
}

#[test]
fn an_undefined_tag_is_not_an_empty_valid_holder_set() {
    let mut graph = Graph::default();
    predicate(
        &mut graph,
        "p:ref",
        json!({"type":"all_of", "terms":"#p:none"}),
    );
    assert_eq!(
        graph.validate().err().unwrap().lines,
        ["Unbound tags in registry ResourceKey[minecraft:root / minecraft:predicate]: [p:none]"]
    );
}

#[test]
fn missing_predicates_in_other_registries_and_number_providers_refuse_the_candidate() {
    let cases = [
        (
            Registry::Table,
            json!({"pools":[{"rolls":1, "entries":[{
                "type":"loot_table", "value":{"pools":[{"condition":"p:missing", "rolls":1, "entries":[]}]}
            }]}]}),
        ),
        (
            Registry::Modifier,
            json!({"type":"filtered", "item_filter":{}, "on_pass":{
                "type":"set_count", "count":1, "condition":"p:missing"
            }}),
        ),
        (
            Registry::Predicate,
            json!({"type":"int_value_check", "value":1, "test":{
                "type":"conditional", "condition":"p:missing", "on_true":1
            }}),
        ),
        (
            Registry::Float,
            json!({"type":"number_dispatcher", "cases":[{
                "condition":"p:missing", "value":1
            }]}),
        ),
        (
            Registry::Advancement,
            json!({"criteria":{"child":{
                "trigger":"bred_animals", "conditions":{"parent":"p:missing"}
            }}}),
        ),
    ];
    for (registry, value) in cases {
        let mut graph = Graph::default();
        graph
            .values
            .insert(Key::element(registry, "p:root".into()), value);
        assert_eq!(
            graph.validate().err().unwrap().lines,
            [
                "Unbound values in registry ResourceKey[minecraft:root / minecraft:predicate]: [p:missing]"
            ],
            "{}",
            registry.directory()
        );
    }
}

#[test]
fn a_number_and_predicate_can_form_a_cross_registry_cycle() {
    let mut graph = Graph::default();
    predicate(
        &mut graph,
        "p:chance",
        json!({"type":"float_value_check", "value":"p:number", "test":1}),
    );
    graph.values.insert(
        Key::element(Registry::Float, "p:number".into()),
        json!({
            "type":"conditional", "condition":"p:chance", "on_true":1
        }),
    );
    let error = graph.validate().err().unwrap();
    assert!(!error.binding);
    assert!(
        error
            .lines
            .iter()
            .any(|line| line.contains("minecraft:context_float_provider"))
    );
}

#[test]
fn a_future_container_loot_table_is_bound_without_being_called() {
    let mut graph = Graph::default();
    let table = Key::element(Registry::Table, "p:table".into());
    graph.values.insert(
        table.clone(),
        json!({"modifier":{
            "type":"set_loot_table", "loot_table_id":"p:table"
        }}),
    );
    assert!(graph.validate().is_ok());
    graph.values.get_mut(&table).unwrap()["modifier"]["loot_table_id"] = json!("p:missing");
    assert!(graph.validate().err().unwrap().binding);
}

#[test]
fn invalid_reference_identifiers_are_not_silently_ignored() {
    let mut graph = Graph::default();
    predicate(
        &mut graph,
        "p:ref",
        json!({"type":"inverted", "term":"Not An Id"}),
    );
    let error = graph.validate().err().unwrap();
    assert!(error.binding);
    assert!(error.lines[0].contains("Not An Id"));
}

#[test]
fn codec_only_references_bind_without_following_unchecked_cycles() {
    let mut graph = Graph::default();
    predicate(
        &mut graph,
        "p:chance",
        json!({"type":"random_chance", "chance":{
            "type":"conditional", "condition":"p:chance", "on_true":1
        }}),
    );
    assert!(graph.validate().is_ok());
    predicate(
        &mut graph,
        "p:chance",
        json!({"type":"random_chance", "chance":{
            "type":"conditional", "condition":"p:missing", "on_true":1
        }}),
    );
    assert!(graph.validate().err().unwrap().binding);
}

#[test]
fn only_the_highest_resource_and_replaced_tag_entries_are_admitted() {
    let dir = horizon_client::datapack::temp_folder().unwrap();
    let write = |pack: &str, path: &str, bytes: &[u8]| {
        let path = dir.join(pack).join("data/p").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    };
    write("low", "predicate/target.json", b"not JSON");
    write(
        "low",
        "tags/predicate/yes.json",
        br#"{"values":["p:missing"]}"#,
    );
    write(
        "high",
        "predicate/target.json",
        br#"{"type":"all_of","terms":[]}"#,
    );
    write(
        "high",
        "predicate/ref.json",
        br##"{"type":"any_of","terms":"#p:yes"}"##,
    );
    write(
        "high",
        "tags/predicate/yes.json",
        br#"{"replace":true,"values":["p:target"]}"#,
    );
    let low = DataPack::builtin(&dir.join("low"));
    let high = DataPack::builtin(&dir.join("high"));
    assert!(check(&[&low, &high]).is_ok());
    assert!(check(&[&high, &low]).is_err());
    write(
        "high",
        "tags/predicate/yes.json",
        br#"{"replace":false,"values":["p:target"]}"#,
    );
    assert!(
        check(&[&low, &high])
            .err()
            .unwrap()
            .lines
            .iter()
            .any(|line| line.contains("p:missing"))
    );
    std::fs::remove_dir_all(dir).unwrap();
}
