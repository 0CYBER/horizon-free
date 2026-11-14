use super::*;
use horizon_mod_package::{enabled::EnabledMods, selection::SelectedPackage};
use mod_api::{Hooks, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::atomic::{AtomicBool, Ordering};

struct EmptyCode;
#[mod_api::__subscriptions]
impl Hooks for EmptyCode {}

fn selected(fixture: &Fixture, id: &str, code: bool, side: &str) -> SelectedPackage {
    // The smallest shared group member: `dylink.0` memory info and the three guest exports.
    let module = [
        b"\0asm\x01\0\0\0".as_slice(),
        b"\0\x0f\x08dylink.0\x01\x04\0\0\0\0",
        b"\x01\x04\x01\x60\0\0",
        b"\x03\x04\x03\0\0\0",
        b"\x07\x2e\x03\x0ehorizon_layout\0\0\x0ahorizon_in\0\x01\x0chorizon_hook\0\x02",
        b"\x0a\x0a\x03\x02\0\x0b\x02\0\x0b\x02\0\x0b",
    ]
    .concat();
    let wasm = if code {
        format!(
            "\n[wasm]\nsha512='{}'\ntoolchain='fixture'\n",
            horizon_mod_package::manifest::sha512_of(&module)
        )
    } else {
        String::new()
    };
    let manifest = format!(
        "id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\nside='{side}'\nnamespaces=['shared_space']\ncapabilities=['registries']\n{wasm}"
    );
    let files: Vec<(&str, &[u8])> = if code {
        vec![
            ("mod.wasm", module.as_slice()),
            (
                "Cargo.toml",
                b"[package]\nname='inventory_fixture'\nversion='1.0.0'\n".as_slice(),
            ),
            ("src/lib.rs", b"pub struct Mod;".as_slice()),
        ]
    } else {
        Vec::new()
    };
    let path = fixture.0.join(format!("{id}.hmod"));
    std::fs::write(
        &path,
        horizon_mod_package::Package::write_files(&manifest, &files).unwrap(),
    )
    .unwrap();
    SelectedPackage {
        package: horizon_mod_package::Package::read(&path).unwrap(),
        path,
    }
}

#[test]
fn admission_uses_active_native_and_wasm_entries_and_accepted_content_not_startup_metadata() {
    let fixture = Fixture::new("inventory-admission");
    hmod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        selected: vec![
            selected(&fixture, "wasm_mod", true, "both"),
            selected(&fixture, "client_code", true, "client"),
            selected(&fixture, "refused_code", true, "both"),
            selected(&fixture, "plain", false, "both"),
            selected(&fixture, "refused_content", false, "both"),
        ]
        .into(),
        ..Default::default()
    };
    with_mods(
        vec![
            ModEntry::new("machine_sample", Box::new(EmptyCode)),
            ModEntry::new("wasm_mod", Box::new(EmptyCode)),
            ModEntry::new("client_code", Box::new(EmptyCode)),
        ]
        .into(),
        || {
            mod_platform::__private::with_test_packages(&["refused_code", "shared_space"], || {
                assert!(
                    mod_api::__private::loaded_mods()
                        .get("refused_code")
                        .is_some()
                );
                let mut startup = sources.clone();
                let disabled =
                    std::collections::BTreeSet::from(["machine_sample".into(), "wasm_mod".into()]);
                startup.capture_code_switches(&disabled);
                assert!(mod_api::__private::mod_index_on(
                    mod_api::__private::mod_index("machine_sample").unwrap()
                ));
                assert_eq!(
                    startup
                        .enabled_inventory(|id| id == "plain")
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["plain"]
                );
                startup.apply_code_switches(&disabled);
                assert!(mod_api::__private::set_mod_enabled("machine_sample", true));
                assert!(mod_api::__private::set_mod_enabled("wasm_mod", true));
                assert_eq!(
                    startup
                        .enabled_inventory(|id| id == "plain")
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["machine_sample", "plain", "wasm_mod"]
                );
                let initial = sources.enabled_inventory(|id| id == "plain");
                assert_eq!(
                    initial.client_ids().collect::<Vec<_>>(),
                    ["client_code", "machine_sample", "plain", "wasm_mod"]
                );
                assert_eq!(
                    initial.server_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain", "wasm_mod"]
                );
                assert!(mod_api::__private::set_mod_enabled("machine_sample", false));
                assert!(mod_api::__private::set_mod_enabled("wasm_mod", false));
                let paused = sources.enabled_inventory(|id| id == "plain");
                assert_eq!(paused.server_ids().collect::<Vec<_>>(), ["plain"]);
                assert_eq!(
                    paused.client_ids().collect::<Vec<_>>(),
                    ["client_code", "plain"]
                );
                assert_eq!(
                    initial.server_ids().collect::<Vec<_>>(),
                    ["machine_sample", "plain", "wasm_mod"]
                );
            });
        },
    );
}

