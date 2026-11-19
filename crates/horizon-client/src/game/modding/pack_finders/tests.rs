use super::*;
use horizon_client::datapack::{DataStore, FunctionLine};
use horizon_client::text::Text;
use mod_api::{AddPackFindersEvent, Hooks, Identifier, ModEntry};
use mod_platform::testing::with_mods;
use std::{fs, io::Write, path::PathBuf};

const ASSET_PACK: &str = "assets/packmod/resourcepacks/optional";
const DATA_PACK: &str = "data/packmod/datapacks/optional";
const ASSET: &str = "assets/packmod/lang/en_us.json";

struct Fixture {
    root: PathBuf,
    package: PathBuf,
    archive: bool,
    files: Vec<(String, Vec<u8>)>,
}
impl Fixture {
    fn new(archive: bool) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "pack-finders-{}-{}-{number}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&root).unwrap();
        let metadata = |format: (i64, i64)| {
            serde_json::to_vec(&serde_json::json!({
                "pack": {"description": {"translate": "packmod.description"},
                    "min_format": [format.0, format.1], "max_format": [format.0, format.1]},
                "filter": {"block": [{"namespace": "packmod", "path": "lang/hidden\\.json"}]},
            }))
            .unwrap()
        };
        let package = root.join(if archive { "packmod.hmod" } else { "packmod" });
        let fixture = Self {
            root, package, archive,
            files: vec![
                ("horizon-mod.toml".into(), b"id='packmod'\nname='Pack mod'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n".to_vec()),
                (ASSET.into(), br#"{"packmod.optional.title":"Optional example","packmod.description":"An optional test pack"}"#.to_vec()),
                (format!("{ASSET_PACK}/pack.mcmeta"), metadata(horizon_core::version::RESOURCE_PACK_FORMAT)),
                (format!("{ASSET_PACK}/{ASSET}"), br#"{"packmod.optional":"first"}"#.to_vec()),
                (format!("{DATA_PACK}/pack.mcmeta"), metadata(horizon_core::version::DATA_PACK_FORMAT)),
                (format!("{DATA_PACK}/data/packmod/function/probe.mcfunction"), b"say from-optional\n".to_vec()),
            ],
        };
        fixture.publish();
        fixture
    }
    fn publish(&self) {
        if self.archive {
            let next = self.package.with_extension("next.hmod");
            let mut zip = zip::ZipWriter::new(fs::File::create(&next).unwrap());
            for (name, bytes) in &self.files {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            fs::rename(next, &self.package).unwrap();
        } else {
            for (name, bytes) in &self.files {
                let path = self.package.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, bytes).unwrap();
            }
        }
    }
    fn sources(&self) -> ModsState {
        ModsState {
            selected: vec![horizon_mod_package::selection::SelectedPackage {
                path: self.package.clone(),
                package: horizon_mod_package::Package::read(&self.package).unwrap(),
            }]
            .into(),
            ..Default::default()
        }
    }
    fn asset_id(&self) -> String {
        format!("mod/packmod:{ASSET_PACK}")
    }
    fn data_id(&self) -> String {
        format!("mod/packmod:{DATA_PACK}")
    }
    fn replace_asset(&mut self) {
        self.files
            .iter_mut()
            .find(|(name, _)| name == &format!("{ASSET_PACK}/{ASSET}"))
            .unwrap()
            .1 = br#"{"packmod.optional":"second"}"#.to_vec();
        self.publish();
    }
    fn add_feature_flag(&mut self) {
        let file = self
            .files
            .iter_mut()
            .find(|(name, _)| name == &format!("{DATA_PACK}/pack.mcmeta"))
            .unwrap();
        let mut meta: serde_json::Value = serde_json::from_slice(&file.1).unwrap();
        meta["features"] = serde_json::json!({"enabled": ["minecraft:trade_rebalance"]});
        file.1 = serde_json::to_vec(&meta).unwrap();
        self.publish();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Finder {
    feature: bool,
    required: bool,
    position: PackPosition,
    hidden: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Finder {
    fn add_pack_finders(&self, _host: HostSide<'_>, event: &mut AddPackFindersEvent<'_>) {
        for (kind, path) in [
            (PackType::ClientResources, ASSET_PACK),
            (PackType::ServerData, DATA_PACK),
        ] {
            let mut pack = PackRegistration::new(
                Identifier::parse(&format!("packmod:{path}")).unwrap(),
                kind,
                mod_api::Text::translate("packmod.optional.title", vec![]),
            );
            pack.always_active = self.required;
            pack.position = self.position;
            pack.hidden = self.hidden;
            if self.feature && kind == PackType::ServerData {
                pack.source = PackSource::Feature;
            }
            event.packs.add_pack_finders(pack).unwrap();
        }
    }
}
fn with_finder(test: impl FnOnce()) {
    with_finder_config(false, false, test);
}
fn with_finder_config(feature: bool, required: bool, test: impl FnOnce()) {
    with_finder_hidden(feature, required, false, test);
}
fn with_finder_hidden(feature: bool, required: bool, hidden: bool, test: impl FnOnce()) {
    with_mods(
        vec![ModEntry::new(
            "packmod",
            Box::new(Finder {
                feature,
                required,
                position: PackPosition::Bottom,
                hidden,
            }),
        )]
        .into(),
        test,
    );
}

#[test]
fn hidden_packs_reach_the_screens_marked_hidden_and_still_apply() {
    let fixture = Fixture::new(false);
    let sources = fixture.sources();
    with_finder_hidden(false, true, true, || {
        let packs = discover(PackType::ClientResources, &sources);
        let infos = crate::game::packs::pack_infos(&fixture.root, &[], &packs);
        let info = infos
            .iter()
            .find(|info| info.id == fixture.asset_id())
            .unwrap();
        assert!(info.hidden && info.required);
        let data = data_packs(&sources);
        assert!(data[0].selection.as_ref().is_some_and(|info| info.hidden));
        // Required and hidden: on without the player picking it.
        let base = fixture.root.join("base");
        fs::create_dir_all(base.join("assets/packmod/lang")).unwrap();
        fs::write(base.join(ASSET), b"base").unwrap();
        let on = packs::resources(
            ResourceSource::directory(base),
            &packs::enabled(&packs, &[fixture.asset_id()]),
        )
        .unwrap();
        assert_eq!(on.read(ASSET).unwrap(), br#"{"packmod.optional":"first"}"#);
    });
}

#[test]
fn contributed_packs_are_discoverable_and_only_selected_resources_apply() {
    for archive in [false, true] {
        let fixture = Fixture::new(archive);
        let sources = fixture.sources();
        with_finder(|| {
            let packs = discover(PackType::ClientResources, &sources);
            assert_eq!(packs.len(), 1);
            let infos = crate::game::packs::pack_infos(&fixture.root, &[], &packs);
            assert!(
                infos
                    .iter()
                    .any(|info| info.id == fixture.asset_id() && !info.required)
            );
            let base = fixture.root.join("base");
            fs::create_dir_all(base.join("assets/packmod/lang")).unwrap();
            fs::write(base.join(ASSET), b"base").unwrap();
            fs::write(base.join("assets/packmod/lang/hidden.json"), b"hidden").unwrap();
            let base = ResourceSource::directory(base);
            let off = packs::resources(base.clone(), &packs::enabled(&packs, &[])).unwrap();
            assert_eq!(off.read(ASSET).unwrap(), b"base");
            let selected = packs::enabled(&packs, &[fixture.asset_id()]);
            let on = packs::resources(base, &selected).unwrap();
            assert_eq!(on.read(ASSET).unwrap(), br#"{"packmod.optional":"first"}"#);
            assert!(on.read("assets/packmod/lang/hidden.json").is_err());

            let data = data_packs(&sources);
            assert_eq!(data.len(), 1);
            let world = fixture.root.join("world");
            fs::create_dir(&world).unwrap();
            let repository = data_repository(&world, &fixture.root, &data);
            assert!(horizon_client::datapack::enabled_ids(&world, &repository, &[]).is_empty());
            fs::write(
                world.join("datapacks.json"),
                serde_json::json!({
                    "enabled": [fixture.data_id()], "disabled": [],
                })
                .to_string(),
            )
            .unwrap();
            let stack = crate::game::functions::pack_stack(
                &world,
                &fixture.root,
                &Default::default(),
                &Default::default(),
                true,
                &[],
                &data,
            );
            let enabled = stack.enabled_packs().collect::<Vec<_>>();
            let store = DataStore::load(&enabled);
            assert_eq!(
                store.functions["packmod:probe"].lines,
                [FunctionLine::Command("say from-optional".into())]
            );
            if archive {
                assert!(!fixture.root.join("packmod").exists());
            }
        });
    }
}

#[test]
fn rediscovery_opens_the_new_archive_without_changing_old_readers() {
    let mut fixture = Fixture::new(true);
    let sources = fixture.sources();
    with_finder(|| {
        let old = discover(PackType::ClientResources, &sources);
        fixture.replace_asset();
        let new = discover(PackType::ClientResources, &sources);
        assert_eq!(
            old[0].source.open().unwrap().read(ASSET).unwrap(),
            br#"{"packmod.optional":"first"}"#
        );
        assert_eq!(
            new[0].source.open().unwrap().read(ASSET).unwrap(),
            br#"{"packmod.optional":"second"}"#
        );
        assert!(!fixture.root.join("packmod").exists());
    });
}

#[test]
fn refused_registrations_leave_the_repository_unchanged() {
    let fixture = Fixture::new(false);
    let mut registry = Registry::new(PackType::ClientResources, fixture.sources());
    let entry = |path: &str| {
        PackRegistration::new(
            Identifier::parse(path).unwrap(),
            PackType::ClientResources,
            mod_api::Text::translate("packmod.optional.title", vec![]),
        )
    };
    let valid = format!("packmod:{ASSET_PACK}");
    registry.add_pack_finders(entry(&valid)).unwrap();
    let mut refused = entry("packmod:missing");
    let mut title = mod_api::Text::literal("leaf");
    for _ in 1..513 {
        title = mod_api::Text::literal("parent").append(title);
    }
    refused.title = title;
    assert_eq!(
        registry.add_pack_finders(refused),
        Err(PackFinderError::Text(mod_api::TextFormError::TooDeep))
    );
    assert_eq!(registry.packs.len(), 1);
    for (path, error) in [
        (valid.as_str(), PackFinderError::Duplicate),
        ("packmod:../outside", PackFinderError::InvalidPath),
        ("absent:optional", PackFinderError::MissingPackage),
        ("packmod:missing", PackFinderError::InvalidMetadata),
    ] {
        assert_eq!(registry.add_pack_finders(entry(path)), Err(error));
        assert_eq!(registry.packs.len(), 1);
    }
    assert_eq!(
        registry.packs[0]
            .source
            .open()
            .unwrap()
            .read(ASSET)
            .unwrap(),
        br#"{"packmod.optional":"first"}"#
    );
}

#[test]
fn required_packs_keep_their_priority_and_cannot_be_cleared() {
    let fixture = Fixture::new(false);
    let sources = fixture.sources();
    with_finder_config(false, true, || {
        let mut all = discover(PackType::ClientResources, &sources);
        let optional = fixture.root.join("override");
        fs::create_dir_all(optional.join("assets/packmod/lang")).unwrap();
        fs::write(optional.join(ASSET), br#"{"packmod.optional":"override"}"#).unwrap();
        fs::write(
            optional.join("pack.mcmeta"),
            &fixture
                .files
                .iter()
                .find(|(name, _)| name == &format!("{ASSET_PACK}/pack.mcmeta"))
                .unwrap()
                .1,
        )
        .unwrap();
        all.push(
            packs::pack_for(
                "file/override".into(),
                Text::translate("packmod.optional.title", &[]),
                Source::Dir(optional),
                packs::RESOURCE_FORMAT,
            )
            .unwrap(),
        );
        assert!(!packs::able_to_clear_any_pack(&all, &[]));
        let empty = ResourceSource::directory(fixture.root.join("base"));
        let required = packs::enabled(&all, &[]);
        assert_eq!(
            packs::resources(empty.clone(), &required)
                .unwrap()
                .read(ASSET)
                .unwrap(),
            br#"{"packmod.optional":"first"}"#
        );
        let selected = ["file/override".to_owned()];
        assert!(packs::able_to_clear_any_pack(&all, &selected));
        assert_eq!(
            packs::resources(empty, &packs::enabled(&all, &selected))
                .unwrap()
                .read(ASSET)
                .unwrap(),
            br#"{"packmod.optional":"override"}"#
        );
        let info = all[0].selection.as_ref().unwrap();
        let model = packs::SelectionModel::new(vec![info.selectable()], &[]);
        assert!(!model.can_unselect(true, 0));

        let data = data_packs(&sources);
        let mut options = horizon_client::title::WorldOptions::default();
        options.data_packs.push("file/already_installed".into());
        options.set_additional_data_packs(data.clone().into());
        assert!(options.data_packs.contains(&fixture.data_id()));
        assert!(
            options
                .data_packs
                .contains(&"file/already_installed".into())
        );
        assert!(options.data_packs.contains(&"vanilla".into()));
        let world = fixture.root.join("world");
        let repository = data_repository(&world, &fixture.root, &data);
        assert!(
            horizon_client::datapack::enabled_ids(&world, &repository, &[])
                .contains(&fixture.data_id())
        );
    });
}

#[test]
fn contributed_experiments_supply_the_chosen_world_flags() {
    let mut fixture = Fixture::new(true);
    fixture.add_feature_flag();
    with_finder_config(true, false, || {
        let data = data_packs(&fixture.sources());
        let features = horizon_client::datapack::feature_repository(&fixture.root, &data);
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].id, fixture.data_id());
        let mut options = horizon_client::title::WorldOptions::default();
        options.set_additional_data_packs(data.into());
        assert_eq!(
            options.enabled_features(&fixture.root),
            ["minecraft:vanilla"]
        );
        options.data_packs.push(fixture.data_id());
        assert_eq!(
            options.enabled_features(&fixture.root),
            ["minecraft:trade_rebalance", "minecraft:vanilla"]
        );
    });
}

#[test]
#[ignore = "daily"]
fn registered_data_commands_respect_flags_required_packs_and_default_priority() {
    let mut ran = 0;
    for (feature, required) in [(false, false), (false, true), (true, false)] {
        let mut fixture = Fixture::new(false);
        if feature {
            fixture.add_feature_flag();
        }
        with_finder_config(feature, required, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                ran += 1;
                let world = fixture.root.join("world");
                let user = world.join("datapacks/user");
                fs::create_dir_all(user.join("data/packmod/function")).unwrap();
                fs::write(
                    user.join("pack.mcmeta"),
                    br#"{"pack":{"description":"User override fixture"}}"#,
                )
                .unwrap();
                fs::write(
                    user.join("data/packmod/function/probe.mcfunction"),
                    b"say from-user\n",
                )
                .unwrap();
                game.world_root = world;
                game.mods = fixture.sources();
                game.set_allow_commands(true);
                game.reload_datapacks(false, &[]).unwrap();
                assert!(game.run_command("datapack list"));
                let enable = format!("datapack enable \"{}\"", fixture.data_id());
                let disable = format!("datapack disable \"{}\"", fixture.data_id());
                if feature {
                    assert!(!game.run_command(&enable));
                    assert!(!game.fns.enabled().contains(&fixture.data_id()));
                    let flags = vec![
                        "minecraft:vanilla".into(),
                        "minecraft:trade_rebalance".into(),
                    ];
                    game.level.enabled_features = flags.clone().into();
                    game.reload_datapacks(false, &flags).unwrap();
                    assert!(game.fns.enabled().contains(&fixture.data_id()));
                    assert!(!game.run_command(&disable));
                    assert!(game.fns.enabled().contains(&fixture.data_id()));
                } else if required {
                    assert!(game.fns.enabled().contains(&fixture.data_id()));
                    assert!(!game.run_command(&disable));
                    assert!(game.fns.enabled().contains(&fixture.data_id()));
                } else {
                    assert!(!game.fns.enabled().contains(&fixture.data_id()));
                    assert!(game.run_command(&enable));
                    assert_eq!(
                        game.fns.store.functions["packmod:probe"].lines,
                        [FunctionLine::Command("say from-user".into())]
                    );
                    assert!(game.run_command(&disable));
                    assert!(game.run_command(&format!("{enable} last")));
                    assert_eq!(
                        game.fns.store.functions["packmod:probe"].lines,
                        [FunctionLine::Command("say from-optional".into())]
                    );
                }
            });
        });
    }
    assert_eq!(ran, 3, "every prepared-resource case must run");
}

#[test]
#[ignore = "daily"]
fn a_mods_pack_is_on_the_screen_and_applies_through_client_reload() {
    // Reload publishes a language just as the other game fixtures do. Keep this
    // fixture's publications visible locally, not at the mercy of their reloads.
    let _language =
        horizon_client::lang::isolate_publication(horizon_client::lang::Language::of("en_us", []));
    let mut fixture = Fixture::new(true);
    let mut ran = false;
    with_finder(|| {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            ran = true;
            game.mods = fixture.sources();
            let staged = crate::game::packs::stage_pack(
                &game.vanilla_root(),
                &[],
                &[],
                &game.mods,
                game.register_client_reload_listeners(),
                &[],
                &[],
                &game.pack.assets,
                &game.res,
                &game.worker_context(),
                false,
                false,
                &crate::game::Progress::new(),
            )
            .unwrap();
            game.apply_staged_pack(staged).unwrap();
            assert_eq!(
                horizon_client::lang::get("packmod.optional.title"),
                "Optional example"
            );
            assert_eq!(
                horizon_client::lang::get("packmod.optional"),
                "packmod.optional"
            );
            game.refresh_pack_lists();
            assert!(
                game.ui
                    .options
                    .pack_infos
                    .iter()
                    .any(|info| info.id == fixture.asset_id())
            );
            game.settings.resource_packs = vec![fixture.asset_id()];
            for expected in ["first", "second"] {
                let listeners = game.register_client_reload_listeners();
                let staged = crate::game::packs::stage_pack(
                    &game.vanilla_root(),
                    &game.settings.resource_packs,
                    &[],
                    &game.mods,
                    listeners,
                    &[],
                    &[],
                    &game.pack.assets,
                    &game.res,
                    &game.worker_context(),
                    false,
                    false,
                    &crate::game::Progress::new(),
                )
                .unwrap();
                assert_eq!(staged.language.get("packmod.optional"), Some(expected));
                game.apply_staged_pack(staged).unwrap();
                assert_eq!(horizon_client::lang::get("packmod.optional"), expected);
                assert!(
                    !game
                        .options
                        .resources
                        .join(format!("{ASSET_PACK}/{ASSET}"))
                        .exists()
                );
                if expected == "first" {
                    fixture.replace_asset();
                }
            }
        });
    });
    assert!(ran, "the prepared-resource fixture must run this case");
}

