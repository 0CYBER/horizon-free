use super::*;
use crate::game::{native_entity_refs::tests::with_game, world_loading::WorldRequest};
use mod_api::{
    self, AddServerReloadListenersEvent, Hooks, Host, ListenerKey, ModEntry, ReloadApply,
    ReloadResourceError, ServerReloadListener,
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct DataListener {
    live: Arc<Mutex<u64>>,
    reject: Arc<AtomicBool>,
}
impl ServerReloadListener for DataListener {
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let id = Identifier::parse("reload_test:value").unwrap();
        assert!(context.list().any(|entry| entry == &id));
        // Arbitrary IDs only perform map lookups, never filesystem reads.
        assert!(
            context
                .read(&Identifier::parse("reload_test:../private").unwrap())
                .is_none()
        );
        let mut value = 0;
        for id in context.list() {
            value += serde_json::from_slice::<u64>(context.read(id).unwrap())
                .map_err(|error| ReloadResourceError::new(id.clone(), error))?;
        }
        let live = self.live.clone();
        Ok(<dyn ReloadApply>::new(move || {
            *live.lock().unwrap() = value
        }))
    }
}
struct CheckListener(Arc<AtomicBool>);
impl ServerReloadListener for CheckListener {
    fn prepare(
        &self,
        _: &ServerReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        if self.0.load(Ordering::Relaxed) {
            return Err(ReloadResourceError::new(
                Identifier::parse("reload_test:value").unwrap(),
                std::io::Error::other("rejected new machine data"),
            ));
        }
        Ok(<dyn ReloadApply>::new(|| {}))
    }
}
#[mod_api::__subscriptions]
impl Hooks for DataListener {
    fn add_server_reload_listeners(
        &self,
        _host: &dyn Host,
        event: &mut AddServerReloadListenersEvent<'_>,
    ) {
        let id = |text| Identifier::parse(text).unwrap();
        event
            .listeners
            .add_listener(
                id("reload_test:data"),
                FileToIdConverter::json("machines").unwrap(),
                Arc::new(Self {
                    live: self.live.clone(),
                    reject: self.reject.clone(),
                }),
            )
            .unwrap();
        event
            .listeners
            .add_listener(
                id("reload_test:check"),
                FileToIdConverter::json("machines").unwrap(),
                Arc::new(CheckListener(self.reject.clone())),
            )
            .unwrap();
        event
            .listeners
            .add_dependency(id("reload_test:data"), id("reload_test:check"))
            .unwrap();
    }
}
fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
// The generated table directory deliberately contains no copied raw mod data.
// Fixtures publish the same retained data views as the real content loader.
fn build_mod_resources(
    base: &Path,
    jar_store: &Path,
    mods: &[horizon_client::mod_content::ContentMod],
    prepared: &Path,
    inventory: &horizon_mod_package::enabled::EnabledMods,
) -> horizon_assets::resource_source::ResourceRoot {
    horizon_client::mod_content::build(base, jar_store, mods, prepared, inventory).unwrap();
    let resolved = horizon_client::mod_content::resolved_resources(
        base,
        prepared.to_owned(),
        jar_store,
        mods,
        prepared.with_extension("cache"),
    )
    .unwrap();
    for name in resolved.data_changes().list("data/reload_test/machines/") {
        assert!(
            !prepared.join(&name).exists(),
            "raw mod data was copied: {name}"
        );
    }
    resolved
}
fn install_mod_resources(
    game: &mut crate::game::Game<'_>,
    root: &horizon_assets::resource_source::ResourceRoot,
    package: &Path,
) {
    // The real loader publishes admission and resource views together. An
    // inventory reload must see the package that supplied this fixture root.
    game.mods.selected = vec![horizon_mod_package::selection::SelectedPackage {
        path: package.to_owned(),
        package: horizon_mod_package::Package::read(package).unwrap(),
    }]
    .into();
    game.options.resources = root.prepared().to_owned();
    game.pack.source = root.source().clone();
    game.pack.data_source = root.data().clone();
    game.pack.data_changes = root.data_changes().clone();
    assert!(game.stage_enabled_inventory().unwrap().is_none());
}

