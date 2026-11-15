//! The skull types native mods register (`create_skull_models`): what is kept, what is left
//! out, and that only a kept kind is a `minecraft:head` kind. Its own test binary, as both the
//! model layers and the skull types are registered once per process.

use horizon_assets::item_model::{SpecialKind, SpecialModel, is_skull_kind};
use horizon_client::{mobs::model_layers, skull_models};
use mod_api::{CubeListBuilder, LayerDefinition, MeshDefinition, PartPose, SkullModel};

fn skull(kind: &str, layer: &str, texture: &str) -> SkullModel {
    SkullModel {
        kind: kind.into(),
        layer: layer.into(),
        texture: texture.into(),
    }
}

#[test]
fn skull_types_keep_a_known_layer_and_leave_out_the_rest() {
    let mut mesh = MeshDefinition::new();
    let root = mesh.root();
    let head = CubeListBuilder::create()
        .tex_offs(0, 0)
        .add_box([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0]);
    mesh.add_or_replace_child(root, "head", head, PartPose::offset(0.0, 1.0, 0.0));
    model_layers::register(vec![(
        "skulltest:goblin_head#main".into(),
        LayerDefinition::create(mesh, 64, 32),
    )]);
    // A vanilla layer, read from `entity-models.json` under the vanilla root.
    let vanilla_root = std::env::temp_dir().join(format!("skull-models-{}", std::process::id()));
    std::fs::create_dir_all(&vanilla_root).unwrap();
    std::fs::write(
        vanilla_root.join("entity-models.json"),
        r#"{"minecraft:zombie_head#main": {"pose": [0,0,0,0,0,0,1,1,1], "children":
            {"head": {"pose": [0,2,0,0,0,0,1,1,1], "polygons": [[[0,0,0,0,0],[1,0,0,1,0],[1,1,0,1,1],[0,1,0,0,1]]]}}}}"#,
    )
    .unwrap();
    let kept = skull_models::register(
        vec![
            skull(
                "skulltest:goblin",
                "skulltest:goblin_head#main",
                "skulltest:entity/goblin",
            ),
            skull(
                "skulltest:zombie_like",
                "minecraft:zombie_head#main",
                "entity/zombie/zombie",
            ),
            // A layer nobody has.
            skull(
                "skulltest:ghost",
                "skulltest:ghost_head#main",
                "skulltest:entity/ghost",
            ),
            // No texture.
            skull("skulltest:blank", "skulltest:goblin_head#main", ""),
            // Vanilla's namespace, an unnamespaced kind, and a kind registered twice.
            skull(
                "minecraft:goblin",
                "skulltest:goblin_head#main",
                "skulltest:entity/goblin",
            ),
            skull(
                "goblin",
                "skulltest:goblin_head#main",
                "skulltest:entity/goblin",
            ),
            skull(
                "skulltest:goblin",
                "minecraft:zombie_head#main",
                "entity/zombie/zombie",
            ),
        ],
        &vanilla_root,
    );
    assert_eq!(kept, ["skulltest:goblin", "skulltest:zombie_like"]);
    // An item renderer loaded afterwards draws a mod head as it draws a vanilla one: the kind's
    // model with its default texture, resolved when the renderer loads.
    let drawn = heads_drawn(&vanilla_root);
    let _ = std::fs::remove_dir_all(&vanilla_root);
    assert_eq!(drawn.len(), 3);
    // The goblin: the mod's 8x8x8 head (six faces, half a block wide), in its own sprite.
    assert_eq!(drawn[0].len(), 6);
    let xs = drawn[0].iter().flat_map(|q| q.pos.map(|p| p.x));
    let (low, high) = xs.fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(x), h.max(x)));
    assert!((high - low - 0.5).abs() < 1e-4, "{low}..{high}");
    assert!(
        drawn[0]
            .iter()
            .flat_map(|q| q.uv)
            .all(|[u, v]| (0.5..=0.75).contains(&u) && (0.0..=0.25).contains(&v)),
        "the goblin sprite"
    );
    // A kind on a vanilla layer: that layer's one face.
    assert_eq!(drawn[1].len(), 1);
    // An unregistered kind draws nothing.
    assert!(drawn[2].is_empty());

    // The first registration of a kind wins, with its own layer and texture.
    let goblin = skull_models::part("skulltest:goblin").expect("the goblin head");
    let (name, head) = &goblin.children[0];
    assert_eq!(
        (name.as_str(), head.pose[1], head.polygons.len()),
        ("head", 1.0, 6)
    );
    assert_eq!(
        skull_models::texture("skulltest:goblin"),
        Some("skulltest:entity/goblin")
    );
    let zombie_like = skull_models::part("skulltest:zombie_like").expect("the vanilla layer");
    assert_eq!(zombie_like.children[0].1.pose[1], 2.0);
    assert_eq!(zombie_like.children[0].1.polygons.len(), 1);

    for left_out in [
        "skulltest:ghost",
        "skulltest:blank",
        "minecraft:goblin",
        "goblin",
    ] {
        assert!(skull_models::part(left_out).is_none(), "{left_out}");
        assert!(skull_models::texture(left_out).is_none(), "{left_out}");
        assert!(!is_skull_kind(left_out), "{left_out}");
    }
    // Only a kept kind is a `minecraft:head` kind.
    let head = |kind: &str| {
        SpecialModel::decode(&serde_json::json!({"type": "minecraft:head", "kind": kind}))
    };
    assert!(matches!(
        head("skulltest:goblin").map(|m| m.kind),
        Ok(SpecialKind::Head { kind, .. }) if kind == "skulltest:goblin"
    ));
    assert!(head("skulltest:ghost").is_err());
    assert!(head("skulltest:blank").is_err());
    // Registration happens once; a second call keeps nothing, not even the kind.
    let late = skull(
        "skulltest:late",
        "skulltest:goblin_head#main",
        "skulltest:entity/late",
    );
    assert!(skull_models::register(vec![late], std::path::Path::new(".")).is_empty());
    assert!(skull_models::part("skulltest:late").is_none());
    assert!(!is_skull_kind("skulltest:late"));
}