#[test]
fn hidden_optional_data_pack_is_command_selectable_and_loads_its_function() {
    let fixture = Fixture::new(false);
    with_finder_hidden(false, false, true, || {
        let data = data_packs(&fixture.sources());
        assert_eq!(data.len(), 1);
        let mut info = data[0].selection.clone().unwrap();
        assert!(info.hidden);
        assert!(!info.required);
        info.title = Text::literal("Optional data fixture");
        let atlas = horizon_client::hud::HudAtlas::build(&ResourceSource::directory(
            "/nonexistent/horizon-test-assets".into(),
        ))
        .unwrap();
        let m = horizon_client::screen_draw::Measure(&atlas.font);
        let drawn_labels = |info| {
            let screen = horizon_client::pack_screen::PackScreen::new(
                &m,
                Text::literal("Data packs"),
                480,
                320,
                false,
                vec![info],
                &[],
            );
            screen
                .draw(&m, (0, 0), 0, 0)
                .into_iter()
                .filter_map(|draw| {
                    if let horizon_client::screen_draw::Draw::Text { text, .. } = draw {
                        Some(text.plain())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert!(
            !drawn_labels(info.clone())
                .iter()
                .any(|s| s == "Optional data fixture")
        );
        info.hidden = false;
        assert!(
            drawn_labels(info)
                .iter()
                .any(|s| s == "Optional data fixture")
        );

        let world = fixture.root.join("world");
        fs::create_dir(&world).unwrap();
        let repository = data_repository(&world, &fixture.root, &data);
        assert!(
            repository
                .iter()
                .any(|p| p.id == "mod/packmod:data/packmod/datapacks/optional")
        );
        assert!(horizon_client::datapack::enabled_ids(&world, &repository, &[]).is_empty());
        fs::write(
            world.join("datapacks.json"),
            r#"{"enabled":["mod/packmod:data/packmod/datapacks/optional"],"disabled":[]}"#,
        )
        .unwrap();
        let stack = crate::game::functions::pack_stack(
            &world,
            &fixture.root,
            &Default::default(),
            &Default::default(),
            true,
            &[],
            &data,
        );
        let enabled = stack.enabled_packs().collect::<Vec<_>>();
        let store = DataStore::load(&enabled);
        assert_eq!(
            store.functions["packmod:probe"].lines,
            [FunctionLine::Command("say from-optional".into())]
        );
    });
}

/// A bundled pack is opened as the package's checks read it: an entry its END record leaves
/// out (one the checks would refuse, here for its `\` separators) is never mounted.
#[test]
fn a_bundled_pack_entry_past_the_end_count_is_not_mounted() {
    let mut fixture = Fixture::new(true);
    fixture.files.push((
        format!("{DATA_PACK}/data\\packmod\\function\\hidden.mcfunction"),
        b"say hidden\n".to_vec(),
    ));
    fixture.publish();
    assert!(horizon_mod_package::Package::read(&fixture.package).is_err());
    let mut bytes = fs::read(&fixture.package).unwrap();
    let end = bytes.len() - 22;
    let count = u16::from_le_bytes([bytes[end + 8], bytes[end + 9]]) - 1;
    bytes[end + 8..end + 10].copy_from_slice(&count.to_le_bytes());
    bytes[end + 10..end + 12].copy_from_slice(&count.to_le_bytes());
    fs::write(&fixture.package, &bytes).unwrap();
    let mut registry = Registry::new(PackType::ServerData, fixture.sources());
    let found = registry.add_pack_finders(PackRegistration::new(
        Identifier::parse(&format!("packmod:{DATA_PACK}")).unwrap(),
        PackType::ServerData,
        mod_api::Text::translate("packmod.optional.title", vec![]),
    ));
    assert!(found.is_err());
}
