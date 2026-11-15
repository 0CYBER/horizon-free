//! Parsed resource models for one startup or resource-pack reload.
use crate::{Result, resource_source::ResourceSource};
use horizon_assets::{
    model::CuboidModel,
    resolve::{AssetReader, ModelLibrary},
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Default)]
pub struct ParsedModels {
    models: BTreeMap<String, Arc<CuboidModel>>,
    source: ResourceSource,
}
/// Asset files under a content root, for models that name a file (obj).
struct RootAssets(ResourceSource);
impl AssetReader for RootAssets {
    fn read_text(&self, location: &str) -> Option<String> {
        let (namespace, path) = location.split_once(':')?;
        // Only a resource location's characters, and no `.`/`..` step: an
        // .obj's `mtllib` names its .mtl as raw text, so it must not reach a
        // file outside the content root (`../..:x` as a namespace would).
        let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
        if !namespace.bytes().all(valid)
            || matches!(namespace, "" | "." | "..")
            || !path.bytes().all(|c| valid(c) || c == b'/')
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return None;
        }
        self.0
            .read_to_string(&format!("assets/{namespace}/{path}"))
            .ok()
    }
}
impl ParsedModels {
    /// Every `models/**.json` of a namespace directory, by identifier.
    pub fn load(source: &ResourceSource) -> Self {
        let files: Vec<_> = source
            .list_under("assets", &["models"])
            .into_iter()
            .filter_map(|name| {
                let (namespace, path) = name.strip_prefix("assets/")?.split_once('/')?;
                let path = path.strip_prefix("models/")?.strip_suffix(".json")?;
                Some((format!("{namespace}:{path}"), name))
            })
            .collect();
        let parsed = crate::par::par_map(&files, |(_, path)| {
            let text = source.read_to_string(path).ok()?;
            CuboidModel::from_json(&text).ok().map(Arc::new)
        });
        let models = files
            .into_iter()
            .zip(parsed)
            .filter_map(|((id, _), model)| Some((id, model?)))
            .collect();
        Self {
            models,
            source: source.clone(),
        }
    }

