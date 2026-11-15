//! Native mods' config files, loaded and reloaded (NeoForge's `ModConfig`
//! and `ModConfigEvent`, docs/modding/src/how-to/native-mods.md). A mod lists its configs
//! (`native_register_configs`), each a file of a kind: `Client` on the
//! window, `Common` once per process (the window, or a dedicated server),
//! `Server` on the server for each world it opens, which a world may
//! override in its `serverconfig/` folder. The game writes a missing file's
//! default text, reads it and hands the text to the mod
//! (`native_config_loading`); the mod parses it. Once a second it looks at
//! each file's modification time and hands changed text over again
//! (`native_config_reloading`). A server's configs are dropped when it stops
//! (`native_config_unloading`).
use super::Game;
use mod_api::ConfigPath;
pub use mod_api::game::ConfigFile;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the files' modification times are read.
const CHECK_EVERY: Duration = Duration::from_secs(1);

/// A loaded config, where it was read and what its file looked like then.
struct Tracked {
    config: ConfigFile,
    path: PathBuf,
}

/// One game's configs: the window's, a server thread's or a dedicated
/// server's.
#[derive(Default)]
pub(super) struct NativeConfigs {
    started: bool,
    files: Vec<Tracked>,
    next_check: Option<Instant>,
}

/// The file `config` is read from: the world's `serverconfig/` copy of a
/// server config when there is one, else `config_dir`'s, written from the
/// default when missing.
fn locate(
    config: &mod_api::Config,
    config_dir: &Path,
    world: Option<&Path>,
) -> std::io::Result<PathBuf> {
    if config.kind == mod_api::ConfigKind::Server
        && let Some(world) = world
    {
        let own = world.join("serverconfig").join(&config.file);
        if own.is_file() {
            return Ok(own);
        }
    }
    let path = config_dir.join(&config.file);
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &config.default)?;
    }
    Ok(path)
}

impl NativeConfigs {
    /// Reads `configs` of `kinds` (a duplicate path, or a file outside the
    /// mod's own in `config/` as `Host::config_files` reads them, is skipped)
    /// and answers each loaded file.
    fn load(
        &mut self,
        configs: Vec<mod_api::Config>,
        kinds: &[mod_api::ConfigKind],
        config_dir: &Path,
        world: Option<&Path>,
    ) -> Vec<ConfigFile> {
        self.started = true;
        let mut loaded = Vec::new();
        for config in configs.into_iter().filter(|c| kinds.contains(&c.kind)) {
            if !ConfigPath::new(&config.file).is_ok_and(|file| file.owned_by(&config.mod_id)) {
                log::warn!(
                    "Horizon: mod {} config {:?} is not one of its files in config/",
                    config.mod_id,
                    config.file
                );
                continue;
            }
            let path = match locate(&config, config_dir, world) {
                Ok(path) => path,
                Err(e) => {
                    log::warn!("Horizon: mod {} config {}: {e}", config.mod_id, config.file);
                    continue;
                }
            };
            if self.files.iter().any(|t| t.path == path) {
                log::warn!(
                    "Horizon: mod {} config {} is registered twice",
                    config.mod_id,
                    path.display()
                );
                continue;
            }
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!(
                        "Horizon: mod {} config {}: {e}",
                        config.mod_id,
                        path.display()
                    );
                    continue;
                }
            };
            let file = ConfigFile::from(mod_api::__private::built::ConfigFile {
                mod_id: config.mod_id,
                kind: config.kind,
                file: config.file,
                text,
            });
            loaded.push(file.clone());
            self.files.push(Tracked { config: file, path });
        }
        loaded
    }

    /// The files whose text changed since they were read, read again; a
    /// file that is gone or unreadable keeps its last text. The text is
    /// compared, not the file's modification time: a save inside the same
    /// timestamp tick as the load (a mod writing its config from
    /// `config_loading`) keeps the time and the length and still changed.
    fn changed(&mut self) -> Vec<ConfigFile> {
        let mut changed = Vec::new();
        for tracked in &mut self.files {
            let Ok(text) = fs::read_to_string(&tracked.path) else {
                continue;
            };
            if text != tracked.config.text {
                tracked.config.text = text;
                changed.push(tracked.config.clone());
            }
        }
        changed
    }

    /// Whether a second passed since the last look at the files.
    fn due(&mut self, now: Instant) -> bool {
        if self.files.is_empty() || self.next_check.is_some_and(|t| now < t) {
            return false;
        }
        self.next_check = Some(now + CHECK_EVERY);
        true
    }

    /// Drops the server's configs and answers them.
    fn drop_server(&mut self) -> Vec<ConfigFile> {
        let (server, rest) = std::mem::take(&mut self.files)
            .into_iter()
            .partition(|t| t.config.kind == mod_api::ConfigKind::Server);
        self.files = rest;
        server.into_iter().map(|t| t.config).collect()
    }
}

