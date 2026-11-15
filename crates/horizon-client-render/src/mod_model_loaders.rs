//! The model loaders native mods register (NeoForge's `ModelEvent.RegisterLoaders`), and the hook
//! point that asks a mod's loader for a model's quads when a resource load resolves it.
use horizon_assets::{
    model::Direction as FaceDirection,
    model_loaders::{LoaderGeometry, LoaderQuad},
};
use horizon_core::{geometry::Direction, nbt::Tag};
use mod_api::{HookId, ModelLoaderQuad, RegistrationContext};
use serde_json::Value;

/// The most quads one model's loader may add. Vanilla's largest block models have 60 faces
/// (`mangrove_propagule_hanging_2` to `_4`), and an obj model has no limit of its own, so this
/// only stops a runaway answer: a model over it is refused whole, never cut.
pub const MAX_LOADER_QUADS: usize = 65_536;

/// Keeps the model loaders native mods registered, before resources load. An id that is not
/// `<namespace>:<path>` in a namespace other than vanilla's, or one registered already, is logged
/// and left out. Returns the ids kept, in registration order.
pub fn register(ids: Vec<String>) -> Vec<String> {
    horizon_assets::model_loaders::register(ids)
}

/// Hook point for native mods (docs/modding/src/how-to/native-mods.md, declared in `hooks/` with
/// the game's): a model whose `loader` a mod registered is baked. `config` is the model's JSON
/// object, `model` its name; the mod that owns `loader` pushes its quads.
#[inline(never)]
pub fn native_bake_model_loader(
    context: &RegistrationContext,
    loader: &str,
    model: &str,
    config: &Tag,
    quads: &mut Vec<ModelLoaderQuad>,
) {
    if !mod_api::__private::listened(HookId::BakeModelLoader) {
        return;
    }
    mod_api::dispatch_bake_model_loader(context, loader, model, config, quads);
}

/// What the mods' loaders answer, for a model library (`ModelLibrary::with_loaders`). It runs on
/// the resource loader, as a registration.
pub struct ModLoaders;

impl LoaderGeometry for ModLoaders {
    fn quads(&self, loader: &str, model: &str, config: &Value) -> Vec<LoaderQuad> {
        use mod_api::__private::game::RegistrationContextNew;
        let Ok(config) = horizon_core::json_nbt::json_to_tag(config) else {
            log::warn!("model {model}: loader {loader}: its JSON doesn't convert to NBT; no quads");
            return Vec::new();
        };
        let _phase =
            mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
        let mut quads = Vec::new();
        native_bake_model_loader(
            &RegistrationContext::new(),
            loader,
            model,
            &config,
            &mut quads,
        );
        checked(loader, model, quads)
    }
}

/// The quads a mod answered, as the baker takes them: a model over [`MAX_LOADER_QUADS`] is
/// refused whole, and a quad with a position or UV that isn't finite is skipped by itself.
fn checked(loader: &str, model: &str, quads: Vec<ModelLoaderQuad>) -> Vec<LoaderQuad> {
    if quads.len() > MAX_LOADER_QUADS {
        log::warn!(
            "model {model}: loader {loader} answered {} quads, over the {MAX_LOADER_QUADS} a \
             model may have; no quads",
            quads.len()
        );
        return Vec::new();
    }
    quads
        .into_iter()
        .filter_map(|quad| {
            let finite = quad
                .positions
                .iter()
                .flatten()
                .chain(quad.uvs.iter().flatten())
                .all(|value| value.is_finite());
            if !finite {
                log::warn!("model {model}: loader {loader}: a quad that isn't finite; skipped");
                return None;
            }
            Some(LoaderQuad {
                positions: quad.positions,
                uvs: quad.uvs,
                texture: quad.texture.trim_start_matches('#').to_owned(),
                cull: quad.cull.map(face),
                tint_index: quad.tint_index,
                shade: quad.shade,
            })
        })
        .collect()
}