    /// Blocks require every referenced resource, including parents. Items instead
    /// skip broken resources and use the resolver's missing-model fallback.
    pub fn block_library(
        &self,
        source: &ResourceSource,
        pending: Vec<String>,
    ) -> Result<ModelLibrary> {
        self.block_models(source, pending, false)
    }
    /// [`Self::block_library`], but a model that does not read takes the
    /// built-in missing model, logged, as vanilla's model discovery does
    /// ("Missing block model"). Mods' content gets this; vanilla's resources
    /// always read.
    pub fn block_library_or_missing(
        &self,
        source: &ResourceSource,
        pending: Vec<String>,
    ) -> Result<ModelLibrary> {
        self.block_models(source, pending, true)
    }
    fn block_models(
        &self,
        source: &ResourceSource,
        mut pending: Vec<String>,
        or_missing: bool,
    ) -> Result<ModelLibrary> {
        let mut models = BTreeMap::new();
        let mut skipped = std::collections::BTreeSet::new();
        while let Some(name) = pending.pop() {
            if models.contains_key(&name) || skipped.contains(&name) {
                continue;
            }
            let model = match self.models.get(&name) {
                Some(model) => Arc::clone(model),
                None => {
                    let read = || -> Result<CuboidModel> {
                        let (namespace, path) =
                            name.split_once(':').ok_or("invalid model identifier")?;
                        let text = source
                            .read_to_string(&format!("assets/{namespace}/models/{path}.json"))?;
                        Ok(CuboidModel::from_json(&text)?)
                    };
                    match read() {
                        Ok(model) => Arc::new(model),
                        Err(e) if or_missing => {
                            log::warn!("Horizon: missing block model {name}: {e}");
                            skipped.insert(name);
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                }
            };
            pending.extend(model.referenced_models());
            models.insert(name, model);
        }
        Ok(ModelLibrary::from_shared(models)?
            .with_assets(Arc::new(RootAssets(source.clone())))
            .with_loaders(Arc::new(crate::mod_model_loaders::ModLoaders)))
    }
    pub fn into_item_library(self) -> std::result::Result<Arc<ModelLibrary>, String> {
        let source = self.source;
        ModelLibrary::with_shared_item_generator(self.models)
            .map(|library| {
                Arc::new(
                    library
                        .with_assets(Arc::new(RootAssets(source)))
                        .with_loaders(Arc::new(crate::mod_model_loaders::ModLoaders)),
                )
            })
            .map_err(|e| format!("item models: {e:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::resolve::{GENERATED_MODEL, MISSING_MODEL};
    use std::fs;

    #[test]
    fn archive_models_and_auxiliary_files_load_without_an_asset_directory() -> Result<()> {
        use std::io::Write;
        let fixture = Fixture(
            std::env::temp_dir().join(format!("horizon-archive-models-{}", std::process::id())),
        );
        fs::create_dir_all(&fixture.0)?;
        let path = fixture.0.join("models.jar");
        let mut writer = zip::ZipWriter::new(fs::File::create(&path)?);
        for (name, bytes) in [
            ("assets/demo/models/block/base.json", br##"{"textures":{"all":"demo:block/stone"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#all"}}}]}"##.as_slice()),
            ("assets/demo/models/block/child.json", br#"{"parent":"demo:block/base"}"#),
            ("assets/demo/models/cube.obj", b"mtllib cube.mtl\nv 0 0 0\n"),
            ("assets/demo/models/cube.mtl", b"newmtl cube\n"),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default())?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
        let source = ResourceSource::archive(&path)?;
        let parsed = ParsedModels::load(&source);
        let library = parsed.block_library(&source, vec!["demo:block/child".into()])?;
        let child = library.resolve("demo:block/child")?;
        assert_eq!(child.elements().len(), 1);
        assert_eq!(child.material("all")?.unwrap().sprite, "demo:block/stone");
        let assets = RootAssets(source);
        assert_eq!(
            assets.read_text("demo:models/cube.obj").as_deref(),
            Some("mtllib cube.mtl\nv 0 0 0\n")
        );
        assert_eq!(
            assets.read_text("demo:models/cube.mtl").as_deref(),
            Some("newmtl cube\n")
        );
        assert!(!fixture.0.join("assets").exists());
        assert_eq!(fs::read_dir(&fixture.0)?.count(), 1);
        Ok(())
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn write(&self, name: &str, json: &str) {
            let (namespace, path) = name.split_once(':').unwrap();
            let path = self
                .0
                .join(format!("assets/{namespace}/models/{path}.json"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, json).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn shared_views_preserve_resolution_errors_builtins_and_reload() -> Result<()> {
        let fixture = Fixture(
            std::env::temp_dir().join(format!("horizon-shared-models-{}", std::process::id())),
        );
        fixture.write("horizon:parent", r##"{"textures":{"side":"#all","all":"block/stone"},"elements":[{"from":[0,0,0],"to":[16,8,16],"faces":{"up":{"texture":"#side"}}}]}"##);
        fixture.write("horizon:nested/child", r#"{"parent":"horizon:parent","textures":{"all":"block/glass"},"display":{"gui":{"rotation":[30,45,0]}}}"#);
        fixture.write("horizon:broken", "{");
        fixture.write("horizon:dangling", r#"{"parent":"horizon:absent"}"#);
        fixture.write("horizon:a", r#"{"parent":"horizon:b"}"#);
        fixture.write("horizon:b", r#"{"parent":"horizon:a"}"#);
        fixture.write(
            GENERATED_MODEL,
            r#"{"elements":[{"from":[0,0,0],"to":[1,1,1],"faces":{"up":{"texture":"particle"}}}],"gui_light":"side"}"#,
        );
        fixture.write(MISSING_MODEL, r#"{"elements":[]}"#);
        let parsed = ParsedModels::load(&ResourceSource::directory(fixture.0.clone()));
        let direct = ParsedModels::default();
        let names = vec![
            "horizon:nested/child".into(),
            GENERATED_MODEL.into(),
            MISSING_MODEL.into(),
            "horizon:a".into(),
        ];
        let old =
            direct.block_library(&ResourceSource::directory(fixture.0.clone()), names.clone())?;
        let shared = parsed.block_library(&ResourceSource::directory(fixture.0.clone()), names)?;
        for name in ["horizon:nested/child", GENERATED_MODEL, MISSING_MODEL] {
            assert_eq!(
                format!("{:?}", old.resolve(name)?),
                format!("{:?}", shared.resolve(name)?)
            );
        }
        assert!(shared.resolve("horizon:a").is_err());
        for name in ["horizon:broken", "horizon:absent", "horizon:dangling"] {
            let load = |models: &ParsedModels| {
                models
                    .block_library(
                        &ResourceSource::directory(fixture.0.clone()),
                        vec![name.into()],
                    )
                    .err()
                    .unwrap()
                    .to_string()
            };
            assert_eq!(load(&direct), load(&parsed));
            // Mods' content: vanilla's missing model instead of an error.
            let lenient = parsed.block_library_or_missing(
                &ResourceSource::directory(fixture.0.clone()),
                vec![name.into()],
            )?;
            assert_eq!(
                format!("{:?}", lenient.resolve(name)?.elements()),
                format!("{:?}", lenient.resolve(MISSING_MODEL)?.elements()),
                "{name}"
            );
        }
        let items = parsed.into_item_library()?;
        let block = shared.resolve("horizon:nested/child")?;
        let item = items.resolve("horizon:nested/child")?;
        assert!(std::ptr::eq(
            block.elements().as_ptr(),
            item.elements().as_ptr()
        ));
        assert_eq!(
            item.material("side")?.unwrap().sprite,
            "minecraft:block/glass"
        );
        assert_eq!(shared.resolve(GENERATED_MODEL)?.elements().len(), 1);
        assert!(items.resolve(GENERATED_MODEL)?.elements().is_empty());
        assert!(items.resolve(GENERATED_MODEL)?.uses_item_generator());
        for name in [
            "horizon:broken",
            "horizon:absent",
            "horizon:dangling",
            MISSING_MODEL,
        ] {
            assert_eq!(items.resolve(name)?.elements().len(), 1);
        }
        assert!(items.resolve("horizon:a").is_err());
        fixture.write("horizon:parent", r#"{"elements":[]}"#);
        let reloaded = ParsedModels::load(&ResourceSource::directory(fixture.0.clone()))
            .into_item_library()?;
        assert!(
            reloaded
                .resolve("horizon:nested/child")?
                .elements()
                .is_empty()
        );
        assert_eq!(items.resolve("horizon:nested/child")?.elements().len(), 1);
        // An unrelated bad item identifier must not make block preparation fail
        // (which would trigger resource-pack rollback at a different stage).
        fixture.write("horizon:INVALID", "{}");
        let parsed = ParsedModels::load(&ResourceSource::directory(fixture.0.clone()));
        assert!(
            parsed
                .block_library(
                    &ResourceSource::directory(fixture.0.clone()),
                    vec!["horizon:nested/child".into()]
                )
                .is_ok()
        );
        assert!(parsed.into_item_library().is_err());
        Ok(())
    }

    /// An .obj's `mtllib` is raw text: no namespace or path step reaches a
    /// file outside the content root.
    #[test]
    fn root_assets_stay_inside_the_content_root() {
        let root = std::env::temp_dir().join(format!("horizon-root-assets-{}", std::process::id()));
        let inside = root.join("content");
        fs::create_dir_all(inside.join("assets/m/models")).unwrap();
        fs::write(inside.join("assets/m/models/a.mtl"), "newmtl a").unwrap();
        fs::write(root.join("secret.mtl"), "newmtl secret").unwrap();
        let assets = RootAssets(ResourceSource::directory(inside));
        assert_eq!(
            assets.read_text("m:models/a.mtl").as_deref(),
            Some("newmtl a")
        );
        for escape in [
            "../..:secret.mtl",
            "..:../secret.mtl",
            "m:../../../secret.mtl",
            "m:models/./a.mtl",
            "M:models/a.mtl",
            ":models/a.mtl",
        ] {
            assert_eq!(assets.read_text(escape), None, "{escape}");
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
