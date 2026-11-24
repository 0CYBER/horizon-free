//! Instances: separate game folders, each with its own worlds, options and packs.
//!
//! Nothing that exists moves. The game's home (`backend::game_home`) stays the default instance,
//! "Horizon", with its saves and options where they are. Imported modpacks (`ports/<pack>/home/`)
//! and registry modpacks (`horizon-packs/<id>/home/`) are instances too, played from their own
//! folders as before. New instances get `instances/<slug>/` in the game's home. What the player
//! chooses about an instance (its name, icon, group) is kept in `instances.json` in the
//! launcher's data folder, so renaming one never moves a folder.
use crate::backend;
use crate::browser::horizon::pack as registry_pack;
use crate::launch::Launch;
use crate::{modpacks, nightly};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The default instance's id: the game's home itself.
pub const DEFAULT: &str = "horizon";
const FILE: &str = "instances.json";
const IDENTITY: &str = ".horizon-instance-id";
/// Where new instances live, in the game's home.
const OWN: &str = "instances";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The game's home: what every player had before instances.
    Default,
    /// Made in the launcher: `instances/<slug>/`.
    Own,
    /// An imported Java modpack (modpacks.rs).
    Port,
    /// A modpack from Horizon's registry.
    Pack,
}

/// What an instance shows in the sidebar: a block or item texture from the player's prepared
/// files (never shipped), or a colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    /// A texture name: `grass_block` (block/ is looked in first, then item/).
    Texture(String),
    /// `#rrggbb`.
    Colour(String),
}

impl Icon {
    /// `#3fa7d6` is a colour; a texture's name is lowercase letters, digits and `_`.
    pub fn parse(text: &str) -> Result<Icon, String> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Icon::Colour(format!("#{}", hex.to_ascii_lowercase())));
            }
        } else if !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Ok(Icon::Texture(text.into()));
        }
        Err(format!(
            "{text}: give a block or item name (grass_block) or a colour (#3fa7d6)"
        ))
    }

    /// The texture's file in prepared resources, if it is a texture they hold.
    pub fn texture(&self, resources: &Path) -> Option<PathBuf> {
        let Icon::Texture(name) = self else {
            return None;
        };
        let textures = resources.join("assets/minecraft/textures");
        ["block", "item"]
            .iter()
            .map(|kind| textures.join(kind).join(format!("{name}.png")))
            .find(|p| p.is_file())
    }
}

/// An instance's own picture, in its game folder: one the player chose, or its modpack's icon.
/// Drawn instead of [`Meta::icon`] while it is there.
pub const PICTURE: &str = "icon.png";

/// The most an instance's picture file may weigh: a photo, not a film.
pub const MAX_PICTURE_FILE: u64 = 64 << 20;
/// The most a small settings text (options.txt, horizon-graphics.txt) may weigh; real ones are
/// kilobytes.
pub const MAX_TEXT_FILE: u64 = 4 << 20;
/// The most a world's level.dat may weigh, as stored (gzip); real ones are kilobytes.
pub const MAX_LEVEL_DAT: u64 = 32 << 20;

/// The file's bytes, or an error when it is larger than `max`. Instance files can come from an
/// imported pack, so a huge one must not be read whole.
pub fn read_capped(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file too large",
        ));
    }
    let mut bytes = Vec::new();
    // The bound holds even when the file grows after the size check.
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file too large",
        ));
    }
    Ok(bytes)
}

/// [`read_capped`] as text; a file that is not UTF-8 is an error as in `fs::read_to_string`.
pub fn read_text_capped(path: &Path, max: u64) -> std::io::Result<String> {
    String::from_utf8(read_capped(path, max)?)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
/// How many pixels a side a picture is kept at: twice the largest it is drawn.
pub const PICTURE_SIDE: u32 = 128;

/// Any image file's `bytes` as a picture: PNG, scaled down to fit [`PICTURE_SIDE`].
pub fn picture(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    crate::browser::images::fit_png(bytes, PICTURE_SIDE, 8192, 256 << 20).ok_or_else(|| {
        "That file isn't a picture the launcher can read (PNG, JPEG, GIF, WebP).".into()
    })
}

/// Keeps `png` (made by [`picture`]) as the picture of the instance playing in `home`, or, with
/// None, gives its picture to `to_trash`.
pub fn set_picture(
    home: &Path,
    png: Option<&[u8]>,
    to_trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let target = home.join(PICTURE);
    let Some(png) = png else {
        return if target.is_file() {
            to_trash(&target)
        } else {
            Ok(())
        };
    };
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    let part = home.join("icon.png.part");
    fs::write(&part, png).map_err(|e| format!("{}: {e}", part.display()))?;
    fs::rename(&part, &target).map_err(|e| format!("{}: {e}", target.display()))
}

/// Makes the icon at `url` (a modpack's, from the source it was installed from) the picture of the
/// instance playing in `home`, unless it has one already. Best effort: a pack without its icon
/// still plays.
pub fn adopt_icon(home: &Path, url: &str) {
    if home.join(PICTURE).exists() {
        return;
    }
    let image = crate::browser::images::fetch(&backend::data_dir(), url, PICTURE_SIDE);
    let png = image
        .ok()
        .and_then(|f| fs::read(f).ok())
        .and_then(|b| picture(b).ok());
    if let Some(png) = png {
        let _ = set_picture(home, Some(&png), &|_| Ok(()));
    }
}

// A malformed preference must not invalidate unrelated instance metadata.
fn saved_profile<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::native_mods::Profile>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

/// Which builds an instance's "latest" follows. Missing in a saved instance: Nightly, so nobody
/// is moved to an older build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    #[default]
    Nightly,
}

