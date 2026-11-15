use super::*;
use mod_api::{
    ClientReloadContext, ClientReloadListener, ClientReloadListeners, FileToIdConverter,
    Identifier, ReloadApply, ReloadResourceError,
};
use std::{path::PathBuf, sync::Arc};

fn id(value: &str) -> Identifier {
    Identifier::parse(value).unwrap()
}
fn hats() -> FileToIdConverter {
    FileToIdConverter::json("hats").unwrap()
}

/// A listener recording the context it prepared and applying to a shared log.
struct HatListener {
    seen: std::sync::Mutex<Vec<String>>,
    applied: Arc<std::sync::Mutex<Vec<String>>>,
}
impl ClientReloadListener for HatListener {
    fn prepare(
        &self,
        context: &ClientReloadContext,
    ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
        let seen: Vec<String> = context
            .list()
            .filter_map(|id| {
                context
                    .read(id)
                    .map(|bytes| format!("{}={}", id, String::from_utf8_lossy(bytes)))
            })
            .collect();
        self.seen.lock().unwrap().extend(seen);
        let applied = self.applied.clone();
        Ok(<dyn ReloadApply>::new(move || {
            applied.lock().unwrap().push("hats".into());
        }))
    }
}

/// A temp root with one asset under `assets/test_mod/hats/`; each test gets
/// its own, so parallel runs write to different folders.
fn root_with_asset(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-client-reload-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let asset = dir.join("assets/test_mod/hats/top.json");
    std::fs::create_dir_all(asset.parent().unwrap()).unwrap();
    std::fs::write(asset, bytes).unwrap();
    dir
}

#[test]
fn stage_reads_resolved_asset_bytes_without_a_directory() {
    use horizon_assets::resource_source::{Resource, ResourceSource};
    let source = ResourceSource::indexed(std::collections::BTreeMap::from([(
        "assets/test_mod/hats/top.json".into(),
        Resource::bytes(b"selected".to_vec()),
    )]));
    let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: applied.clone(),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:hats"), hats(), listener.clone())
        .unwrap();
    let prepared =
        stage_client_reload(&source, listeners, mod_api::__private::ModPhase::World).unwrap();
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top=selected"]
    );
    assert!(applied.lock().unwrap().is_empty());
    apply_client_reload(
        &mod_api::__private::PhaseOwner::new(mod_api::__private::ModPhase::World),
        prepared,
    );
    assert_eq!(applied.lock().unwrap().as_slice(), ["hats"]);
}

