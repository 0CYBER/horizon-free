//! One checked implementation for config, game bulk data and world bulk data.
use super::Game;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use mod_api::{
    CONFIG_FILE_MAX, CONFIG_LIST_MAX, ConfigEntry, ConfigEntryKind, ConfigFileError, ConfigFiles,
    ConfigPath, ConfigStat, DATA_FILE_MAX,
};
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

struct GameConfigFiles {
    bulk: bool,
}

static CONFIG_FILES: GameConfigFiles = GameConfigFiles { bulk: false };
static DATA_FILES: GameConfigFiles = GameConfigFiles { bulk: true };

/// A root selected by the game, never supplied by a mod. Bulk roots bind the
/// mod's directory implicitly; config keeps its established shared spelling.
struct FileRoot {
    path: PathBuf,
    bulk: bool,
}

impl FileRoot {
    fn cap(&self) -> u64 {
        if self.bulk {
            DATA_FILE_MAX
        } else {
            CONFIG_FILE_MAX
        }
    }
}

impl Game<'_> {
    /// The calling mod's own files in the game's `config/` folder, beside
    /// `options.txt` (beside `server.properties` on a dedicated server):
    /// its folder `config/<mod id>/` and the files beside it named
    /// `<mod id>.<ext>` or `<mod id>-<rest>`, as Fabric and NeoForge mods
    /// keep theirs. The service is bound to the mod whose hook or part is
    /// running, so a mod never names its id and can't reach another mod's
    /// files; a world's `serverconfig/` is read only through
    /// `config_loading`.
    pub(super) fn config_files() -> &'static dyn ConfigFiles {
        &CONFIG_FILES
    }
    pub(super) fn data_files() -> &'static dyn ConfigFiles {
        &DATA_FILES
    }

    pub(super) fn world_data_files(&self) -> &dyn ConfigFiles {
        self
    }
    /// Whether a mod with the `horizon-mod.toml` id `id` is installed: a
    /// native, wasm or content package (Fabric's `FabricLoader.isModLoaded`,
    /// NeoForge's `ModList.isLoaded`). The list is fixed at startup.
    pub(super) fn is_loaded(id: &str) -> bool {
        mod_api::__private::loaded_mods().get(id).is_some()
    }
}

fn io(error: std::io::Error) -> ConfigFileError {
    match error.kind() {
        ErrorKind::NotFound => ConfigFileError::NotFound,
        _ => ConfigFileError::Io(error.to_string()),
    }
}

/// Open every directory without following links. Keeping the parent handle
/// makes subsequent opens and atomic replacement safe even if a directory is
/// renamed or replaced between operations.
fn resolve(
    root: &FileRoot,
    path: &ConfigPath,
    create: bool,
) -> Result<(Dir, String), ConfigFileError> {
    let mod_id =
        mod_api::content::crash::current_native_mod().ok_or(ConfigFileError::NoCallingMod)?;
    if !root.bulk && !path.owned_by(mod_id) {
        return Err(ConfigFileError::NotOwned);
    }
    let mut directory =
        Dir::open_ambient_dir(&root.path, cap_std::ambient_authority()).map_err(io)?;
    let mut components = root
        .bulk
        .then_some("horizon-mod-data")
        .into_iter()
        .chain(root.bulk.then_some(mod_id))
        .chain(path.as_str().split('/').filter(|part| !part.is_empty()))
        .peekable();
    while let Some(part) = components.next() {
        match directory.symlink_metadata(part) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(ConfigFileError::Symlink),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        if components.peek().is_none() {
            return Ok((directory, part.to_owned()));
        }
        if create {
            match directory.create_dir(part) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io(e)),
            }
        }
        directory = directory.open_dir_nofollow(part).map_err(io)?;
    }
    Err(ConfigFileError::NotOwned)
}