#[test]
#[ignore = "daily"]
fn reload_command_uses_applied_listener_state_and_preserves_tables_on_refusal() {
    use mod_api::{DataPackFilesEvent, Registrar, RegistrationContext, ServerHost};
    use std::sync::atomic::AtomicUsize;
    struct Listener(Arc<AtomicUsize>);
    impl ServerReloadListener for Listener {
        fn prepare(
            &self,
            _: &ServerReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            let state = self.0.clone();
            Ok(<dyn ReloadApply>::new(move || {
                state.fetch_add(1, Ordering::SeqCst);
            }))
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn add_server_reload_listeners(
            &self,
            _: &dyn Host,
            event: &mut AddServerReloadListenersEvent<'_>,
        ) {
            event
                .listeners
                .add_listener(
                    Identifier::parse("reload_order:listener").unwrap(),
                    FileToIdConverter::json("reload_order").unwrap(),
                    Arc::new(Listener(self.0.clone())),
                )
                .unwrap();
        }
        fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
            let item = if self.0.load(Ordering::SeqCst) == 1 {
                "minecraft:stone"
            } else {
                "minecraft:diamond"
            };
            event.files.push((
                "tags/reload_order:block/derived".into(),
                format!(r#"{{"values":["{item}"]}}"#).into_bytes(),
            ));
        }
    }
    struct Missing;
    #[mod_api::__subscriptions]
    impl Hooks for Missing {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            registrar.item_key("reload_order:missing");
        }
    }
    let state = Arc::new(AtomicUsize::new(0));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "reload_order",
            Box::new(Listener(state.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let root =
                    std::env::temp_dir().join(format!("reload-order-{}", std::process::id()));
                game.load_world(request(&root)).unwrap();
                let class =
                    mod_api::__private::tag_key(&Identifier::parse("horizon:class/fence").unwrap())
                        .unwrap();
                let fence = mod_api::__private::block_key("minecraft:oak_fence").unwrap();
                assert!(mod_api::Tags::blocks(game, class).unwrap().contains(&fence));
                assert!(
                    game.items()
                        .block_in_tag("horizon:class/fence", "minecraft:oak_fence")
                );
                assert_eq!(game.block_in_tag(fence, "horizon:class/fence"), Some(true));
                state.store(0, Ordering::SeqCst);
                assert!(game.run_command("reload"));
                assert_eq!(state.load(Ordering::SeqCst), 1);
                assert!(
                    game.items().block_tags["reload_order:derived"].contains("minecraft:stone")
                );
                assert!(mod_api::Tags::blocks(game, class).unwrap().contains(&fence));
                let old_bound = crate::game::native_world_read::installed_block_tag_snapshot();
                let old_order = game.items().block_tag_order.clone();
                let old_sets = game.items().block_tags.clone();
                let old = game.world_tables.as_ref().map(|tables| tables as *const _);
                crate::game::api_values::install_parts(&[ModEntry::new(
                    "reload_order",
                    Box::new(Missing),
                )])
                .unwrap();
                assert!(!game.run_command("reload"));
                assert_eq!(state.load(Ordering::SeqCst), 2);
                assert_eq!(
                    old,
                    game.world_tables.as_ref().map(|tables| tables as *const _)
                );
                assert!(
                    game.items().block_tags["reload_order:derived"].contains("minecraft:stone")
                );
                assert_eq!(
                    crate::game::native_world_read::installed_block_tag_snapshot(),
                    old_bound
                );
                assert_eq!(game.items().block_tag_order, old_order);
                assert_eq!(game.items().block_tags, old_sets);
                let _ = std::fs::remove_dir_all(root);
            });
        },
    );
}

fn request(root: &Path) -> WorldRequest {
    let mut request = WorldRequest::new(
        "test",
        Some("test"),
        Some(1),
        true,
        None,
        std::sync::Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
    )
    .unwrap();
    request.save_root = root.to_owned();
    request
}

