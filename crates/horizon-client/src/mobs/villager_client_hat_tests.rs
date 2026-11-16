use super::*;
#[test]
#[ignore = "daily"]
fn engineer_hat_full_and_none_control_type_head() {
    use horizon_assets::resource_source::Resource;
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    for (species, folder) in [
        ("villager", "entity/villager"),
        ("zombie_villager", "entity/zombie_villager"),
    ] {
        let model = crate::mobs::load_model(&root, &format!("minecraft:{species}#main")).unwrap();
        let no_hat =
            crate::mobs::load_model(&root, &format!("minecraft:{species}_no_hat#main")).unwrap();
        for (hat, type_hat, hidden) in [
            ("full", "none", true),
            ("none", "none", false),
            ("partial", "full", true),
            ("partial", "none", false),
        ] {
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
                    format!("assets/test/textures/{folder}/profession/engineer.png"),
                    Resource::bytes(vec![]),
                ),
                (
                    format!("assets/test/textures/{folder}/profession/engineer.png.mcmeta"),
                    Resource::bytes(format!(r#"{{"villager":{{"hat":"{hat}"}}}}"#).into_bytes()),
                ),
            ]));
            let hats = VillagerHats::load(&source);
            let mut poses = Poses::new(&model);
            hats.apply(folder, "plains", "test:engineer", &mut poses);
            let (mut actual, mut expected) = (Vec::new(), Vec::new());
            let expected_model = if hidden { &no_hat } else { &model };
            crate::mobs::render::draw(
                &model,
                glam::Mat4::IDENTITY,
                &poses,
                [0.0, 0.0, 1.0, 1.0],
                u32::MAX,
                0,
                0,
                &mut actual,
            );
            crate::mobs::render::draw(
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
                "{species}/{hat}"
            );
        }
    }
}
