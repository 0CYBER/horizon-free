use super::*;

#[test]
#[ignore = "daily"]
fn content_shapes_drive_outline_and_collision() {
    use horizon_core::{
        geometry::{Aabb, Axis, Vec3},
        position::BlockPos,
    };
    use horizon_world::{collision::CachedCollisionRegistry, registry::BlockStateId};
    let Some(base) = prepared() else {
        return;
    };
    let scratch =
        std::env::temp_dir().join(format!("horizon-content-shapes-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("horizon-mod.toml", &manifest("shapetest", "")),
            (
                "data/shapetest/horizon/block/rod.json",
                r#"{"template":"minecraft:oak_log","shapes":{"properties":["axis"],"values":{"x":[[0,0.25,0.25,1,0.75,0.75]],"y":[[0.25,0,0.25,0.75,1,0.75]],"z":[[0.25,0.25,0,0.75,0.75,1]]}}}"#,
            ),
            (
                "data/shapetest/horizon/block/plain.json",
                r#"{"template":"minecraft:oak_log"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let (named, _) = nbt::read_named(
        &fs::read(root.join("cached-collision.nbt")).unwrap(),
        nbt::Limits::tables(),
    )
    .unwrap();
    let collision = CachedCollisionRegistry::from_nbt(&named.tag, registry).unwrap();
    for axis in ["x", "y", "z"] {
        let state = (0..registry.len())
            .map(|i| BlockStateId(i as u32))
            .find(|id| {
                let s = registry.state(*id).unwrap();
                s.name == "shapetest:rod" && s.properties.get("axis").is_some_and(|v| v == axis)
            })
            .unwrap();
        let shape = collision.cached_shape(state).unwrap().unwrap();
        let f = facts.get(state);
        assert_eq!(shape.boxes().len(), 1);
        assert_eq!(f.outline.len(), 1);
        assert_eq!(f.outline_edges.len(), 12);
        let expected = match axis {
            "x" => [0., 0.25, 0.25, 1., 0.75, 0.75],
            "y" => [0.25, 0., 0.25, 0.75, 1., 0.75],
            _ => [0.25, 0.25, 0., 0.75, 0.75, 1.],
        };
        assert_eq!(f.outline[0], expected);
        let start = Vec3::new(-1., 0.5, 0.5);
        let end = Vec3::new(2., 0.5, 0.5);
        let target = BlockPos::new(0, 0, 0);
        assert!(shape.clip(start, end, target).is_some());
        let bounds = f.outline[0];
        let outline = Aabb::new(
            Vec3::new(bounds[0], bounds[1], bounds[2]),
            Vec3::new(bounds[3], bounds[4], bounds[5]),
        );
        assert_eq!(
            shape.clip(start, end, target),
            Aabb::clip_boxes([outline], start, end, target)
        );
        let player = Aabb::new(Vec3::new(-1., 0.4, 0.4), Vec3::new(-0.2, 0.6, 0.6));
        assert!((shape.collide(Axis::X, player, 2.) - (expected[0] + 0.2)).abs() < 1e-9);
        let miss = Aabb::new(Vec3::new(-1., 0.05, 0.05), Vec3::new(-0.2, 0.15, 0.15));
        assert_eq!(shape.collide(Axis::X, miss, 2.), 2.);
    }
    let plain = registry.default_state("shapetest:plain").unwrap();
    let oak = registry.default_state("minecraft:oak_log").unwrap();
    assert_eq!(facts.get(plain).outline, facts.get(oak).outline);
    assert_eq!(
        collision.cached_shape(plain).unwrap().unwrap().boxes(),
        collision.cached_shape(oak).unwrap().unwrap().boxes()
    );
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn content_block_default_state_is_validated_and_overlaid() {
    let scratch = std::env::temp_dir().join(format!("horizon-mod-default-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let base = scratch.join("base");
    fs::create_dir_all(&base).unwrap();
    let facts = r#"{"states":[
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"up","extended":"false"}}],
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"false"}}],
      ["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"true"}}]],
      "defaults":{"minecraft:piston":0}}"#;
    fs::write(base.join("facts.json"), facts).unwrap();
    fs::write(base.join("gameplay-facts.json"), r#"{"states":[{"item":"minecraft:piston"},{"item":"minecraft:piston"},{"item":"minecraft:piston"}]}"#).unwrap();
    fs::write(base.join("items.json"), r#"{"minecraft:piston":{}}"#).unwrap();
    fs::write(base.join("registries.json"), r#"{"minecraft:item":{"entries":{"minecraft:piston":{"protocol_id":0}}},"minecraft:block":{"entries":{"minecraft:piston":{"protocol_id":0}}}}"#).unwrap();
    for (declaration, expected) in [
        (r#"{"template":"minecraft:piston"}"#, Some(3)),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"north"}}"#,
            Some(4),
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"north","extended":"true"}}"#,
            Some(5),
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"facing":"sideways"}}"#,
            None,
        ),
        (
            r#"{"template":"minecraft:piston","default_state":{"direction":"north"}}"#,
            None,
        ),
    ] {
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("horizon-mod.toml", &manifest("testmod", "")),
                ("data/testmod/horizon/block/test.json", declaration),
            ],
        );
        let mods = [m];
        match (Plan::new(&base, &mods), expected) {
            (Ok(plan), Some(id)) => {
                let mut doc: J = serde_json::from_str(facts).unwrap();
                plan.json("facts.json", &mut doc).unwrap();
                assert_eq!(
                    doc.get("defaults").unwrap().get("testmod:test"),
                    Some(&J::Num(id.into()))
                );
            }
            (Err(e), None) => assert!(e.to_string().contains("testmod:test"), "{e}"),
            _ => panic!("unexpected result for {declaration}"),
        }
    }
    // A malformed registry with duplicate states must not choose one silently.
    let duplicate = facts.replace(
        r#"["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"true"}}]"#,
        r#"["minecraft:piston",false,false,"",{"id":"minecraft:piston","properties":{"facing":"north","extended":"false"}}]"#,
    );
    fs::write(base.join("facts.json"), duplicate).unwrap();
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("horizon-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/horizon/block/test.json",
                r#"{"template":"minecraft:piston","default_state":{"facing":"north"}}"#,
            ),
        ],
    );
    let error = match Plan::new(&base, &[m]) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("duplicate states accepted"),
    };
    assert!(error.contains("matches 2 states"), "{error}");
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn shapes_reject_unknown_or_missing_template_values() {
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-shape-invalid-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    for declaration in [
        r#"{"template":"minecraft:oak_log","shapes":{"properties":["typo"],"values":{"x":[]}}}"#,
        r#"{"template":"minecraft:oak_log","shapes":{"properties":["axis"],"values":{"x":[]}}}"#,
        r#"{"template":"minecraft:oak_log","shapes":{"properties":["axis"],"values":{"x":[],"y":[],"z":[],"bad":[]}}}"#,
    ] {
        let m = folder_mod(
            &scratch.join("mod"),
            &[
                ("horizon-mod.toml", &manifest("shapetest", "")),
                ("data/shapetest/horizon/block/rod.json", declaration),
            ],
        );
        let mods = [m];
        let e = match Plan::new(&base, &mods) {
            Ok(_) => panic!("accepted {declaration}"),
            Err(e) => e.to_string(),
        };
        assert!(e.contains("shapetest:rod"), "{e}");
    }
    fs::remove_dir_all(scratch).unwrap();
}

