use super::*;

#[test]
fn tuple_shapes_follow_final_state_order_and_separate_collision() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "state_properties":{"active":{"type":"boolean"}},
      "shapes":{"properties":["axis","active"],
        "values":{"x,true":[[0,0,0,0.5,1,1]]},"default":[[0,0,0,1,1,1]]},
      "collision_shapes":{"properties":["active","axis"],
        "values":{"true,x":[[0,0,0,0.25,1,1]]},"default":[]}}"#;
    let (base, mods) = block_base("tuple-shapes", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    let block = &plan.blocks[0];
    let shapes = block.shapes.as_ref().unwrap();
    let collisions = block.collision_shapes.as_ref().unwrap();
    assert_eq!(shapes.len(), 4);
    assert_eq!(collisions.len(), 4);
    for (index, (template, combo)) in block.state_combinations().enumerate() {
        let special = template == 0
            && combo
                .iter()
                .any(|(name, value)| name == "active" && value == "true");
        assert_eq!(
            shapes[index].bounds().unwrap().max(Axis::X),
            if special { 0.5 } else { 1.0 }
        );
        assert_eq!(collisions[index].is_empty(), !special);
        if special {
            assert_eq!(collisions[index].bounds().unwrap().max(Axis::X), 0.25);
        }
    }
    let mut gameplay: J =
        serde_json::from_slice(&fs::read(base.join("gameplay-facts.json")).unwrap()).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    for (index, row) in rows[2..].iter().enumerate() {
        assert_eq!(row.get("full_collision"), Some(&J::Bool(false)));
        let Some(J::Arr(outline)) = row.get("outline") else {
            panic!("outline")
        };
        assert_eq!(outline.len(), shapes[index].boxes().len());
    }
    let mut tag = Tag::Compound(Compound::from([
        (
            JavaString::from("states"),
            Tag::List(vec![test_state_row("x"), test_state_row("y")]),
        ),
        (JavaString::from("shapes"), Tag::List(Vec::new())),
        (
            JavaString::from("cached_shapes"),
            Tag::IntArray(vec![-1, -1]),
        ),
    ]));
    plan.nbt(&mut tag);
    plan.declared_shapes(&mut tag, true).unwrap();
    let Some(Tag::List(entries)) = tag.get("shapes") else {
        panic!("shapes")
    };
    let Some(Tag::IntArray(mapping)) = tag.get("cached_shapes") else {
        panic!("mapping")
    };
    for (index, shape) in collisions.iter().enumerate() {
        assert_eq!(entries[mapping[2 + index] as usize], encode_shape(shape));
    }
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// Declared outline shapes feed gameplay lighting and both occlusion tables,
/// independently of the optional collision shape.
#[test]
fn declared_shapes_drive_lighting_and_occlusion() {
    for (label, no_occlusion) in [("occluding", false), ("nonoccluding", true)] {
        let declaration = format!(
            r#"{{"template":"minecraft:test","item":false,
          "properties":{{"occludes":{}}},
          "shapes":{{"properties":["axis"],"values":{{"x":[[0,0,0,1,1,1]],"y":[[0,0,0,0.5,1,1]]}}}},
          "collision_shapes":{{"default":[]}}}}"#,
            !no_occlusion
        );
        let (base, mods) = block_base(label, &[&declaration]);
        let plan = Plan::new(&base, &mods).unwrap();
        let row =
            r#"{"can_occlude":true,"solid_render":true,"dampening":15,"skylight_down":false}"#;
        let mut gameplay: J =
            serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
        plan.json("gameplay-facts.json", &mut gameplay).unwrap();
        let Some(J::Arr(rows)) = gameplay.get("states") else {
            panic!("states")
        };
        let full = &rows[2];
        let partial = &rows[3];
        assert_eq!(full.get("solid_render"), Some(&J::Bool(!no_occlusion)));
        assert_eq!(
            full.get("dampening"),
            Some(&J::Num((if no_occlusion { 1 } else { 15 }).into()))
        );
        assert_eq!(partial.get("solid_render"), Some(&J::Bool(false)));
        assert_eq!(full.get("shape_occlusion"), Some(&J::Bool(false)));
        assert_eq!(
            partial.get("shape_occlusion"),
            Some(&J::Bool(!no_occlusion))
        );
        assert_eq!(partial.get("skylight_down"), Some(&J::Bool(true)));
        assert_eq!(partial.get("dampening"), Some(&J::Num(0.into())));
        let mut shapes = Tag::Compound(Compound::from([
            (
                JavaString::from("states"),
                Tag::List(vec![test_state_row("x"), test_state_row("y")]),
            ),
            (JavaString::from("shapes"), Tag::List(Vec::new())),
            (
                JavaString::from("cached_shapes"),
                Tag::IntArray(vec![-1, -1]),
            ),
        ]));
        plan.nbt(&mut shapes);
        plan.declared_shapes(&mut shapes, false).unwrap();
        let Some(Tag::List(entries)) = shapes.get("shapes") else {
            panic!("shapes")
        };
        let Some(Tag::IntArray(indices)) = shapes.get("cached_shapes") else {
            panic!("indices")
        };
        let expected = plan.blocks[0].shapes.as_ref().unwrap();
        for (index, shape) in expected.iter().enumerate() {
            assert_eq!(entries[indices[2 + index] as usize], encode_shape(shape));
        }
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}