struct Guard(Arc<AtomicBool>);
impl mod_api::ServerReloadListener for Guard {
    fn prepare(
        &self,
        _: &mod_api::ServerReloadContext,
    ) -> std::result::Result<Box<dyn mod_api::ReloadApply>, mod_api::ReloadResourceError> {
        if self.0.load(Ordering::Relaxed) {
            return Err(mod_api::ReloadResourceError::new(
                mod_api::Identifier::parse("reload_guard:guard").unwrap(),
                std::io::Error::other("inventory fixture rejected the new generation"),
            ));
        }
        Ok(<dyn mod_api::ReloadApply>::new(|| {}))
    }
}
#[mod_api::__subscriptions]
impl Hooks for Guard {
    fn add_server_reload_listeners(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::AddServerReloadListenersEvent<'_>,
    ) {
        event
            .listeners
            .add_listener(
                mod_api::Identifier::parse("reload_guard:guard").unwrap(),
                mod_api::FileToIdConverter::json("inventory_test").unwrap(),
                Arc::new(Guard(self.0.clone())),
            )
            .unwrap();
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn code_only_reload_publishes_one_new_inventory_after_prepare_and_preserves_old_readers() {
    let fixture = Fixture::new("inventory-reload");
    hmod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        ..Default::default()
    };
    let reject = Arc::new(AtomicBool::new(false));
    with_mods(
        vec![
            ModEntry::new("machine_sample", Box::new(EmptyCode)),
            ModEntry::new("reload_guard", Box::new(Guard(reject.clone()))),
        ]
        .into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let original = game.options.resources.clone();
                let initial = content_overlay(&original, &sources, &fixture.0.join("jars"))
                    .unwrap()
                    .root;
                game.options.resources = initial.prepared().to_owned();
                game.pack.data_source = initial.data().clone();
                game.pack.data_changes = initial.data_changes().clone();
                game.mods = sources.clone();
                game.world_root = fixture.0.join("world");
                let pack = game.world_root.join("datapacks/inventory");
                let recipes = pack.join("data/inventorytest/recipe");
                std::fs::create_dir_all(&recipes).unwrap();
                std::fs::write(pack.join("pack.mcmeta"), serde_json::to_vec(&serde_json::json!({
                "pack": {"description":"inventory fixture", "min_format":horizon_core::version::DATA_PACK_FORMAT, "max_format":horizon_core::version::DATA_PACK_FORMAT}
            })).unwrap()).unwrap();
                std::fs::write(recipes.join("requires_code.json"), br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"machine_sample"}]}"#).unwrap();
                let tags = pack.join("data/inventorytest/tags/item");
                std::fs::create_dir_all(&tags).unwrap();
                std::fs::write(tags.join("requires_code.json"), br#"{"neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"machine_sample"}],"values":["minecraft:diamond"]}"#).unwrap();
                for (kind, mut value) in [
                    ("loot_table", serde_json::json!({"pools":[]})),
                    (
                        "predicate",
                        serde_json::json!({"condition":"minecraft:random_chance","chance":1}),
                    ),
                    (
                        "item_modifier",
                        serde_json::json!({"function":"minecraft:set_count","count":1}),
                    ),
                    (
                        "jukebox_song",
                        serde_json::json!({"sound_event":"minecraft:music_disc.cat","length_in_seconds":1,"comparator_output":1,"description":"condition test"}),
                    ),
                    (
                        "instrument",
                        serde_json::json!({"sound_event":"minecraft:item.goat_horn.sound.0","use_duration":7,"range":256}),
                    ),
                    (
                        "enchantment",
                        serde_json::json!({"description":"condition test","max_level":3}),
                    ),
                ] {
                    value["neoforge:conditions"] = serde_json::json!([{"type":"neoforge:mod_loaded","modid":"machine_sample"}]);
                    let dir = pack.join("data/inventorytest").join(kind);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("requires_code.json"), value.to_string()).unwrap();
                    std::fs::write(
                        dir.join("unsupported.json"),
                        br#"{"neoforge:conditions":[{"type":"unsupported"}]}"#,
                    )
                    .unwrap();
                }
                game.reload_datapacks(true, &[]).unwrap();
                let loaded = |game: &Game<'_>, id: &str| {
                    let items = game.items();
                    (
                        items.has_loot_table(id),
                        items.songs.contains_key(id),
                        items.instruments.contains_key(id),
                        items.enchantments.list.iter().any(|e| &*e.id == id),
                        game.fns.store.predicates.contains_key(id),
                        game.fns.store.item_modifiers.contains_key(id),
                    )
                };
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert_eq!(
                    loaded(game, "inventorytest:unsupported"),
                    (false, false, false, false, false, false)
                );
                let old_songs = game.items().songs.clone();
                let read_tags = |game: &Game<'_>| game.items().item_tags.clone();
                let read = |game: &Game<'_>| {
                    crate::game::lan::sync::server_sources(
                        &game.options.resources,
                        &game.res.core.item_data,
                        &game.pack_files(),
                    )
                    .unwrap()
                };
                let has_recipe = |sources: &horizon_client::crafting::ResolvedSources| {
                    sources
                        .recipes
                        .iter()
                        .any(|recipe| recipe.name == "inventorytest:requires_code")
                };
                let old = read(game);
                assert!(has_recipe(&old));
                let old_tags = read_tags(game);
                assert!(old_tags["inventorytest:requires_code"].contains("minecraft:diamond"));
                let old_root = game.options.resources.clone();
                let old_data = game.pack.data_source.clone();
                assert!(mod_api::__private::set_mod_enabled("machine_sample", false));
                reject.store(true, Ordering::Relaxed);
                assert!(game.reload_datapacks(true, &[]).is_err());
                assert_eq!(game.options.resources, old_root);
                assert!(game.pack.data_source.same_generation(&old_data));
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert_eq!(
                    EnabledMods::read(&old_root)
                        .unwrap()
                        .server_ids()
                        .collect::<Vec<_>>(),
                    ["machine_sample"]
                );
                reject.store(false, Ordering::Relaxed);
                game.reload_datapacks(true, &[]).unwrap();
                assert_eq!(game.options.resources, original);
                assert!(!has_recipe(&read(game)));
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (false, false, false, false, false, false)
                );
                assert!(old_songs.contains_key("inventorytest:requires_code"));
                assert!(!read_tags(game).contains_key("inventorytest:requires_code"));
                assert!(old_tags["inventorytest:requires_code"].contains("minecraft:diamond"));
                assert!(has_recipe(&old));
                assert!(mod_api::__private::set_mod_enabled("machine_sample", true));
                game.reload_datapacks(true, &[]).unwrap();
                assert!(has_recipe(&read(game)));
                assert_eq!(
                    loaded(game, "inventorytest:requires_code"),
                    (true, true, true, true, true, true)
                );
                assert!(
                    read_tags(game)["inventorytest:requires_code"].contains("minecraft:diamond")
                );
                assert_eq!(game.options.resources, old_root);
            });
        },
    );
}

