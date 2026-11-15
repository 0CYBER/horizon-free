use crate::game::{
    execution_role::ExecutionRole, native_entity_refs::tests::with_game,
    native_guest_turn_tests::with_guests,
};
use mod_api::{Hooks, ModEntry, ServerHost};
use mod_platform::testing::with_mods;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

struct PanicAt {
    stopping: AtomicBool,
    save: AtomicBool,
    config_unloading: AtomicBool,
    logout: AtomicBool,
}

struct StoppedProbe {
    serving_stopped: Box<dyn Fn() -> bool + Send + Sync>,
    lan: std::sync::Weak<horizon_client::lan::Shared>,
    root: std::path::PathBuf,
    save: Arc<horizon_client::save::SaveDir>,
}

/// Exercise the real stop/save paths, not just direct calls to hook dispatch.
#[test]
#[ignore = "daily"]
fn every_started_server_stops_and_saves_even_when_startup_or_ticks_fail() {
    struct Log(
        Arc<Mutex<Vec<&'static str>>>,
        Arc<PanicAt>,
        Arc<Mutex<Option<StoppedProbe>>>,
    );
    #[mod_api::__subscriptions]
    impl Hooks for Log {
        fn register_configs(
            &self,
            _: mod_api::HostSide,
            e: &mut mod_api::RegisterConfigsEvent<'_>,
        ) {
            if self.1.config_unloading.load(Ordering::Relaxed) {
                e.configs
                    .push(mod_api::Config::new("log", mod_api::ConfigKind::Server, ""));
            }
        }
        fn config_unloading(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::ConfigUnloadingEvent<'_>,
        ) {
            self.0.lock().unwrap().push("config-unloading");
            panic!("config unloading listener");
        }
        fn server_starting(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartingEvent) {
            self.0.lock().unwrap().push("starting");
        }
        fn server_stopping(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppingEvent) {
            self.0.lock().unwrap().push("stopping");
            if self.1.stopping.load(Ordering::Relaxed) {
                panic!("stopping listener");
            }
        }
        fn world_saved(&self, _: &mut dyn ServerHost, _: &mut mod_api::WorldSavedEvent) {
            self.0.lock().unwrap().push("saved");
            if self.1.save.load(Ordering::Relaxed) {
                panic!("save listener");
            }
        }
        fn player_logged_out(
            &self,
            _: &mut dyn ServerHost,
            _: &mut mod_api::PlayerLoggedOutEvent<'_>,
        ) {
            if self.1.logout.load(Ordering::Relaxed) {
                self.0.lock().unwrap().push("logout");
                panic!("logout listener");
            }
        }
        fn server_stopped(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStoppedEvent) {
            if let Some(probe) = self.2.lock().unwrap().as_ref() {
                assert!((probe.serving_stopped)(), "serving retired before Stopped");
                assert!(probe.lan.upgrade().is_none(), "LAN dropped before Stopped");
                let saved = horizon_client::save::Level::read(&probe.root).unwrap();
                assert_eq!(saved.name, "lifecycle final save");
                assert!(
                    probe.save.raw_chunk(0, 0).is_some(),
                    "dirty chunk saved before Stopped"
                );
                let last = if self.1.logout.load(Ordering::Relaxed) {
                    "logout"
                } else {
                    "saved"
                };
                assert_eq!(self.0.lock().unwrap().last(), Some(&last));
                // Storage is released before the notification, even if the
                // final-save listener itself panicked after writing metadata.
                let _lock = horizon_client::save::SessionLock::acquire(&probe.root).unwrap();
            }
            self.0.lock().unwrap().push("stopped");
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let panic_at = Arc::new(PanicAt {
        stopping: AtomicBool::new(false),
        save: AtomicBool::new(false),
        config_unloading: AtomicBool::new(false),
        logout: AtomicBool::new(false),
    });
    let stopped_probe = Arc::new(Mutex::new(None));
    with_mods(
        vec![ModEntry::new(
            "log",
            Box::new(Log(log.clone(), panic_at.clone(), stopped_probe.clone())),
        )]
        .into(),
        || {
            for dedicated in [false, true] {
                for case in [
                    "normal",
                    "startup-error",
                    "tick-error",
                    "panic",
                    "save-error",
                    "unopened",
                    "locked-unadopted",
                    "stopping-panic",
                    "save-panic",
                    "config-unloading-panic",
                    "config-unloading-save-panic",
                    "config-unloading-logout-panic",
                    "before-start",
                ] {
                    log.lock().unwrap().clear();
                    panic_at
                        .stopping
                        .store(case == "stopping-panic", Ordering::Relaxed);
                    panic_at.save.store(
                        matches!(case, "save-panic" | "config-unloading-save-panic"),
                        Ordering::Relaxed,
                    );
                    panic_at
                        .logout
                        .store(case == "config-unloading-logout-panic", Ordering::Relaxed);
                    let config_panic = case.starts_with("config-unloading");
                    panic_at
                        .config_unloading
                        .store(config_panic, Ordering::Relaxed);
                    *stopped_probe.lock().unwrap() = None;
                    let run_case = |game: &mut crate::game::Game<'_>| {
                        game.options.execution_role = if dedicated {
                            ExecutionRole::DedicatedServer
                        } else {
                            ExecutionRole::IntegratedServer
                        };
                        let root = std::env::temp_dir().join(format!(
                            "server-lifecycle-{}-{dedicated}-{case}",
                            std::process::id()
                        ));
                        std::fs::create_dir_all(&root).unwrap();
                        game.world_root = root.clone();
                        game.save = Arc::new(horizon_client::save::SaveDir::new(
                            root.clone(),
                            horizon_client::save::LevelShape::OVERWORLD,
                        ));
                        game.level.name = "lifecycle final save".into();
                        let adopted =
                            !matches!(case, "unopened" | "before-start" | "locked-unadopted");
                        if adopted || case == "locked-unadopted" {
                            game.session_lock =
                                Some(horizon_client::save::SessionLock::acquire(&root).unwrap());
                        }
                        // The lock is held while tags are readable, before the prepared
                        // level replaces the placeholder. Only adoption may save.
                        game.world_adopted = adopted;
                        if matches!(case, "unopened" | "locked-unadopted") {
                            // A failed open must not overwrite an existing, unreadable save.
                            std::fs::write(root.join("level.dat"), b"unadopted save").unwrap();
                        }
                        if config_panic {
                            game.level.name = "previous save".into();
                            game.level.write(&root).unwrap();
                            // Nothing queues these metadata/block changes. A flush
                            // without the final save must leave the old name and no chunk.
                            game.level.name = "lifecycle final save".into();
                            game.world.chunks.insert(
                                (0, 0),
                                Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                            );
                            let stone = game
                                .pack
                                .assets
                                .registry
                                .default_state("minecraft:stone")
                                .unwrap();
                            game.world.set(1, 64, 1, stone).unwrap();
                            assert!(game.save.raw_chunk(0, 0).is_none());
                            let serving_stopped = if dedicated {
                                Box::new(|| true) as Box<dyn Fn() -> bool + Send + Sync>
                            } else {
                                // Cover both a published and an unpublished integrated
                                // server. Neither may merely unpublish on shutdown.
                                Box::new(super::super::integrated::tests::install_serving(
                                    game,
                                    case == "config-unloading-save-panic",
                                ))
                            };
                            let lan = Arc::downgrade(&game.lan.as_ref().unwrap().hub.lan.shared);
                            *stopped_probe.lock().unwrap() = Some(StoppedProbe {
                                serving_stopped,
                                lan,
                                root: root.clone(),
                                save: game.save.clone(),
                            });
                            game.native_configs_server_start(&root, dedicated);
                        }
                        let result = game.run_server_lifecycle(dedicated, |game, began| {
                            if case == "before-start" {
                                return Err("before startup".into());
                            }
                            *began = true;
                            game.native_server_about_to_start(dedicated);
                            game.native_server_starting(dedicated);
                            game.native_lifecycle_open_started();
                            if matches!(case, "startup-error" | "unopened" | "locked-unadopted") {
                                return Err("failed init".into());
                            }
                            game.native_lifecycle_world_ready();
                            match case {
                                "tick-error" => Err("failed tick".into()),
                                "panic" => panic!("failed tick"),
                                "save-error" => {
                                    let file = root.join("not-a-directory");
                                    std::fs::write(&file, b"occupied").unwrap();
                                    game.world_root = file;
                                    Ok(())
                                }
                                _ => Ok(()),
                            }
                        });
                        assert_eq!(result.is_ok(), case == "normal", "{dedicated} {case}");
                        if config_panic || matches!(case, "stopping-panic" | "save-panic") {
                            let message = result.unwrap_err().to_string();
                            let needle = if config_panic {
                                "config unloading listener"
                            } else if case == "stopping-panic" {
                                "stopping listener"
                            } else {
                                "save listener"
                            };
                            assert!(message.contains(needle), "{dedicated} {case}: {message}");
                            if config_panic {
                                assert_eq!(
                                    message,
                                    "the server shutdown panicked: config unloading listener"
                                );
                            }
                        }
                        let expected: &[&str] = match case {
                            "before-start" => &[],
                            "unopened" | "save-error" | "locked-unadopted" => {
                                &["starting", "stopping", "stopped"]
                            }
                            "config-unloading-panic" | "config-unloading-save-panic" => &[
                                "starting",
                                "stopping",
                                "config-unloading",
                                "saved",
                                "stopped",
                            ],
                            "config-unloading-logout-panic" => &[
                                "starting",
                                "stopping",
                                "config-unloading",
                                "saved",
                                "logout",
                                "stopped",
                            ],
                            _ => &["starting", "stopping", "saved", "stopped"],
                        };
                        assert_eq!(*log.lock().unwrap(), expected, "{dedicated} {case}");
                        assert!(game.session_lock.is_none(), "storage released after {case}");
                        assert!(!game.world_adopted, "adoption cleared after {case}");
                        assert!(game.serving.is_none(), "serving retired after {case}");
                        assert!(game.lan.is_none(), "LAN dropped after {case}");
                        if config_panic {
                            let registry = &game.pack.assets.registry;
                            let chunk =
                                game.save.load(0, 0, registry, world_biomes!(game)).unwrap();
                            assert_eq!(
                                chunk.block(1, 64, 1),
                                registry.default_state("minecraft:stone").unwrap()
                            );
                        }
                        if matches!(case, "unopened" | "locked-unadopted") {
                            assert_eq!(
                                std::fs::read(root.join("level.dat")).unwrap(),
                                b"unadopted save",
                                "{dedicated} {case}"
                            );
                        } else if !matches!(case, "save-error" | "before-start") {
                            let saved = horizon_client::save::Level::read(&root).unwrap();
                            assert_eq!(saved.name, "lifecycle final save");
                        }
                        std::fs::remove_dir_all(root).unwrap();
                    };
                    if config_panic {
                        with_guests(run_case);
                    } else {
                        with_game(run_case);
                    }
                }
            }
        },
    );
}