#[test]
#[ignore = "daily"]
fn missing_original_at_reload_preserves_the_published_data_and_reports_its_owners() {
    const MODE: &str = "HORIZON_JAR_RELOAD_CHILD";
    let root = if let Some(root) = std::env::var_os(MODE) {
        std::path::PathBuf::from(root)
    } else {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!("server-reload-jar-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--ignored", "--exact", "game::server_reload::tests::missing_original_at_reload_preserves_the_published_data_and_reports_its_owners", "--nocapture"])
            .env(MODE, &root).env("HORIZON_NO_AUDIO", "1").env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1");
        for key in [
            "HORIZON_HOME",
            "HORIZON_LAUNCHER_HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "HORIZON_LAUNCHER_TRASH",
            "HORIZON_LAUNCHER_CLIPBOARD",
            "HORIZON_LAUNCHER_DESKTOP",
        ] {
            command.env(key, root.join(key));
        }
        let status = command.status().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(status.success());
        return;
    };
    let jar =
        horizon_mod_package::manifest::JarRef::new("original".into(), "1".into(), "a".repeat(128))
            .unwrap();
    let jar_file = jar.cache_path(&horizon_client::logging::game_home().join("jars"));
    std::fs::create_dir_all(jar_file.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar_file).unwrap());
    zip.start_file(
        "data/reload_test/function/keep.mcfunction",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    std::io::Write::write_all(&mut zip, b"say original").unwrap();
    zip.finish().unwrap();
    let original = std::fs::read(&jar_file).unwrap();
    let manifest = format!(
        "id='reload_test'\nname='Reload Test'\nversion='1'\napi='0.2'\nside='both'\ncapabilities=['registries']\n[[jar]]\nid='original'\nversion='1'\nsha512='{}'\n",
        jar.sha512()
    );
    let package = root.join("reload_test.hmod");
    write(
        &package,
        horizon_mod_package::Package::write_files(&manifest, &[]).unwrap(),
    );
    let package_manifest = horizon_mod_package::Package::read(&package)
        .unwrap()
        .manifest;
    let content = horizon_mod_package::Content::read(&package, &package_manifest).unwrap();
    let resources = root.join("resources");
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let resources = build_mod_resources(
        &base,
        jar_file.parent().unwrap(),
        &[horizon_client::mod_content::ContentMod {
            id: "reload_test".into(),
            content,
            jars: vec![jar.clone()],
        }],
        &resources,
        &horizon_mod_package::enabled::EnabledMods::new([(
            "reload_test".into(),
            mod_api::Side::Both,
        )]),
    );
    let world = root.join("world");
    let function = world.join("datapacks/test/data/reload_test/function/keep.mcfunction");
    write(
        &world.join("datapacks/test/pack.mcmeta"),
        r#"{"pack":{"description":"Reload fixture"}}"#,
    );
    write(&function, "say old");
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["file/test"],"disabled":[]}"#,
    );
    with_game(|game| {
        install_mod_resources(game, &resources, &package);
        game.load_world(request(&world)).unwrap();
        let previous = game.fns.store.functions["reload_test:keep"].clone();
        let enabled = game.fns.enabled().to_vec();
        write(&function, "say new");
        std::fs::remove_file(&jar_file).unwrap();
        let error = game
            .reload_datapacks(true, &game.level.enabled_features.clone())
            .err()
            .unwrap();
        let ReloadStageError::OriginalJars(mod_api::JarInputsError::Missing(missing)) = error
        else {
            panic!("expected typed missing original, got {error:?}");
        };
        assert_eq!(missing.owners, ["reload_test"]);
        assert_eq!(missing.id, jar.id());
        assert_eq!(missing.version, jar.version());
        assert_eq!(missing.sha512, jar.sha512());
        assert_eq!(missing.path, jar_file.display().to_string());
        let reported = game.cmd_reload().unwrap_err().message.plain();
        assert!(reported.contains("reload_test: original 1:"), "{reported}");
        assert!(reported.contains(jar.sha512()), "{reported}");
        assert_eq!(game.options.resources, resources.prepared().to_owned());
        assert_eq!(game.fns.store.functions["reload_test:keep"], previous);
        assert_eq!(game.fns.enabled(), enabled);
        write(&jar_file, &original);
        assert!(game.run_command("reload"));
        assert_ne!(game.fns.store.functions["reload_test:keep"], previous);
    });
}