// An unknown channel must not invalidate unrelated instance metadata.
fn saved_channel<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Channel>, D::Error> {
    Ok(serde_json::from_value(serde_json::Value::deserialize(deserializer)?).ok())
}

/// The player's choices about one instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The Horizon version this instance plays, kept when newer ones are installed. None follows
    /// its channel's Latest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The builds "latest" means for it; missing is [`Channel::Nightly`].
    #[serde(
        default,
        deserialize_with = "saved_channel",
        skip_serializing_if = "Option::is_none"
    )]
    pub channel: Option<Channel>,
    /// This instance's saved native build choice; missing or unknown starts at Optimized.
    #[serde(
        default,
        deserialize_with = "saved_profile",
        skip_serializing_if = "Option::is_none"
    )]
    pub native_profile: Option<crate::native_mods::Profile>,
    /// Seconds played in it, every run started by the launcher (the window or `play`) added up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub played_secs: u64,
    /// When its last run ended, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<u64>,
    /// What runs around the game: a pre-launch command, a wrapper, variables (`launch`).
    #[serde(default, skip_serializing_if = "Launch::is_empty")]
    pub launch: Launch,
    /// Set for a Minecraft: Java Edition instance: it plays the real game, not Horizon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java: Option<JavaGame>,
    /// A Java instance's own settings while it plays Horizon, back when it plays Java again
    /// ([`switch`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_kept: Option<JavaGame>,
}

/// A Minecraft: Java Edition instance: the real game of one version, with the Java it runs on
/// (`crate::java` picks one when `home` is none).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaGame {
    /// The Minecraft version, as Mojang names it (`1.21.4`).
    pub minecraft: String,
    /// A Java home to run it on instead of the automatic pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    /// The most memory the game may use, in MiB (`-Xmx`); none: [`DEFAULT_MEMORY_MB`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    /// Extra JVM arguments, split on whitespace.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub jvm_args: String,
    /// The mod loader it plays with, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<crate::loaders::Loader>,
    /// The game window's size, width then height; none: the game's own 854×480.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<(u32, u32)>,
}

impl Instance {
    /// Its variables for the game started as `program` and the commands around it
    /// ([`crate::launch::instance_vars`]).
    pub fn vars(&self, program: &std::ffi::OsStr) -> crate::launch::Vars {
        let java = self.java.as_ref().map(|_| program);
        crate::launch::instance_vars(&self.name, &self.id, &self.home, java)
    }
}

impl JavaGame {
    /// What the mod browser installs for: its Minecraft version and its loader.
    pub fn target(&self) -> crate::browser::java_instance::Target {
        crate::browser::java_instance::Target {
            minecraft: self.minecraft.clone(),
            loader: self.loader.as_ref().map(|l| l.kind.id().to_string()),
        }
    }
}

/// 4 GiB: enough for vanilla and most modpacks, as other launchers start with.
pub const DEFAULT_MEMORY_MB: u32 = 4096;

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// `instances.json`: each instance's choices, by id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    instances: BTreeMap<String, Meta>,
}