/// The quads `minecraft:head` draws for the goblin, the zombie-like kind and an unregistered kind,
/// from an item renderer loaded from a bare resource root at `root`.
fn heads_drawn(root: &std::path::Path) -> Vec<Vec<horizon_client::item_render::ItemQuad>> {
    use horizon_client::{
        hud::Rect,
        item_render::{ItemRenderer, NoHost},
    };
    use horizon_world::{
        item_defaults::ItemData,
        native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    };
    for (path, contents) in [
        ("items.json", "{}"),
        ("facts.json", r#"{"states":[]}"#),
        (
            "registries.json",
            r#"{"minecraft:item":{"entries":{}},"minecraft:data_component_type":{"entries":{}}}"#,
        ),
    ] {
        std::fs::write(root.join(path), contents).unwrap();
    }
    let context = LoadContext::new(
        DataLoadOrigin::ClientPreload,
        std::sync::Arc::new(LoadInputs::new(Default::default(), root.join("settings"))),
        [],
    );
    let data = ItemData::load(root, context).unwrap();
    let renderer = ItemRenderer::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &data,
    )
    .unwrap();
    let sprite = |u0, v0| Rect {
        u0,
        v0,
        u1: u0 + 0.25,
        v1: v0 + 0.25,
        w: 64,
        h: 32,
    };
    let sprites = std::collections::HashMap::from([
        ("skulltest:entity/goblin".to_owned(), sprite(0.5, 0.0)),
        ("entity/zombie/zombie".to_owned(), sprite(0.0, 0.5)),
    ]);
    [
        "skulltest:goblin",
        "skulltest:zombie_like",
        "skulltest:ghost",
    ]
    .map(|kind| {
        let model = SpecialModel {
            kind: SpecialKind::Head {
                kind: kind.into(),
                texture: None,
                animation: 0.0,
            },
        };
        let mut quads = Vec::new();
        renderer.submit_special(
            &model,
            &serde_json::Value::Null,
            false,
            glam::Mat4::IDENTITY,
            &sprites,
            &NoHost,
            &mut quads,
        );
        quads
    })
    .into()
}
