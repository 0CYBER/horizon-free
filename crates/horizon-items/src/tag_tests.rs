use crate::{enchant::Enchantments, items::load_tag_lists};
use horizon_world::{item_defaults, loot::PackOverlay};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn tag(registry: &str, id: &str, layers: Vec<Value>) -> (String, Vec<Vec<u8>>) {
    let (namespace, path) = id.split_once(':').unwrap();
    (
        format!("tags/{namespace}:{registry}/{path}"),
        layers
            .into_iter()
            .map(|layer| serde_json::to_vec(&layer).unwrap())
            .collect(),
    )
}

fn diamond(registry: &str, members: [&str; 3]) -> Vec<(String, Vec<Vec<u8>>)> {
    let mut files = Vec::new();
    for depth in 0..40 {
        for side in ["left", "right"] {
            let entries = if depth == 0 && side == "right" {
                json!([members[0], format!("#test:join/{}", depth + 1), members[2]])
            } else {
                json!([format!("#test:join/{}", depth + 1)])
            };
            files.push(tag(
                registry,
                &format!("test:{side}/{depth}"),
                vec![json!({"values": entries})],
            ));
        }
        files.push(tag(
            registry,
            &format!("test:join/{depth}"),
            vec![
                json!({"values": [format!("#test:left/{depth}"), format!("#test:right/{depth}")]}),
            ],
        ));
    }
    files.push(tag(
        registry,
        "test:join/40",
        vec![json!({"values": [members[1], {"id": members[0]}, members[1]]})],
    ));
    files
}

#[test]
fn item_tag_cycle_rejects_every_member_and_its_dependents() {
    let files = [
        tag(
            "item",
            "c:dough",
            vec![json!({"values": ["minecraft:wheat", "#c:doughs", "#c:foods/dough"]})],
        ),
        tag(
            "item",
            "c:doughs",
            vec![json!({"values": ["minecraft:bread", "#c:foods/dough", "#c:dough"]})],
        ),
        tag(
            "item",
            "c:foods/dough",
            vec![json!({"values": ["#c:dough", "minecraft:cookie", "#c:doughs"]})],
        ),
        tag(
            "item",
            "test:dependent",
            vec![json!({"values": ["#c:doughs", "minecraft:egg"]})],
        ),
    ];
    let tags = load_tag_lists(Path::new(""), "item", &PackOverlay::new(&files)).unwrap();
    for id in ["c:dough", "c:doughs", "c:foods/dough", "test:dependent"] {
        assert!(tags[id].is_empty(), "{id}");
    }
}

#[test]
fn acyclic_item_tag_layers_apply_replace_removal_and_readdition_in_order() {
    let files = [
        tag(
            "item",
            "test:recipe",
            vec![
                json!({"values": ["stone"]}),
                json!({"replace": true, "values": ["#test:base", "egg"], "remove": ["#test:removed"]}),
                json!({"values": ["cookie", "bread", "egg"]}),
            ],
        ),
        tag(
            "item",
            "test:base",
            vec![json!({"values": ["cookie", "apple", "wheat"]})],
        ),
        tag("item", "test:removed", vec![json!({"values": ["cookie"]})]),
    ];
    let tags = load_tag_lists(Path::new(""), "item", &PackOverlay::new(&files)).unwrap();
    assert_eq!(
        tags["test:recipe"],
        [
            "minecraft:apple",
            "minecraft:wheat",
            "minecraft:egg",
            "minecraft:cookie",
            "minecraft:bread"
        ]
    );
}

#[test]
fn item_tag_diamond_resolves_past_the_old_depth_limit() {
    let files = diamond(
        "item",
        ["minecraft:stick", "minecraft:apple", "minecraft:bread"],
    );
    let tags = load_tag_lists(Path::new(""), "item", &PackOverlay::new(&files)).unwrap();
    assert_eq!(
        tags["test:join/0"],
        ["minecraft:apple", "minecraft:stick", "minecraft:bread"]
    );
}

fn load_enchantments(fixture: &str, files: &[(String, Vec<Vec<u8>>)]) -> Enchantments {
    let root = std::env::temp_dir().join(format!(
        "horizon-enchantment-tags-{fixture}-{}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("items.json"), "{}").unwrap();
    let defaults = item_defaults::read(&root).unwrap();
    let enchantments = Enchantments::load_with(&root, &PackOverlay::new(files), &defaults);
    fs::remove_dir_all(root).unwrap();
    enchantments.unwrap()
}

#[test]
fn enchantment_tag_cycle_rejects_its_dependent_tooltip_order() {
    let files = [
        tag(
            "enchantment",
            "test:a",
            vec![json!({"values": ["sharpness", "#test:b", "#test:c"]})],
        ),
        tag(
            "enchantment",
            "test:b",
            vec![json!({"values": ["unbreaking", "#test:c", "#test:a"]})],
        ),
        tag(
            "enchantment",
            "test:c",
            vec![json!({"values": ["mending", "#test:a", "#test:b"]})],
        ),
        tag(
            "enchantment",
            "minecraft:tooltip_order",
            vec![json!({"values": ["protection", "#test:b", "mending"]})],
        ),
    ];
    let enchantments = load_enchantments("cycle", &files);
    for id in ["test:a", "test:b", "test:c", "minecraft:tooltip_order"] {
        assert!(enchantments.tag(id).is_empty(), "{id}");
    }
}

#[test]
fn enchantment_tag_diamond_preserves_first_seen_order_at_depth() {
    let mut files = diamond("enchantment", ["unbreaking", "mending", "sharpness"]);
    files.push(tag(
        "enchantment",
        "minecraft:in_enchanting_table",
        vec![json!({"values": ["efficiency", "#test:join/0", "mending"]})],
    ));
    let enchantments = load_enchantments("diamond", &files);
    let order: Vec<&str> = enchantments
        .tag("minecraft:in_enchanting_table")
        .iter()
        .map(AsRef::as_ref)
        .collect();
    assert_eq!(
        order,
        [
            "minecraft:efficiency",
            "minecraft:mending",
            "minecraft:unbreaking",
            "minecraft:sharpness"
        ]
    );
}
