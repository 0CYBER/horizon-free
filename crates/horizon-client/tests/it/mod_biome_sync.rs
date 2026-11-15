//! A content mod's own biome (`zinc:foo` from `data/zinc/worldgen/biome/foo.json`, which the
//! content build puts in the root's `worldgen/biomes.json`) joins the biome table after the
//! built-in ones and reaches a joining client with its data.
use horizon_client::{
    biome::Biomes,
    chunk::LevelShapes,
    lan::data::{BIOME_REGISTRY, SyncData, client_biomes},
};
use std::path::PathBuf;

/// Removes the folder when the test ends, passed or not.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A resource folder with the built-in biomes plains and river and 1x1 colormaps, and the
/// biome bundle `bundle` when there is one.
fn write_root(root: &std::path::Path, bundle: Option<&str>) {
    let maps = root.join("assets/minecraft/textures/colormap");
    let biome_dir = root.join("data/minecraft/worldgen/biome");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::create_dir_all(&biome_dir).unwrap();
    std::fs::write(root.join("registries.json"), "{}").unwrap();
    std::fs::write(root.join("items.json"), "{}").unwrap();
    for name in ["grass", "foliage", "dry_foliage"] {
        let file = std::fs::File::create(maps.join(format!("{name}.png"))).unwrap();
        let mut png = png::Encoder::new(file, 1, 1);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        let mut writer = png.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3]).unwrap();
    }
    for name in ["plains", "river"] {
        std::fs::write(biome_dir.join(format!("{name}.json")), "{}").unwrap();
    }
    if let Some(bundle) = bundle {
        std::fs::create_dir_all(root.join("worldgen")).unwrap();
        std::fs::write(root.join("worldgen/biomes.json"), bundle).unwrap();
    }
}

fn load(root: &std::path::Path) -> Biomes {
    Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))
    .unwrap()
}

/// The bundle a content build writes: the built-in biomes and two mods' (the second sorts last
/// by path although its namespace sorts first).
const BUNDLE: &str = r##"{
    "minecraft:plains": "{}",
    "minecraft:river": "{}",
    "zinc:foo": "{\"effects\":{\"grass_color\":\"#445566\"},\"attributes\":{\"minecraft:visual/ambient_particles\":{\"argument\":[{\"particle\":{\"type\":\"minecraft:ash\"},\"probability\":0.25}]}}}",
    "alpha:zed": "{\"effects\":{\"grass_color\":\"#778899\"}}"
}"##;

fn server_load() -> horizon_world::native_load_context::LoadContext {
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}

fn sync_of(root: &std::path::Path) -> SyncData {
    let defaults = horizon_world::item_defaults::read(root).unwrap();
    SyncData::load(
        root,
        &horizon_world::loot::PackOverlay::NONE,
        &defaults,
        &horizon_client::crafting::ResolvedSources::load_or_empty(
            root,
            &horizon_world::loot::PackOverlay::NONE,
            &server_load(),
        ),
    )
    .unwrap()
}

#[test]
fn mod_biomes_follow_the_builtin_ones_and_leave_their_ids_alone() {
    let plain = std::env::temp_dir().join(format!("horizon-mod-biomes-a-{}", std::process::id()));
    let modded = std::env::temp_dir().join(format!("horizon-mod-biomes-b-{}", std::process::id()));
    let _scratch = (Scratch(plain.clone()), Scratch(modded.clone()));
    write_root(&plain, None);
    write_root(&modded, Some(BUNDLE));
    let (without, with) = (load(&plain), load(&modded));

    // Without the mod: two biomes, and a mod's name is no biome (a saved column names it plains).
    assert_eq!(without.list.len(), 2);
    assert!(!without.contains("zinc:foo"));
    assert_eq!(without.id("zinc:foo"), without.id("minecraft:plains"));

    // With it: the built-in ids are the same, the mods' biomes are 2 and 3, sorted by id.
    assert_eq!(with.list.len(), 4);
    assert_eq!(with.id("minecraft:plains"), 0);
    assert_eq!(with.id("minecraft:river"), 1);
    assert_eq!(with.id("zinc:foo"), 2);
    assert_eq!(with.id("alpha:zed"), 3);
    assert_eq!(with.get(2).grass, 0x445566);
    let particles = &with.get(2).ambient_particles;
    assert_eq!(particles.len(), 1);
    assert_eq!(particles[0].0.name(), "ash");
    assert_eq!(particles[0].1, 0.25);
    assert_eq!(with.name(3), "alpha:zed");
    // The tables that read a biome's file see the mod's, and never a built-in's.
    assert!(with.pack_file("zinc:foo").is_some());
    assert!(with.pack_file("minecraft:plains").is_none());
    // A world's packs go over the same table: a new pack biome joins after the mods'.
    let pack: serde_json::Value = serde_json::from_str("{}").unwrap();
    let world = with.with_packs([("pack:late", &pack), ("zinc:foo", &pack)]);
    assert_eq!(world.id("zinc:foo"), 2);
    assert!(world.get(2).ambient_particles.is_empty());
    assert_eq!(world.id("pack:late"), 4);
}

#[test]
fn a_server_sends_mod_biomes_with_data_to_a_client_that_has_the_vanilla_pack() {
    let server = std::env::temp_dir().join(format!("horizon-mod-sync-s-{}", std::process::id()));
    let client = std::env::temp_dir().join(format!("horizon-mod-sync-c-{}", std::process::id()));
    let _scratch = (Scratch(server.clone()), Scratch(client.clone()));
    write_root(&server, Some(BUNDLE));
    write_root(&client, None);

    let sync = sync_of(&server);
    let sent = sync.registry_packets(true, &LevelShapes::VANILLA);
    let biomes = sent.iter().find(|r| r.registry == BIOME_REGISTRY).unwrap();
    // Network order: path, then namespace.
    let ids: Vec<_> = biomes.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "zinc:foo",
            "minecraft:plains",
            "minecraft:river",
            "alpha:zed"
        ]
    );
    let has_data: Vec<_> = biomes.entries.iter().map(|e| e.data.is_some()).collect();
    assert_eq!(has_data, [true, false, false, true]);

    // A client without the mod builds both from what arrived, after its own biomes.
    let received: Vec<_> = biomes
        .entries
        .iter()
        .map(|e| (e.id.clone(), e.data.clone()))
        .collect();
    let table = client_biomes(&load(&client), &received).expect("the mods' biomes are new");
    assert_eq!(table.id("minecraft:plains"), 0);
    assert_eq!(table.id("zinc:foo"), 2);
    assert_eq!(table.get(2).grass, 0x445566);
    assert_eq!(
        table.get(2).ambient_particles,
        load(&server).get(2).ambient_particles
    );
    assert_eq!(table.id("alpha:zed"), 3);

    // A client with the same mod has them already: nothing to rebuild.
    let modded = std::env::temp_dir().join(format!("horizon-mod-sync-m-{}", std::process::id()));
    let _scratch = Scratch(modded.clone());
    write_root(&modded, Some(BUNDLE));
    assert!(client_biomes(&load(&modded), &received).is_none());
}

#[test]
fn a_server_without_mod_biomes_sends_the_vanilla_registry_as_before() {
    let root = std::env::temp_dir().join(format!("horizon-mod-sync-v-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    write_root(&root, None);
    let sent = sync_of(&root).registry_packets(true, &LevelShapes::VANILLA);
    let biomes = sent.iter().find(|r| r.registry == BIOME_REGISTRY).unwrap();
    let ids: Vec<_> = biomes.entries.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["minecraft:plains", "minecraft:river"]);
    assert!(biomes.entries.iter().all(|e| e.data.is_none()));
}