/// Capture metadata uses the same typed runtime and selected wasm provenance as loading.
#[test]
fn capture_metadata_distinguishes_content_native_wasm_and_unknown_code() {
    use horizon_perf_format::v1::ModKind;
    let fixture = Fixture::new("perf-inventory");
    hmod(&fixture.0, "machine_sample");
    let sources = ModsState {
        native: Arc::new(fixture.runtime()),
        selected: vec![selected(&fixture, "wasm_mod", true, "both")].into(),
        ..Default::default()
    };
    let inventory: Vec<_> = [
        ("plain", mod_api::PackageKind::Content),
        ("machine_sample", mod_api::PackageKind::Code),
        ("wasm_mod", mod_api::PackageKind::Code),
        ("builtin_code", mod_api::PackageKind::Code),
    ]
    .into_iter()
    .map(|(id, kind)| {
        mod_api::PackageInfo::new(
            id.to_owned(),
            "2.3".to_owned(),
            kind,
            mod_api::Side::Both,
            vec![id.to_owned()],
            vec![],
        )
    })
    .collect();
    let metadata = sources.perf_metadata(inventory.iter());
    assert_eq!(
        metadata
            .iter()
            .map(|m| (m.id.as_str(), m.version.as_str(), m.kind))
            .collect::<Vec<_>>(),
        vec![
            ("plain", "2.3", ModKind::Content as i32),
            ("machine_sample", "2.3", ModKind::Native as i32),
            ("wasm_mod", "2.3", ModKind::Wasm as i32),
            ("builtin_code", "2.3", ModKind::Unspecified as i32),
        ]
    );
    assert!(metadata.iter().all(|m| m.crate_name.is_empty()));
}