impl Store {
    fn load(data: &Path) -> Result<Store, String> {
        let path = data.join(FILE);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !path.is_symlink() => {
                return Ok(Self::default());
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if !text.trim_start().starts_with('{') {
            return Err(format!("{}: expected a JSON object", path.display()));
        }
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn forget_missing(&mut self, own: &[String]) -> bool {
        let before = self.instances.len();
        self.instances
            .retain(|id, _| id == DEFAULT || id.contains(':') || own.contains(id));
        self.instances.len() != before
    }

    fn save(&self, data: &Path) -> Result<(), String> {
        fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
        // A launcher interrupted during migration must leave either all pins or the old store.
        let write = || -> std::io::Result<()> {
            let mut pending = tempfile::NamedTempFile::new_in(data)?;
            pending.write_all(&serde_json::to_vec_pretty(self).unwrap())?;
            pending.persist(data.join(FILE))?;
            Ok(())
        };
        write().map_err(|e| format!("{FILE}: {e}"))
    }
}

/// Serializes instance metadata edits and removal of builds they might use across launchers.
pub(crate) fn lock_store(data: &Path) -> Result<fs::File, String> {
    fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    backend::lock_update(&data.join("instances.lock"))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// `horizon`, an own instance's folder name, `port:<pack>` or `pack:<id>`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The folder the pack's tools work in: the game's home for Default and Own instances.
    pub work: PathBuf,
    /// The game folder it plays in: saves, options.txt, packs.
    pub home: PathBuf,
    pub icon: Option<Icon>,
    pub group: Option<String>,
    /// Its pinned Horizon version, if any (`Meta::version`).
    pub version: Option<String>,
    /// Which builds it follows when it keeps no version.
    pub channel: Channel,
    /// Its saved native build choice, or Optimized before the player chooses one.
    pub native_profile: crate::native_mods::Profile,
    pub played: Duration,
    pub last_played: Option<SystemTime>,
    /// What runs around the game (`launch::wrap`).
    pub launch: Launch,
    /// The Minecraft: Java Edition it plays, for a Java instance.
    pub java: Option<JavaGame>,
}

impl Instance {
    /// Ok when it plays Horizon; for a Java instance, which starts its own game
    /// (`backend::java_game`), that Horizon's options don't apply to it.
    fn horizon(&self) -> Result<(), String> {
        match &self.java {
            None => Ok(()),
            Some(java) => Err(format!(
                "{} plays Minecraft {} (Java Edition), not Horizon: Play starts it, without a \
                 Horizon version.",
                self.name, java.minecraft
            )),
        }
    }

    /// The Horizon version it pins when that version isn't in `installed` (deleted while the
    /// window was open, or not installed yet). Such an instance never falls back to another
    /// version: opening its worlds in one could upgrade them.
    pub fn missing_pin(&self, installed: &[String]) -> Option<&str> {
        let pin = self.version.as_deref().filter(|_| self.java.is_none())?;
        (!installed.iter().any(|t| t == pin)).then_some(pin)
    }

    /// It takes Home's Horizon update offers: it pins no version and plays Horizon, not Java (#18394).
    pub fn follows_horizon(&self) -> bool {
        self.version.is_none() && self.java.is_none()
    }

    /// The installed build this Horizon instance plays. A missing pin cannot fall back.
    pub fn installed_version(
        &self,
        installed: &[String],
        selected: Option<&str>,
    ) -> Option<String> {
        if self.java.is_some() {
            return None;
        }
        match &self.version {
            Some(pin) => installed.iter().find(|tag| *tag == pin).cloned(),
            None => nightly::latest_for(self.channel, installed, selected),
        }
    }

    /// The installed version it plays: an explicit CLI request, its pin, or its channel's Latest.
    pub fn resolve_version(&self, requested: Option<&str>) -> Result<String, String> {
        self.horizon()?;
        if let Some(tag) = requested {
            return backend::resolve_version(Some(tag));
        }
        let installed = backend::installed();
        if let Some(pinned) = self.missing_pin(&installed) {
            return Err(crate::i18n::trf(
                "{instance} plays {version}, which is not installed: install it, or horizon-launcher instance version \"{instance}\" latest",
                &[("instance", &self.name), ("version", pinned)],
            ));
        }
        let settings = backend::Settings::load();
        self.installed_version(&installed, settings.selected.as_deref())
            .ok_or_else(backend::no_version_installed)
    }
}

/// The instances, by the default game home and the launcher's data folder: Horizon first, then
/// by group and name.
pub fn list() -> Vec<Instance> {
    list_in(&backend::game_home(), &backend::data_dir())
}

fn list_in(game_home: &Path, data: &Path) -> Vec<Instance> {
    let own = own_folders(game_home).ok().flatten();
    let mut store = Store::load(data).unwrap_or_default();
    if own.as_ref().is_some_and(|own| store.forget_missing(own)) {
        let forget_missing = || {
            let _lock = lock_store(data)?;
            let mut saved = Store::load(data)?;
            if let Some(own) = own_folders(game_home)? {
                if saved.forget_missing(&own) {
                    saved.save(data)?;
                }
            }
            Ok::<_, String>(saved)
        };
        match forget_missing() {
            Ok(saved) => store = saved,
            Err(e) => eprintln!("Horizon: instances.json is not saved: {e}"),
        }
    }
    list_with_store(game_home, own.unwrap_or_default(), &store)
}

fn own_folders(game_home: &Path) -> Result<Option<Vec<String>>, String> {
    let path = game_home.join(OWN);
    let read = || -> std::io::Result<Option<Vec<String>>> {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !path.is_symlink() => {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry?;
            let metadata = match fs::metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if metadata.is_dir() {
                if let Ok(name) = entry.file_name().into_string() {
                    names.push(name);
                }
            }
        }
        names.sort();
        Ok(Some(names))
    };
    read().map_err(|e| format!("{}: {e}", path.display()))
}

fn list_with_store(game_home: &Path, own: Vec<String>, store: &Store) -> Vec<Instance> {
    list_with_packs(
        game_home,
        own,
        store,
        modpacks::listed_in(&game_home.join("ports")),
        registry_pack::list_in(&game_home.join("horizon-packs")),
    )
}

fn list_checked_with_store(game_home: &Path, store: &Store) -> Result<Vec<Instance>, String> {
    Ok(list_with_packs(
        game_home,
        own_folders(game_home)?.unwrap_or_default(),
        store,
        modpacks::listed_checked_in(&game_home.join("ports"))?,
        registry_pack::list_checked_in(&game_home.join("horizon-packs"))?,
    ))
}

fn list_with_packs(
    game_home: &Path,
    own: Vec<String>,
    store: &Store,
    ports: Vec<modpacks::Listed>,
    packs: Vec<registry_pack::Installed>,
) -> Vec<Instance> {
    let mut out = vec![instance_at(
        store,
        DEFAULT.into(),
        "Horizon".into(),
        Kind::Default,
        game_home.to_path_buf(),
        game_home.to_path_buf(),
    )];
    out.extend(own.into_iter().map(|slug| {
        let dir = game_home.join(OWN).join(&slug);
        instance_at(store, slug.clone(), slug, Kind::Own, dir.clone(), dir)
    }));
    out.extend(ports.into_iter().map(|pack| port_instance(store, pack)));
    out.extend(packs.into_iter().map(|pack| registry_instance(store, pack)));
    out[1..].sort_by_key(|i| (i.group.clone(), i.name.to_lowercase()));
    out
}

fn port_instance(store: &Store, pack: modpacks::Listed) -> Instance {
    let home = modpacks::profile_home(&pack.work);
    instance_at(
        store,
        format!("port:{}", pack.slug),
        pack.title,
        Kind::Port,
        pack.work,
        home,
    )
}

fn registry_instance(store: &Store, pack: registry_pack::Installed) -> Instance {
    let home = registry_pack::home(&pack.work);
    instance_at(
        store,
        format!("pack:{}", pack.id),
        pack.name,
        Kind::Pack,
        pack.work,
        home,
    )
}

fn instance_at(
    store: &Store,
    id: String,
    name: String,
    kind: Kind,
    work: PathBuf,
    home: PathBuf,
) -> Instance {
    let meta = store.instances.get(&id).cloned().unwrap_or_default();
    Instance {
        name: meta.name.unwrap_or(name),
        icon: meta.icon,
        group: meta.group,
        version: meta.version,
        channel: meta.channel.unwrap_or_default(),
        native_profile: meta
            .native_profile
            .unwrap_or(crate::native_mods::Profile::Optimized),
        played: Duration::from_secs(meta.played_secs),
        last_played: meta
            .last_played
            .map(|d| UNIX_EPOCH + Duration::from_secs(d)),
        launch: meta.launch,
        java: meta.java,
        id,
        kind,
        work,
        home,
    }
}

/// The instance an id, a name (any case) or a pack's folder name names.
pub fn find(query: &str) -> Result<Instance, String> {
    find_in(&list(), query)
}

/// Read saved metadata before launching. A damaged store cannot select an unpinned default.
pub fn for_launch(query: Option<&str>) -> Result<Instance, String> {
    let all = launch_instances()?;
    match query {
        Some(query) => find_in(&all, query),
        None => {
            let settings = backend::Settings::load_checked()?;
            let id = settings.instance.as_deref().unwrap_or(DEFAULT);
            all.into_iter()
                .find(|i| i.id == id)
                .ok_or_else(|| instance_unavailable(id))
        }
    }
}

/// A window's Play button targets the exact instance it displayed.
pub fn for_launch_id(id: &str) -> Result<Instance, String> {
    launch_instances()?
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| instance_unavailable(id))
}

fn instance_unavailable(id: &str) -> String {
    crate::i18n::trf(
        "Instance {instance} is no longer available.",
        &[("instance", id)],
    )
}

/// Reads the requested pack's manifest and saved instance metadata before it can be played.
pub fn pack_for_launch(work: &Path) -> Result<Instance, String> {
    let store = load_launch_store()?;
    let parent = work.parent();
    if parent == Some(modpacks::ports_dir().as_path()) {
        return Ok(port_instance(
            &store,
            modpacks::read_listed(work.to_owned())?,
        ));
    }
    if parent == Some(registry_pack::packs_dir().as_path()) {
        return Ok(registry_instance(&store, registry_pack::read_record(work)?));
    }
    Err(instance_unavailable(&work.display().to_string()))
}

fn load_launch_store() -> Result<Store, String> {
    Store::load(&backend::data_dir()).map_err(|e| {
        crate::i18n::trf(
            "Could not read instance settings: {error}",
            &[("error", &e)],
        )
    })
}

fn launch_instances() -> Result<Vec<Instance>, String> {
    let game_home = backend::game_home();
    let store = load_launch_store()?;
    Ok(list_with_store(
        &game_home,
        own_folders(&game_home)?.unwrap_or_default(),
        &store,
    ))
}

fn find_in(all: &[Instance], query: &str) -> Result<Instance, String> {
    let q = query.trim();
    let bare = |id: &str| id.split_once(':').map_or(id, |(_, rest)| rest).to_string();
    all.iter()
        .find(|i| i.id == q)
        .or_else(|| all.iter().find(|i| i.name.eq_ignore_ascii_case(q)))
        .or_else(|| all.iter().find(|i| bare(&i.id) == q))
        .cloned()
        .ok_or_else(|| format!("No instance {q}: horizon-launcher instance list shows them"))
}

/// Moves an instance's folder to the Trash (where the player can get it back, worlds and all)
/// and forgets it: its choices, and the selection when Play started it. Horizon's own instance,
/// the game's home, stays.
pub fn delete(query: &str) -> Result<Instance, String> {
    let (gone, saved) = delete_with_selection(query)?;
    saved?;
    Ok(gone)
}

/// A completed deletion stays completed even if saving the new selection fails.
pub fn delete_with_selection(query: &str) -> Result<(Instance, Result<(), String>), String> {
    let gone = delete_in(
        &backend::game_home(),
        &backend::data_dir(),
        query,
        &crate::desktop_seams::to_trash,
    )?;
    let mut settings = backend::Settings::load();
    let saved = if settings.instance.as_ref() == Some(&gone.id) {
        settings.instance = None;
        settings.save()
    } else {
        Ok(())
    };
    crate::storage::eviction::request_cleanup();
    Ok((gone, saved))
}

fn delete_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<Instance, String> {
    let _lock = lock_store(data)?;
    let mut store = Store::load(data)?;
    let own = own_folders(game_home)?.unwrap_or_default();
    let instance = find_in(&list_with_store(game_home, own, &store), query)?;
    if instance.kind == Kind::Default {
        return Err(format!(
            "{} is the game's own folder: it can't be deleted.",
            instance.name
        ));
    }
    trash(&instance.work)
        .map_err(|e| format!("{} could not go to the Trash: {e}", instance.name))?;
    if store.instances.remove(&instance.id).is_some() {
        store.save(data)?;
    }
    Ok(instance)
}

/// The instance Play starts: the one picked in the launcher, else Horizon.
pub fn selected() -> Instance {
    selected_with(&backend::Settings::load())
}

pub(crate) fn selected_with(settings: &backend::Settings) -> Instance {
    selected_in(&list(), settings).clone()
}

fn selected_in<'a>(all: &'a [Instance], settings: &backend::Settings) -> &'a Instance {
    all.iter()
        .find(|i| Some(i.id.as_str()) == settings.instance.as_deref())
        .unwrap_or(&all[0])
}

