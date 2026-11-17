//! `model_baking_completed` heard by a trait mod through the game's own hooks: once per
//! successful model load, after both atlases and before `client_setup` or
//! `client_resources_reloaded`, with the new store's standalone models resolvable inside it.
use super::native_entity_refs::tests::with_client_game;
use super::packs::ReloadKind;
use mod_api::{
    ClientHost, ClientResourcesReloadedEvent, ClientSetupEvent, Hooks, Identifier, ModEntry,
    ModelBakingCompletedEvent, ModelError, ModelRef, TextureAtlasStitchedEvent,
};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};

/// The standalone model the mod asked for, installed by the fixture as `standalone_models`
/// would register it.
const PART: &str = "minecraft:block/grass_block";
/// A model nobody registered.
const NOBODY: &str = "keeper:block/nobody";

/// What the mod kept: the order of what it heard, and the lookups of each
/// `model_baking_completed`.
#[derive(Default)]
struct Kept {
    heard: Vec<String>,
    part: Vec<Result<ModelRef, ModelError>>,
    nobody: Vec<Result<ModelRef, ModelError>>,
    forms: Vec<[Result<ModelRef, ModelError>; 3]>,
}
struct Keeper(Arc<Mutex<Kept>>);
#[mod_api::__subscriptions]
impl Hooks for Keeper {
    fn texture_atlas_stitched(
        &self,
        _: &mut dyn ClientHost,
        event: &mut TextureAtlasStitchedEvent,
    ) {
        self.0
            .lock()
            .unwrap()
            .heard
            .push(format!("atlas {:?}", event.atlas));
    }
    fn model_baking_completed(&self, host: &mut dyn ClientHost, _: &mut ModelBakingCompletedEvent) {
        let mut kept = self.0.lock().unwrap();
        kept.heard.push("models".into());
        let assets = host.render_assets();
        kept.part.push(assets.model(&id(PART)));
        kept.nobody.push(assets.model(&id(NOBODY)));
        kept.forms.push([
            assets.model(&id("keeper:turned")),
            assets.model(&id("keeper:assembly")),
            assets.model(&id("keeper:missing")),
        ]);
    }
    fn client_setup(&self, _: &mut dyn ClientHost, _: &mut ClientSetupEvent) {
        self.0.lock().unwrap().heard.push("setup".into());
    }
    fn client_resources_reloaded(
        &self,
        _: &mut dyn ClientHost,
        _: &mut ClientResourcesReloadedEvent,
    ) {
        self.0.lock().unwrap().heard.push("reloaded".into());
    }
}

fn id(text: &str) -> Identifier {
    Identifier::parse(text).unwrap()
}

fn keeper() -> (Arc<Mutex<Kept>>, mod_api::Mods) {
    let kept = Arc::new(Mutex::new(Kept::default()));
    let mods = vec![ModEntry::new("keeper", Box::new(Keeper(Arc::clone(&kept))))].into();
    (kept, mods)
}