#[test]
fn stage_reads_the_merged_assets_and_apply_publishes_after_all_prepares() {
    let root = root_with_asset("merged", b"\"top\"");
    let applied = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: applied.clone(),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:hats"), hats(), listener.clone())
        .unwrap();
    let prepared = stage_client_reload(
        &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        listeners,
        mod_api::__private::ModPhase::World,
    )
    .unwrap();
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top=\"top\""]
    );
    assert!(applied.lock().unwrap().is_empty(), "nothing published yet");
    apply_client_reload(
        &mod_api::__private::PhaseOwner::new(mod_api::__private::ModPhase::World),
        prepared,
    );
    assert_eq!(applied.lock().unwrap().as_slice(), ["hats"]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_listener_error_keeps_every_previous_state() {
    struct Failing;
    #[derive(Debug)]
    struct NotAHat;
    impl std::fmt::Display for NotAHat {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "not a hat")
        }
    }
    impl std::error::Error for NotAHat {}
    impl ClientReloadListener for Failing {
        fn prepare(
            &self,
            _: &ClientReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            Err(ReloadResourceError::new(id("test_mod:broken"), NotAHat))
        }
    }
    let root = root_with_asset("failing", b"{}");
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:failing"), hats(), Arc::new(Failing))
        .unwrap();
    let Err(error) = stage_client_reload(
        &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        listeners,
        mod_api::__private::ModPhase::World,
    ) else {
        panic!("a failing listener stops the whole reload")
    };
    assert_eq!(
        error.to_string(),
        "reload listener test_mod:failing: test_mod:broken: not a hat"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_converted_file_outside_the_pack_is_absent_not_empty() {
    let root = root_with_asset("absent", b"{}");
    let listener = Arc::new(HatListener {
        seen: std::sync::Mutex::new(Vec::new()),
        applied: Arc::new(std::sync::Mutex::new(Vec::new())),
    });
    let mut listeners = <mod_api::ClientListenerRegistry as mod_api::__private::game::ClientReloadListenersOrder>::new();
    listeners
        .add_listener(id("test_mod:hats"), hats(), listener.clone())
        .unwrap();
    stage_client_reload(
        &horizon_assets::resource_source::ResourceSource::directory(root.to_path_buf()),
        listeners,
        mod_api::__private::ModPhase::World,
    )
    .unwrap();
    assert_eq!(
        listener.seen.lock().unwrap().as_slice(),
        ["test_mod:top={}"],
        "the staged root holds only the merged asset"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// The initial load runs the client reload listeners: the load itself
/// registers and stages them against the real merged root (here the direct
/// `--world` path; the title screen's loader thread runs the same code), and
/// the applies it delivers run once the game exists — before any F3+T
/// reload.
#[test]
#[ignore = "daily"]
fn initial_load_stages_and_applies_client_reload_listeners() {
    use crate::game::{Progress, load_resources, native_entity_refs::tests::with_game};
    use mod_api::{self, ModEntry};
    use mod_api::{AddClientReloadListenersEvent, ClientHost, Hooks};
    use mod_platform::testing::with_mods;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// Counts the real root's own `post_effect` assets, like a mod reading
    /// vanilla files on the first load.
    struct StartupProbe {
        applied: Arc<AtomicBool>,
        count: Arc<AtomicUsize>,
    }
    impl ClientReloadListener for StartupProbe {
        fn prepare(
            &self,
            context: &ClientReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            let seen = context
                .list()
                .filter(|id| id.namespace() == "minecraft")
                .count();
            let applied = self.applied.clone();
            let count = self.count.clone();
            Ok(<dyn ReloadApply>::new(move || {
                count.fetch_max(seen, Ordering::SeqCst);
                applied.store(true, Ordering::SeqCst);
            }))
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for StartupProbe {
        fn add_client_reload_listeners(
            &self,
            _host: &dyn ClientHost,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            event
                .listeners
                .add_listener(
                    id("client_test:startup"),
                    FileToIdConverter::json("post_effect").unwrap(),
                    Arc::new(StartupProbe {
                        applied: self.applied.clone(),
                        count: self.count.clone(),
                    }),
                )
                .unwrap();
        }
    }

    let applied = Arc::new(AtomicBool::new(false));
    let count = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "client_test",
        Box::new(StartupProbe {
            applied: applied.clone(),
            count: count.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        with_game(|game| {
            let root = crate::game::packs::startup_root(
                &game.options.resources,
                &game.settings.resource_packs,
                &[],
                &game.options.mods,
            )
            .or_else(|error| {
                log::warn!("Horizon: resource packs not applied: {error}");
                crate::game::mods::content_root(&game.options.resources, &game.options.mods)
            })
            .unwrap();
            let mut loaded = load_resources(
                root,
                Vec::new(),
                None,
                &game.options.resources,
                &game.settings,
                std::time::Instant::now(),
                false,
                crate::game::execution_role::ExecutionRole::Client,
                &Progress::new(),
                &mut |_, _| {},
            )
            .expect("initial load");
            assert!(
                !loaded.client_applies.is_empty(),
                "the load itself registered and staged the listener"
            );
            assert!(
                !applied.load(Ordering::SeqCst),
                "nothing applied before handoff"
            );
            // The same call `start_game` makes once the game owns the assets.
            apply_client_reload(&game.mod_phase, std::mem::take(&mut loaded.client_applies));
            assert!(applied.load(Ordering::SeqCst), "the first load applies");
            assert!(
                count.load(Ordering::SeqCst) >= 5,
                "the initial stage read the real root's own assets"
            );
        });
    });
}

/// A listener registered through the real dispatch, staged against a real
/// merged root (the game's own resources with a dev mod and a resource pack).
#[test]
#[ignore = "daily"]
fn client_reload_reads_real_assets_with_pack_overrides() {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{self, ModEntry};
    use mod_api::{AddClientReloadListenersEvent, ClientHost, Hooks};
    use mod_platform::testing::with_mods;
    use std::{
        collections::BTreeSet,
        sync::Mutex,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    let dir = std::env::temp_dir().join(format!("client-reload-real-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mod_dir = dir.join("mod");
    let pack_dir = dir.join("packs/override");
    let asset = "assets/client_test/hats/top.json";
    for root in [&mod_dir, &pack_dir] {
        std::fs::create_dir_all(root.join(asset).parent().unwrap()).unwrap();
    }
    std::fs::write(
        mod_dir.join("horizon-mod.toml"),
        "id = \"client_test\"\nname = \"Client test\"\nversion = \"1\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
    )
    .unwrap();
    std::fs::write(mod_dir.join(asset), b"mod").unwrap();
    std::fs::write(pack_dir.join(asset), b"pack").unwrap();
    std::fs::write(
        pack_dir.join("pack.mcmeta"),
        br#"{"pack":{"description":"test","pack_format":97}}"#,
    )
    .unwrap();

    // Only the archive remains when loading starts.
    {
        use std::io::Write;
        let path = dir.join("packs/override.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in ["pack.mcmeta", asset] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&std::fs::read(pack_dir.join(name)).unwrap())
                .unwrap();
        }
        zip.finish().unwrap();
        std::fs::remove_dir_all(&pack_dir).unwrap();
    }

    struct HatListener {
        live: Arc<Mutex<Vec<String>>>,
        reject: Arc<AtomicBool>,
        vanilla: Arc<AtomicUsize>,
    }
    impl ClientReloadListener for HatListener {
        fn prepare(
            &self,
            context: &ClientReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            if self.reject.load(Ordering::Relaxed) {
                return Err(ReloadResourceError::new(
                    id("client_test:top"),
                    std::io::Error::other("rejected new hats"),
                ));
            }
            let top = id("client_test:top");
            let value = context
                .read(&top)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            let live = self.live.clone();
            Ok(<dyn ReloadApply>::new(move || {
                live.lock().unwrap().push(value)
            }))
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for HatListener {
        fn add_client_reload_listeners(
            &self,
            _host: &dyn ClientHost,
            event: &mut AddClientReloadListenersEvent<'_>,
        ) {
            event
                .listeners
                .add_listener(
                    id("client_test:hats"),
                    hats(),
                    Arc::new(HatListener {
                        live: self.live.clone(),
                        reject: self.reject.clone(),
                        vanilla: self.vanilla.clone(),
                    }),
                )
                .unwrap();
            // The real root's own assets, through a directory vanilla ships.
            event
                .listeners
                .add_listener(
                    id("client_test:vanilla"),
                    FileToIdConverter::json("post_effect").unwrap(),
                    Arc::new(VanillaProbe(self.vanilla.clone())),
                )
                .unwrap();
        }
    }

    /// Counts vanilla's own `post_effect` files in the same merged view.
    struct VanillaProbe(Arc<AtomicUsize>);
    impl ClientReloadListener for VanillaProbe {
        fn prepare(
            &self,
            context: &ClientReloadContext,
        ) -> Result<Box<dyn ReloadApply>, ReloadResourceError> {
            let count = context
                .list()
                .filter(|id| id.namespace() == "minecraft")
                .count();
            let seen = self.0.clone();
            Ok(<dyn ReloadApply>::new(move || {
                seen.fetch_max(count, Ordering::SeqCst);
            }))
        }
    }

    let live = Arc::new(Mutex::new(Vec::new()));
    let reject = Arc::new(AtomicBool::new(false));
    let vanilla = Arc::new(AtomicUsize::new(0));
    let mods = vec![ModEntry::new(
        "client_test",
        Box::new(HatListener {
            live: live.clone(),
            reject: reject.clone(),
            vanilla: vanilla.clone(),
        }),
    )]
    .into();
    with_mods(mods, || {
        with_game(|game| {
            let resources = game.options.resources.clone();
            let enabled = BTreeSet::new();
            let mut sources = crate::game::mods::ModsState {
                dev: vec![mod_dir.clone()],
                ..Default::default()
            };
            sources.select(&std::env::current_dir().unwrap(), &enabled);
            // The merged root without `startup_root`'s process-wide mod
            // registrations, which a test must not freeze for the others
            // (`brewing`'s mod effects among them).
            let merged = |packs: &[horizon_client::packs::Pack]| {
                let base = crate::game::mods::content_overlay(
                    &resources,
                    &sources,
                    &horizon_client::logging::game_home().join("jars"),
                )
                .unwrap()
                .root;
                horizon_client::packs::resources(base.source().clone(), packs).unwrap()
            };
            // The mod's packaged asset over nothing, with the real root's own
            // assets in the same view.
            let root = merged(&[]);
            let prepared = stage_client_reload(
                &root,
                game.register_client_reload_listeners(),
                mod_api::__private::ModPhase::World,
            )
            .unwrap();
            assert!(live.lock().unwrap().is_empty(), "nothing published yet");
            apply_client_reload(
                &mod_api::__private::PhaseOwner::new(mod_api::__private::ModPhase::World),
                prepared,
            );
            assert_eq!(live.lock().unwrap().as_slice(), ["mod"]);
            assert!(
                vanilla.load(Ordering::SeqCst) >= 5,
                "the real root's own post_effect assets join the view"
            );
            // A selected resource pack overrides the mod's packaged asset.
            let packs = horizon_client::packs::folder(
                &dir.join("packs"),
                horizon_client::packs::RESOURCE_FORMAT,
            );
            assert_eq!(packs.len(), 1);
            let root = merged(&packs);
            let prepared = stage_client_reload(
                &root,
                game.register_client_reload_listeners(),
                mod_api::__private::ModPhase::World,
            )
            .unwrap();
            apply_client_reload(
                &mod_api::__private::PhaseOwner::new(mod_api::__private::ModPhase::World),
                prepared,
            );
            assert_eq!(live.lock().unwrap().as_slice(), ["mod", "pack"]);
            // A rejected prepare aborts the reload and keeps the previous state.
            reject.store(true, Ordering::Relaxed);
            let root = merged(&[]);
            let Err(error) = stage_client_reload(
                &root,
                game.register_client_reload_listeners(),
                mod_api::__private::ModPhase::World,
            ) else {
                panic!("a failing listener stops the whole reload")
            };
            assert!(error.to_string().contains("client_test:hats"));
            assert_eq!(
                live.lock().unwrap().as_slice(),
                ["mod", "pack"],
                "the refused reload publishes nothing"
            );
        });
    });
    std::fs::remove_dir_all(dir).unwrap();
}

/// `client_setup` reaches every listener with the game's client host, one call per
/// firing; `run` fires it once per process, so a listener sees one start.
#[test]
#[ignore = "daily"]
fn client_setup_reaches_each_listener_once_per_firing() {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{ClientHost, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Setup(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for Setup {
        fn client_setup(&self, _host: &mut dyn ClientHost, _event: &mut mod_api::ClientSetupEvent) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let (first, second) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let mods = vec![
        ModEntry::new("setup_a", Box::new(Setup(first.clone()))),
        ModEntry::new("setup_b", Box::new(Setup(second.clone()))),
    ]
    .into();
    with_mods(mods, || {
        with_game(|game| {
            game.native_client_setup();
        });
    });
    assert_eq!(
        (first.load(Ordering::SeqCst), second.load(Ordering::SeqCst)),
        (1, 1)
    );
}

/// The process's client start is a client that is not a splitscreen guest: a dedicated
/// server has no client, and a guest's `run` is another local player of the same process.
#[test]
#[ignore = "daily"]
fn only_the_first_local_client_is_the_client_start() {
    use crate::game::execution_role::ExecutionRole;
    use crate::game::native_entity_refs::tests::with_game;

    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.options.split_guest = false;
        assert!(game.options.is_client_start());
        game.options.split_guest = true;
        assert!(!game.options.is_client_start());
        game.options.split_guest = false;
        game.options.execution_role = ExecutionRole::DedicatedServer;
        assert!(!game.options.is_client_start());
    });
}

/// A world's moments reach a listener once each and in order: the server's start
/// (`server_about_to_start`, then `server_started` where `WorldOpenReady` was, only once
/// per open and not for a failed one), its stop (`server_stopping`, then the final close),
/// and the client's own close (`client_stopping`).
#[test]
#[ignore = "daily"]
fn the_lifecycle_moments_fire_once_each_in_order() {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{ClientHost, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::Mutex;

    struct Log(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn server_about_to_start(
            &self,
            _host: &mut dyn ServerHost,
            e: &mut mod_api::ServerAboutToStartEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("about_to_start dedicated={}", e.dedicated));
        }
        fn server_starting(
            &self,
            _host: &mut dyn ServerHost,
            e: &mut mod_api::ServerStartingEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("starting dedicated={}", e.dedicated));
        }
        fn server_stopped(&self, _host: &mut dyn ServerHost, e: &mut mod_api::ServerStoppedEvent) {
            self.0
                .lock()
                .unwrap()
                .push(format!("stopped dedicated={}", e.dedicated));
        }
        fn server_started(&self, _host: &mut dyn ServerHost, e: &mut mod_api::ServerStartedEvent) {
            self.0
                .lock()
                .unwrap()
                .push(format!("started dedicated={}", e.dedicated));
        }
        fn server_stopping(
            &self,
            _host: &mut dyn ServerHost,
            e: &mut mod_api::ServerStoppingEvent,
        ) {
            self.0
                .lock()
                .unwrap()
                .push(format!("stopping dedicated={}", e.dedicated));
        }
        fn world_lifecycle(
            &self,
            _host: &mut dyn ServerHost,
            event: &mut mod_api::WorldLifecycleEvent<'_>,
        ) {
            self.0.lock().unwrap().push(format!("{:?}", event.phase));
        }
        fn client_stopping(
            &self,
            _host: &mut dyn ClientHost,
            _event: &mut mod_api::ClientStoppingEvent,
        ) {
            self.0.lock().unwrap().push("client_stopping".into());
        }
        fn client_stopped(
            &self,
            _host: &mut dyn ClientHost,
            _event: &mut mod_api::ClientStoppedEvent,
        ) {
            self.0.lock().unwrap().push("client_stopped".into());
        }
        fn game_shutting_down(
            &self,
            host: mod_api::HostSide<'_>,
            _event: &mut mod_api::GameShuttingDownEvent,
        ) {
            let side = match host {
                mod_api::HostSide::Client(_) => "client",
                mod_api::HostSide::Server(_) => "server",
            };
            self.0.lock().unwrap().push(format!("shutting_down {side}"));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("log", Box::new(Log(log.clone())))].into();
    with_mods(mods, || {
        with_game(|game| {
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::DedicatedServer;
            game.native_lifecycle_world_closed();
            log.lock().unwrap().clear();
            game.native_server_about_to_start(true);
            game.native_server_starting(true);
            game.native_lifecycle_open_started();
            game.native_lifecycle_world_ready();
            game.native_lifecycle_world_ready();
            game.native_lifecycle.begin_close();
            game.native_server_stopping(true);
            game.native_lifecycle_world_closed();
            game.native_lifecycle_world_closed();
            game.native_server_stopped(true);
            game.native_game_shutting_down();
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            game.native_client_stopping();
            game.native_client_stopped();
            game.native_game_shutting_down();
        });
    });
    assert_eq!(
        *log.lock().unwrap(),
        [
            "about_to_start dedicated=true",
            "starting dedicated=true",
            "started dedicated=true",
            "stopping dedicated=true",
            "WorldCloseFinal",
            "stopped dedicated=true",
            "shutting_down server",
            "client_stopping",
            "client_stopped",
            "shutting_down client",
        ]
    );
}

/// `get_arm_pose`'s answer is the pose of that hand's stack; a hand it does not answer for,
/// or no listener at all, leaves the item's own (declared) pose, which a vanilla stick lacks.
#[test]
#[ignore = "daily"]
fn an_arm_pose_answer_replaces_the_pose_the_item_gives() {
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{ArmPose, ClientHost, GetArmPoseEvent, Hand, Hooks, ModEntry};
    use mod_platform::testing::with_mods;

    struct Horn;
    #[mod_api::__subscriptions]
    impl Hooks for Horn {
        fn get_arm_pose(
            &self,
            _: &dyn ClientHost,
            event: &mut GetArmPoseEvent<'_>,
        ) -> Option<ArmPose> {
            (event.hand == Hand::Main).then_some(ArmPose::TootHorn)
        }
    }

    let stick = horizon_client::items::ItemStack::new("minecraft:stick", 1);
    with_game(|game| {
        let mut m = horizon_client::mobs::Mob::new(
            1,
            game.res.player_type().clone().unwrap(),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        );
        for slot in [
            horizon_items::equipment::MAINHAND,
            horizon_items::equipment::OFFHAND,
        ] {
            m.extra.set_slot_stack(slot, Some(stick.clone()));
        }
        assert_eq!(game.held_arm_poses(1, &m), [None, None]);
        let mods = vec![ModEntry::new("horn", Box::new(Horn))].into();
        with_mods(mods, || {
            assert_eq!(
                game.held_arm_poses(1, &m),
                [Some(horizon_core::arm_pose::ArmPose::TootHorn), None]
            );
        });
    });
}

#[test]
fn resolved_ui_resources_do_not_restore_a_filtered_panorama_face() {
    use horizon_assets::resource_source::{Resource, ResourceSource};
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let scratch = Scratch(std::env::temp_dir().join(format!(
            "horizon-ui-filter-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
    let folder = scratch
        .0
        .join("assets/minecraft/textures/gui/title/background");
    std::fs::create_dir_all(&folder).unwrap();
    let base = ResourceSource::directory(scratch.0.clone());
    let mut files = std::collections::BTreeMap::new();
    for face in 0..6 {
        let name = format!("assets/minecraft/textures/gui/title/background/panorama_{face}.png");
        let mut png =
            png::Encoder::new(std::fs::File::create(scratch.0.join(&name)).unwrap(), 1, 1);
        png.set_color(png::ColorType::Rgba);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .unwrap()
            .write_image_data(&[200, 30, 40, 255])
            .unwrap();
        // The resolved selection hides face 1 even though the base directory has it.
        if face != 1 {
            files.insert(name.clone(), Resource::file(base.clone(), name));
        }
    }
    assert!(base.contains("assets/minecraft/textures/gui/title/background/panorama_1.png"));
    let gui = crate::game::UiResources::load(&ResourceSource::indexed(files)).unwrap();
    assert_eq!(gui.panorama.texels(), [96, 110, 111, 255].repeat(6));
}

#[test]
#[ignore = "daily"]
fn refused_client_context_keeps_pack_and_listeners() {
    use crate::game::{Progress, native_entity_refs::tests::with_client_game, packs::stage_pack};
    use mod_api::__private::reload::PreparedStep;
    use std::sync::atomic::{AtomicBool, Ordering};

    with_client_game(|game| {
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
        let root =
            std::env::temp_dir().join(format!("client-reload-refusal-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(horizon_mod_package::enabled::MARKER),
            b"invalid json",
        )
        .unwrap();
        staged.root = root.clone();
        let applied = Arc::new(AtomicBool::new(false));
        let flag = applied.clone();
        staged.client_applies.push(PreparedStep::Mod {
            apply: <dyn ReloadApply>::new(move || {
                flag.store(true, Ordering::SeqCst);
            }),
            owner: None,
            id: id("test:listener"),
        });
        let old_root = game.options.resources.clone();
        let old_pack = game.pack.source.clone();
        let old_context = game.loop_context.clone();
        assert!(game.apply_staged_pack(staged).is_err());
        assert_eq!(game.options.resources, old_root);
        assert!(game.pack.source.same_generation(&old_pack));
        assert!(Arc::ptr_eq(&game.loop_context, &old_context));
        assert!(!applied.load(Ordering::SeqCst));
        std::fs::remove_dir_all(root).unwrap();
    });
}

#[test]
#[ignore = "daily"]
fn client_completion_reads_published_resources_without_world_access() {
    use crate::game::native_entity_refs::tests::with_client_game;
    use mod_api::__private::{ModPhase, PhaseOwner};
    use mod_api::{ClientHost, ClientResourcesReloadedEvent, Hooks, ModEntry};
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Mutex,
    };

    struct Probe(Arc<Mutex<Vec<u64>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn client_resources_reloaded(
            &self,
            host: &mut dyn ClientHost,
            _: &mut ClientResourcesReloadedEvent,
        ) {
            assert!(
                host.block_sprites(&[host.block_state_id("minecraft:stone").unwrap()])[0].is_some()
            );
            assert!(host.texture_sprites(&["minecraft:block/stone"])[0].is_some());
            let version = host.block_sprites_version();
            let refused =
                catch_unwind(AssertUnwindSafe(|| host.level("minecraft:overworld"))).unwrap_err();
            let text = refused
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| refused.downcast_ref::<&str>().copied())
                .unwrap();
            assert!(text.contains("Reloading"), "{text}");
            self.0.lock().unwrap().push(version);
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "published_client",
        Box::new(Probe(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let version = game.block_sprites_version();
            for entering in [ModPhase::Setup, ModPhase::World] {
                game.mod_phase = PhaseOwner::new(entering);
                game.native_client_resources_reloaded();
                assert_eq!(mod_api::__private::current_phase(), entering);
            }
            assert_eq!(*seen.lock().unwrap(), [version, version]);
        })
    });
}

#[test]
#[ignore = "daily"]
fn client_startup_finishes_configs_before_the_first_runtime_callback() {
    // Startup holds two Game values, which exceed libtest's default thread stack.
    std::thread::Builder::new()
        .name("client-startup-fixture".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            use crate::game::{
                Audio, UiResources, native_entity_refs::tests::with_client_game, start_game,
            };
            use mod_api::__private::{ModPhase, PhaseOwner};
            use mod_api::{
                ClientHost, ClientSetupEvent, ClientTickPreEvent, ConfigLoadingEvent, Hooks,
                HostSide, ModEntry, RegisterConfigsEvent, ScreenOpening, ScreenOpeningEvent,
            };
            use std::{sync::Mutex, time::Instant};

            struct Probe(Arc<Mutex<Vec<String>>>);
            #[mod_api::__subscriptions]
            impl Hooks for Probe {
                fn register_configs(&self, _: HostSide<'_>, event: &mut RegisterConfigsEvent<'_>) {
                    event.configs.push(mod_api::Config::new(
                        "startup_phase",
                        mod_api::ConfigKind::Client,
                        "enabled = true\n",
                    ));
                    event.configs.push(mod_api::Config::new(
                        "startup_phase",
                        mod_api::ConfigKind::Common,
                        "enabled = true\n",
                    ));
                }
                fn client_setup(&self, _: &mut dyn ClientHost, _: &mut ClientSetupEvent) {
                    assert_eq!(mod_api::__private::current_phase(), ModPhase::Setup);
                    self.0.lock().unwrap().push("setup".into());
                }
                fn config_loading(&self, _: HostSide<'_>, event: &mut ConfigLoadingEvent<'_>) {
                    assert_eq!(mod_api::__private::current_phase(), ModPhase::Config);
                    self.0
                        .lock()
                        .unwrap()
                        .push(format!("{:?}", event.config.kind));
                }
                fn screen_opening(
                    &self,
                    host: &dyn ClientHost,
                    _: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    assert!(host.level("minecraft:overworld").is_none());
                    assert_eq!(mod_api::__private::current_phase(), ModPhase::World);
                    self.0.lock().unwrap().push("screen".into());
                    None
                }
                fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
                    assert!(host.level("minecraft:overworld").is_none());
                    assert!(host.entity(1).is_none());
                    assert_eq!(mod_api::__private::current_phase(), ModPhase::World);
                    self.0.lock().unwrap().push("tick".into());
                }
            }
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "startup_phase",
                Box::new(Probe(seen.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                for pack_error in [None, Some("selected resource pack failed")] {
                    seen.lock().unwrap().clear();
                    with_client_game(|game| {
                        let resources = game.res.clone();
                        let options = game.options.clone();
                        let root = options.resources.clone();
                        let gui = UiResources::server(
                            &horizon_assets::resource_source::ResourceSource::directory(
                                root.clone(),
                            ),
                        )
                        .unwrap();
                        let _setup = PhaseOwner::scope(ModPhase::Setup);
                        let mut started = start_game(
                            &resources,
                            game.pack.clone(),
                            gui,
                            options,
                            pack_error.map(str::to_owned),
                            Audio::silent(
                                &horizon_assets::resource_source::ResourceSource::directory(
                                    root.clone(),
                                ),
                            ),
                            root,
                            Instant::now(),
                            None,
                            Some(game.settings.clone()),
                            None,
                            Vec::new(),
                            None,
                        )
                        .unwrap()
                        .unwrap();
                        // The event loop dispatches this before any per-tick work,
                        // even on the title screen, without a world or renderer.
                        started.native_client_tick_pre();
                        assert_eq!(started.ui.options.resource_error.as_deref(), pack_error);
                        let mut expected = vec!["setup", "Client", "Common", "screen"];
                        if pack_error.is_some() {
                            assert!(started.ui.options.resource_packs_open());
                            expected.push("screen");
                        }
                        expected.push("tick");
                        assert_eq!(*seen.lock().unwrap(), expected);
                    })
                }
            });
        })
        .unwrap()
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
}

/// The existing item hook sees borrowed mob extraction data, and its answer reaches
/// the animated model. Other entities, both player paths, and a later no-listener
/// extraction retain their normal poses.
#[test]
#[ignore = "daily"]
fn humanoid_arm_pose_mod_reads_mob_state_and_changes_the_model() {
    use crate::game::native_entity_refs::tests::with_client_game;
    use horizon_client::{
        items::ItemStack,
        mobs::{
            Mob,
            anim::{self, Poses},
        },
        motion::PlayerMotion,
    };
    use horizon_core::{geometry::Vec3, use_animation::UseAnimation};
    use horizon_items::equipment::{MAINHAND, OFFHAND};
    use mod_api::{ArmPose, ClientHost, GetArmPoseEvent, Hand, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    struct MobPose(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for MobPose {
        fn get_arm_pose(
            &self,
            _: &dyn ClientHost,
            event: &mut GetArmPoseEvent<'_>,
        ) -> Option<ArmPose> {
            assert_ne!(
                event.entity_id, 62,
                "a quadruped does not extract humanoid poses"
            );
            if event.entity_id == 7 {
                assert_eq!(event.humanoid.off_hand.unwrap().damage(), 9);
                assert_eq!(event.humanoid.off_hand.unwrap().count(), 3);
                assert_eq!(event.humanoid.main_arm_pose, ArmPose::Item);
                assert_eq!(event.humanoid.off_arm_pose, ArmPose::Item);
            }
            if event.entity_id != 60 {
                return None;
            }
            let state = event.humanoid;
            assert!(state.main_arm_left);
            assert_eq!(state.use_hand, Some(Hand::Off));
            assert_eq!(state.ticks_using_item, 12);
            assert_eq!(state.use_animation, UseAnimation::Brush);
            assert_eq!(state.main_arm_pose, ArmPose::Empty);
            assert_eq!(state.off_arm_pose, ArmPose::Empty);
            let main = state.main_hand.unwrap();
            assert_eq!(
                (main.item(), main.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 2)
            );
            let off = state.off_hand.unwrap();
            assert_eq!(
                (off.item(), off.count(), off.damage()),
                (
                    mod_api::__private::item_key("minecraft:brush").unwrap(),
                    3,
                    9
                )
            );
            assert!(std::ptr::eq(
                event.stack,
                if event.hand == Hand::Main { main } else { off }
            ));
            self.0.fetch_add(1, Ordering::Relaxed);
            (event.hand == Hand::Main).then_some(ArmPose::ThrowTrident)
        }
    }
    fn arms(m: &Mob) -> [[f32; 9]; 2] {
        let mut poses = Poses::new(&m.kind.model);
        anim::setup(m, &anim::state(m, 0.0), &mut poses, &HashMap::new());
        [*poses.get("right_arm"), *poses.get("left_arm")]
    }
    with_client_game(|game| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        game.player.as_mut().unwrap().entity_id = 7;
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:stick", 1));
        game.offhand = Some(ItemStack::new("minecraft:brush", 3));
        game.offhand.as_mut().unwrap().damage = 9;
        game.tick_player_model();
        let own_before =
            anim::humanoid_arm_poses(game.player_model.as_ref().unwrap(), [None; 2], 0.0);
        assert_eq!(own_before, [horizon_core::arm_pose::ArmPose::Item; 2]);
        for (id, kind) in [
            (60, "minecraft:zombie"),
            (61, "minecraft:skeleton"),
            (62, "minecraft:pig"),
        ] {
            let mut m = Mob::new(
                id,
                game.res.mob_types().get(kind).unwrap().clone(),
                Vec3::ZERO,
                0.0,
            );
            m.extra
                .set_slot_stack(MAINHAND, Some(ItemStack::new("minecraft:stick", 2)));
            m.extra
                .set_slot_stack(OFFHAND, Some(ItemStack::new("minecraft:brush", 3)));
            game.mobs.push(m);
        }
        let mut player = Mob::new(8, game.res.player_type().clone().unwrap(), Vec3::ZERO, 0.0);
        player
            .extra
            .set_slot_stack(MAINHAND, Some(ItemStack::new("minecraft:stick", 1)));
        game.mobs.push(player);
        let m = &mut game.mobs[0];
        let mut brush = ItemStack::new("minecraft:brush", 3);
        brush.damage = 9;
        m.extra.set_slot_stack(OFFHAND, Some(brush));
        m.extra.main_hand_left = true;
        m.extra.using_item = 12;
        m.extra.using_off_hand = true;
        let baseline: Vec<_> = game.mobs.iter().map(arms).collect();
        game.tick_remote_arm_poses();
        assert_eq!(game.mobs.iter().map(arms).collect::<Vec<_>>(), baseline);
        let calls = Arc::new(AtomicUsize::new(0));
        with_mods(
            vec![ModEntry::new("mob-pose", Box::new(MobPose(calls.clone())))].into(),
            || {
                game.tick_remote_arm_poses();
                assert_eq!(
                    game.mobs[0].extra.held_arm_pose,
                    [Some(horizon_core::arm_pose::ArmPose::ThrowTrident), None]
                );
                let posed = arms(&game.mobs[0]);
                assert!((posed[1][3] + std::f32::consts::PI).abs() < 1e-6);
                assert_ne!(posed, baseline[0]);
                for (m, expected) in game.mobs[1..].iter().zip(&baseline[1..]) {
                    assert_eq!(arms(m), *expected);
                }
                let empty = Mob::new(60, game.mobs[0].kind.clone(), Vec3::ZERO, 0.0);
                assert_eq!(game.held_arm_poses(60, &empty), [None; 2]);
                // The local player's extraction also exposes its actual off-hand stack.
                game.tick_player_model();
                let own = game.player_model.as_ref().unwrap();
                assert_eq!(
                    anim::humanoid_arm_poses(own, own.extra.held_arm_pose, 0.0),
                    own_before
                );
            },
        );
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        game.tick_remote_arm_poses();
        assert_eq!(game.mobs.iter().map(arms).collect::<Vec<_>>(), baseline);
    });
}

#[path = "custom_arm_pose_tests.rs"]
mod custom_arm_pose_tests;