/// Use an installed build for the instance Play starts. Choosing the channel's Latest follows it.
pub fn use_version(tag: &str) -> Result<(), String> {
    use_version_in(None, None, tag).map(|_| ())
}

/// Use a build for the named instance, even if another launcher changed the selected instance.
pub fn use_version_for(id: &str, tag: &str) -> Result<(), String> {
    match use_version_in(Some(id), None, tag)? {
        true => Ok(()),
        false => Err(instance_unavailable(id)),
    }
}

/// The instance that started a manual download. Recreated folders have their own identity.
pub struct ManualInstallTarget {
    id: String,
    identity: Option<[u8; 16]>,
}

/// Captures the instance's identity before downloading. The default instance cannot be deleted.
pub fn manual_install_target(id: String) -> Result<ManualInstallTarget, String> {
    let identity = if id == DEFAULT {
        None
    } else {
        let _lock = lock_store(&backend::data_dir())?;
        launch_instances()?
            .into_iter()
            .find(|instance| instance.id == id && instance.java.is_none())
            .map(|instance| ensure_identity(&instance.work))
            .and_then(|identity| identity.inspect_err(|e| eprintln!("{e}")).ok())
    };
    Ok(ManualInstallTarget { id, identity })
}

fn read_identity(file: &Path) -> std::io::Result<Option<[u8; 16]>> {
    match fs::read(file) {
        Ok(bytes) => bytes.try_into().map(Some).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid instance identity")
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn ensure_identity(work: &Path) -> Result<[u8; 16], String> {
    let path = work.join(IDENTITY);
    let ensure = || -> std::io::Result<[u8; 16]> {
        if let Some(existing) = read_identity(&path)? {
            return Ok(existing);
        }
        let mut identity = [0; 16];
        getrandom::fill(&mut identity).map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut pending = tempfile::NamedTempFile::new_in(work)?;
        pending.write_all(&identity)?;
        match pending.persist_noclobber(&path) {
            Ok(_) => Ok(identity),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                read_identity(&path)?.ok_or(e.error)
            }
            Err(e) => Err(e.error),
        }
    };
    ensure().map_err(|e| format!("{}: {e}", path.display()))
}

/// Selects a manual download for the original instance if it still exists.
pub fn finish_manual_install(target: &ManualInstallTarget, tag: &str) -> Result<(), String> {
    use_version_in(Some(&target.id), Some(target), tag).map(|_| ())
}

fn use_version_in(
    id: Option<&str>,
    target: Option<&ManualInstallTarget>,
    tag: &str,
) -> Result<bool, String> {
    let data = backend::data_dir();
    let _lock = lock_store(&data)?;
    let settings = backend::Settings::load_checked()?;
    let mut store = Store::load(&data)?;
    let game_home = backend::game_home();
    let all = list_with_store(
        &game_home,
        own_folders(&game_home)?.unwrap_or_default(),
        &store,
    );
    let instance = match id {
        Some(id) => all.iter().find(|i| i.id == id),
        None => Some(selected_in(&all, &settings)),
    };
    let Some(instance) = instance else {
        return Ok(false);
    };
    if instance.java.is_some() {
        return Ok(true);
    }
    if let Some(target) = target.filter(|_| instance.id != DEFAULT) {
        let file = instance.work.join(IDENTITY);
        if target.identity.is_none()
            || read_identity(&file).map_err(|e| format!("{}: {e}", file.display()))?
                != target.identity
        {
            return Ok(false);
        }
    }
    let installed = backend::installed_checked()?;
    if !installed.iter().any(|installed| installed == tag) {
        return Err(backend::version_gone(tag));
    }
    let latest = nightly::latest_for(instance.channel, &installed, settings.selected.as_deref());
    let pin = (latest.as_deref() != Some(tag)).then(|| tag.to_owned());
    if instance.version != pin {
        store
            .instances
            .entry(instance.id.clone())
            .or_default()
            .version = pin;
        store.save(&data)?;
    }
    Ok(true)
}

/// Instances' pins and resolved builds that must survive cleanup. A failed read must veto removal.
fn used_versions(
    installed: &[String],
    settings: &backend::Settings,
) -> Result<impl Iterator<Item = (String, String)>, String> {
    let game_home = backend::game_home();
    let store = Store::load(&backend::data_dir())?;
    let all = list_checked_with_store(&game_home, &store)?;
    let mut used = Vec::new();
    for instance in all {
        if let Some(tag) = instance
            .version
            .clone()
            .or_else(|| instance.installed_version(installed, settings.selected.as_deref()))
        {
            used.push((tag, instance.name));
        }
    }
    // Keep saved pins if a removed pack's folder is put back later.
    used.extend(
        store
            .instances
            .into_iter()
            .filter(|(id, _)| id.contains(':'))
            .filter_map(|(id, meta)| Some((meta.version?, meta.name.unwrap_or(id)))),
    );
    // A launcher's old selection can still be the only record of which build a world used.
    if !settings.instance_versions_migrated {
        if let Some(tag) = &settings.selected {
            used.push((tag.clone(), crate::i18n::tr("Horizon").into()));
        }
    }
    Ok(used.into_iter())
}

pub(crate) fn protected_versions(
    installed: &[String],
    settings: &backend::Settings,
) -> Result<Vec<String>, String> {
    Ok(used_versions(installed, settings)?
        .map(|(tag, _)| tag)
        .collect())
}

pub(crate) fn version_user(installed: &[String], tag: &str) -> Result<Option<String>, String> {
    let settings = backend::Settings::load_checked()?;
    Ok(used_versions(installed, &settings)?
        .find(|(used, _)| used == tag)
        .map(|(_, name)| name))
}

/// Preserve what old launchers played before Latest became independent of their global pick.
pub fn absorb_held_selection() -> Result<(), String> {
    if backend::Settings::load_checked()?.instance_versions_migrated {
        return Ok(());
    }
    let data = backend::data_dir();
    let _lock = lock_store(&data)?;
    let mut settings = backend::Settings::load_checked()?;
    if settings.instance_versions_migrated {
        return Ok(());
    }
    let installed = backend::installed_checked()?;
    settings.adopt_legacy_selection(&installed);
    absorb_held_selection_in(&backend::game_home(), &data, &installed, &mut settings)?;
    settings.save()
}

fn absorb_held_selection_in(
    game_home: &Path,
    data: &Path,
    installed: &[String],
    settings: &mut backend::Settings,
) -> Result<(), String> {
    if settings.instance_versions_migrated {
        return Ok(());
    }
    let old_selection = installed
        .iter()
        .find(|t| Some(t.as_str()) == settings.selected.as_deref())
        .or_else(|| installed.first());
    let newest = nightly::latest_for(Channel::Nightly, installed, None);
    let mut store = Store::load(data)?;
    let instances = list_checked_with_store(game_home, &store)?;
    let mut changed = false;
    for instance in instances.into_iter().filter(Instance::follows_horizon) {
        let old = match instance.channel {
            Channel::Stable => {
                nightly::legacy_stable_latest(installed, old_selection.map(String::as_str))
            }
            Channel::Nightly => None,
        }
        .or_else(|| old_selection.cloned());
        let latest = nightly::latest_for(instance.channel, installed, newest.as_deref());
        if old != latest {
            store.instances.entry(instance.id).or_default().version = old;
            changed = true;
        }
    }
    if changed {
        store.save(data)?;
    }
    settings.selected = newest.clone();
    settings.following = newest;
    settings.instance_versions_migrated = true;
    Ok(())
}

pub fn select(query: &str) -> Result<Instance, String> {
    let instance = find(query)?;
    let mut settings = backend::Settings::load();
    settings.instance = (instance.kind != Kind::Default).then(|| instance.id.clone());
    settings.save()?;
    Ok(instance)
}

/// The game's cache folder in each instance; the game rebuilds it on start.
const GAME_CACHE: &str = ".horizon-cache";

/// Where the game makes its cache under `game_home`: directly in each instance's home, never
/// deeper, since a world may carry the same name. Every instance, port and pack folder counts,
/// whether or not its record reads. Links are never followed: the copy refuses them afterwards.
pub(crate) fn game_caches(game_home: &Path) -> Vec<PathBuf> {
    let plain_dir = |dir: &Path| {
        fs::symlink_metadata(dir).is_ok_and(|meta| {
            meta.is_dir() && crate::data_location::reject_link(dir, &meta).is_ok()
        })
    };
    let children = |dir: PathBuf| {
        (plain_dir(game_home) && plain_dir(&dir))
            .then(|| fs::read_dir(dir).ok())
            .flatten()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
    };
    std::iter::once(game_home.to_path_buf())
        .chain(children(game_home.join(OWN)))
        .chain(children(game_home.join("ports")).map(|w| modpacks::profile_home(&w)))
        .chain(children(game_home.join("horizon-packs")).map(|w| registry_pack::home(&w)))
        .map(|home| home.join(GAME_CACHE))
        .collect()
}

/// What a copy of a game folder leaves out: other instances and packs, logs and crash reports,
/// caches, data collection's state, and sign-in sessions (a session never leaves its folder).
const NOT_COPIED: &[&str] = &[
    OWN,
    IDENTITY,
    "ports",
    "horizon-packs",
    "logs",
    "crash-reports",
    "sessions",
    "mod-browser",
    GAME_CACHE,
    // Far terrain's copies of what servers sent, rebuilt as the instance plays.
    "server-terrain",
];

/// Files of the player, not of one instance, that a new instance starts with: the keys and video
/// settings, and the offline name (so the player stays the same player in its worlds).
const PLAYER_FILES: &[&str] = &["options.txt", "horizon-player.txt"];

/// Creates an instance named `name`: empty but for the player's options and offline name, or a
/// copy of `from`'s game folder (its worlds, options and packs). Nothing of `from` changes.
pub fn create(name: &str, from: Option<&str>) -> Result<Instance, String> {
    let from = from.map(find).transpose()?;
    create_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
        None,
    )
}