#[test]
fn declared_partial_shapes_observe_waterlogging_and_template_light_overrides() {
    let declaration = r#"{"template":"minecraft:test","item":false,
      "shapes":{"default":[[0,0,0,1,0.5,1]]}}"#;
    let facts = r#"{"states":[
      ["minecraft:test",false,false,"",{"id":"minecraft:test","properties":{"waterlogged":"false"}}],
      ["minecraft:test",false,true,"",{"id":"minecraft:test","properties":{"waterlogged":"true"}}]],
      "defaults":{"minecraft:test":0}}"#;
    let (base, mods) = block_base_of("shape-light-water", &[declaration], facts);
    let plan = Plan::new(&base, &mods).unwrap();
    let row = r#"{"can_occlude":true,"solid_render":true,"dampening":15,"skylight_down":false}"#;
    let mut gameplay: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[2].get("skylight_down"), Some(&J::Bool(true)));
    assert_eq!(rows[2].get("dampening"), Some(&J::Num(0.into())));
    assert_eq!(rows[3].get("skylight_down"), Some(&J::Bool(false)));
    assert_eq!(rows[3].get("dampening"), Some(&J::Num(1.into())));
    drop(plan);
    fs::write(base.join("block-behaviours.json"), r#"{"blocks":[{"name":"minecraft:test","class":["TintedGlassBlock","TransparentBlock","Block"]}]}"#).unwrap();
    let plan = Plan::new(&base, &mods).unwrap();
    let mut gameplay: J = serde_json::from_str(&format!(r#"{{"states":[{row},{row}]}}"#)).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[2].get("skylight_down"), Some(&J::Bool(false)));
    assert_eq!(rows[2].get("dampening"), Some(&J::Num(15.into())));
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
}

/// A declared waterlogged property changes a fluid-dependent class override,
/// but not TransparentBlock's constant skylight rule.
#[test]
#[ignore = "daily"]
fn declared_waterlogged_vegetation_uses_resolved_fluid_for_skylight() {
    use crate::{
        blocks::Fluid,
        chunk::Chunk,
        light::{LightTables, apply_lighting, light_chunk},
    };
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-vegetation-light-{}", std::process::id()));
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("horizon-mod.toml", &manifest("testmod", "")),
            (
                "data/testmod/horizon/block/poppy.json",
                r#"{
                  "template": "minecraft:poppy",
                  "item": false,
                  "state_properties": {"waterlogged": {"type": "boolean"}},
                  "shapes": {"default": [[0.3125,0,0.3125,0.6875,0.625,0.6875]]}
                }"#,
            ),
            (
                "data/testmod/horizon/block/glass.json",
                r#"{"template":"minecraft:glass","item":false,
                  "state_properties":{"waterlogged":{"type":"boolean"}},
                  "shapes":{"default":[[0,0,0,1,1,1]]}}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(&root, registry).unwrap();
    let tables = LightTables::load(&root, registry, &facts).unwrap();
    // VegetationBlock checks fluid emptiness independently of the outline;
    // TransparentBlock always passes skylight. Values come from those rules,
    // not from the generated facts or from another run of the derivation.
    for (name, wet, down, dampening, sky) in [
        ("testmod:poppy", false, true, 0, 15),
        ("testmod:poppy", true, false, 1, 14),
        ("testmod:glass", false, true, 0, 15),
        ("testmod:glass", true, true, 0, 15),
    ] {
        let default = registry.default_state(name).unwrap();
        let mut state = registry.state(default).unwrap().clone();
        state
            .properties
            .insert("waterlogged".into(), wet.to_string());
        let id = registry.id(&state).unwrap();
        let declared = facts.get(id);
        assert_eq!(declared.waterlogged, wet, "{state:?}");
        assert_eq!(
            declared.fluid,
            if wet { Fluid::Water } else { Fluid::None },
            "{state:?}"
        );
        assert!(!declared.can_occlude, "{state:?}");
        assert_eq!(
            (declared.skylight_down, declared.dampening),
            (down, dampening),
            "{state:?}"
        );
        let mut chunk = Chunk::empty(0, 0);
        chunk.set(8, 3, 8, id.0);
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(&chunk);
        let lights = light_chunk(&region, &tables, true);
        apply_lighting(&mut chunk, lights);
        assert_eq!(chunk.light(8, 4, 8) >> 4, 15, "exposed above {state:?}");
        assert_eq!(chunk.light(8, 3, 8) >> 4, sky, "exposed cell {state:?}");
    }
    fs::remove_dir_all(scratch).unwrap();
}

/// Load the actual generated tables as the renderer and world generator do.
#[test]
#[ignore = "daily"]
fn declared_shapes_reach_render_and_worldgen_occlusion() {
    use crate::{
        chunk::Chunk,
        light::{LightTables, apply_lighting, light_chunk},
    };
    use horizon_core::geometry::Direction;
    use horizon_world::{
        block_properties::{BlockPropertiesRegistry, BlockProperty},
        collision::CachedCollisionRegistry,
        registry::BlockTableFile,
    };
    let Some(base) = prepared() else { return };
    let scratch =
        std::env::temp_dir().join(format!("horizon-shape-occlusion-{}", std::process::id()));
    let manifest = manifest("testmod", "");
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("horizon-mod.toml", &manifest),
            (
                "data/testmod/horizon/block/full.json",
                r#"{"template":"minecraft:stone","item":false,"shapes":{"default":[[0,0,0,1,1,1]]}}"#,
            ),
            (
                "data/testmod/horizon/block/partial.json",
                r#"{"template":"minecraft:stone","item":false,"shapes":{"default":[[0,0,0,1,0.5,1]]}}"#,
            ),
            (
                "data/testmod/horizon/block/open.json",
                r#"{"template":"minecraft:stone","item":false,"properties":{"occludes":false},"shapes":{"default":[[0,0,0,1,1,1]]}}"#,
            ),
            (
                "data/testmod/horizon/block/partial_open.json",
                r#"{"template":"minecraft:stone","item":false,"properties":{"occludes":false},"shapes":{"default":[[0,0,0,1,0.5,1]]}}"#,
            ),
            (
                "data/testmod/horizon/block/bubble.json",
                r#"{"template":"minecraft:bubble_column","item":false,"shapes":{"default":[]}}"#,
            ),
            (
                "data/testmod/horizon/block/kelp.json",
                r#"{"template":"minecraft:kelp","item":false,"shapes":{"default":[[0,0,0,1,0.5625,1]]}}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let assets = Assets::load(&root).unwrap();
    let registry = assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let full = registry.default_state("testmod:full").unwrap();
    let partial = registry.default_state("testmod:partial").unwrap();
    let open = registry.default_state("testmod:open").unwrap();
    let facts = BlockFacts::load(&root, registry).unwrap();
    assert_eq!(
        (facts.get(full).solid_render, facts.get(full).dampening),
        (true, 15)
    );
    assert_eq!(
        (
            facts.get(partial).solid_render,
            facts.get(partial).dampening
        ),
        (false, 0)
    );
    assert_eq!(
        (facts.get(open).solid_render, facts.get(open).dampening),
        (false, 1)
    );
    // Compare against the actual vanilla exports, including the lower slab's
    // partial outline and leaves' full, non-occluding shape.
    let slab = registry.default_state("minecraft:stone_slab").unwrap();
    let leaves = registry.default_state("minecraft:oak_leaves").unwrap();
    for (content, vanilla) in [(full, stone), (partial, slab), (open, leaves)] {
        assert_eq!(
            (
                facts.get(content).solid_render,
                facts.get(content).dampening,
                facts.get(content).skylight_down
            ),
            (
                facts.get(vanilla).solid_render,
                facts.get(vanilla).dampening,
                facts.get(vanilla).skylight_down
            ),
        );
    }
    // Intrinsic water is present without a waterlogged property. Identical
    // declared outlines must retain the template's exported light facts.
    for (content, template) in [
        ("testmod:bubble", "minecraft:bubble_column"),
        ("testmod:kelp", "minecraft:kelp"),
    ] {
        let vanilla = facts.get(registry.default_state(template).unwrap());
        let declared = facts.get(registry.default_state(content).unwrap());
        assert_eq!(declared.outline, vanilla.outline);
        for state in [vanilla, declared] {
            assert!(!state.waterlogged);
            assert_eq!((state.skylight_down, state.dampening), (false, 1));
        }
    }
    let tables = LightTables::load(&root, registry, &facts).unwrap();
    let partial_open = registry.default_state("testmod:partial_open").unwrap();
    for (roof, below) in [(partial, 0), (slab, 0), (partial_open, 15)] {
        // A sealed room with a bottom-half roof: its open top admits sky,
        // but only occluding shapes stop it at the covered bottom face.
        let mut chunk = Chunk::empty(0, 0);
        for x in 4..=12 {
            for z in 4..=12 {
                chunk.set(x, -1, z, stone.0);
                chunk.set(x, 3, z, roof.0);
                if x == 4 || x == 12 || z == 4 || z == 12 {
                    for y in 0..3 {
                        chunk.set(x, y, z, stone.0);
                    }
                }
            }
        }
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(&chunk);
        let lights = light_chunk(&region, &tables, true);
        apply_lighting(&mut chunk, lights);
        assert_eq!(chunk.light(8, 3, 8) >> 4, 15, "open roof top {roof:?}");
        assert_eq!(chunk.light(8, 2, 8) >> 4, below, "roof bottom {roof:?}");
    }
    let mut visibility = crate::models::load_visibility(&root, registry).unwrap();
    for (content, vanilla) in [(full, stone), (partial, slab), (open, leaves)] {
        assert_eq!(
            visibility
                .should_render_face(stone, content, Direction::East, false)
                .unwrap(),
            visibility
                .should_render_face(stone, vanilla, Direction::East, false)
                .unwrap(),
        );
    }
    assert!(
        !visibility
            .should_render_face(stone, full, Direction::East, false)
            .unwrap()
    );
    assert!(
        visibility
            .should_render_face(stone, partial, Direction::East, false)
            .unwrap()
    );
    assert!(
        visibility
            .should_render_face(stone, open, Direction::East, false)
            .unwrap()
    );
    let collisions =
        BlockTableFile::open(&root.join("worldgen/blocks/cached-collision.nbt"), registry).unwrap();
    let collisions =
        CachedCollisionRegistry::from_table(collisions.table("collision").unwrap()).unwrap();
    let properties =
        BlockTableFile::open(&root.join("worldgen/blocks/block-properties.nbt"), registry).unwrap();
    let properties =
        BlockPropertiesRegistry::from_table(properties.table("properties").unwrap(), &collisions)
            .unwrap();
    assert!(
        properties
            .property(full, BlockProperty::SolidRender)
            .unwrap()
    );
    assert!(
        !properties
            .property(partial, BlockProperty::SolidRender)
            .unwrap()
    );
    assert!(
        !properties
            .property(open, BlockProperty::SolidRender)
            .unwrap()
    );
    assert!(!properties.bind_can_occlude().contains(open));
    fs::remove_dir_all(scratch).unwrap();
}

#[test]
fn collision_only_preserves_outline_and_shape_typos_are_errors() {
    let declaration = r#"{"template":"minecraft:test","collision_shapes":{"default":[]}}"#;
    let (base, mods) = block_base("collision-only", &[declaration]);
    let plan = Plan::new(&base, &mods).unwrap();
    assert!(plan.blocks[0].shapes.is_none());
    assert!(
        plan.blocks[0]
            .collision_shapes
            .as_ref()
            .unwrap()
            .iter()
            .all(VoxelShape::is_empty)
    );
    let mut gameplay: J =
        serde_json::from_str(r#"{"states":[{"outline":[[0,0,0,1,1,1]]},{"outline":[]}]}"#).unwrap();
    plan.json("gameplay-facts.json", &mut gameplay).unwrap();
    let Some(J::Arr(rows)) = gameplay.get("states") else {
        panic!("states")
    };
    assert_eq!(rows[0].get("outline"), rows[2].get("outline"));
    assert_eq!(rows[1].get("outline"), rows[3].get("outline"));
    assert_eq!(rows[2].get("full_collision"), Some(&J::Bool(false)));
    fs::remove_dir_all(base.parent().unwrap()).unwrap();
    for (index, table) in [
        r#"{"properties":["axis","typo"],"default":[]}"#,
        r#"{"properties":["axis"],"values":{"bad":[]},"default":[]}"#,
        r#"{"properties":["axis"],"values":{"x":[]}}"#,
    ]
    .iter()
    .enumerate()
    {
        let declaration = format!(r#"{{"template":"minecraft:test","collision_shapes":{table}}}"#);
        let (base, mods) = block_base(&format!("collision-typo-{index}"), &[&declaration]);
        assert!(Plan::new(&base, &mods).is_err(), "{table}");
        fs::remove_dir_all(base.parent().unwrap()).unwrap();
    }
}
