#[test]
fn bundled_mod_world_presets_do_not_require_a_repository_pack_selection() {
    let mut f = Fixture::new();
    let data = f.root.join("mod-data/data/example/worldgen/world_preset");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("passive.json"), r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}}}"#).unwrap();
    let tags = f
        .root
        .join("mod-data/data/minecraft/tags/worldgen/world_preset");
    fs::create_dir_all(&tags).unwrap();
    fs::write(
        tags.join("normal.json"),
        r#"{"values":["example:passive"]}"#,
    )
    .unwrap();
    f.title
        .set_preset_data_source(crate::resource_source::ResourceSource::directory(
            f.root.join("mod-data"),
        ));
    f.title.enter_singleplayer();
    let Mode::Create { options, .. } = &mut f.title.mode else {
        panic!("create")
    };
    assert_eq!(options.data_packs, ["vanilla"]);
    options.cycle_preset(-1);
    assert_eq!(options.preset_id(), "example:passive");
    assert!(options.dimensions().is_some());
}

#[test]
fn an_empty_normal_world_preset_tag_falls_back_to_all_registry_presets() {
    let mut f = Fixture::new();
    let data = f.root.join("mod-data/data/example/worldgen/world_preset");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("islands.json"), r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}}}"#).unwrap();
    let tags = f
        .root
        .join("mod-data/data/minecraft/tags/worldgen/world_preset");
    fs::create_dir_all(&tags).unwrap();
    fs::write(tags.join("normal.json"), r#"{"replace":true,"values":[]}"#).unwrap();
    f.title
        .set_preset_data_source(crate::resource_source::ResourceSource::directory(
            f.root.join("mod-data"),
        ));
    f.title.enter_singleplayer();
    let Mode::Create { options, .. } = &mut f.title.mode else {
        panic!("create")
    };
    assert_eq!(
        options.preset_order.len(),
        crate::save::WorldPreset::ALL.len() + 2
    );
    for preset in crate::save::WorldPreset::ALL {
        assert!(options.preset_order.iter().any(|id| id == preset.id()));
    }
    options.cycle_preset(-1);
    assert_eq!(options.preset_id(), "example:islands");
    options.cycle_preset(-1);
    assert_eq!(options.preset_id(), "minecraft:debug_all_block_states");
    assert_eq!(options.world_type, crate::save::WorldPreset::Debug);
    options.cycle_preset(1);
    assert_eq!(options.preset_id(), "example:islands");
    options.cycle_preset(1);
    assert_eq!(options.preset_id(), "minecraft:normal");
}

#[test]
fn recreating_a_debug_world_preset_keeps_its_hidden_type() {
    let mut f = Fixture::new();
    f.world("Debug", "Debug");
    crate::save::write_world_gen_settings(
        &f.title.saves.join("Debug"),
        123456,
        crate::save::WorldGen {
            preset: crate::save::WorldPreset::Debug,
            ..Default::default()
        },
    )
    .unwrap();
    f.select();
    f.click("Re-Create");
    let Mode::Create { options, .. } = &f.title.mode else {
        panic!("create")
    };
    assert_eq!(options.preset_id(), "minecraft:debug_all_block_states");
    assert!(
        !options
            .preset_order
            .iter()
            .any(|id| id == "minecraft:debug_all_block_states")
    );
    let Action::Create { options, .. } = f.title.create_action() else {
        panic!("create action")
    };
    assert_eq!(options.world_gen().preset, crate::save::WorldPreset::Debug);
}

#[test]
fn world_preset_narration_matches_the_cycle_and_only_hints_actual_amplified() {
    let mut options = WorldOptions {
        custom_preset: Some("example:islands".into()),
        world_type: crate::save::WorldPreset::Amplified,
        preset_order: vec![
            "minecraft:normal".into(),
            "example:islands".into(),
            "minecraft:flat".into(),
        ],
        tab: 1,
        ..Default::default()
    };
    let button = create_world::buttons(480.0, 262.0, false, &options, true)
        .into_iter()
        .find(|b| b.id == 25)
        .unwrap();
    let narration = create_world::button_narration(&button, false, &options);
    assert!(narration.hint.is_none());
    let crate::widgets::What::Cycle { next, .. } = narration.what else {
        panic!("cycle")
    };
    assert_eq!(next, lang::get("generator.minecraft.flat"));
    options.cycle_preset(1);
    assert_eq!(options.preset_id(), "minecraft:flat");
    options.custom_preset = None;
    options.world_type = crate::save::WorldPreset::Amplified;
    assert!(
        create_world::button_narration(&button, false, &options)
            .hint
            .is_some()
    );
}