/// Creates a Horizon instance with its pin. The version is checked under the same lock as
/// deletion, before any files are copied or the instance is published.
pub fn create_with_installation(
    name: &str,
    from: Option<&str>,
    pin: Option<String>,
    channel: Channel,
    downloading: Option<&str>,
) -> Result<Instance, String> {
    let from = from.map(find).transpose()?;
    create_checked_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
        None,
        |meta| {
            validate_new_pin(pin.as_deref(), downloading)?;
            meta.version = pin;
            meta.channel = Some(channel);
            Ok(())
        },
    )
}

/// Creates a Minecraft: Java Edition instance named `name` playing `minecraft` (a version id
/// from Mojang's list), with an empty game folder: the real game keeps its own options.
pub fn create_java(name: &str, minecraft: &str) -> Result<Instance, String> {
    create_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        None,
        Some(minecraft),
    )
}

/// What creating or renaming an instance says when another already has `name`.
pub fn name_taken(name: &str) -> String {
    crate::i18n::trf(
        "There is already an instance called {name}. Choose another name.",
        &[("name", name)],
    )
}

/// `name_taken` for `instance copy-from`, whose flag for another name is `--as`.
pub fn name_taken_with_flag(name: &str) -> String {
    format!("{} (`--as NAME`)", name_taken(name))
}

