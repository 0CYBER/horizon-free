use super::*;
use crate::{assets::Assets, blocks::BlockFacts, items::Items};

#[test]
fn renames_ids_keys_and_states() {
    let mut r = Rename::new("minecraft:iron_block", "ruby:ruby_block");
    r.states = 10..12;
    r.first = 100;
    assert_eq!(
        r.string("minecraft:iron_block").as_deref(),
        Some("ruby:ruby_block")
    );
    assert_eq!(
        r.string("Block{minecraft:iron_block}").as_deref(),
        Some("Block{ruby:ruby_block}")
    );
    assert_eq!(
        r.string("block.minecraft.iron_block").as_deref(),
        Some("block.ruby.ruby_block")
    );
    assert_eq!(r.string("minecraft:iron_blocks"), None);
    assert_eq!(r.string("block.minecraft.iron_bars"), None);
    assert_eq!(
        (r.id(9), r.id(10), r.id(11), r.id(12)),
        (None, Some(100), Some(101), None)
    );
}

#[test]
fn json_keeps_key_order_and_numbers() {
    let text = r#"{"z":1,"a":[0.6,5.0,-1,{"m":null,"b":true}],"k":"v"}"#;
    let doc: J = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&doc).unwrap(), text);
}

#[test]
fn tags_merge_without_repeats() {
    let vanilla = r#"{"values": ["minecraft:stone"]}"#;
    let more = [
        J::Str("ruby:ruby_block".into()),
        J::Str("minecraft:stone".into()),
    ];
    let merged: serde_json::Value =
        serde_json::from_str(&merge_tag(Some(vanilla), &more, "t").unwrap()).unwrap();
    assert_eq!(
        merged["values"],
        serde_json::json!(["minecraft:stone", "ruby:ruby_block"])
    );
}

fn prepared() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("../../.local/horizon-client/26.3"),
        manifest.join("../../../../.local/horizon-client/26.3"),
    ]
    .into_iter()
    .find(|root| root.join("gameplay-facts.json").exists())
}

/// A content mod folder: a full block, a door and an item-less glass from
/// templates, an item, a vanilla tag addition and a language file.
fn ruby_mod(dir: &Path) -> ContentMod {
    let files: [(&str, &str); 8] = [
        (
            "horizon-mod.toml",
            "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/ruby/horizon/block/ruby_block.json",
            r#"{"template": "minecraft:iron_block", "properties": {"hardness": 4.0, "light": 3,
                "sounds": "minecraft:amethyst_block", "map_color": 28, "friction": 0.8}}"#,
        ),
        (
            "data/ruby/horizon/block/ruby_door.json",
            r#"{"template": "minecraft:oak_door"}"#,
        ),
        (
            "data/ruby/horizon/block/ruby_glass.json",
            r#"{"template": "minecraft:glass", "item": false}"#,
        ),
        (
            "data/ruby/horizon/item/ruby.json",
            r#"{"template": "minecraft:diamond", "max_stack_size": 16, "rarity": "rare"}"#,
        ),
        // Carried-over tags name blocks left out and other mods' ids too.
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            r##"{"values": ["ruby:ruby_block", "ruby:left_out", "othermod:thing",
                {"id": "othermod:optional", "required": false}, "#c:storage_blocks/ruby"]}"##,
        ),
        (
            "data/c/tags/item/gems/ruby.json",
            r#"{"values": ["ruby:ruby", "othermod:ruby"]}"#,
        ),
        (
            "assets/ruby/lang/en_us.json",
            r#"{"block.ruby.ruby_block": "Block of Ruby"}"#,
        ),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mods::Package::read(dir).unwrap();
    ContentMod {
        id: "ruby".into(),
        content: Content::read(dir, &package.manifest).unwrap(),
    }
}