/// F3+T: the reload stages on a worker and applies here.
pub(in crate::game) fn reload(game: &mut super::Game<'_>) {
    game.spawn_pack_reload(ReloadKind::Selection).unwrap();
    let started = std::time::Instant::now();
    while game.pack_reload.is_some() {
        assert!(
            started.elapsed().as_secs() < 300,
            "the reload never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        game.drive_pack_reload();
    }
}

/// `ModelEvent.BakingCompleted`: the mod hears it once on the first load, after both atlases and
/// before `client_setup`, and its standalone model resolves inside it while an unregistered name
/// does not; a reload fires it once more before `client_resources_reloaded`, the handle from it
/// resolves and the first load's is stale.
#[test]
#[ignore = "daily"]
fn a_client_mod_hears_model_baking_completed_once_per_load_and_keeps_its_handles() {
    let _fixture = horizon_client::models::standalone_fixture(vec![PART.into()]);
    let (kept, mods) = keeper();
    with_mods(mods, || {
        with_client_game(|game| {
            // The first load, as `run` fires them before `client_setup`.
            game.publish_initial_client_models().unwrap();
            let first = {
                let kept = kept.lock().unwrap();
                assert_eq!(
                    kept.heard,
                    ["atlas Blocks", "atlas Gui", "models", "setup"],
                    "once, after both atlases, before client_setup"
                );
                assert_eq!(kept.nobody, [Err(ModelError::UnknownModel)]);
                let [Ok(first)] = kept.part[..] else {
                    panic!("the standalone model resolves in the hook: {:?}", kept.part);
                };
                first
            };
            assert!(
                game.pack.render.resolve_model(first).is_ok(),
                "the handle is of the store the hook published"
            );
            kept.lock().unwrap().heard.clear();
            reload(game);
            let kept = kept.lock().unwrap();
            assert_eq!(
                kept.heard,
                ["atlas Blocks", "atlas Gui", "models", "reloaded"],
                "once more, before client_resources_reloaded"
            );
            assert_eq!(
                kept.nobody,
                [Err(ModelError::UnknownModel), Err(ModelError::UnknownModel)]
            );
            let [Ok(_), Ok(second)] = kept.part[..] else {
                panic!(
                    "the standalone model resolves in both hooks: {:?}",
                    kept.part
                );
            };
            assert!(game.pack.render.resolve_model(second).is_ok());
            assert_eq!(
                game.pack.render.resolve_model(first).err(),
                Some(ModelError::StaleModel),
                "the first load's handle is superseded"
            );
        });
    });
}

/// Real Game/resource-load/model-registry proof, not a GPU render proof.
#[test]
#[ignore = "daily"]
fn standalone_forms_resolve_as_one_model_and_republish_after_resource_reload() {
    use horizon_client::models::StandaloneFormRegistration;
    use mod_api::StandaloneModelForm;
    let cube = "minecraft:block/stone";
    let entry = |form| StandaloneFormRegistration {
        owner: "keeper",
        form,
    };
    let _fixture = horizon_client::models::standalone_fixture(vec![PART.into()]).with_forms(vec![
        entry(StandaloneModelForm::Rotated {
            id: "keeper:turned".into(),
            model: cube.into(),
            turns: [0, 1, 0],
            uvlock: true,
        }),
        entry(StandaloneModelForm::Combined {
            id: "keeper:assembly".into(),
            parts: vec![cube.into(), "minecraft:block/stone_slab".into()],
        }),
        entry(StandaloneModelForm::Combined {
            id: "keeper:missing".into(),
            parts: vec![cube.into(), "keeper:absent_dependency".into()],
        }),
    ]);
    let (kept, mods) = keeper();
    with_mods(mods, || {
        with_client_game(|game| {
            game.publish_initial_client_models().unwrap();
            let [Ok(turned), Ok(assembly), Err(ModelError::UnknownModel)] =
                kept.lock().unwrap().forms[0]
            else {
                panic!("forms not published atomically");
            };
            assert_eq!(
                game.pack.render.resolve_model(turned).unwrap().faces.len(),
                6
            );
            let model = game.pack.render.resolve_model(assembly).unwrap();
            assert_eq!(model.faces.len(), 12);
            // Both original geometries survive one ModelRef, not two draws or two copies
            // of the first part. The full block and the bottom slab have distinct tops.
            for (faces, height) in [(&model.faces[..6], 1.0), (&model.faces[6..], 0.5)] {
                assert!(
                    faces
                        .iter()
                        .any(|face| face.quad.positions.iter().all(|p| p[1] == height))
                );
            }
            reload(game);
            let kept = kept.lock().unwrap();
            assert_eq!(kept.forms.len(), 2);
            let [
                Ok(second_turned),
                Ok(second_assembly),
                Err(ModelError::UnknownModel),
            ] = kept.forms[1]
            else {
                panic!("forms not republished");
            };
            assert_eq!(
                game.pack.render.resolve_model(turned).err(),
                Some(ModelError::StaleModel)
            );
            assert_eq!(
                game.pack.render.resolve_model(assembly).err(),
                Some(ModelError::StaleModel)
            );
            assert_eq!(
                game.pack
                    .render
                    .resolve_model(second_turned)
                    .unwrap()
                    .faces
                    .len(),
                6
            );
            assert_eq!(
                game.pack
                    .render
                    .resolve_model(second_assembly)
                    .unwrap()
                    .faces
                    .len(),
                12
            );
        });
    });
}

/// A reload whose application is refused fires nothing: the mod hears no
/// `model_baking_completed`, and the handle from the last successful load stays right.
#[test]
#[ignore = "daily"]
fn a_failed_reload_fires_no_model_baking_completed() {
    use super::{Progress, packs::stage_pack};
    let _fixture = horizon_client::models::standalone_fixture(vec![PART.into()]);
    let (kept, mods) = keeper();
    with_mods(mods, || {
        with_client_game(|game| {
            game.texture_atlases_stitched();
            game.native_model_baking_completed();
            let first = kept.lock().unwrap().part[0].unwrap();
            kept.lock().unwrap().heard.clear();
            let mut staged = stage_pack(
                &game.vanilla_root(),
                &[],
                &[],
                &game.mods,
                game.register_client_reload_listeners(),
                &[],
                &[],
                &game.pack.assets,
                &game.res,
                &game.pack.context,
                game.settings.force_unicode,
                game.settings.japanese_glyphs,
                &Progress::new(),
            )
            .unwrap();
            // An unreadable enabled-mods marker refuses the staged pack's application.
            let root =
                std::env::temp_dir().join(format!("model-baking-refusal-{}", std::process::id()));
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(
                root.join(horizon_mod_package::enabled::MARKER),
                b"invalid json",
            )
            .unwrap();
            staged.root = root.clone();
            assert!(game.apply_staged_pack(staged).is_err());
            std::fs::remove_dir_all(root).unwrap();
            assert!(kept.lock().unwrap().heard.is_empty(), "nothing fired");
            assert!(
                game.pack.render.resolve_model(first).is_ok(),
                "the last successful load's handle stays right"
            );
        });
    });
}