fn face(direction: Direction) -> FaceDirection {
    match direction {
        Direction::Down => FaceDirection::Down,
        Direction::Up => FaceDirection::Up,
        Direction::North => FaceDirection::North,
        Direction::South => FaceDirection::South,
        Direction::West => FaceDirection::West,
        Direction::East => FaceDirection::East,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::{
        bake::{BlockModelTransform, bake_block_model},
        model::CuboidModel,
        resolve::ModelLibrary,
    };
    use std::sync::Arc;

    fn quad(texture: &str, x: f32) -> ModelLoaderQuad {
        ModelLoaderQuad {
            positions: [
                [x, 0.0, 0.5],
                [0.0, 0.0, 0.5],
                [0.0, 1.0, 0.5],
                [x, 1.0, 0.5],
            ],
            uvs: [[1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
            texture: texture.into(),
            cull: Some(Direction::Up),
            tint_index: -1,
            shade: true,
        }
    }

    #[test]
    fn a_model_over_the_budget_is_refused_whole_and_one_at_it_is_kept() {
        let at = vec![quad("side", 1.0); MAX_LOADER_QUADS];
        assert_eq!(checked("l", "m", at).len(), 65_536);
        let over = vec![quad("side", 1.0); 65_537];
        assert!(checked("l", "m", over).is_empty());
    }

    #[test]
    fn a_quad_that_isnt_finite_is_skipped_alone() {
        let mut uv_nan = quad("side", 1.0);
        uv_nan.uvs[2][1] = f32::NAN;
        let kept = checked(
            "l",
            "m",
            vec![quad("side", f32::INFINITY), uv_nan, quad("#side", 0.5)],
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].positions[0][0], 0.5);
        assert_eq!(kept[0].texture, "side", "a #reference names the same key");
        assert_eq!(kept[0].cull, Some(FaceDirection::Up));
    }

    /// A native mod's loader: a quad as wide as the model's `width`, in sixteenths.
    struct Panel;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Panel {
        fn register_model_loaders(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::RegisterModelLoadersEvent<'_>,
        ) {
            event.loaders.push("rendtest:panel".into());
        }
        fn bake_model_loader(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::BakeModelLoaderEvent<'_>,
        ) {
            if event.loader != "rendtest:panel" {
                return;
            }
            let width = match event.config.get("width") {
                Some(Tag::Byte(width)) => f32::from(*width) / 16.0,
                _ => 1.0,
            };
            event.quads.push(quad("side", width));
        }
    }

    /// The registered loader's quads are baked with the model; a reload, a new library over the
    /// model as the packs now give it, asks again; with no mod the model has no loader quads.
    #[test]
    fn a_mods_loader_bakes_its_quads_and_answers_again_on_reload() {
        use mod_api::__private::game::RegistrationContextNew;
        let library = |width: u8| {
            let json = format!(
                r#"{{"loader":"rendtest:panel","width":{width},"textures":{{"side":"block/panel"}}}}"#
            );
            ModelLibrary::new(
                [("t:panel".to_owned(), CuboidModel::from_json(&json).unwrap())].into(),
            )
            .unwrap()
            .with_loaders(Arc::new(ModLoaders))
        };
        let widths = |library: &ModelLibrary| {
            bake_block_model(
                &library.resolve("t:panel").unwrap(),
                BlockModelTransform::new([0; 3], false),
            )
            .unwrap()
            .iter()
            .map(|q| (q.positions[0][0], q.material.sprite.clone()))
            .collect::<Vec<_>>()
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("panel", Box::new(Panel))].into();
        mod_platform::testing::with_mods(mods, || {
            let mut ids = Vec::new();
            mod_api::dispatch_register_model_loaders(&RegistrationContext::new(), &mut ids);
            assert_eq!(register(ids), ["rendtest:panel"]);
            assert_eq!(
                widths(&library(8)),
                [(0.5, "minecraft:block/panel".to_owned())]
            );
            assert_eq!(
                widths(&library(4)),
                [(0.25, "minecraft:block/panel".to_owned())]
            );
        });
        mod_platform::testing::with_mods(Vec::new().into(), || {
            assert!(widths(&library(8)).is_empty());
        });
    }
}