/// The overlay over the real prepared resources: every loader reads it, the
/// new blocks behave like their templates with the overrides, and vanilla's
/// states are where they were.
#[test]
fn content_root_loads_with_every_table() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-content-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = ruby_mod(&scratch.join("ruby"));
    // The base as a relative path (cargo runs tests in the crate's folder):
    // the overlay's links still resolve.
    let relative = Path::new("../../.local/horizon-client/26.3");
    let root = resource_root(relative, std::slice::from_ref(&m), &scratch.join("cache")).unwrap();
    assert_ne!(root, base);

    let vanilla = Assets::load(&base).unwrap();
    let assets = Assets::load(&root).unwrap();
    let (old, new) = (vanilla.registry, assets.registry);
    let n = old.len() as u32;
    let iron = old.default_state("minecraft:iron_block").unwrap();
    let door = old.default_state("minecraft:oak_door").unwrap();
    let door_first = (0..n)
        .map(BlockStateId)
        .find(|&s| old.state(s).unwrap().name == "minecraft:oak_door")
        .unwrap();
    // Vanilla keeps its ids; blocks come after, sorted: ruby_block, ruby_door, ruby_glass.
    assert_eq!(new.len(), old.len() + 1 + 64 + 1);
    assert_eq!(new.default_state("minecraft:iron_block"), Some(iron));
    let ruby = new.default_state("ruby:ruby_block").unwrap();
    assert_eq!(ruby, BlockStateId(n));
    let ruby_door = new.default_state("ruby:ruby_door").unwrap();
    assert_eq!(ruby_door, BlockStateId(n + 1 + (door.0 - door_first.0)));
    assert_eq!(
        new.default_state("ruby:ruby_glass"),
        Some(BlockStateId(n + 65))
    );

    let old_facts = BlockFacts::load(&base, old).unwrap();
    let facts = BlockFacts::load(&root, new).unwrap();
    let shared_json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        fs::File::open(root.join("block-behaviours.json")).unwrap(),
    ))
    .unwrap();
    let (shared_facts, shared_behaviours) =
        BlockFacts::load_with_behaviours(&root, new, &shared_json).unwrap();
    let independent_behaviours = horizon_world::level::Behaviours::load(new, &root).unwrap();
    let (deferred_facts, gameplay) =
        BlockFacts::load_with_projection(&root, new, &shared_json).unwrap();
    let deferred_behaviours =
        horizon_world::level::Behaviours::load_with_gameplay(new, &root, &shared_json, &gameplay)
            .unwrap();
    assert_eq!(shared_facts.states.len(), facts.states.len());
    for state in [ruby, ruby_door, BlockStateId(n + 65)] {
        assert_eq!(shared_facts.get(state).hardness, facts.get(state).hardness);
        assert_eq!(
            deferred_facts.get(state).hardness,
            facts.get(state).hardness
        );
        assert_eq!(deferred_facts.get(state).classes, facts.get(state).classes);
        assert_eq!(
            deferred_behaviours.facts(state).flags,
            independent_behaviours.facts(state).flags
        );
        assert_eq!(
            deferred_behaviours.data(state).sounds.place,
            independent_behaviours.data(state).sounds.place
        );
        assert_eq!(shared_facts.get(state).emission, facts.get(state).emission);
        assert_eq!(shared_facts.get(state).classes, facts.get(state).classes);
        assert_eq!(
            shared_behaviours.facts(state).flags,
            independent_behaviours.facts(state).flags,
        );
        assert_eq!(
            shared_behaviours.data(state).sounds.place,
            independent_behaviours.data(state).sounds.place,
        );
    }

    let (a, b) = (old_facts.get(iron), facts.get(ruby));
    assert_eq!((b.hardness, b.emission, b.friction), (4.0, 3, 0.8));
    assert_eq!(b.explosion_resistance, a.explosion_resistance);
    assert_eq!(b.outline, a.outline);
    assert_eq!(b.classes, a.classes);
    assert_eq!(&*b.item, "ruby:ruby_block");
    assert_eq!(b.map_color, 28);
    let amethyst = old_facts.get(old.default_state("minecraft:amethyst_block").unwrap());
    assert_eq!(b.sounds.break_sound, amethyst.sounds.break_sound);
    for i in 0..64 {
        let (v, m) = (
            old_facts.get(BlockStateId(door_first.0 + i)),
            facts.get(BlockStateId(n + 1 + i)),
        );
        assert_eq!(
            (v.outline.clone(), v.hardness, v.classes.clone()),
            (m.outline.clone(), m.hardness, m.classes.clone())
        );
        assert_eq!(&*m.name, "ruby:ruby_door");
    }
    let glass = facts.get(BlockStateId(n + 65));
    assert_eq!(&*glass.item, "minecraft:air");
    // Vanilla's own rows are untouched.
    assert_eq!(facts.get(iron).hardness, a.hardness);
    assert_eq!(&*facts.get(iron).name, "minecraft:iron_block");

    let items = Items::load(&root, new, &facts).unwrap();
    let ruby_block = &items.items["ruby:ruby_block"];
    assert_eq!(ruby_block.block, Some(ruby));
    let gem = &items.items["ruby:ruby"];
    assert_eq!(gem.max_stack, 16);
    assert!(!items.items.contains_key("ruby:ruby_glass"));
    let pickaxe = &items.block_tags["minecraft:mineable/pickaxe"];
    assert!(pickaxe.contains("ruby:ruby_block"));
    assert!(!pickaxe.contains("ruby:left_out") && !pickaxe.contains("othermod:thing"));
    assert!(pickaxe.contains("minecraft:stone"));
    // Convention tags load, with the ids nobody registers dropped.
    assert_eq!(items.item_tag_order["c:gems/ruby"], ["ruby:ruby"]);

    crate::collisions::load(&root, new).unwrap();
    crate::models::load_visibility(&root, new).unwrap();
    crate::light::LightTables::load(&root, new, &facts).unwrap();
    crate::motion::MotionResources::load(&root, &assets).unwrap();
    horizon_world::worldgen::generator::GeneratorData::load(&root.join("worldgen"), new).unwrap();
    assert!(root.join("assets/ruby/lang/en_us.json").is_file());
    // Blocks without a blockstate of their own take their template's.
    assert_eq!(
        fs::read(root.join("assets/ruby/blockstates/ruby_door.json")).unwrap(),
        fs::read(base.join("assets/minecraft/blockstates/oak_door.json")).unwrap()
    );

    // The base root was only read.
    assert!(
        fs::symlink_metadata(base.join("facts.json"))
            .unwrap()
            .is_file()
    );
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_unknown_templates_and_vanilla_ids() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-refuse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    m.content.blocks[0].template = "minecraft:not_a_block".into();
    let e = build(&base, std::slice::from_ref(&m), &scratch.join("out")).unwrap_err();
    assert!(e.to_string().contains("is not a block"), "{e}");
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn refuses_more_states_than_chunks_hold() {
    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-states-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let mut m = ruby_mod(&scratch.join("ruby"));
    // Redstone wire has 1296 states: 40 copies pass any game's 65536.
    let mut wire = m.content.blocks[0].clone();
    wire.template = "minecraft:redstone_wire".into();
    for i in 0..40 {
        let mut b = wire.clone();
        b.id = format!("ruby:wire_{i}");
        m.content.blocks.push(b);
    }
    let e = build(&base, std::slice::from_ref(&m), &scratch.join("out")).unwrap_err();
    assert!(
        e.to_string()
            .contains("more than the 65536 chunks can hold"),
        "{e}"
    );
    assert!(e.to_string().contains("the mods: ruby "), "{e}");
    fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn no_content_keeps_the_base_root() {
    let base = Path::new("/nonexistent/base");
    assert_eq!(
        resource_root(base, &[], Path::new("/nonexistent/cache")).unwrap(),
        base
    );
}

use horizon_world::registry::BlockStateId;

#[test]
fn shared_behaviour_document_matches_independent_loaders() {
    let Some(root) = prepared() else { return };
    let assets = Assets::load(&root).expect("assets");
    let doc: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(root.join("block-behaviours.json")).unwrap(),
    ))
    .unwrap();
    let original = BlockFacts::load(&root, assets.registry).unwrap();
    let (shared, b) = BlockFacts::load_with_behaviours(&root, assets.registry, &doc).unwrap();
    let (deferred_facts, projection) =
        BlockFacts::load_with_projection(&root, assets.registry, &doc).unwrap();
    let deferred = horizon_world::level::Behaviours::load_with_gameplay(
        assets.registry,
        &root,
        &doc,
        &projection,
    )
    .unwrap();
    assert_eq!(deferred_facts.states.len(), shared.states.len());
    assert_eq!(original.states.len(), shared.states.len());
    for (a, b) in original.states.iter().zip(&shared.states) {
        assert_eq!(a.classes, b.classes);
        assert_eq!(a.legacy_solid, b.legacy_solid);
        assert_eq!(a.sturdy_full, b.sturdy_full);
        assert_eq!(a.emission, b.emission);
        assert_eq!(a.dampening, b.dampening);
        assert_eq!(a.hardness, b.hardness);
        assert_eq!(a.sounds.volume, b.sounds.volume);
        assert_eq!(a.sounds.pitch, b.sounds.pitch);
    }
    let a = horizon_world::level::Behaviours::load(assets.registry, &root).unwrap();
    for id in 0..assets.registry.len() {
        let state = horizon_world::registry::BlockStateId(id as u32);
        let (a_state, b_state) = (a.facts(state), b.facts(state));
        let deferred_state = deferred.facts(state);
        assert_eq!(b_state.flags, deferred_state.flags);
        assert_eq!(b_state.emission, deferred_state.emission);
        assert_eq!(b_state.dampening, deferred_state.dampening);
        assert_eq!(b_state.hardness, deferred_state.hardness);
        assert_eq!(b_state.push, deferred_state.push);
        assert_eq!(a_state.flags, b_state.flags);
        assert_eq!(a_state.sturdy, b_state.sturdy);
        assert_eq!(a_state.weak, b_state.weak);
        assert_eq!(a_state.strong, b_state.strong);
        assert_eq!(a_state.fluid, b_state.fluid);
        assert_eq!(a_state.emission, b_state.emission);
        assert_eq!(a_state.dampening, b_state.dampening);
        assert_eq!(a_state.hardness, b_state.hardness);
        assert_eq!(a_state.push, b_state.push);
        assert_eq!(a_state.instrument, b_state.instrument);
        assert_eq!(a_state.shape, b_state.shape);
        let (a_sound, b_sound) = (&a.data(state).sounds, &b.data(state).sounds);
        assert_eq!(a_sound.volume, b_sound.volume);
        assert_eq!(a_sound.pitch, b_sound.pitch);
        assert_eq!(a_sound.place, b_sound.place);
        assert_eq!(a_sound.break_, b_sound.break_);
        assert_eq!(a_sound.fall, b_sound.fall);
    }
}