fn create_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
    minecraft: Option<&str>,
) -> Result<Instance, String> {
    create_checked_in(game_home, data, name, from, minecraft, |_| Ok(()))
}

fn create_checked_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
    minecraft: Option<&str>,
    change: impl FnOnce(&mut Meta) -> Result<(), String>,
) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An instance needs a name.".into());
    }
    let _lock = lock_store(data)?;
    let mut store = Store::load(data)?;
    let own = own_folders(game_home)?;
    if let Some(own) = &own {
        store.forget_missing(own);
    }
    let all = list_with_store(game_home, own.unwrap_or_default(), &store);
    if all.iter().any(|i| i.name.eq_ignore_ascii_case(name)) {
        return Err(name_taken(name));
    }
    if let Some(from) = from.filter(|f| matches!(f.kind, Kind::Port | Kind::Pack)) {
        return Err(format!(
            "{} plays its pack's mods: import the pack again for a second copy.",
            from.name
        ));
    }
    let base = modpacks::slug(name);
    let taken = |s: &str| s == DEFAULT || game_home.join(OWN).join(s).exists();
    let slug = (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|s| !taken(s))
        .unwrap();
    let meta = store.instances.entry(slug.clone()).or_default();
    meta.name = (name != slug).then(|| name.to_string());
    meta.icon = from.and_then(|f| f.icon.clone());
    meta.group = from.and_then(|f| f.group.clone());
    meta.java = from.and_then(|f| f.java.clone()).or_else(|| {
        minecraft.map(|v| JavaGame {
            minecraft: v.to_string(),
            ..JavaGame::default()
        })
    });
    change(meta)?;
    let dir = game_home.join(OWN).join(&slug);
    let made = match from {
        Some(from) => copy_home(&from.home, &dir),
        None if minecraft.is_some() => fs::create_dir_all(&dir),
        None => fs::create_dir_all(&dir).and_then(|()| {
            for f in PLAYER_FILES {
                let own = game_home.join(f);
                if own.is_file() {
                    fs::copy(own, dir.join(f))?;
                }
            }
            Ok(())
        }),
    };
    if let Err(e) = made {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("Could not make {}: {e}", dir.display()));
    }
    crate::run_leases::publish(&dir).map_err(crate::native_mods::cache_error)?;
    store.save(data)?;
    find_in(
        &list_with_store(
            game_home,
            own_folders(game_home)?.unwrap_or_default(),
            &store,
        ),
        &slug,
    )
}

