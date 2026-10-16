//! VillagerProfessionLayer's cached texture metadata and type-layer head visibility.
use super::{
    anim::Poses,
    render::{PROFESSIONS, VILLAGER_TYPES},
};
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Hat {
    #[default]
    None,
    Partial,
    Full,
}

#[derive(Default)]
pub struct VillagerHats(HashMap<String, Hat>);

impl VillagerHats {
    pub fn load(root: &Path) -> Self {
        let mut hats = HashMap::new();
        for folder in ["villager", "zombie_villager"] {
            for (group, names) in [
                ("type", VILLAGER_TYPES.as_slice()),
                ("profession", PROFESSIONS.as_slice()),
            ] {
                for name in names {
                    let texture = format!("entity/{folder}/{group}/{name}");
                    let path = root.join(format!("assets/minecraft/textures/{texture}.png.mcmeta"));
                    let metadata = fs::read(path)
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
            }
        }
        Self(hats)
    }

    pub fn apply(&self, folder: &str, variant: &str, profession: &str, poses: &mut Poses) {
        let hat = |group, name| {
            self.0
                .get(&format!("{folder}/{group}/{name}"))
                .copied()
                .unwrap_or_default()
        };
        let visible = match hat("profession", profession) {
            Hat::None => true,
            Hat::Partial => hat("type", variant) != Hat::Full,
            Hat::Full => false,
        };
        // VillagerModel.createNoHatModel clears the head and every child.
        poses.visible("head", visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires prepared 26.3 models and textures in HORIZON_TEST_RESOURCES"]
    fn metadata_head_visibility_matches_exported_no_hat_models() {
        use crate::mobs::{load_model, render::draw};
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES");
        let mut hats = VillagerHats::load(Path::new(&root));
        assert_eq!(hats.0["entity/villager/profession/farmer"], Hat::Full);
        assert_eq!(hats.0["entity/villager/type/plains"], Hat::None);
        for (name, folder) in [
            ("villager", "entity/villager"),
            ("villager_baby", "entity/villager"),
            ("zombie_villager", "entity/zombie_villager"),
            ("zombie_villager_baby", "entity/zombie_villager"),
        ] {
            let model = load_model(Path::new(&root), &format!("minecraft:{name}#main")).unwrap();
            let no_hat =
                load_model(Path::new(&root), &format!("minecraft:{name}_no_hat#main")).unwrap();
            for (type_hat, profession_hat, hidden) in [
                (Hat::None, Hat::Full, true),
                (Hat::Full, Hat::Partial, true),
                (Hat::Partial, Hat::Partial, false),
                (Hat::Full, Hat::None, false),
            ] {
                hats.0.insert(format!("{folder}/type/plains"), type_hat);
                hats.0
                    .insert(format!("{folder}/profession/farmer"), profession_hat);
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