#[test]
fn registered_world_preset_editor_commits_only_on_done_and_saves_its_dimensions() {
    use mod_api::PresetEditor;
    let mut f = Fixture::new();
    let data = f.root.join("pack/data/example/worldgen/world_preset");
    fs::create_dir_all(&data).unwrap();
    let tag_dir = f
        .root
        .join("pack/data/minecraft/tags/worldgen/world_preset");
    fs::create_dir_all(&tag_dir).unwrap();
    fs::write(
        tag_dir.join("normal.json"),
        r#"{"values":["example:islands"]}"#,
    )
    .unwrap();
    let dimensions = serde_json::json!({"minecraft:overworld": {
        "type": "minecraft:overworld", "generator": {
            "type": "minecraft:flat", "settings": {
                "biome": "minecraft:plains", "layers": [{"block": "minecraft:stone", "height": 5}],
                "features": false, "lakes": false
            }
        }
    }});
    fs::write(
        data.join("islands.json"),
        serde_json::json!({"dimensions": dimensions}).to_string(),
    )
    .unwrap();
    fs::write(
        data.join("hidden.json"),
        serde_json::json!({"dimensions": dimensions}).to_string(),
    )
    .unwrap();
    let pack = crate::datapack::DataPack::from_source(
        "example".into(),
        crate::resource_source::ResourceSource::directory(f.root.join("pack")),
    );
    f.title
        .set_additional_data_packs(vec![std::sync::Arc::new(pack)].into());
    f.title.enter_singleplayer();
    f.title
        .set_data_packs(vec!["vanilla".into(), "example".into()]);
    preset_editors::register(vec![
        PresetEditor {
            preset: "example:islands".into(),
            screen: "example:editor".into(),
        },
        PresetEditor {
            preset: "example:islands".into(),
            screen: "example:duplicate".into(),
        },
        PresetEditor {
            preset: "example:islands".into(),
            screen: "minecraft:invalid".into(),
        },
    ]);
    let Mode::Create { options, .. } = &mut f.title.mode else {
        panic!("create")
    };
    options.tab = 1;
    // Five vanilla presets then the pack's one custom preset.
    for _ in 0..5 {
        options.cycle_preset(1);
    }
    assert_eq!(options.preset_id(), "example:islands");
    assert!(!options.preset_order.iter().any(|id| id == "example:hidden"));
    let original = options.dimensions().unwrap();
    assert!(
        !f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.id == 26)
            .unwrap()
            .enabled
    );
    f.title
        .set_preset_data_source(crate::resource_source::ResourceSource::directory(
            f.root.join("no-mods"),
        ));
    let click = |title: &mut Title, id, size: (f32, f32)| {
        let button = title
            .buttons(size.0, size.1)
            .into_iter()
            .find(|b| b.id == id)
            .unwrap();
        assert!(button.enabled);
        title.click(button.x + 2.0, button.y + 2.0, size);
    };
    click(&mut f.title, 26, (480.0, 262.0));
    assert_eq!(f.title.mode.id(), "world_preset_editor");
    assert_eq!(f.title.preset_editor().unwrap().screen, "example:editor");
    assert!(
        !f.title
            .set_preset_editor_dimensions(Tag::Compound(Default::default()))
    );
    let oversized = crate::save::json_tag(&serde_json::json!({"minecraft:overworld": {
        "type": "minecraft:overworld", "generator": {"type": "minecraft:flat", "settings": {
            "layers": [{"block": "minecraft:stone", "height": 1000000000}]
        }}
    }}));
    assert!(!f.title.set_preset_editor_dimensions(oversized));
    assert_eq!(f.title.preset_editor_dimensions(), Some(original.clone()));
    let mut edited = original.clone();
    let Tag::Compound(stems) = &mut edited else {
        unreachable!()
    };
    let Tag::Compound(overworld) = stems.get_mut(&"minecraft:overworld".into()).unwrap() else {
        unreachable!()
    };
    overworld.insert(
        "type".into(),
        Tag::String("minecraft:overworld_caves".into()),
    );
    assert!(f.title.set_preset_editor_dimensions(edited.clone()));
    f.title.escape();
    let Mode::Create { options, .. } = &f.title.mode else {
        panic!("back")
    };
    assert_eq!(options.dimensions(), Some(original));
    assert!(f.title.preset_editor().is_none());
    assert!(!f.title.set_preset_editor_dimensions(edited.clone()));
    f.title.focused_button = Some(26);
    f.title.confirm_key_size((320.0, 240.0));
    assert_eq!(f.title.mode.id(), "world_preset_editor");
    assert!(f.title.set_preset_editor_dimensions(edited.clone()));
    f.title.focused_button = Some(94);
    f.title.confirm_key_size((320.0, 240.0));
    let Mode::Create { options, .. } = &f.title.mode else {
        panic!("back")
    };
    assert_eq!(options.dimensions(), Some(edited.clone()));
    let save = f.root.join("created");
    options.write_world_gen(&save, 123).unwrap();
    assert_eq!(
        crate::save::world_gen_settings(&save)
            .unwrap()
            .get("dimensions"),
        Some(&edited)
    );
    assert_eq!(
        crate::save::world_gen_settings(&save).unwrap().get("seed"),
        Some(&Tag::Long(123))
    );
    let saved = crate::save::world_gen_settings(&save);
    let mut refused = options.clone();
    refused.preset_dimensions = Some(Tag::Compound(Default::default()));
    assert!(refused.write_world_gen(&save, 999).is_err());
    assert_eq!(crate::save::world_gen_settings(&save), saved);
    preset_editors::register(Vec::new());
    assert!(
        !f.title
            .buttons(320.0, 240.0)
            .iter()
            .find(|b| b.id == 26)
            .unwrap()
            .enabled
    );
    f.title.focused_button = Some(26);
    f.title.confirm_key_size((320.0, 240.0));
    assert_eq!(f.title.mode.id(), "world_create");
    f.title.set_data_packs(vec!["vanilla".into()]);
    let Mode::Create { options, .. } = &f.title.mode else {
        panic!("create")
    };
    assert_eq!(options.preset_id(), "minecraft:normal");
    assert!(options.dimensions().is_none());
    assert!(
        !f.title
            .buttons(480.0, 262.0)
            .iter()
            .find(|b| b.id == 26)
            .unwrap()
            .enabled
    );
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn finish_backup(&mut self) {
        self.title.backup_shown.set(true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while matches!(self.title.mode, Mode::Backup { .. }) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
            self.title.poll_backup();
        }
    }
}