/// A connecting block (template `minecraft:chorus_plant`, `connects_to` a
/// tag) joins itself and its tag's blocks on each side, needs no support, and
/// leaves vanilla's chorus plant as it was.
#[test]
#[ignore = "daily"]
fn connecting_blocks_and_vanilla_fences_use_content_tags() {
    use horizon_core::position::BlockPos;
    use horizon_core::random::LegacyRandom;
    use horizon_world::level::{Behaviours, LevelAccess, Ticks};
    use std::collections::HashMap;

    let Some(base) = prepared() else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!("horizon-mod-pipe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    let dir = scratch.join("cable");
    for (name, text) in [
        (
            "horizon-mod.toml",
            "id = \"cable\"\nname = \"Cable\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n",
        ),
        (
            "data/cable/horizon/block/tin_cable.json",
            r##"{"template": "minecraft:chorus_plant", "connects_to": "#cable:joins", "thickness": 4}"##,
        ),
        (
            "data/cable/horizon/block/machine.json",
            r#"{"template": "minecraft:iron_block"}"#,
        ),
        (
            "data/cable/tags/block/joins.json",
            r#"{"values": ["cable:machine"]}"#,
        ),
        (
            "data/cable/horizon/block/wooden_post.json",
            r#"{"template": "minecraft:end_rod"}"#,
        ),
        (
            "data/cable/horizon/block/nether_post.json",
            r#"{"template": "minecraft:end_rod"}"#,
        ),
        (
            "data/cable/horizon/block/untagged_post.json",
            r#"{"template": "minecraft:end_rod"}"#,
        ),
        (
            "data/minecraft/tags/block/fences.json",
            r#"{"values": ["cable:wooden_post", "cable:nether_post"]}"#,
        ),
        (
            "data/minecraft/tags/block/wooden_fences.json",
            r#"{"values": ["cable:wooden_post"]}"#,
        ),
    ] {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let package = horizon_mod_package::Package::read(&dir).unwrap();
    let m = ContentMod {
        id: "cable".into(),
        content: Content::read(&dir, &package.manifest).unwrap(),
        jars: Vec::new(),
    };
    let relative = relative_base(&base);
    let root = resource_root_fixture(
        &relative,
        Path::new(""),
        std::slice::from_ref(&m),
        &scratch.join("cache"),
    )
    .unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let b = Behaviours::load(registry, &root).unwrap();

    struct Level {
        blocks: HashMap<BlockPos, BlockStateId>,
        air: BlockStateId,
        random: LegacyRandom,
    }
    impl horizon_world::level::BlockEntityNeighbors for Level {}
    impl LevelAccess for Level {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.blocks.get(&pos).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            Some(self.blocks.insert(pos, state).unwrap_or(self.air))
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
    }
    let air = registry.default_state("minecraft:air").unwrap();
    let mut level = Level {
        blocks: HashMap::new(),
        air,
        random: LegacyRandom::new(1),
    };
    let mut ticks = Ticks::new(0);
    ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
        .unwrap();
    let sides = |state: BlockStateId| {
        let s = registry.state(state).unwrap();
        ["down", "up", "north", "south", "west", "east"]
            .into_iter()
            .filter(|k| s.properties.get(*k).map(String::as_str) == Some("true"))
            .collect::<Vec<_>>()
    };
    let at = |x, y, z| BlockPos::new(x, y, z);
    let machine = registry.default_state("cable:machine").unwrap();
    let cable = registry.default_state("cable:tin_cable").unwrap();
    // A cable beside a machine (in its tag) and a stone joins the machine only,
    // and floats: no chorus plant support.
    b.set_block(&mut level, &mut ticks, at(1, 64, 0), machine, 3);
    b.set_block(
        &mut level,
        &mut ticks,
        at(-1, 64, 0),
        registry.default_state("minecraft:stone").unwrap(),
        3,
    );
    let placed = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(0, 64, 0), cable);
    b.set_block(&mut level, &mut ticks, at(0, 64, 0), placed, 3);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["east"]);
    assert!(b.can_survive(&mut level, &mut ticks, at(0, 64, 0), placed));
    // A second cable above: both join, through the neighbour's shape update.
    let above = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(0, 65, 0), cable);
    b.set_block(&mut level, &mut ticks, at(0, 65, 0), above, 3);
    assert_eq!(sides(level.block_state(at(0, 65, 0))), ["down"]);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["up", "east"]);
    // The machine gone: the arm goes too.
    b.set_block(&mut level, &mut ticks, at(1, 64, 0), air, 3);
    assert_eq!(sides(level.block_state(at(0, 64, 0))), ["up"]);
    // Its shapes are 4 pixels across: the outline and the cached collision.
    let joined = level.block_state(at(0, 64, 0));
    let thin = horizon_world::collision::pipe_shape(4.0, [false, true, false, false, false, false])
        .unwrap();
    let facts = BlockFacts::load(&root, registry).unwrap();
    let outline = &facts.get(joined).outline;
    assert_eq!(outline.len(), thin.boxes().len());
    let file =
        horizon_world::registry::BlockTableFile::open(&root.join("cached-collision.nbt"), registry)
            .unwrap();
    let cached = horizon_world::collision::CachedCollisionRegistry::from_table(
        file.table("collision").unwrap(),
    )
    .unwrap();
    assert!(cached.has_canonical_identities());
    let shape = cached.cached_shape(joined).unwrap().unwrap();
    assert_eq!(shape.boxes(), thin.boxes());
    assert_eq!(
        shape
            .bounds()
            .map(|b| b.min(horizon_core::geometry::Axis::X)),
        Some(0.375)
    );
    // Vanilla's chorus plant keeps its own rules: it doesn't join the machine.
    b.set_block(&mut level, &mut ticks, at(5, 64, 0), machine, 3);
    let chorus = registry.default_state("minecraft:chorus_plant").unwrap();
    let chorus = b.update_from_neighbour_shapes(&mut level, &mut ticks, at(4, 64, 0), chorus);
    assert!(sides(chorus).is_empty());

    // The same non-sturdy template joins vanilla fences only through the matching tag.
    let oak = registry.default_state("minecraft:oak_fence").unwrap();
    let nether = registry
        .default_state("minecraft:nether_brick_fence")
        .unwrap();
    let fence_pos = at(8, 64, 0);
    let post_pos = at(9, 64, 0);
    for (id, joins_oak, joins_nether) in [
        ("cable:untagged_post", false, false),
        ("cable:wooden_post", true, false),
        ("cable:nether_post", false, true),
    ] {
        let post = registry.default_state(id).unwrap();
        assert!(!b.is_face_sturdy(post, horizon_core::geometry::Direction::West));
        b.set_block(&mut level, &mut ticks, post_pos, post, 3);
        for (fence, joins) in [(oak, joins_oak), (nether, joins_nether)] {
            let state = b.update_from_neighbour_shapes(&mut level, &mut ticks, fence_pos, fence);
            assert_eq!(sides(state).contains(&"east"), joins, "{id}");
        }
    }
    fs::remove_dir_all(&scratch).unwrap();
}