#[cfg(unix)]
#[test]
fn independent_reloads_read_changed_behaviour_json_and_report_invalid_files() {
    let Some(base) = prepared() else { return };
    let scratch = std::env::temp_dir().join(format!("horizon-json-reloads-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).unwrap();
    // Keep the remaining resource tree intact without copying a huge prepared client.
    for entry in fs::read_dir(&base).unwrap() {
        let entry = entry.unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(entry.path(), scratch.join(entry.file_name())).unwrap();
    }
    let assets = Assets::load(&base).unwrap();
    let registry = assets.registry;
    let original = fs::read(base.join("block-behaviours.json")).unwrap();
    fs::remove_file(scratch.join("block-behaviours.json")).unwrap();
    fs::write(scratch.join("block-behaviours.json"), &original).unwrap();
    // Independent facts and behaviours can load first, followed by paired data.
    let independent_first = BlockFacts::load(&scratch, registry).unwrap();
    let independent_world_first =
        horizon_world::level::Behaviours::load(registry, &scratch).unwrap();
    let (facts, paired_world) = BlockFacts::load_pair(&scratch, registry).unwrap();
    assert_eq!(facts.states.len(), registry.len());
    for state in [0, 1, registry.len() - 1] {
        let state = horizon_world::registry::BlockStateId(state as u32);
        assert_eq!(
            facts.get(state).classes,
            independent_first.get(state).classes
        );
        assert_eq!(
            facts.get(state).sounds.place,
            independent_first.get(state).sounds.place
        );
        assert_eq!(
            paired_world.facts(state).flags,
            independent_world_first.facts(state).flags
        );
    }

    // Override one exported state bit at the same path and verify that a later
    // independent and paired read sees it while the original resource root does not.
    let mut changed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let original_bits = changed["states"][0][1].as_u64().unwrap();
    changed["states"][0][1] = (original_bits ^ 1).into();
    fs::write(
        scratch.join("block-behaviours.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    let changed_pair = BlockFacts::load_pair(&scratch, registry).unwrap();
    let changed_independent = BlockFacts::load(&scratch, registry).unwrap();
    let base_facts = BlockFacts::load(&base, registry).unwrap();
    let state = horizon_world::registry::BlockStateId(0);
    assert_ne!(
        changed_pair.0.get(state).legacy_solid,
        base_facts.get(state).legacy_solid
    );
    assert_eq!(
        changed_pair.0.get(state).legacy_solid,
        changed_independent.get(state).legacy_solid
    );
    // The world loader sees the same overridden legacy solidity flag.
    let changed_world = horizon_world::level::Behaviours::load(registry, &scratch).unwrap();
    assert_eq!(
        changed_world.facts(state).flags & 1 != 0,
        changed_pair.0.get(state).legacy_solid
    );

    fs::write(scratch.join("block-behaviours.json"), b"{").unwrap();
    let paired_error = BlockFacts::load_pair(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    let independent_error = BlockFacts::load(&scratch, registry)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(paired_error, independent_error);
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), b"{}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());

    fs::write(scratch.join("block-behaviours.json"), original).unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_file(scratch.join("gameplay-facts.json")).unwrap();
    fs::write(scratch.join("gameplay-facts.json"), b"{\"states\":[]}").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(scratch.join("gameplay-facts.json"), b"{").unwrap();
    assert!(BlockFacts::load_pair(&scratch, registry).is_err());
    assert!(BlockFacts::load(&scratch, registry).is_err());
    assert!(horizon_world::level::Behaviours::load(registry, &scratch).is_err());
    fs::write(
        scratch.join("gameplay-facts.json"),
        fs::read(base.join("gameplay-facts.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        BlockFacts::load_pair(&scratch, registry)
            .unwrap()
            .0
            .states
            .len(),
        registry.len()
    );
    fs::remove_dir_all(&scratch).unwrap();
}