impl Game<'_> {
    /// Asks the mods for their configs and loads those of `kinds`.
    fn native_configs_load(&mut self, kinds: &[mod_api::ConfigKind], world: Option<&Path>) {
        let phase = self.mod_phase.clone();
        let _config = phase
            .config()
            .expect("config load on its owning game thread");
        let config_dir = match Self::native_config_dir() {
            Ok(dir) => dir,
            Err(e) => {
                log::warn!("Horizon: mod configs: {e}");
                return;
            }
        };
        let mut configs = Vec::new();
        self.native_register_configs(&mut configs);
        if configs.is_empty() {
            self.native_configs.started = true;
            return;
        }
        for file in self.native_configs.load(configs, kinds, &config_dir, world) {
            self.native_config_loading(&file);
        }
    }

    /// The window's `Client` and `Common` configs, once: after client setup
    /// and before its title, connection or ordinary runtime callbacks.
    pub(super) fn native_configs_client_start(&mut self) {
        // Other local players share the process's client setup, not another
        // Client/Common load or a second config watcher.
        if self.options.split_guest {
            return;
        }
        if !self.native_configs.started {
            use mod_api::ConfigKind::{Client, Common};
            self.native_configs_load(&[Client, Common], None);
        }
    }

    /// The window's frame step: changed config files reload once a second.
    pub(super) fn native_configs_tick_client(&mut self) {
        self.native_configs_poll();
    }

    /// A server about to open `world`: a dedicated server loads its
    /// `Common` configs too, a single player's server thread only `Server`.
    pub(super) fn native_configs_server_start(&mut self, world: &Path, dedicated: bool) {
        use mod_api::ConfigKind::{Common, Server};
        let kinds: &[mod_api::ConfigKind] = if dedicated && !self.native_configs.started {
            &[Common, Server]
        } else {
            &[Server]
        };
        self.native_configs_load(kinds, Some(world));
    }

    /// The server stops: its `Server` configs unload.
    pub(super) fn native_configs_server_stop(&mut self) {
        for file in self.native_configs.drop_server() {
            self.native_config_unloading(&file);
        }
    }

    /// Hands changed files over again, at most once a second.
    pub(super) fn native_configs_poll(&mut self) {
        if !self.native_configs.due(Instant::now()) {
            return;
        }
        for file in self.native_configs.changed() {
            self.native_config_reloading(&file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::ConfigKind::{Client, Common, Server};

    #[test]
    fn engine_test_host_operation_reads_the_real_phase_provider() {
        use mod_api::{
            __private::{ModPhase, PhaseOwner},
            Host,
            prelude::EntitiesHost,
        };
        Game::initialize_mod_runtime_for_tests();
        struct EmptyHost;
        impl Host for EmptyHost {}
        mod_api::impl_host_areas!(EmptyHost);
        let _phase = PhaseOwner::scope(ModPhase::Setup);
        assert_eq!(mod_api::__private::current_phase(), ModPhase::Setup);
        let refused = std::panic::catch_unwind(|| EmptyHost.entity(1)).unwrap_err();
        let message = refused.downcast_ref::<String>().unwrap();
        assert!(message.contains("Setup"), "{message}");
    }

    #[test]
    #[ignore = "daily"]
    fn config_permissions_and_once_per_side_survive_two_worlds() {
        use mod_api::{Hooks, Host, HostSide, HostTransaction, ModEntry};
        use std::sync::{Arc, Mutex};
        struct Probe(Arc<Mutex<Vec<(bool, mod_api::ConfigKind)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Probe {
            fn register_configs(
                &self,
                _: HostSide<'_>,
                event: &mut mod_api::RegisterConfigsEvent<'_>,
            ) {
                for kind in [Client, Common, Server] {
                    event
                        .configs
                        .push(mod_api::Config::new("phase_probe", kind, "enabled=true"));
                }
            }
            fn config_loading(
                &self,
                mut host: HostSide<'_>,
                event: &mut mod_api::ConfigLoadingEvent<'_>,
            ) {
                assert_eq!(
                    mod_api::__private::current_phase(),
                    mod_api::__private::ModPhase::Config
                );
                let read: &dyn Host = match &host {
                    HostSide::Server(host) => &**host,
                    HostSide::Client(host) => &**host,
                };
                let _ = read.setting("phase_probe", "unused");
                let refused =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read.entity(1)))
                        .unwrap_err();
                assert!(refused.downcast_ref::<String>().unwrap().contains("Config"));
                if let HostSide::Server(host) = &mut host {
                    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _: Result<(), ()> = host.transaction(|_, _| Ok(()));
                    }))
                    .unwrap_err();
                    assert!(refused.downcast_ref::<String>().unwrap().contains("Config"));
                }
                self.0.lock().unwrap().push((true, event.config.kind));
            }
            fn config_unloading(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::ConfigUnloadingEvent<'_>,
            ) {
                assert_eq!(
                    mod_api::__private::current_phase(),
                    mod_api::__private::ModPhase::World
                );
                self.0.lock().unwrap().push((false, event.config.kind));
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("phase_probe", Box::new(Probe(calls.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                game.mod_phase =
                    mod_api::__private::PhaseOwner::new(mod_api::__private::ModPhase::Setup);
                game.native_configs_client_start();
                game.native_configs_client_start();
                game.options.split_guest = true;
                game.native_configs = NativeConfigs::default();
                game.native_configs_client_start();
                game.options.split_guest = false;
                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::IntegratedServer;
                for world in ["phase-world-one", "phase-world-two"] {
                    game.native_configs_server_start(Path::new(world), false);
                    game.native_configs_server_stop();
                }
            });
        });
        assert_eq!(
            *calls.lock().unwrap(),
            [
                (true, Client),
                (true, Common),
                (true, Server),
                (false, Server),
                (true, Server),
                (false, Server),
            ]
        );
    }

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "horizon-native-configs-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_file_gets_its_default_and_an_edited_one_is_read() {
        let root = Folder::new("default");
        let config = root.0.join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("gt-common.toml"), "voltage = 512\n").unwrap();
        let mut configs = NativeConfigs::default();
        let loaded = configs.load(
            vec![
                mod_api::Config::new("gt", Client, "hud = true\n"),
                mod_api::Config::new("gt", Common, "voltage = 32\n"),
                mod_api::Config::new("gt", Server, "ores = 1\n"),
            ],
            &[Client, Common],
            &config,
            None,
        );
        let got: Vec<_> = loaded
            .iter()
            .map(|f| (f.file.as_str(), f.text.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("gt-client.toml", "hud = true\n"),
                ("gt-common.toml", "voltage = 512\n")
            ]
        );
        assert_eq!(
            fs::read_to_string(config.join("gt-client.toml")).unwrap(),
            "hud = true\n"
        );
        // Another side's kind is neither read nor written.
        assert!(!config.join("gt-server.toml").exists());
    }

    #[test]
    fn a_world_overrides_a_server_config_and_the_folder_serves_the_rest() {
        let root = Folder::new("world");
        let (config, world) = (root.0.join("config"), root.0.join("saves/W"));
        fs::create_dir_all(world.join("serverconfig/gt")).unwrap();
        fs::write(world.join("serverconfig/gt/ores.toml"), "veins = 9\n").unwrap();
        let mut configs = NativeConfigs::default();
        let loaded = configs.load(
            vec![
                mod_api::Config::new("gt", Server, "veins = 3\n").file("gt/ores.toml"),
                mod_api::Config::new("gt", Server, "tiers = 4\n"),
                // A Common config of the world's folder is not looked for.
                mod_api::Config::new("gt", Common, "a = 1\n"),
            ],
            &[Server],
            &config,
            Some(&world),
        );
        let got: Vec<_> = configs
            .files
            .iter()
            .map(|t| (t.path.clone(), t.config.text.as_str()))
            .collect();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            got,
            [
                (world.join("serverconfig/gt/ores.toml"), "veins = 9\n"),
                (config.join("gt-server.toml"), "tiers = 4\n"),
            ]
        );
        // The world's copy stands for the default: nothing is written beside it.
        assert!(!config.join("gt/ores.toml").exists());
        assert_eq!(configs.drop_server().len(), 2);
        assert!(configs.files.is_empty());
    }

    #[test]
    fn a_file_not_the_mods_own_or_registered_twice_is_skipped() {
        let root = Folder::new("skip");
        let config = root.0.join("config");
        let mut configs = NativeConfigs::default();
        let loaded = configs.load(
            vec![
                mod_api::Config::new("a", Common, "x").file("../options.txt"),
                mod_api::Config::new("a", Common, "x").file(""),
                mod_api::Config::new("a", Common, "first"),
                mod_api::Config::new("a", Common, "again"),
                // Another mod's file, as `Host::config_files` refuses it.
                mod_api::Config::new("b", Common, "second").file("a-common.toml"),
            ],
            &[Common],
            &config,
            None,
        );
        assert_eq!(
            loaded.iter().map(|f| f.mod_id.as_str()).collect::<Vec<_>>(),
            ["a"]
        );
        assert!(!root.0.join("options.txt").exists());
    }

    #[test]
    fn a_change_reloads_once_and_a_touch_without_one_does_not() {
        let root = Folder::new("reload");
        let config = root.0.join("config");
        let mut configs = NativeConfigs::default();
        configs.load(
            vec![mod_api::Config::new("gt", Common, "a = 1\n")],
            &[Common],
            &config,
            None,
        );
        let path = config.join("gt-common.toml");
        assert!(configs.changed().is_empty());
        fs::write(&path, "a = 2\n").unwrap();
        let changed = configs.changed();
        assert_eq!(
            changed.iter().map(|f| f.text.as_str()).collect::<Vec<_>>(),
            ["a = 2\n"]
        );
        assert!(configs.changed().is_empty());
        // Saved again with the same text: nothing to reload.
        fs::write(&path, "a = 2\n").unwrap();
        assert!(configs.changed().is_empty());
        // Same length, written at once after the load, as a mod's own save
        // from `config_loading` is: still a change.
        fs::write(&path, "a = 3\n").unwrap();
        assert_eq!(configs.changed().len(), 1);
        fs::write(&path, "a = 2\n").unwrap();
        configs.changed();
        // Gone: the last text stays, and nothing fires.
        fs::remove_file(&path).unwrap();
        assert!(configs.changed().is_empty());
        assert_eq!(configs.files[0].config.text, "a = 2\n");
    }

    #[test]
    fn files_are_looked_at_once_a_second_and_never_without_configs() {
        let mut configs = NativeConfigs::default();
        let start = Instant::now();
        assert!(!configs.due(start));
        configs.files.push(Tracked {
            config: ConfigFile::from(mod_api::__private::built::ConfigFile {
                mod_id: "gt".into(),
                kind: Common,
                file: "gt-common.toml".into(),
                text: String::new(),
            }),
            path: PathBuf::new(),
        });
        assert!(configs.due(start));
        assert!(!configs.due(start + Duration::from_millis(999)));
        assert!(configs.due(start + CHECK_EVERY));
    }
}