fn copy_home(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if NOT_COPIED.iter().any(|n| name == *n) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Changes what the player chose about an instance: its name, icon or group. No folder moves.
pub fn edit(query: &str, change: impl FnOnce(&mut Meta)) -> Result<Instance, String> {
    edit_in(&backend::game_home(), &backend::data_dir(), query, change)
}

fn edit_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    edit_checked_in(game_home, data, query, |meta| {
        change(meta);
        Ok(())
    })
}

/// Sets a CLI instance pin, validating the requested install under the same lock as deletion.
pub fn pin_installed_version(query: &str, pin: Option<&str>) -> Result<Instance, String> {
    edit_checked_in(&backend::game_home(), &backend::data_dir(), query, |meta| {
        meta.version = pin
            .map(|tag| backend::resolve_version(Some(tag)))
            .transpose()?;
        Ok(())
    })
}

/// Applies a dialog's edits and verifies a new pin before committing them. An absent build can
/// be pinned only when the dialog has a download ready for it.
pub fn edit_installation(
    query: &str,
    downloading: Option<&str>,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    edit_checked_in(&backend::game_home(), &backend::data_dir(), query, |meta| {
        let before = meta.version.clone();
        change(meta);
        if meta.version != before {
            validate_new_pin(meta.version.as_deref(), downloading)?;
        }
        Ok(())
    })
}

