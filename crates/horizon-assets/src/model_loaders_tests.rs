use super::*;
use crate::{
    bake::{BlockModelTransform, ModelQuad, bake_block_model},
    model::{CuboidModel, Loader},
    resolve::ModelLibrary,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// Answers a fixed list of quads and counts the times it is asked.
struct Fixed(Vec<LoaderQuad>, AtomicUsize);
impl LoaderGeometry for Fixed {
    fn quads(&self, _: &str, _: &str, _: &Value) -> Vec<LoaderQuad> {
        self.1.fetch_add(1, Ordering::Relaxed);
        self.0.clone()
    }
}

/// The north face of a 4-pixel post in the middle of the block.
fn post(texture: &str) -> LoaderQuad {
    LoaderQuad {
        positions: [
            [0.625, 0.0, 0.375],
            [0.375, 0.0, 0.375],
            [0.375, 1.0, 0.375],
            [0.625, 1.0, 0.375],
        ],
        uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
        texture: texture.into(),
        cull: Some(Direction::North),
        tint_index: 2,
        shade: false,
    }
}

fn library(models: &[(&str, &str)], loaders: Arc<Fixed>) -> ModelLibrary {
    let models = models
        .iter()
        .map(|(name, json)| ((*name).to_owned(), CuboidModel::from_json(json).unwrap()))
        .collect();
    ModelLibrary::new(models).unwrap().with_loaders(loaders)
}

fn bake(library: &ModelLibrary, name: &str, turns: [u8; 3]) -> Vec<ModelQuad> {
    bake_block_model(
        &library.resolve(name).unwrap(),
        BlockModelTransform::new(turns, false),
    )
    .unwrap()
}

const BASE: &str = r#"{"textures":{"side":"block/post"}}"#;

#[test]
fn a_registered_loader_keeps_the_model_json_and_an_unknown_one_stays_unknown() {
    register(vec!["asmtest:kept".into()]);
    let kept = CuboidModel::from_json(r#"{"loader":"asmtest:kept","width":3}"#).unwrap();
    let Some(Loader::Registered { kind, config }) = kept.loader else {
        panic!("a registered loader decodes as registered");
    };
    assert_eq!(kind, "asmtest:kept");
    assert_eq!(config["width"], 3);
    assert_eq!(config["loader"], "asmtest:kept");
    let unknown = CuboidModel::from_json(r#"{"loader":"asmtest:nobody"}"#).unwrap();
    assert_eq!(
        unknown.loader,
        Some(Loader::Unknown("asmtest:nobody".into()))
    );
}

#[test]
fn loader_quads_turn_with_the_variant_and_take_the_models_textures() {
    register(vec!["asmtest:post".into()]);
    let loaders = Arc::new(Fixed(
        vec![post("side"), post("absent")],
        AtomicUsize::new(0),
    ));
    let library = library(
        &[
            ("t:base", BASE),
            ("t:post", r#"{"parent":"t:base","loader":"asmtest:post"}"#),
        ],
        Arc::clone(&loaders),
    );
    let quads = bake(&library, "t:post", [0, 1, 0]);
    // The quad whose texture the model lacks is skipped; the other is baked.
    assert_eq!(quads.len(), 1);
    let quad = &quads[0];
    // A y quarter turn takes (x, z) to (1 - z, x): the north face faces east.
    assert_eq!(
        quad.positions,
        [
            [0.625, 0.0, 0.625],
            [0.625, 0.0, 0.375],
            [0.625, 1.0, 0.375],
            [0.625, 1.0, 0.625],
        ]
    );
    assert_eq!(quad.cull, Some(Direction::East));
    assert_eq!(quad.material.sprite, "minecraft:block/post");
    assert_eq!(quad.uv, post("side").uvs);
    assert_eq!((quad.tint_index, quad.shade), (2, false));
    // Each model is asked once, however often it is resolved.
    bake(&library, "t:post", [0, 0, 0]);
    assert_eq!(loaders.1.load(Ordering::Relaxed), 1);
}

#[test]
fn an_unknown_loader_bakes_only_the_elements_and_asks_nobody() {
    let loaders = Arc::new(Fixed(vec![post("side")], AtomicUsize::new(0)));
    let library = library(
        &[(
            "t:other",
            r##"{"loader":"asmtest:unregistered","textures":{"side":"block/post"},
                "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#side"}}}]}"##,
        )],
        Arc::clone(&loaders),
    );
    let quads = bake(&library, "t:other", [0, 0, 0]);
    assert_eq!(quads.len(), 1, "the element's face only");
    assert_eq!(loaders.1.load(Ordering::Relaxed), 0);
}

#[test]
fn inherited_loader_geometry_is_cached_per_model_identity() {
    struct Named(std::sync::Mutex<Vec<String>>);
    impl LoaderGeometry for Named {
        fn quads(&self, _: &str, model: &str, _: &Value) -> Vec<LoaderQuad> {
            self.0.lock().unwrap().push(model.to_owned());
            let mut quad = post("side");
            quad.tint_index = if model == "t:parent" { 1 } else { 7 };
            vec![quad]
        }
    }
    register(vec!["asmtest:named".into()]);
    let callbacks = Arc::new(Named(std::sync::Mutex::new(Vec::new())));
    let models = [
        (
            "t:parent",
            r#"{"loader":"asmtest:named","textures":{"side":"block/post"}}"#,
        ),
        ("t:child", r#"{"parent":"t:parent"}"#),
    ]
    .into_iter()
    .map(|(name, json)| (name.to_owned(), CuboidModel::from_json(json).unwrap()))
    .collect();
    let library = ModelLibrary::new(models)
        .unwrap()
        .with_loaders(callbacks.clone());
    for _ in 0..2 {
        assert_eq!(bake(&library, "t:parent", [0; 3])[0].tint_index, 1);
        assert_eq!(bake(&library, "t:child", [0; 3])[0].tint_index, 7);
    }
    assert_eq!(*callbacks.0.lock().unwrap(), ["t:parent", "t:child"]);
}