#[test]
#[ignore = "daily"]
fn server_reload_delivers_mod_files_overrides_and_preserves_all_data_on_failure() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let manifest = "id = \"reload_test\"\nname = \"Reload Test\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let package = root.join("reload_test.hmod");
    write(
        &package,
        horizon_mod_package::Package::write_files(
            manifest,
            &[("data/reload_test/machines/value.json", b"1")],
        )
        .unwrap(),
    );
    let package_manifest = horizon_mod_package::Package::read(&package)
        .unwrap()
        .manifest;
    let content = horizon_mod_package::Content::read(&package, &package_manifest).unwrap();
    let resources = root.join("resources");
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let resources = build_mod_resources(
        &base,
        Path::new(""),
        &[horizon_client::mod_content::ContentMod {
            id: "reload_test".into(),
            content,
            jars: Vec::new(),
        }],
        &resources,
        &horizon_mod_package::enabled::EnabledMods::new([(
            "reload_test".into(),
            mod_api::Side::Both,
        )]),
    );
    let world = root.join("world");
    let pack = world.join("datapacks/test");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"Reload fixture"}}"#,
    );
    let data = pack.join("data/reload_test/machines/value.json");
    write(&data, "2");
    // Other namespaces and subdirectories join the same directory listener.
    write(&pack.join("data/other/machines/heavy/extra.json"), "5");
    write(&pack.join("data/other/machines/ignored.txt"), "not json");
    let function = pack.join("data/reload_test/function/keep.mcfunction");
    write(&function, "say old");
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["file/test"],"disabled":[]}"#,
    );
    let live = Arc::new(Mutex::new(0));
    let reject = Arc::new(AtomicBool::new(false));
    let mods = vec![ModEntry::new(
        "reload_test",
        Box::new(DataListener {
            live: live.clone(),
            reject: reject.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            install_mod_resources(game, &resources, &package);
            game.load_world(request(&world)).unwrap();
            assert_eq!(
                *live.lock().unwrap(),
                7,
                "override 2 plus other namespace 5"
            );
            assert!(game.run_command("datapack disable \"file/test\""));
            assert_eq!(
                *live.lock().unwrap(),
                1,
                "the mod's packaged file is the fallback"
            );
            assert!(game.run_command("datapack enable \"file/test\""));
            assert_eq!(*live.lock().unwrap(), 7);
            write(&data, "3");
            assert!(game.run_command("reload"));
            assert_eq!(*live.lock().unwrap(), 8);
            let old_function = game.fns.store.functions["reload_test:keep"].clone();
            let old_selection = std::fs::read(world.join("datapacks.json")).unwrap();
            let old_enabled = game.fns.enabled().to_vec();
            let old_vanilla_disabled = game.cmds.vanilla_disabled;
            write(&data, "4");
            write(&function, "say new");
            reject.store(true, Ordering::Relaxed);
            assert!(!game.run_command("reload"));
            assert_eq!(
                *live.lock().unwrap(),
                8,
                "earlier successful prepare must not publish"
            );
            assert_eq!(game.fns.store.functions["reload_test:keep"], old_function);
            let error = game
                .reload_datapacks(true, &game.level.enabled_features.clone())
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("reload listener reload_test:check: reload_test:value"),
                "{error}"
            );
            assert!(!game.run_command("datapack disable \"file/test\""));
            assert_eq!(
                std::fs::read(world.join("datapacks.json")).unwrap(),
                old_selection
            );
            assert_eq!(
                game.fns.enabled(),
                old_enabled,
                "the refused change publishes nothing"
            );
            assert_eq!(
                game.cmds.vanilla_disabled, old_vanilla_disabled,
                "the refused change flips no command switch"
            );
            assert!(game.fns.store.functions.contains_key("reload_test:keep"));
            assert_eq!(*live.lock().unwrap(), 8);
            reject.store(false, Ordering::Relaxed);
            write(&data, "not JSON");
            let error = game
                .reload_datapacks(true, &game.level.enabled_features.clone())
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("reload listener reload_test:data: reload_test:value"),
                "{error}"
            );
            assert_eq!(*live.lock().unwrap(), 8);
            assert_eq!(game.fns.store.functions["reload_test:keep"], old_function);
        })
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn server_reload_skips_unparsable_files_and_delivers_the_rest() {
    // Vanilla's `SimpleJsonResourceReloadListener.scanDirectory` logs and skips one bad
    // file; only a listener's own error aborts the reload. This listener follows it.
    struct ParsingListener(Arc<Mutex<Vec<(String, String)>>>);
    impl ServerReloadListener for ParsingListener {
        fn prepare(
            &self,
            context: &ServerReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            let mut parsed = Vec::new();
            for id in context.list() {
                match serde_json::from_slice::<String>(context.read(id).unwrap()) {
                    Ok(value) => parsed.push((id.to_string(), value)),
                    Err(error) => log::warn!("Couldn't parse data file '{id}': {error}"),
                }
            }
            let live = self.0.clone();
            Ok(<dyn ReloadApply>::new(move || {
                *live.lock().unwrap() = parsed
            }))
        }
    }
    struct SkipHooks(Arc<Mutex<Vec<(String, String)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for SkipHooks {
        fn add_server_reload_listeners(
            &self,
            _host: &dyn Host,
            event: &mut AddServerReloadListenersEvent<'_>,
        ) {
            event
                .listeners
                .add_listener(
                    Identifier::parse("reload_test:data").unwrap(),
                    FileToIdConverter::json("machines").unwrap(),
                    Arc::new(ParsingListener(self.0.clone())),
                )
                .unwrap();
        }
    }
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-skip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let manifest = "id = \"reload_test\"\nname = \"Reload Test\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let package = root.join("reload_test.hmod");
    write(
        &package,
        horizon_mod_package::Package::write_files(
            manifest,
            &[("data/reload_test/machines/bar.json", b"\"good\"")],
        )
        .unwrap(),
    );
    let package_manifest = horizon_mod_package::Package::read(&package)
        .unwrap()
        .manifest;
    let content = horizon_mod_package::Content::read(&package, &package_manifest).unwrap();
    let resources = root.join("resources");
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let resources = build_mod_resources(
        &base,
        Path::new(""),
        &[horizon_client::mod_content::ContentMod {
            id: "reload_test".into(),
            content,
            jars: Vec::new(),
        }],
        &resources,
        &horizon_mod_package::enabled::EnabledMods::new([(
            "reload_test".into(),
            mod_api::Side::Both,
        )]),
    );
    let world = root.join("world");
    let pack = world.join("datapacks/test");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"Reload fixture"}}"#,
    );
    let malformed = pack.join("data/reload_test/machines/foo.json");
    write(&malformed, "{bad json");
    write(
        &pack.join("data/reload_test/machines/bar.json"),
        "\"better\"",
    );
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["file/test"],"disabled":[]}"#,
    );
    let live = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "reload_test",
            Box::new(SkipHooks(live.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                install_mod_resources(game, &resources, &package);
                game.load_world(request(&world)).unwrap();
                assert_eq!(
                    *live.lock().unwrap(),
                    vec![("reload_test:bar".into(), "better".into())],
                    "the malformed file is skipped and the override wins"
                );
                write(&malformed, "\"fixed\"");
                assert!(game.run_command("reload"));
                assert_eq!(
                    *live.lock().unwrap(),
                    vec![
                        ("reload_test:bar".into(), "better".into()),
                        ("reload_test:foo".into(), "fixed".into()),
                    ],
                    "a repaired file joins the next reload"
                );
                write(&malformed, "still not json");
                assert!(
                    game.run_command("reload"),
                    "a skipped file never fails the reload"
                );
                assert_eq!(
                    *live.lock().unwrap(),
                    vec![("reload_test:bar".into(), "better".into())],
                    "the bad file is skipped again"
                );
            })
        },
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn server_reload_registration_failure_stops_world_open_and_quiet_mods_never_listen() {
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Broken;
    #[mod_api::__subscriptions]
    impl Hooks for Broken {
        fn add_server_reload_listeners(
            &self,
            _host: &dyn Host,
            event: &mut AddServerReloadListenersEvent<'_>,
        ) {
            let _ = event.listeners.add_dependency(
                Identifier::parse("mistyped:listener").unwrap(),
                VanillaServerListeners::functions(),
            );
        }
    }
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    mod_platform::testing::with_mods(vec![ModEntry::new("quiet", Box::new(Quiet))].into(), || {
        with_game(|game| {
            // A mod that doesn't implement the hook is never subscribed, so
            // world open and `/reload` skip the registration graph.
            assert!(!mod_api::__private::listened(
                mod_api::HookId::AddServerReloadListeners
            ));
            game.load_world(request(&root)).unwrap();
            assert!(!mod_api::__private::listened(
                mod_api::HookId::AddServerReloadListeners
            ));
            assert!(game.run_command("reload"));
            assert!(!mod_api::__private::listened(
                mod_api::HookId::AddServerReloadListeners
            ));
        })
    });
    std::fs::remove_dir_all(&root).unwrap();
    mod_platform::testing::with_mods(
        vec![ModEntry::new("broken_mod", Box::new(Broken))].into(),
        || {
            with_game(|game| {
                let error = game.load_world(request(&root)).unwrap_err().to_string();
                assert!(
                    error.contains("unknown reload listener mistyped:listener"),
                    "{error}"
                );
                assert!(game.fns.store.functions.is_empty());
            })
        },
    );
    std::fs::remove_dir_all(root).unwrap();
}