fn validate_new_pin(pin: Option<&str>, downloading: Option<&str>) -> Result<(), String> {
    if let Some(tag) = pin.filter(|tag| Some(*tag) != downloading) {
        if !backend::installed_checked()?
            .iter()
            .any(|installed| installed == tag)
        {
            return Err(backend::version_gone(tag));
        }
    }
    Ok(())
}

fn edit_checked_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta) -> Result<(), String>,
) -> Result<Instance, String> {
    let _lock = lock_store(data)?;
    let mut store = Store::load(data)?;
    let own = own_folders(game_home)?.unwrap_or_default();
    let all = list_with_store(game_home, own.clone(), &store);
    let instance = find_in(&all, query)?;
    let meta = store.instances.entry(instance.id.clone()).or_default();
    change(meta)?;
    meta.name = meta.name.take().map(|n| n.trim().to_string());
    meta.group = meta
        .group
        .take()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    if let Some(name) = &meta.name {
        if name.is_empty() {
            return Err("An instance needs a name.".into());
        }
        // Only a new name is checked: an import can share a name, and changing anything
        // else about it must still work.
        let renamed = !name.eq_ignore_ascii_case(&instance.name);
        if renamed
            && all
                .iter()
                .any(|i| i.id != instance.id && i.name.eq_ignore_ascii_case(name))
        {
            return Err(name_taken(name));
        }
    }
    store.save(data)?;
    find_in(&list_with_store(game_home, own, &store), &instance.id)
}

/// Adds a finished run of the game to the instance's play time (by id: a renamed instance keeps
/// it).
pub fn record_play(id: &str, played: Duration) -> Result<(), String> {
    record_play_in(&backend::data_dir(), id, played, SystemTime::now())
}

fn record_play_in(
    data: &Path,
    id: &str,
    played: Duration,
    ended: SystemTime,
) -> Result<(), String> {
    let _lock = lock_store(data)?;
    let mut store = Store::load(data)?;
    let meta = store.instances.entry(id.to_string()).or_default();
    meta.played_secs = meta.played_secs.saturating_add(played.as_secs());
    meta.last_played = ended.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    store.save(data)
}

/// "12 min", "3 h 05 min", "140 h": the play time as players read it.
pub fn play_time(played: Duration) -> String {
    let minutes = played.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => crate::i18n::tr("under a minute").into(),
        (0, m) => format!("{m} min"),
        (h, m) if h < 100 => format!("{h} h {m:02} min"),
        (h, _) => format!("{h} h"),
    }
}

/// Turns the game's command into this instance: its own game folder, and a pack's mods. The
/// player's answers that hold everywhere follow it in: data collection and No AI as the
/// launcher has them, and options.txt and the offline name when it has none yet.
pub fn apply(cmd: &mut Command, instance: &Instance) -> Result<(), String> {
    instance.horizon()?;
    match instance.kind {
        Kind::Default => return Ok(()),
        Kind::Own => {
            fs::create_dir_all(&instance.home)
                .map_err(|e| format!("{}: {e}", instance.home.display()))?;
            cmd.current_dir(&instance.home)
                .env("HORIZON_HOME", &instance.home);
        }
        Kind::Port | Kind::Pack => modpacks::apply_profile(cmd, &instance.work)?,
    }
    follow_player(&backend::game_home(), &instance.home);
    Ok(())
}

fn follow_player(game_home: &Path, home: &Path) {
    for f in PLAYER_FILES {
        if !home.join(f).exists() && game_home.join(f).is_file() {
            let _ = fs::copy(game_home.join(f), home.join(f));
        }
    }
    let consent = horizon_telemetry::consent::Consent::path(game_home);
    if consent.is_file() {
        let to = horizon_telemetry::consent::Consent::path(home);
        let _ = fs::create_dir_all(to.parent().unwrap());
        let _ = fs::copy(consent, to);
    }
    let no_ai = horizon_platform::no_ai::is_on(game_home);
    if horizon_platform::no_ai::is_on(home) != no_ai {
        let _ = horizon_platform::no_ai::set(home, no_ai);
    }
}

#[path = "instances/switch.rs"]
mod switch;
pub use switch::{switch, unkept, Plays, Unkept};

#[cfg(test)]
#[path = "instances/tests.rs"]
mod tests;
