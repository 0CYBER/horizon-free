//! Processor and pool admission uses the gameplay block-property bindings.
#![cfg(test)]

use super::native_entity_refs::tests::with_game;
use std::{path::Path, sync::Arc};

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "needs prepared resources and isolated homes"]
fn generic_processor_and_referencing_pool_accept_pillar_block_properties() {
    let _publication = horizon_client::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let pack = scratch.path().join("datapacks/processors");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    write(
        &scratch.path().join("datapacks.json"),
        r#"{"enabled":["vanilla","file/processors"],"disabled":[]}"#,
    );
    // The supported pillar schema from the processor conversion codec fixture.
    write(
        &pack.join("data/registry_processor_test/worldgen/processor_list/pillar.json"),
        r#"{"processors":[{"processor_type":"horizon:pillar","pillar_length":15,"pillar_trigger_and_replacements":[{"trigger":{"id":"minecraft:red_stained_glass"},"replacement":{"id":"minecraft:cobblestone"}}]}]}"#,
    );
    write(
        &pack.join("data/registry_processor_test/worldgen/template_pool/pillar.json"),
        r#"{"fallback":"minecraft:empty","elements":[{"weight":1,"element":{"element_type":"minecraft:single_pool_element","location":"minecraft:village/plains/houses/plains_small_house_1","processors":"registry_processor_test:pillar","projection":"rigid"}}]}"#,
    );
    // Unsupported processors must still fail admission, including their pools.
    write(
        &pack.join("data/registry_processor_test/worldgen/processor_list/rejected.json"),
        r#"{"processors":[{"processor_type":"registry_processor_test:unknown"}]}"#,
    );
    write(
        &pack.join("data/registry_processor_test/worldgen/template_pool/rejected.json"),
        r#"{"fallback":"minecraft:empty","elements":[{"weight":1,"element":{"element_type":"minecraft:single_pool_element","location":"minecraft:village/plains/houses/plains_small_house_1","processors":"registry_processor_test:rejected","projection":"rigid"}}]}"#,
    );
    with_game(|game| {
        let mut request = super::world_loading::WorldRequest::new(
            "processor registries",
            Some("processor registries"),
            Some(1),
            true,
            None,
            Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = scratch.path().to_owned();
        game.load_world(request).unwrap();
        let host: &dyn mod_api::Host = game;
        let accepted = mod_api::Identifier::parse("registry_processor_test:pillar").unwrap();
        let rejected = mod_api::Identifier::parse("registry_processor_test:rejected").unwrap();
        for registry in [
            "minecraft:worldgen/processor_list",
            "minecraft:worldgen/template_pool",
        ] {
            assert!(host.registry_entry(registry, "registry_processor_test:pillar").is_some(), "{registry}");
            let entries = host.registry_entries(registry).unwrap();
            assert!(entries.contains(&accepted), "{registry}");
            assert!(!entries.contains(&rejected), "{registry}");
            assert_eq!(host.registry_entry(registry, "registry_processor_test:rejected"), None);
        }
    });
}