struct RetainedData {
    live: Arc<Mutex<u64>>,
    reject: Arc<AtomicBool>,
}
impl ServerReloadListener for RetainedData {
    fn prepare(
        &self,
        context: &ServerReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let mut value = 0;
        for id in context.list() {
            value += serde_json::from_slice::<u64>(context.read(id).unwrap())
                .map_err(|error| ReloadResourceError::new(id.clone(), error))?;
        }
        if self.reject.load(Ordering::Relaxed) {
            return Err(ReloadResourceError::new(
                Identifier::parse("reload_test:value").unwrap(),
                std::io::Error::other("rejected new machine data"),
            ));
        }
        let live = self.live.clone();
        Ok(<dyn ReloadApply>::new(move || {
            *live.lock().unwrap() = value
        }))
    }
}
#[mod_api::__subscriptions]
impl Hooks for RetainedData {
    fn add_server_reload_listeners(
        &self,
        _host: &dyn Host,
        event: &mut AddServerReloadListenersEvent<'_>,
    ) {
        let key = ListenerKey::<Self>::new(Identifier::parse("reload_test:kept").unwrap());
        event
            .listeners
            .add_retained_listener(
                &key,
                FileToIdConverter::json("machines").unwrap(),
                Arc::new(Self {
                    live: self.live.clone(),
                    reject: self.reject.clone(),
                }),
            )
            .unwrap();
    }
}

