//! Serializer/type separation and unread-type resolution after conditions.
use super::*;
#[test]
fn serializer_of_no_code_skips_one_file_after_false_conditions() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("data/minecraft/recipe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("valid.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":3}}"#).unwrap();
    std::fs::write(folder.join("known_bad.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":100}}"#).unwrap();
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer"}"#,
    )
    .unwrap();
    let items = Items::for_tests(Default::default());
    let load = crate::test_resources::server_load();
    // No installed code mod owns `absent`: the file is skipped, the load goes on.
    let recipes = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load).unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    assert_eq!(recipes.by_type.len(), 1);
    assert_eq!(
        recipes.by_type["minecraft:crafting"][0].id,
        "minecraft:valid"
    );
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer","neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:true"}}]}"#,
    )
    .unwrap();
    let recipes = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load).unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    assert_eq!(
        recipes.by_type["minecraft:crafting"][0].id,
        "minecraft:valid"
    );
    assert_eq!(
        recipes.by_type["minecraft:crafting"][0].json["type"],
        "minecraft:crafting_shapeless"
    );
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer","neoforge:conditions":[{"type":"absent:condition"}]}"#,
    )
    .unwrap();
    let error = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("unknown.json") && error.contains("absent:condition"),
        "{error}"
    );
    assert!(
        !error.contains("unregistered recipe serializer"),
        "conditions must run before serializer lookup: {error}"
    );
}

struct Silent;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Silent {}

/// A carried content mod's own recipe type is skipped; the same namespace
/// owned by installed code stays fatal.
#[test]
fn carried_content_recipe_type_is_skipped_and_code_owned_is_fatal() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join(horizon_items::content_namespaces::MARKER),
        br#"["oritech"]"#,
    )
    .unwrap();
    let vanilla = root.path().join("data/minecraft/recipe");
    std::fs::create_dir_all(&vanilla).unwrap();
    std::fs::write(vanilla.join("valid.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":3}}"#).unwrap();
    let folder = root.path().join("data/oritech/recipe/assembler");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("amethystbud.json"),
        r#"{"type":"oritech:assembler"}"#,
    )
    .unwrap();
    let items = Items::for_tests(Default::default());
    let load = crate::test_resources::server_load();
    let recipes = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load).unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    assert_eq!(recipes.by_type.len(), 1);
    assert_eq!(
        recipes.by_type["minecraft:crafting"][0].id,
        "minecraft:valid"
    );
    assert!(!recipes.by_type.contains_key("oritech:assembler"));
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("oritech", Box::new(Silent))].into(),
        || {
            let error = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load)
                .err()
                .unwrap()
                .to_string();
            assert_eq!(
                error,
                "recipe data/oritech/recipe/assembler/amethystbud.json, unregistered recipe serializer oritech:assembler"
            );
        },
    );
}
struct Machine;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Machine {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_recipe_type("machine:grinding");
        r.register_raw_recipe_serializer("machine:grinder_json", "machine:grinding");
    }
}
#[test]
fn read_only_machine_serializer_reports_separate_type_without_crafting() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("machine", Box::new(Machine))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
                "machine",
                Box::new(Machine),
            )])
            .unwrap();
            let root = tempfile::tempdir().unwrap();
            let recipes = Recipes::from_sources(
                root.path(),
                &Items::empty(),
                &PackOverlay::NONE,
                vec![(
                    "machine:iron".into(),
                    serde_json::json!({"type":"machine:grinder_json","value":42}),
                )],
                &crate::test_resources::server_load(),
            )
            .unwrap();
            let machine = &recipes.by_type["machine:grinding"];
            assert_eq!(machine.len(), 1);
            assert_eq!(machine[0].kind, "machine:grinding");
            assert_eq!(machine[0].json["type"], "machine:grinder_json");
            assert_eq!(machine[0].json["value"], 42);
            assert!(recipes.crafting.is_empty() && recipes.cooking.is_empty());
            assert!(!recipes.by_type.contains_key("machine:grinder_json"));
            assert!(
                mod_kinds::recipe_type("wrong", "machine:grinding")
                    .unwrap_err()
                    .contains("machine:grinding")
            );
            assert!(
                recipes
                    .find(
                        crate::test_resources::server_loop_context(),
                        &[None, None, None, None],
                        2
                    )
                    .is_none()
            );
        },
    );
}

#[test]
fn unknown_serializer_refuses_before_unclaimed_ingredient_is_dropped() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let file: Value = serde_json::from_str(
            r#"{"type":"minecraft:future_serializer","ingredient":{"type":"absent:ingredient"}}"#,
        )
        .unwrap();
        let sources = vec![("minecraft:future".into(), file)];
        let context = crate::test_resources::server_load();
        let error = ResolvedSources::resolve(sources.clone(), &context)
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            error,
            "recipe data/minecraft/recipe/future.json, unregistered recipe serializer minecraft:future_serializer"
        );
        let root = tempfile::tempdir().unwrap();
        let error = Recipes::from_sources(
            root.path(),
            &Items::empty(),
            &PackOverlay::NONE,
            sources,
            &context,
        )
        .err()
        .unwrap()
        .to_string();
        assert_eq!(
            error,
            "recipe data/minecraft/recipe/future.json, unregistered recipe serializer minecraft:future_serializer"
        );
    });
}
