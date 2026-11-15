use super::*;
use crate::mobs::{anim::Poses, load_model, render::draw};
use horizon_assets::resource_source::{Resource, ResourceSource};

#[test]
#[ignore = "daily"]
fn villager_client_partial_hat_cache_refreshes() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let table = ResolvedProfessions::vanilla();
    for (species, folder, zombie) in [
        ("villager", "entity/villager", false),
        ("zombie_villager", "entity/zombie_villager", true),
    ] {
        let model = load_model(&root, &format!("minecraft:{species}#main")).unwrap();
        let no_hat = load_model(&root, &format!("minecraft:{species}_no_hat#main")).unwrap();
        for (type_hat, hidden) in [("full", true), ("none", false)] {
            let source = ResourceSource::indexed(std::collections::BTreeMap::from([
                (
                    format!("assets/minecraft/textures/{folder}/type/plains.png"),
                    Resource::bytes(vec![]),
                ),
                (
                    format!("assets/minecraft/textures/{folder}/type/plains.png.mcmeta"),
                    Resource::bytes(
                        format!(r#"{{"villager":{{"hat":"{type_hat}"}}}}"#).into_bytes(),
                    ),
                ),
                (
                    format!("assets/minecraft/textures/{folder}/profession/farmer.png"),
                    Resource::bytes(vec![]),
                ),
                (
                    format!("assets/minecraft/textures/{folder}/profession/farmer.png.mcmeta"),
                    Resource::bytes(br#"{"villager":{"hat":"partial"}}"#.to_vec()),
                ),
            ]));
            install(&table, &VillagerHats::load(&source));
            let mut poses = Poses::new(&model);
            poses.visible(
                "head",
                type_hat_shown(&ProfessionState::Vanilla(5), zombie, 0),
            );
            let expected_model = if hidden { &no_hat } else { &model };
            let (mut actual, mut expected) = (Vec::new(), Vec::new());
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
                "{species}/{type_hat}"
            );
        }
    }
}

#[test]
fn profession_cache_names_remain_borrowed_after_interner_limit() {
    for index in 0..4096 {
        interned(format_args!("test/interner/{index}"));
    }
    assert!(matches!(
        profession_texture("entity/villager", "test:beyond_limit"),
        Cow::Borrowed("test:entity/villager/profession/beyond_limit")
    ));
    assert!(
        texture(
            &ProfessionState::Unresolved("missing:retired".into()),
            false
        )
        .is_none()
    );
    assert!(texture(&ProfessionState::None, false).is_none());
}