#[test]
#[ignore = "daily"]
fn server_reload_retains_listeners_for_other_hooks() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("server-reload-retained-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let manifest = "id = \"reload_test\"\nname = \"Reload Test\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let package = root.join("reload_test.hmod");
    write(
        &package,
        horizon_mod_package::Package::write_files(
            manifest,
            &[("data/reload_test/machines/value.json", b"1")],
        )
        .unwrap(),
    );
    let package_manifest = horizon_mod_package::Package::read(&package)
        .unwrap()
        .manifest;
    let content = horizon_mod_package::Content::read(&package, &package_manifest).unwrap();
    let resources = root.join("resources");
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let resources = build_mod_resources(
        &base,
        Path::new(""),
        &[horizon_client::mod_content::ContentMod {
            id: "reload_test".into(),
            content,
            jars: Vec::new(),
        }],
        &resources,
        &horizon_mod_package::enabled::EnabledMods::new([(
            "reload_test".into(),
            mod_api::Side::Both,
        )]),
    );
    let world = root.join("world");
    let pack = world.join("datapacks/test");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"Reload fixture"}}"#,
    );
    let data = pack.join("data/reload_test/machines/value.json");
    write(&data, "2");
    write(
        &world.join("datapacks.json"),
        r#"{"enabled":["file/test"],"disabled":[]}"#,
    );
    let live = Arc::new(Mutex::new(0));
    let reject = Arc::new(AtomicBool::new(false));
    let mods = vec![ModEntry::new(
        "reload_test",
        Box::new(RetainedData {
            live: live.clone(),
            reject: reject.clone(),
        }),
    )]
    .into();
    let key = ListenerKey::<RetainedData>::new(Identifier::parse("reload_test:kept").unwrap());
    let wrong_type =
        ListenerKey::<CheckListener>::new(Identifier::parse("reload_test:kept").unwrap());
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            assert!(game.reload_resources().listener(&key).is_none());
            install_mod_resources(game, &resources, &package);
            game.load_world(request(&world)).unwrap();
            let kept = game.reload_resources().listener(&key).unwrap();
            assert_eq!(*kept.live.lock().unwrap(), 2);
            assert!(Arc::ptr_eq(
                &kept,
                &game.reload_resources().listener(&key).unwrap()
            ));
            assert!(game.reload_resources().listener(&wrong_type).is_none());
            let before = Arc::as_ptr(&kept);
            write(&data, "3");
            assert!(game.run_command("reload"));
            let kept = game.reload_resources().listener(&key).unwrap();
            assert_eq!(*kept.live.lock().unwrap(), 3);
            assert_ne!(
                Arc::as_ptr(&kept),
                before,
                "each reload retains fresh objects"
            );
            write(&data, "4");
            reject.store(true, Ordering::Relaxed);
            assert!(!game.run_command("reload"));
            let kept = game.reload_resources().listener(&key).unwrap();
            assert_eq!(
                *kept.live.lock().unwrap(),
                3,
                "a failed reload keeps the last successful store"
            );
        })
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn server_completion_reads_published_data_without_world_access_on_first_open() {
    use mod_api::__private::{ModPhase, PhaseOwner};
    use mod_api::{HostSide, TagsUpdateCause, TagsUpdatedEvent};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    struct Probe(Arc<Mutex<Vec<TagsUpdateCause>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn tags_updated(&self, host: HostSide<'_>, event: &mut TagsUpdatedEvent) {
            let HostSide::Server(host) = host else {
                panic!("a data load is the server's");
            };
            assert!(!host.recipes_of_type("minecraft:smelting").is_empty());
            let refused =
                catch_unwind(AssertUnwindSafe(|| host.level("minecraft:overworld"))).unwrap_err();
            let text = refused
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| refused.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(text.contains("Reloading"), "{text}");
            self.0.lock().unwrap().push(event.cause);
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "published_server",
        Box::new(Probe(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.mod_phase = PhaseOwner::new(ModPhase::Setup);
            let root = std::env::current_dir().unwrap().join("published-world");
            game.load_world(request(&root)).unwrap();
            assert_eq!(mod_api::__private::current_phase(), ModPhase::Setup);
            game.mod_phase = PhaseOwner::new(ModPhase::World);
            game.native_tags_updated(TagsUpdateCause::ServerDataLoad { reload: true });
            assert_eq!(mod_api::__private::current_phase(), ModPhase::World);
            assert_eq!(
                *seen.lock().unwrap(),
                [
                    TagsUpdateCause::ServerDataLoad { reload: false },
                    TagsUpdateCause::ServerDataLoad { reload: true },
                ]
            );
        })
    });
}

