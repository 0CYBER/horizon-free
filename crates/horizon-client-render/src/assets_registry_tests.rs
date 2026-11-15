//! Exercise the actual asset-loader delegation with independent resource roots.
use super::Assets;
use std::sync::Arc;

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let names = [
        "minecraft:bedrock",
        "minecraft:dirt",
        "minecraft:stone",
        "minecraft:sand",
        "minecraft:oak_planks",
        "minecraft:bricks",
    ];
    let defaults: serde_json::Map<_, _> = names
        .iter()
        .enumerate()
        .map(|(id, name)| (name.to_string(), serde_json::json!(id)))
        .collect();
    let states: Vec<_> = names
        .iter()
        .map(|name| serde_json::json!(["", false, false, "", name]))
        .collect();
    std::fs::write(
        root.path().join("horizon-resources.json"),
        serde_json::json!({"data_version": horizon_world::DATA_VERSION}).to_string(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("facts.json"),
        serde_json::json!({"defaults": defaults, "states": states}).to_string(),
    )
    .unwrap();
    let mut tags = serde_json::Map::new();
    for name in [
        "minecraft:blocks_motion_in_heightmap",
        "minecraft:blocks_motion_in_heightmap_no_leaves",
        "minecraft:blocks_fluid_flow",
    ] {
        tags.insert(
            name.to_owned(),
            serde_json::json!(serde_json::json!({"values": []}).to_string()),
        );
    }
    std::fs::write(
        root.path().join("block-tags.json"),
        serde_json::Value::Object(tags).to_string(),
    )
    .unwrap();
    root
}

struct Hooks;
impl mod_api::Hooks for Hooks {
    fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[]
    }
}

#[test]
fn asset_loading_shares_equal_roots_only_within_the_installed_metadata_epoch() {
    let first_root = root();
    let equal_root = root();
    let first_mods: mod_api::Mods =
        Arc::from([mod_api::ModEntry::new("asset:first", Box::new(Hooks))]);
    let other_mods: mod_api::Mods =
        Arc::from([mod_api::ModEntry::new("asset:second", Box::new(Hooks))]);
    mod_platform::testing::with_mods(first_mods, || {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let first = Assets::load(first_root.path()).unwrap();
        let equal = Assets::load(equal_root.path()).unwrap();
        assert!(
            std::ptr::eq(first.registry, equal.registry),
            "resource paths are not registry identities"
        );
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let epoch = Assets::load(first_root.path()).unwrap();
        assert!(!std::ptr::eq(first.registry, epoch.registry));
        mod_platform::testing::with_mods(other_mods, || {
            mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
            let other = Assets::load(first_root.path()).unwrap();
            assert!(!std::ptr::eq(first.registry, other.registry));
        });
    });
}
