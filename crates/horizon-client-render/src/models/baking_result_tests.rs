//! A native trait mod reads real staged faces and wraps a variant before terrain can select it.
use super::*;
use crate::{
    assets::{Assets, Properties, RenderAssets},
    resource_source::ResourceRoot,
};
use horizon_world::registry::BlockStateId;
use mod_api::{
    BakedModelEditError, BakedModelKey, BakedQuadChange, Direction, Hooks, ModEntry,
    ModifyBakingResultEvent, RegistrationContext,
};
use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicUsize, Ordering},
    },
};

const WRAPPED: BakedModelKey<'static> = BakedModelKey::Variant {
    model: "fixture:block/panel",
    rotations: [0, 1, 0],
    uv_lock: true,
};
const CONTROL: BakedModelKey<'static> = BakedModelKey::Variant {
    model: "fixture:block/panel",
    rotations: [0; 3],
    uv_lock: false,
};
const WALL_UVS: [[f32; 2]; 4] = [[0.125, 0.25], [0.125, 0.75], [0.625, 0.75], [0.625, 0.25]];
const POWERED: &[(&str, &str)] = &[("powered", "true")];

struct PanelWrap(Arc<AtomicUsize>);

#[mod_api::__subscriptions]
impl Hooks for PanelWrap {
    fn modify_baking_result(
        &self,
        _: &RegistrationContext,
        event: &mut ModifyBakingResultEvent<'_, '_>,
    ) {
        let (height, sprite) = match self.0.fetch_add(1, Ordering::Relaxed) {
            0 => (1.0, "fixture:block/first"),
            1 => (0.5, "fixture:block/second"),
            other => panic!("unexpected bake {other}"),
        };
        let view = event.models.model(WRAPPED).unwrap();
        assert!(!view.ambient_occlusion());
        assert_eq!(view.quads().len(), 2, "never read back the previous edit");
        let east = view
            .quads()
            .find(|quad| quad.facing == Direction::East)
            .unwrap();
        close(
            east.positions,
            &[
                [1., height, 1.],
                [1., 0., 1.],
                [1., 0., 0.],
                [1., height, 0.],
            ],
        );
        close(east.uvs, &WALL_UVS);
        assert_eq!(east.sprite, sprite);
        assert_eq!(east.cull_face, Some(Direction::East));
        assert_eq!(east.tint_index, Some(2));
        assert!(east.shade);
        assert_eq!(east.shade_direction_override, Some(Direction::North));
        assert_eq!(east.ambient_occlusion, Some(true));
        assert_eq!(east.color, Some(0xffabcdef_u32 as i32));
        assert_eq!(east.light, [6, 3]);

        let mut visited = Vec::new();
        assert!(
            event
                .models
                .visit_block_state("fixture:panel", POWERED, |key, state_view| {
                    visited.push(key);
                    if key == WRAPPED {
                        let same = state_view
                            .quads()
                            .find(|quad| quad.facing == Direction::East)
                            .unwrap();
                        assert!(std::ptr::eq(east.positions, same.positions));
                    }
                })
        );
        assert_eq!(visited, [WRAPPED, CONTROL]);
        assert!(
            !event
                .models
                .visit_block_state("fixture:panel", &[], |_, _| unreachable!())
        );

        let invalid = event.models.edit_quads(WRAPPED, |quad| {
            if quad.facing == Direction::East {
                BakedQuadChange::Remove
            } else {
                BakedQuadChange::Replace {
                    positions: [[f32::NAN; 3]; 4],
                    uvs: *quad.uvs,
                    cull_face: None,
                }
            }
        });
        assert_eq!(invalid, Err(BakedModelEditError::InvalidGeometry));
        assert_eq!(event.models.model(WRAPPED).unwrap().quads().len(), 2);
        // A UV the terrain packing would clamp is rejected, not stored.
        let clamped = event
            .models
            .edit_quads(WRAPPED, |quad| BakedQuadChange::Replace {
                positions: *quad.positions,
                uvs: quad.uvs.map(|[u, v]| [u - 1.0, v]),
                cull_face: None,
            });
        assert_eq!(clamped, Err(BakedModelEditError::InvalidGeometry));
        assert_eq!(event.models.model(WRAPPED).unwrap().quads().len(), 2);

        event
            .models
            .edit_quads(WRAPPED, |quad| {
                if quad.facing == Direction::East {
                    BakedQuadChange::Replace {
                        positions: quad.positions.map(|[x, y, z]| [x - 0.125, y, z]),
                        uvs: quad.uvs.map(|[u, v]| [u, v * 0.5]),
                        cull_face: None,
                    }
                } else {
                    BakedQuadChange::Remove
                }
            })
            .unwrap();
    }
}

fn close<const N: usize>(actual: &[[f32; N]; 4], expected: &[[f32; N]; 4]) {
    for (&actual, &expected) in actual.iter().flatten().zip(expected.iter().flatten()) {
        assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
    }
}

fn publish_panel(path: &Path, pixels: u8, height: u8, sprite: &str) {
    let texture = path.join(format!("assets/fixture/textures/block/{sprite}.png"));
    fs::create_dir_all(texture.parent().unwrap()).unwrap();
    let mut png = png::Encoder::new(fs::File::create(texture).unwrap(), 1, 1);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .unwrap()
        .write_image_data(&[pixels, 0, 0, 255])
        .unwrap();
    write_json(
        path,
        "assets/fixture/models/block/panel.json",
        json!({
            "ambientocclusion": false,
            "textures": {"side": format!("fixture:block/{sprite}")},
            "elements": [{"from": [0, 0, 0], "to": [16, height, 16], "shade_direction_override": "north",
                "faces": {
                    "north": {"texture": "#side", "uv": [2, 4, 10, 12], "cullface": "north", "tintindex": 2,
                        "neoforge_data": {"ambient_occlusion": true, "color": "#ffabcdef", "block_light": 6, "sky_light": 3}},
                    "up": {"texture": "#side", "cullface": "up"}
                }}]
        }),
    );
}