fn stat_of(meta: &Metadata) -> ConfigStat {
    let modified_millis = meta
        .modified()
        .ok()
        .and_then(|t| {
            t.duration_since(cap_std::time::SystemTime::from_std(UNIX_EPOCH))
                .ok()
        })
        .map_or(0, |d| d.as_millis() as u64);
    let (kind, len) = if meta.is_dir() {
        (ConfigEntryKind::Folder, 0)
    } else {
        (ConfigEntryKind::File, meta.len())
    };
    ConfigStat::from(mod_api::__private::built::ConfigStat {
        kind,
        len,
        modified_millis,
    })
}

fn read(root: &FileRoot, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError> {
    let (directory, name) = resolve(root, file, false)?;
    let meta = directory.symlink_metadata(&name).map_err(io)?;
    if !meta.is_file() {
        return Err(ConfigFileError::NotAFile);
    }
    if meta.len() > root.cap() {
        return Err(ConfigFileError::TooLarge);
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let opened = directory.open_with(&name, &options).map_err(io)?;
    if !opened.metadata().map_err(io)?.is_file() {
        return Err(ConfigFileError::NotAFile);
    }
    let mut bytes = Vec::new();
    // One byte past the cap detects growth without an unbounded allocation.
    opened
        .take(root.cap() + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > root.cap() {
        return Err(ConfigFileError::TooLarge);
    }
    Ok(bytes)
}

/// Whether `name` is the temporary file of a write, `<name>.<pid>-<n>.tmp`.
fn is_write_temp(name: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    name.strip_suffix(".tmp")
        .and_then(|rest| rest.rsplit_once('.'))
        .and_then(|(_, tail)| tail.split_once('-'))
        .is_some_and(|(pid, n)| digits(pid) && digits(n))
}

/// Written beside the file, then renamed over it. The temporary name starts
/// with the file's own, so it stays the mod's own; `list` leaves it out, so a
/// scan never meets a half-written file, nor one a crash left behind.
fn write(root: &FileRoot, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if bytes.len() as u64 > root.cap() {
        return Err(ConfigFileError::TooLarge);
    }
    // Neither the implicit bulk root nor a config mod folder can be a file.
    if file.as_str().is_empty()
        || (!root.bulk && Some(file.as_str()) == mod_api::content::crash::current_native_mod())
    {
        return Err(ConfigFileError::NotAFile);
    }
    let (directory, name) = resolve(root, file, true)?;
    match directory.symlink_metadata(&name) {
        Ok(meta) if meta.file_type().is_symlink() => return Err(ConfigFileError::Symlink),
        Ok(meta) if !meta.is_file() => return Err(ConfigFileError::NotAFile),
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(io(e)),
    }
    let temp = format!(
        "{name}.{}-{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    );
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let written = directory
        .open_with(&temp, &options)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| directory.rename(&temp, &directory, &name));
    if written.is_err() {
        let _ = directory.remove_file(&temp);
    }
    written.map_err(io)
}

fn list(
    root: &FileRoot,
    folder: &ConfigPath,
    after: Option<&str>,
    limit: u32,
) -> Result<Vec<ConfigEntry>, ConfigFileError> {
    if limit == 0
        || limit as usize > CONFIG_LIST_MAX
        || after.is_some_and(|name| !ConfigPath::is_valid_name(name))
    {
        return Err(ConfigFileError::InvalidPage);
    }
    let (parent, name) = resolve(root, folder, false)?;
    if !parent.symlink_metadata(&name).map_err(io)?.is_dir() {
        return Err(ConfigFileError::NotAFolder);
    }
    let directory = parent.open_dir_nofollow(&name).map_err(io)?;
    // Keep only the next bounded page, not every tile in the directory.
    let mut entries = BTreeMap::new();
    for entry in directory.entries().map_err(io)? {
        let entry = entry.map_err(io)?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if after.is_some_and(|cursor| name.as_str() <= cursor)
            || is_write_temp(&name)
            || !ConfigPath::is_valid_name(&name)
            || (entries.len() == limit as usize
                && entries
                    .last_key_value()
                    .is_some_and(|(last, _)| &name >= last))
        {
            continue;
        }
        let meta = directory.symlink_metadata(&name).map_err(io)?;
        if !meta.is_file() && !meta.is_dir() {
            continue;
        }
        entries.insert(name, stat_of(&meta));
        if entries.len() > limit as usize {
            entries.pop_last();
        }
    }
    Ok(entries
        .into_iter()
        .map(|(name, stat)| {
            ConfigEntry::from(mod_api::__private::built::ConfigEntry { name, stat })
        })
        .collect())
}

fn stat(root: &FileRoot, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError> {
    match resolve(root, path, false).and_then(|(dir, name)| dir.symlink_metadata(name).map_err(io))
    {
        Ok(meta) if meta.file_type().is_symlink() => Err(ConfigFileError::Symlink),
        Ok(meta) => Ok(Some(stat_of(&meta))),
        Err(ConfigFileError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

impl GameConfigFiles {
    fn root(&self) -> Result<FileRoot, ConfigFileError> {
        Ok(FileRoot {
            path: if self.bulk {
                Game::game_dir()
            } else {
                Game::native_config_dir().map_err(io)?
            },
            bulk: self.bulk,
        })
    }
}

fn world_root(game: &Game<'_>) -> Result<FileRoot, ConfigFileError> {
    if game.world_root.as_os_str().is_empty() {
        return Err(ConfigFileError::NoWorld);
    }
    Ok(FileRoot {
        path: game.world_root.clone(),
        bulk: true,
    })
}

macro_rules! files {
    ($owner:ty, $root:expr) => {
        impl ConfigFiles for $owner {
            fn read(&self, file: &ConfigPath) -> Result<Vec<u8>, ConfigFileError> {
                read(&$root(self)?, file)
            }
            fn write(&self, file: &ConfigPath, bytes: &[u8]) -> Result<(), ConfigFileError> {
                write(&$root(self)?, file, bytes)
            }
            fn list(
                &self,
                folder: &ConfigPath,
                after: Option<&str>,
                limit: u32,
            ) -> Result<Vec<ConfigEntry>, ConfigFileError> {
                list(&$root(self)?, folder, after, limit)
            }
            fn stat(&self, path: &ConfigPath) -> Result<Option<ConfigStat>, ConfigFileError> {
                stat(&$root(self)?, path)
            }
        }
    };
}

files!(GameConfigFiles, GameConfigFiles::root);
files!(Game<'_>, world_root);

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::crash::{ModCall, enter_native_mod};

    struct Root(FileRoot);
    impl Root {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "horizon-config-files-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(FileRoot {
                path: root,
                bulk: false,
            })
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.path);
        }
    }

    fn path(path: &str) -> ConfigPath {
        ConfigPath::new(path).unwrap()
    }

    fn as_mod<R>(id: &'static str, f: impl FnOnce() -> R) -> R {
        mod_platform::__private::initialize().unwrap();
        let _call = enter_native_mod(ModCall {
            id,
            hook: "world_load",
            part: None,
        });
        f()
    }

    #[test]
    fn a_mod_writes_reads_lists_and_stats_its_own_files() {
        let root = Root::new("own");
        let root = &root.0;
        as_mod("fd", || {
            assert_eq!(stat(root, &path("fd/state.json")), Ok(None));
            write(root, &path("fd/state.json"), b"{\"a\":1}").unwrap();
            write(root, &path("fd.json"), b"top").unwrap();
            assert_eq!(read(root, &path("fd/state.json")).unwrap(), b"{\"a\":1}");
            assert_eq!(read(root, &path("fd.json")).unwrap(), b"top");
            // A second write replaces the bytes and leaves no temporary file.
            write(root, &path("fd/state.json"), b"{}").unwrap();
            let entries = list(root, &path("fd"), None, 1024).unwrap();
            assert_eq!(
                entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
                ["state.json"]
            );
            assert_eq!(entries[0].stat.kind, ConfigEntryKind::File);
            assert_eq!(entries[0].stat.len, 2);
            assert!(entries[0].stat.modified_millis > 0);
            assert_eq!(
                stat(root, &path("fd")).unwrap().map(|s| s.kind),
                Some(ConfigEntryKind::Folder)
            );
            assert_eq!(read(root, &path("fd")), Err(ConfigFileError::NotAFile));
            assert_eq!(
                list(root, &path("fd.json"), None, 1024),
                Err(ConfigFileError::NotAFolder)
            );
            assert_eq!(read(root, &path("fd/gone")), Err(ConfigFileError::NotFound));
        });
        assert_eq!(fs::read(root.path.join("fd/state.json")).unwrap(), b"{}");
    }

    #[test]
    fn a_file_named_for_the_mod_cant_block_its_folder_and_list_hides_write_temps() {
        let root = Root::new("folder-name");
        let root = &root.0;
        as_mod("fd", || {
            assert_eq!(
                write(root, &path("fd"), b"x"),
                Err(ConfigFileError::NotAFile)
            );
            assert!(!root.path.join("fd").exists());
            write(root, &path("fd/state.json"), b"{}").unwrap();
            // What a crash between the temp file and the rename leaves behind.
            fs::write(root.path.join("fd.json.1-0.tmp"), b"half").unwrap();
            fs::write(root.path.join("fd/state.json.1-0.tmp"), b"half").unwrap();
            fs::write(root.path.join("fd/notes.tmp"), b"mine").unwrap();
            assert_eq!(read(root, &path("fd.json.1-0.tmp")).unwrap(), b"half");
            // `list` leaves out a write's temporary file, not a mod's own `.tmp`.
            let names: Vec<_> = list(root, &path("fd"), None, 1024)
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert_eq!(names, ["notes.tmp", "state.json"]);
        });
    }

    #[test]
    fn a_mod_never_reaches_another_mods_files() {
        let root = Root::new("other");
        let root = &root.0;
        as_mod("b", || write(root, &path("b/secret.json"), b"b's").unwrap());
        as_mod("a", || {
            for other in ["b/secret.json", "b.json", "b-common.toml"] {
                let other = path(other);
                assert_eq!(read(root, &other), Err(ConfigFileError::NotOwned));
                assert_eq!(write(root, &other, b"a's"), Err(ConfigFileError::NotOwned));
                assert_eq!(stat(root, &other), Err(ConfigFileError::NotOwned));
            }
            assert_eq!(
                list(root, &path("b"), None, 1024),
                Err(ConfigFileError::NotOwned)
            );
        });
        assert_eq!(fs::read(root.path.join("b/secret.json")).unwrap(), b"b's");
        // Outside any mod's call nothing is anyone's.
        assert_eq!(
            read(root, &path("b/secret.json")),
            Err(ConfigFileError::NoCallingMod)
        );
    }

    /// The service through a real game and the hooks' own dispatch: a mod
    /// saves its registered config from `config_loading` and keeps a file in
    /// its folder, the reload poll hands the saved text back, and the same
    /// call for another mod's file is refused. In a fresh process, run in a
    /// scratch folder, since `config/` is beside the game's working folder.
    #[test]
    #[ignore = "needs prepared resources"]
    fn a_mod_saves_its_config_through_the_game_and_the_reload_hears_it() {
        use mod_api::{
            self, ConfigLoadingEvent, ConfigReloadingEvent, Hooks, HostSide, ModEntry, Mods,
            RegisterConfigsEvent,
        };
        use std::sync::Mutex;
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        fn heard() -> std::sync::MutexGuard<'static, Vec<String>> {
            HEARD
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "game::native_config_files::tests::a_mod_saves_its_config_through_the_game_and_the_reload_hears_it",
            "HORIZON_CONFIG_FILES_CHILD",
            &["game"],
        ) {
            return;
        }
        fn files(host: &HostSide) -> &'static dyn ConfigFiles {
            match host {
                HostSide::Server(host) => host.config_files(),
                HostSide::Client(host) => host.config_files(),
            }
        }
        struct Proof;
        #[mod_api::__subscriptions]
        impl Hooks for Proof {
            fn register_configs(&self, _: HostSide, event: &mut RegisterConfigsEvent<'_>) {
                event.configs.push(mod_api::Config::new(
                    "proof",
                    mod_api::ConfigKind::Common,
                    "count = 0\n",
                ));
            }
            fn config_loading(&self, host: HostSide, event: &mut ConfigLoadingEvent<'_>) {
                let files = files(&host);
                heard().push(format!("loading {}", event.config.text.trim()));
                files
                    .write(&path("proof-common.toml"), b"count = 1\n")
                    .unwrap();
                files.write(&path("proof/state.bin"), &[1, 2, 3]).unwrap();
                assert_eq!(files.read(&path("proof/state.bin")).unwrap(), [1, 2, 3]);
                let other = path("other/secret.json");
                assert_eq!(files.read(&other), Err(ConfigFileError::NotOwned));
                assert_eq!(files.write(&other, b"x"), Err(ConfigFileError::NotOwned));
            }
            fn config_reloading(&self, _: HostSide, event: &mut ConfigReloadingEvent<'_>) {
                heard().push(format!("reloading {}", event.config.text.trim()));
            }
        }
        let mods: Mods = vec![ModEntry::new("proof", Box::new(Proof))].into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_configs_client_start();
                game.native_configs_poll();
            });
        });
        assert_eq!(*heard(), ["loading count = 0", "reloading count = 1"]);
        let config = std::env::current_dir().unwrap().join("config");
        assert_eq!(fs::read(config.join("proof/state.bin")).unwrap(), [1, 2, 3]);
        assert!(!config.join("other").exists());
        // Outside the mod's hooks the service is no one's.
        assert_eq!(
            Game::config_files().read(&path("proof-common.toml")),
            Err(ConfigFileError::NoCallingMod)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_never_leads_out_of_config() {
        let root = Root::new("link");
        let outside = Root::new("link-outside");
        fs::write(outside.0.path.join("passwd"), b"outside").unwrap();
        std::os::unix::fs::symlink(&outside.0.path, root.0.path.join("fd")).unwrap();
        std::os::unix::fs::symlink(outside.0.path.join("passwd"), root.0.path.join("fd.json"))
            .unwrap();
        as_mod("fd", || {
            assert_eq!(
                read(&root.0, &path("fd/passwd")),
                Err(ConfigFileError::Symlink)
            );
            assert_eq!(
                write(&root.0, &path("fd/new"), b"x"),
                Err(ConfigFileError::Symlink)
            );
            assert_eq!(
                read(&root.0, &path("fd.json")),
                Err(ConfigFileError::Symlink)
            );
            assert_eq!(
                list(&root.0, &path("fd"), None, 1024),
                Err(ConfigFileError::Symlink)
            );
        });
        assert!(!outside.0.path.join("new").exists());
    }

    #[test]
    fn a_file_holds_four_mebibytes_at_most() {
        let root = Root::new("size");
        let root = &root.0;
        let max = vec![b'x'; CONFIG_FILE_MAX as usize];
        as_mod("fd", || {
            write(root, &path("fd/max"), &max).unwrap();
            assert_eq!(read(root, &path("fd/max")).unwrap().len(), max.len());
            let over = [max.as_slice(), b"x"].concat();
            assert_eq!(
                write(root, &path("fd/over"), &over),
                Err(ConfigFileError::TooLarge)
            );
            fs::write(root.path.join("fd/over"), &over).unwrap();
            assert_eq!(read(root, &path("fd/over")), Err(ConfigFileError::TooLarge));
        });
    }
    #[test]
    fn bulk_files_are_mod_bound_paged_and_separate_from_settings() {
        let mut root = Root::new("bulk");
        root.0.bulk = true;
        let root = &root.0;
        let settings = root.path.join("horizon-mods/.horizon-mod-data");
        fs::create_dir_all(settings.join("fd")).unwrap();
        fs::write(settings.join("fd/enabled"), b"true").unwrap();
        as_mod("fd", || {
            for name in ["z.bin", "a.bin", "m.bin", "other/secret.bin"] {
                write(root, &path(name), b"tile:42").unwrap();
            }
            assert_eq!(read(root, &path("a.bin")).unwrap(), b"tile:42");
            let first = list(root, &ConfigPath::root(), None, 2).unwrap();
            assert_eq!(
                first.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
                ["a.bin", "m.bin"]
            );
            assert_eq!(first[0].stat.len, 7);
            assert_eq!(first[0].stat.kind, ConfigEntryKind::File);
            let second = list(root, &ConfigPath::root(), Some("m.bin"), 2).unwrap();
            assert_eq!(
                second.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
                ["other", "z.bin"]
            );
            assert!(
                list(root, &ConfigPath::root(), Some("z.bin"), 2)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(stat(root, &path("a.bin")).unwrap().unwrap().len, 7);
            assert_eq!(
                list(root, &ConfigPath::root(), None, 0),
                Err(ConfigFileError::InvalidPage)
            );
            assert_eq!(
                list(root, &ConfigPath::root(), None, 1025),
                Err(ConfigFileError::InvalidPage)
            );
            assert_eq!(
                list(root, &ConfigPath::root(), Some("../fd"), 2),
                Err(ConfigFileError::InvalidPage)
            );
            let captured = horizon_world::native_load_context::capture_settings(&settings, ["fd"]);
            assert_eq!(
                captured["fd"]
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str()))
                    .collect::<Vec<_>>(),
                [("enabled", "true")]
            );
        });
        as_mod("other", || {
            assert_eq!(read(root, &path("a.bin")), Err(ConfigFileError::NotFound));
            assert_eq!(
                read(root, &path("fd/a.bin")),
                Err(ConfigFileError::NotFound)
            );
            write(root, &path("a.bin"), b"other's tile").unwrap();
            assert_eq!(read(root, &path("a.bin")).unwrap(), b"other's tile");
        });
        assert_eq!(
            fs::read(root.path.join("horizon-mod-data/fd/a.bin")).unwrap(),
            b"tile:42"
        );
        assert_eq!(
            read(root, &path("a.bin")),
            Err(ConfigFileError::NoCallingMod)
        );
    }

    #[test]
    fn bulk_cap_exceeds_config_and_refuses_without_partial_replacement() {
        let mut root = Root::new("bulk-cap");
        root.0.bulk = true;
        let root = &root.0;
        as_mod("fd", || {
            let larger_than_config = vec![3; (CONFIG_FILE_MAX + 1) as usize];
            write(root, &path("tile"), &larger_than_config).unwrap();
            assert_eq!(read(root, &path("tile")).unwrap(), larger_than_config);
            write(root, &path("tile"), b"keep").unwrap();
            let over = vec![0; DATA_FILE_MAX as usize + 1];
            assert_eq!(
                write(root, &path("tile"), &over),
                Err(ConfigFileError::TooLarge)
            );
            assert_eq!(read(root, &path("tile")).unwrap(), b"keep");
            assert_eq!(
                write(root, &path("absent"), &over),
                Err(ConfigFileError::TooLarge)
            );
            assert_eq!(stat(root, &path("absent")), Ok(None));
            assert_eq!(list(root, &ConfigPath::root(), None, 10).unwrap().len(), 1);
            fs::File::create(root.path.join("horizon-mod-data/fd/oversized"))
                .unwrap()
                .set_len(DATA_FILE_MAX + 1)
                .unwrap();
            assert_eq!(
                read(root, &path("oversized")),
                Err(ConfigFileError::TooLarge)
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn bulk_links_cannot_reach_another_mod_or_an_external_folder() {
        let mut root = Root::new("bulk-links");
        root.0.bulk = true;
        let outside = Root::new("bulk-links-outside");
        fs::write(outside.0.path.join("secret"), b"outside").unwrap();
        as_mod("other", || {
            write(&root.0, &path("secret"), b"other").unwrap()
        });
        fs::create_dir(root.0.path.join("horizon-mod-data/fd")).unwrap();
        std::os::unix::fs::symlink("../other", root.0.path.join("horizon-mod-data/fd/other"))
            .unwrap();
        std::os::unix::fs::symlink(
            &outside.0.path,
            root.0.path.join("horizon-mod-data/fd/outside"),
        )
        .unwrap();
        as_mod("fd", || {
            for name in ["other/secret", "outside/secret"] {
                assert_eq!(read(&root.0, &path(name)), Err(ConfigFileError::Symlink));
                assert_eq!(
                    write(&root.0, &path(name), b"bad"),
                    Err(ConfigFileError::Symlink)
                );
                assert_eq!(stat(&root.0, &path(name)), Err(ConfigFileError::Symlink));
            }
            assert!(
                list(&root.0, &ConfigPath::root(), None, 10)
                    .unwrap()
                    .is_empty()
            );
        });
        assert_eq!(fs::read(outside.0.path.join("secret")).unwrap(), b"outside");
        assert_eq!(
            fs::read(root.0.path.join("horizon-mod-data/other/secret")).unwrap(),
            b"other"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn world_bulk_files_survive_save_reload_backup_and_world_deletion() {
        use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
        use crate::game::world_loading::WorldRequest;
        if run_fresh_cases(
            "game::native_config_files::tests::world_bulk_files_survive_save_reload_backup_and_world_deletion",
            "HORIZON_BULK_FILES_CHILD",
            &["game"],
        ) {
            return;
        }
        static WARNINGS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        struct SettingsLog;
        impl log::Log for SettingsLog {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                if record.level() == log::Level::Warn
                    && record.target() == "horizon_world::native_load_context"
                {
                    WARNINGS.fetch_add(1, Ordering::Relaxed);
                }
            }
            fn flush(&self) {}
        }
        log::set_logger(&SettingsLog).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        let scratch = Root::new("world-lifecycle");
        let saves = scratch.0.path.join("saves");
        let world = saves.join("Bulk World");
        let request = |create: bool| {
            let rules = std::sync::Arc::new(horizon_client::gamerules::RuleRegistry::vanilla());
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                rules: horizon_client::gamerules::GameRules::with_registry(rules.clone(), &[]),
                ..Default::default()
            };
            let mut request = WorldRequest::new(
                "Bulk World",
                Some("Bulk World"),
                Some(1),
                true,
                create.then_some(&options),
                rules,
            )
            .unwrap();
            request.save_root = world.clone();
            request
        };
        with_game(|game| {
            game.world_root.clear();
            as_mod("fd", || {
                assert_eq!(
                    Game::world_data_files(game).read(&path("tiles/nether/0.bin")),
                    Err(ConfigFileError::NoWorld)
                );
            });
            game.load_world(request(true)).unwrap();
            as_mod("fd", || {
                let host: &dyn mod_api::ServerHost = game;
                host.data_files()
                    .write(&path("server-cache"), b"game-cache")
                    .unwrap();
                host.world_data_files()
                    .write(&path("tiles/nether/0.bin"), b"world:42")
                    .unwrap();
                assert_eq!(
                    host.world_data_files()
                        .read(&path("tiles/nether/0.bin"))
                        .unwrap(),
                    b"world:42"
                );
                let settings = Game::mod_data_root();
                fs::create_dir_all(settings.join("fd")).unwrap();
                fs::write(settings.join("fd/enabled"), b"true").unwrap();
                WARNINGS.store(0, Ordering::Relaxed);
                let captured =
                    horizon_world::native_load_context::capture_settings(&settings, ["fd"]);
                assert_eq!(captured["fd"].len(), 1);
                assert_eq!(captured["fd"]["enabled"], "true");
                assert_eq!(WARNINGS.load(Ordering::Relaxed), 0);
            });
            game.save_game().unwrap();
        });
        with_game(|game| {
            game.load_world(request(false)).unwrap();
            as_mod("fd", || {
                let host: &dyn mod_api::ServerHost = game;
                assert_eq!(
                    host.world_data_files()
                        .read(&path("tiles/nether/0.bin"))
                        .unwrap(),
                    b"world:42"
                );
                assert_eq!(
                    host.world_data_files()
                        .list(&path("tiles/nether"), None, 10)
                        .unwrap()[0]
                        .name,
                    "0.bin"
                );
            });
        });
        let (backup, _) =
            horizon_client::title::world_management::backup(&saves, "Bulk World").unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(backup).unwrap()).unwrap();
        let mut bytes = Vec::new();
        zip.by_name("Bulk World/horizon-mod-data/fd/tiles/nether/0.bin")
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"world:42");
        horizon_client::title::world_management::delete(&saves, "Bulk World").unwrap();
        assert!(!world.exists());
        as_mod("fd", || {
            assert_eq!(
                Game::data_files().read(&path("server-cache")).unwrap(),
                b"game-cache"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn compiled_wasm_bulk_files_use_actual_game_filesystem_roots() {
        use crate::game::native_entity_refs::tests::run_fresh_cases;
        use crate::game::native_guest_turn_tests::with_guests_at;
        if run_fresh_cases(
            "game::native_config_files::tests::compiled_wasm_bulk_files_use_actual_game_filesystem_roots",
            "HORIZON_WASM_BULK_FILES_CHILD",
            &["game"],
        ) {
            return;
        }
        horizon_guest_stack::run_on_guest_stack(|| {
            horizon_mod_wasm::copy_probe::with_load_budget(|| {
                use horizon_mod_package::manifest::Package;
                use horizon_mod_wasm::{WasmPackage, load};
                let package_path = PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
                let mut manifest = Package::read(&package_path).unwrap().manifest;
                manifest.inject.clear();
                let info = mod_api::PackageInfo::new(
                    manifest.id.clone(),
                    manifest.version.clone(),
                    mod_api::PackageKind::Code,
                    mod_api::Side::Both,
                    vec![manifest.id.clone()],
                    vec![],
                );
                let loaded = load(
                    &fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
                    vec![WasmPackage {
                        manifest,
                        module: Package::read_wasm(&package_path).unwrap(),
                    }],
                );
                assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
                loaded.initialize(&[info]).unwrap();
                let mods: mod_api::Mods = loaded.mods.into();
                mod_platform::testing::with_mods(mods.clone(), || {
                    crate::game::api_values::install_parts(&mods).unwrap();
                    let base = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
                    let root =
                        super::super::mods::content_root(&base, &Default::default()).unwrap();
                    with_guests_at(root.prepared().to_owned(), |game| {
                        let world = Root::new("wasm-world");
                        game.world_root = world.0.path.clone();
                        game.set_allow_commands(true);
                        game.build_registries();
                        game.register_native_commands();
                        assert!(game.native_run_command("wasmbulkfiles"));
                        let game_dir = Game::game_dir();
                        assert_eq!(
                            fs::read(game_dir.join("config/wasm_fixture/a.bin")).unwrap(),
                            [0]
                        );
                        assert_eq!(
                            fs::read(
                                game_dir.join("horizon-mod-data/wasm_fixture/wasm_fixture/a.bin")
                            )
                            .unwrap(),
                            [1, 1]
                        );
                        assert_eq!(
                            fs::read(
                                world
                                    .0
                                    .path
                                    .join("horizon-mod-data/wasm_fixture/wasm_fixture/a.bin")
                            )
                            .unwrap(),
                            [2, 2, 2]
                        );
                        as_mod("wasm_fixture", || {
                            let host: &dyn mod_api::ServerHost = game;
                            assert_eq!(
                                host.data_files().read(&path("wasm_fixture/a.bin")).unwrap(),
                                [1, 1]
                            );
                            assert_eq!(
                                host.world_data_files()
                                    .read(&path("wasm_fixture/a.bin"))
                                    .unwrap(),
                                [2, 2, 2]
                            );
                        });
                        as_mod("other", || {
                            assert_eq!(
                                Game::data_files().read(&path("wasm_fixture/a.bin")),
                                Err(ConfigFileError::NotFound)
                            );
                        });
                        game.world_root.clear();
                        assert!(game.native_run_command("wasmbulkfilesnoworld"));
                    });
                });
            });
        });
    }
}