/// `DataMapsUpdatedEvent`: once for each registry with a type on a world's
/// first open and after every `/reload`, with the reloaded values readable
/// through the handles: a newer file's override, a tag's members, a removal
/// and an entry with no value, typed.
#[test]
#[ignore = "daily"]
fn data_maps_updated_fires_for_each_registry_on_first_open_and_every_reload() {
    use mod_api::{
        BlockKey, DataMap, DataMapError, DataMapTypesEvent, DataMapsUpdateCause,
        DataMapsUpdatedEvent, DataPackFilesEvent, HostSide, ItemKey, ServerHost, Tag,
    };
    use std::sync::atomic::AtomicUsize;

    fn number(value: &Tag) -> Option<i64> {
        match value {
            Tag::Int(value) => Some(i64::from(*value)),
            Tag::Long(value) => Some(*value),
            _ => None,
        }
    }
    struct Fuel(&'static str);
    impl DataMap for Fuel {
        type Entry = ItemKey;
        type Value = i64;
        fn id(&self) -> &str {
            self.0
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    struct Hardness;
    impl DataMap for Hardness {
        type Entry = BlockKey;
        type Value = i64;
        fn id(&self) -> &str {
            "probe:hardness"
        }
        fn decode(&self, value: &Tag) -> Option<i64> {
            number(value)
        }
    }
    const FUEL: Fuel = Fuel("probe:fuel");
    /// What one load's listener read: coal, an oak log (through `#minecraft:logs`),
    /// stone (no entry), coal in a map no mod registered, and stone's hardness.
    type Reads = [Result<Option<i64>, DataMapError>; 5];

    struct Probe {
        loads: Arc<AtomicUsize>,
        seen: Arc<Mutex<Vec<(String, DataMapsUpdateCause)>>>,
        reads: Arc<Mutex<Vec<Reads>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn data_map_types(&self, _: &mut dyn ServerHost, event: &mut DataMapTypesEvent<'_>) {
            event.types.push(FUEL.ty());
            event.types.push(Hardness.ty());
        }
        fn data_pack_files(&self, _: &mut dyn ServerHost, event: &mut DataPackFilesEvent<'_>) {
            let load = self.loads.fetch_add(1, Ordering::SeqCst) + 1;
            let mut push = |key: &str, file: &str| {
                event.files.push((key.into(), file.as_bytes().to_vec()));
            };
            push(
                "data_maps/probe:item/fuel",
                r##"{"values":{"minecraft:coal":10,"#minecraft:logs":5}}"##,
            );
            push(
                "data_maps/probe:block/hardness",
                r#"{"values":{"minecraft:stone":3}}"#,
            );
            // The reload adds a newer file over the first: it overrides coal and
            // takes the logs away.
            if load > 1 {
                push(
                    "data_maps/probe:item/fuel",
                    r##"{"values":{"minecraft:coal":25},"remove":["#minecraft:logs"]}"##,
                );
            }
        }
        fn data_maps_updated(&self, host: HostSide<'_>, event: &mut DataMapsUpdatedEvent<'_>) {
            let HostSide::Server(host) = host else {
                panic!("the server's reload");
            };
            self.seen
                .lock()
                .unwrap()
                .push((event.registry.to_owned(), event.cause));
            if event.registry != "minecraft:item" {
                return;
            }
            let host: &dyn Host = &*host;
            let item = |id: &str| host.item_key(&Identifier::parse(id).unwrap()).unwrap();
            let stone = mod_api::__private::block_key("minecraft:stone").unwrap();
            self.reads.lock().unwrap().push([
                item("minecraft:coal").data(host, &FUEL),
                item("minecraft:oak_log").data(host, &FUEL),
                item("minecraft:stone").data(host, &FUEL),
                item("minecraft:coal").data(host, &Fuel("probe:unregistered")),
                stone.data(host, &Hardness),
            ]);
        }
    }
    let loads = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let reads = Arc::new(Mutex::new(Vec::new()));
    let probe = Probe {
        loads: loads.clone(),
        seen: seen.clone(),
        reads: reads.clone(),
    };
    mod_platform::testing::with_mods(vec![ModEntry::new("probe", Box::new(probe))].into(), || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let root =
                std::env::temp_dir().join(format!("data-maps-updated-{}", std::process::id()));
            game.load_world(request(&root)).unwrap();
            let both = |count: usize| -> Vec<(String, DataMapsUpdateCause)> {
                // In registry id order.
                [
                    "minecraft:block",
                    "minecraft:entity_type",
                    "minecraft:game_event",
                    "minecraft:item",
                    "minecraft:villager_profession",
                    "minecraft:worldgen/biome",
                ]
                .into_iter()
                .cycle()
                .take(count)
                .map(|registry| (registry.to_owned(), DataMapsUpdateCause::ServerReload))
                .collect()
            };
            let unknown = Err(DataMapError::UnknownType);
            assert_eq!(*seen.lock().unwrap(), both(6));
            assert_eq!(
                *reads.lock().unwrap(),
                [[Ok(Some(10)), Ok(Some(5)), Ok(None), unknown, Ok(Some(3))]]
            );
            assert!(game.run_command("reload"));
            assert_eq!(*seen.lock().unwrap(), both(12));
            assert_eq!(
                reads.lock().unwrap()[1],
                [Ok(Some(25)), Ok(None), Ok(None), unknown, Ok(Some(3))]
            );
            // A game that isn't a server holds no maps it may answer, even with
            // tables built.
            game.options.dedicated = None;
            assert_eq!(
                game.data_map_value("minecraft:item", "minecraft:coal", "probe:fuel"),
                Err(DataMapError::Unavailable)
            );
            let _ = std::fs::remove_dir_all(root);
        });
    });
}

#[path = "villager_secondary_reload_tests.rs"]
mod villager_secondary_reload_tests;

#[path = "datapack_publication_tests.rs"]
mod datapack_publication_tests;