fn write_json(path: &Path, name: &str, value: Value) {
    let file = path.join(name);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, value.to_string()).unwrap();
}

static REGISTRY: LazyLock<BlockRegistry> = LazyLock::new(fixture_registry);

fn fixture_registry() -> BlockRegistry {
    let mut states = vec![
        json!(["air", true, false, "", "minecraft:air"]),
        json!(["panel", false, false, "", {"id":"fixture:panel", "properties":{"powered":"false"}}]),
        json!(["panel", false, false, "", {"id":"fixture:panel", "properties":{"powered":"true"}}]),
        json!(["alias", false, false, "", "fixture:alias"]),
    ];
    let mut defaults = BTreeMap::from([
        ("minecraft:air".to_owned(), 0),
        ("fixture:panel".to_owned(), 1),
        ("fixture:alias".to_owned(), 3),
    ]);
    // The resource loader's built-in demo gallery resolves these selectors even without models.
    for (name, properties) in GALLERY {
        defaults
            .entry(format!("minecraft:{name}"))
            .or_insert_with_key(|name| {
                let id = states.len();
                let properties: BTreeMap<_, _> = properties.iter().copied().collect();
                states.push(
                    json!(["gallery", false, false, "", {"id":name, "properties":properties}]),
                );
                id
            });
    }
    let facts = json!({"defaults":defaults,"states":states});
    BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap()
}

#[test]
fn a_native_wrap_uses_fresh_baked_faces_and_reaches_the_render_catalog() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let rotated = json!({"model":"fixture:block/panel", "y":90, "uvlock":true});
    let original = json!({"model":"fixture:block/panel"});
    write_json(
        path,
        "assets/fixture/blockstates/panel.json",
        json!({"variants":{"powered=false": original, "powered=true": [rotated, original]}}),
    );
    write_json(
        path,
        "assets/fixture/blockstates/alias.json",
        json!({"variants":{"":rotated}}),
    );
    write_json(
        path,
        "model-states.json",
        json!({
            "fixture:panel": {"schema":{"powered":{"named":["false","true"]}}, "states":[{"powered":"false"},{"powered":"true"}]},
            "fixture:alias": {"schema":{}, "states":[{}]}
        }),
    );
    publish_panel(path, 127, 16, "first");
    let registry = &*REGISTRY;
    let assets = Assets {
        registry,
        properties: Properties::for_tests(&vec![(false, false, false, false); registry.len()]),
        material_states: [BlockStateId(0); 6],
        blocks_fluid_flow: Default::default(),
    };
    let reload = || {
        assets
            .reload(&ResourceRoot::directory(path.to_owned()))
            .unwrap()
    };
    let quiet = mod_platform::testing::with_mods(Vec::new().into(), reload);
    let alias = registry.default_state("fixture:alias").unwrap();
    let control = registry.default_state("fixture:panel").unwrap();
    assert_eq!(chosen(&quiet, alias).faces.len(), 2);
    assert_eq!(chosen(&quiet, control).faces.len(), 2);

    let calls = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new("fixture", Box::new(PanelWrap(calls.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        let first = reload();
        check_draw_model(&first, alias, 1.0, "fixture:block/first");
        let quiet_control = chosen(&quiet, control);
        let edit_control = chosen(&first, control);
        assert_eq!(edit_control.faces.len(), quiet_control.faces.len());
        assert!(
            edit_control
                .faces
                .iter()
                .zip(&quiet_control.faces)
                .all(|(actual, original)| actual.quad == original.quad)
        );

        publish_panel(path, 191, 8, "second");
        let second = reload();
        assert!(second.generation > first.generation);
        check_draw_model(&second, alias, 0.5, "fixture:block/second");
        check_draw_model(&first, alias, 1.0, "fixture:block/first");
        let fresh_control = chosen(&second, control);
        assert_eq!(fresh_control.faces.len(), 2);
        assert_eq!(
            fresh_control.faces[0].quad.material.sprite,
            "fixture:block/second"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    });
}

fn chosen(render: &RenderAssets, state: BlockStateId) -> &catalog::BakedModel {
    // This is the actual per-state catalog lookup used by the terrain mesher.
    let mut models = Vec::new();
    render
        .model_catalog
        .models_at(state, BlockPos::new(0, 0, 0), &mut models)
        .unwrap();
    assert_eq!(models.len(), 1);
    models[0]
}

fn check_draw_model(render: &RenderAssets, state: BlockStateId, height: f32, sprite: &str) {
    let [face] = chosen(render, state).faces.as_slice() else {
        panic!("expected one wrapped face");
    };
    let q = &face.quad;
    close(
        &q.positions,
        &[
            [0.875, height, 1.],
            [0.875, 0., 1.],
            [0.875, 0., 0.],
            [0.875, height, 0.],
        ],
    );
    close(&q.uv, &WALL_UVS.map(|[u, v]| [u, v * 0.5]));
    assert_eq!(
        face.uv_fixed,
        [[4096, 4096], [4096, 12288], [20480, 12288], [20480, 4096]]
    );
    assert_eq!(q.cull, None);
    assert_eq!(q.tint_index, 2);
    assert_eq!(face.layer, render.sprite_layers[sprite]);
}
