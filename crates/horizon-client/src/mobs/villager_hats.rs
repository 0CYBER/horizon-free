//! VillagerProfessionLayer's cached texture metadata and type-layer head visibility.
#[cfg(test)]
use super::anim::Poses;
use horizon_assets::resource_source::ResourceSource;
use std::collections::HashMap;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum Hat {
    #[default]
    None,
    Partial,
    Full,
}

#[derive(Default)]
pub struct VillagerHats(HashMap<String, Hat>);

impl VillagerHats {
    pub fn load(root: &ResourceSource) -> Self {
        let mut hats = HashMap::new();
        for texture in crate::atlas_sources::list_textures(root) {
            let (namespace, path) = texture.split_once(':').expect("qualified texture");
            if ![
                "entity/villager/type/",
                "entity/villager/profession/",
                "entity/zombie_villager/type/",
                "entity/zombie_villager/profession/",
            ]
            .iter()
            .any(|prefix| path.starts_with(prefix))
            {
                continue;
            }
            let metadata_path = format!("assets/{namespace}/textures/{path}.png.mcmeta");
            let metadata = root
                .read(&metadata_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let hat = match metadata
                .as_ref()
                .and_then(|v| v["villager"]["hat"].as_str())
            {
                Some("partial") => Hat::Partial,
                Some("full") => Hat::Full,
                _ => Hat::None,
            };
            hats.insert(texture, hat);
        }
        Self(hats)
    }

    pub(crate) fn policy(&self, folder: &str, group: &str, name: &str) -> Hat {
        let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
        self.0
            .get(&format!("{namespace}:{folder}/{group}/{path}"))
            .copied()
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn apply(&self, folder: &str, variant: &str, profession: &str, poses: &mut Poses) {
        let visible = match self.policy(folder, "profession", profession) {
            Hat::None => true,
            Hat::Partial => self.policy(folder, "type", variant) != Hat::Full,
            Hat::Full => false,
        };
        // VillagerModel.createNoHatModel clears the head and every child.
        poses.visible("head", visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolved_texture_metadata_controls_villager_hats_without_files() {
        use horizon_assets::resource_source::Resource;
        let source = ResourceSource::indexed(std::collections::BTreeMap::from([
            (
                "assets/minecraft/textures/entity/villager/profession/farmer.png".into(),
                Resource::bytes(Vec::new()),
            ),
            (
                "assets/minecraft/textures/entity/villager/profession/farmer.png.mcmeta".into(),
                Resource::bytes(br#"{"villager":{"hat":"full"}}"#.to_vec()),
            ),
            (
                "assets/example/textures/entity/villager/type/nested/plains.png".into(),
                Resource::bytes(Vec::new()),
            ),
            (
                "assets/example/textures/entity/villager/type/nested/plains.png.mcmeta".into(),
                Resource::bytes(br#"{"villager":{"hat":"partial"}}"#.to_vec()),
            ),
        ]));
        let hats = VillagerHats::load(&source);
        assert_eq!(
            hats.0["minecraft:entity/villager/profession/farmer"],
            Hat::Full
        );
        assert_eq!(
            hats.0["example:entity/villager/type/nested/plains"],
            Hat::Partial
        );
    }

    #[test]
    fn namespaced_nested_hat_metadata_comes_from_effective_root() {
        let root =
            std::env::temp_dir().join(format!("horizon-profession-hats-{}", std::process::id()));
        let path =
            root.join("assets/example/textures/entity/villager/profession/nested/engineer.png");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, []).unwrap();
        fs::write(
            path.with_extension("png.mcmeta"),
            r#"{"villager":{"hat":"full"}}"#,
        )
        .unwrap();
        let hats = VillagerHats::load(&ResourceSource::directory(root.clone()));
        assert_eq!(
            hats.0["example:entity/villager/profession/nested/engineer"],
            Hat::Full
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn metadata_head_visibility_matches_exported_no_hat_models() {
        use crate::mobs::{load_model, render::draw};
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES");
        let mut hats =
            VillagerHats::load(&ResourceSource::directory(std::path::PathBuf::from(&root)));
        assert_eq!(
            hats.0["minecraft:entity/villager/profession/farmer"],
            Hat::Full
        );
        assert_eq!(hats.0["minecraft:entity/villager/type/plains"], Hat::None);
        for (name, folder) in [
            ("villager", "entity/villager"),
            ("villager_baby", "entity/villager"),
            ("zombie_villager", "entity/zombie_villager"),
            ("zombie_villager_baby", "entity/zombie_villager"),
        ] {
            let model = load_model(
                std::path::Path::new(&root),
                &format!("minecraft:{name}#main"),
            )
            .unwrap();
            let no_hat = load_model(
                std::path::Path::new(&root),
                &format!("minecraft:{name}_no_hat#main"),
            )
            .unwrap();
            for (type_hat, profession_hat, hidden) in [
                (Hat::None, Hat::Full, true),
                (Hat::Full, Hat::Partial, true),
                (Hat::Partial, Hat::Partial, false),
                (Hat::Full, Hat::None, false),
            ] {
                hats.0
                    .insert(format!("minecraft:{folder}/type/plains"), type_hat);
                hats.0.insert(
                    format!("minecraft:{folder}/profession/farmer"),
                    profession_hat,
                );
                let mut poses = Poses::new(&model);
                hats.apply(folder, "plains", "farmer", &mut poses);
                let mut actual = Vec::new();
                draw(
                    &model,
                    glam::Mat4::IDENTITY,
                    &poses,
                    [0.0, 0.0, 1.0, 1.0],
                    u32::MAX,
                    0,
                    0,
                    &mut actual,
                );
                let expected_model = if hidden { &no_hat } else { &model };
                let mut expected = Vec::new();
                draw(
                    expected_model,
                    glam::Mat4::IDENTITY,
                    &Poses::new(expected_model),
                    [0.0, 0.0, 1.0, 1.0],
                    u32::MAX,
                    0,
                    0,
                    &mut expected,
                );
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&actual),
                    bytemuck::cast_slice::<_, u8>(&expected),
                    "{name}: {type_hat:?}/{profession_hat:?}"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "villager_client_hat_tests.rs"]
mod profession_tests;
