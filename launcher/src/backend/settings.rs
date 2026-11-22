//! The launcher's settings (`settings.json`): what the Settings page and `horizon-launcher settings` change.
use super::*;

/// The launcher's appearance, as older launchers saved it: a theme is one look now, and a saved
/// `light` opens on the Taiga theme (`Settings::parse`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow the system's light or dark.
    System,
}

/// What the launcher's window does while the game runs, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhilePlaying {
    /// The window stays where it is.
    #[default]
    Stay,
    /// The window goes to the Dock or the taskbar, and comes back when the game closes.
    Minimize,
    /// The launcher hides (macOS only: elsewhere it minimises), and comes back when the game
    /// closes.
    Hide,
}

impl WhilePlaying {
    pub const ALL: [WhilePlaying; 3] = [Self::Stay, Self::Minimize, Self::Hide];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stay => "stay",
            Self::Minimize => "minimize",
            Self::Hide => "hide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = match name.as_str() {
            "minimise" => "minimize",
            "keep" | "open" | "off" => "stay",
            n => n,
        };
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// What this computer does for it: only macOS can hide a whole app.
    pub fn here(self) -> Self {
        match self {
            Self::Hide if !cfg!(target_os = "macos") => Self::Minimize,
            w => w,
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Compatibility selection for older launchers and for channels' fallback rules.
    pub selected: Option<String>,
    /// Compatibility record of the last version followed by the launcher.
    #[serde(default)]
    pub following: Option<String>,
    /// Older settings may hold Latest on a global pick. Once absorbed, a later install can
    /// advance Latest without turning the former build into a pin at the next start.
    #[serde(default)]
    pub instance_versions_migrated: bool,
    /// Game asset architecture. Auto preserves the launcher's process-architecture behavior.
    #[serde(default)]
    pub game_architecture: GameArchitecture,
    /// The instance Play starts (`instances`), by id. Unset means Horizon, the game's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
    /// Unset means on: only a player who turned it off has `false` here. 0.1.1 wrote its
    /// `auto_install` (off by default) for everyone, so this is a new key and that one is ignored.
    #[serde(default)]
    pub install_new_versions: Option<bool>,
    /// Unset means on: after an install the launcher deletes the release versions nothing needs
    /// (`prune_old_versions`). Only a player who turned it off has `false` here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prune_old_versions: Option<bool>,
    /// How many of the newest release versions pruning keeps. Unset means [`KEEP_NEWEST`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_versions: Option<usize>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// The proxy every download goes through (`proxy`), such as `http://127.0.0.1:7890` or
    /// `socks5://127.0.0.1:1080`.
    /// Unset means none: the environment's `HTTPS_PROXY` and `HTTP_PROXY` apply instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Only read to move an old light appearance to the Taiga theme (`parse`): always dark now,
    /// a theme being one look.
    #[serde(default)]
    pub appearance: Appearance,
    /// The player has read what the "I'm playing" ping holds (`telemetry::PING`): on the
    /// first-run question, or on the Play page once if they answered that before it existed.
    #[serde(default)]
    pub ping_notice_seen: bool,
    /// The player turned the anonymous "I'm playing" ping off (`telemetry::PING`): the game is
    /// started with `HORIZON_NO_HEARTBEAT`. On by default, as upstream sends it.
    #[serde(default)]
    pub ping_off: bool,
    /// The look's flourishes, each on or off (`effects`): glow, gradients, the hover shine.
    #[serde(default)]
    pub effects: crate::effects::Effects,
    /// The player has been through the first-run welcome (theme, accent, effects).
    #[serde(default)]
    pub welcomed: bool,
    /// The player without an account that owns the game has read, once, why the launcher needs
    /// one now (the owner gate's screen).
    #[serde(default)]
    pub owner_notice_seen: bool,
    /// Seconds a partner banner (the sidebar's card or the Servers page's banner) has been on
    /// screen, over every start, up to `partner::SEEN_BEFORE_HIDE`: only then can the player hide it.
    #[serde(default)]
    pub partner_seen_secs: u64,
    /// The player hid a partner banner: the sidebar's card and the Servers page's banner are both gone for good.
    #[serde(default)]
    pub partner_hidden: bool,
    /// What the window does while the game runs. Stays by default.
    #[serde(default)]
    pub while_playing: WhilePlaying,
    /// The accent colour on top of the appearance (`themes::Accent`). Horizon's violet by default.
    #[serde(default)]
    pub accent: crate::themes::Accent,
    /// How much white the glass look's plates take, in percent (the mockup's Glass, 0 to 16).
    /// Unset is the mockup's 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<f32>,
    /// How much the glass look's plates blur the scene behind them, in px (0 to 48). Unset is
    /// the mockup's 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blur: Option<f32>,
    /// The launcher theme (`themes::theme`): its sky behind the light appearance. Unset is
    /// Horizon's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The cheaper scene for older computers (a lower-resolution sky, a box blur, the scene
    /// following the mouse less often and resizes baked in the background). Unset is off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance: Option<bool>,
    /// Home's background: a picture the player picked, kept in the data folder under this name
    /// (`themes::keep_background`). Unset shows Horizon's landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The window's language. Unset follows the system's (`i18n::Language::system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::i18n::Language>,
    /// Whether what the browser installed from Horizon's registry updates by itself, at start
    /// and before Play (`browser::horizon::auto`). Unset is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mods_auto_update: Option<bool>,
    /// First-use native code permission for this launcher install, Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_mods_allowed_at: Option<u64>,
    /// Managed cache limit in GiB (unset: 25; zero: no automatic cleanup). Player data isn't cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_limit_gib: Option<u64>,
    /// The accent picked under each launcher theme, by its id (`themes::HORIZON` for Horizon's
    /// own), so going back to a theme brings back its colour. A launcher older than this map
    /// drops it when it saves: the themes then only start from their own accents again.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub theme_accents: std::collections::BTreeMap<String, crate::themes::Accent>,
    /// Reduced motion: unset follows the system's, true keeps the launcher still, false lets it
    /// move even when the system asks for less.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
}

impl Settings {
    /// Takes the selection (`selected`, `following`) as `saved` has it: what the command line,
    /// or another window, saved meanwhile.
    pub fn selection_from(&mut self, saved: &Settings) {
        self.selected = saved.selected.clone();
        self.following = saved.following.clone();
    }

    /// Remember an explicit approval; merely finding an installed toolchain never grants it.
    pub fn allow_native_mods(&mut self) -> Result<(), String> {
        self.native_mods_allowed_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_secs(),
        );
        self.save()
    }

    /// The saved settings. Only reads: a file from before `following` is migrated in memory
    /// ([`Settings::adopt_legacy_selection`] against what is installed now), and written as such
    /// at the next save.
    pub fn load() -> Self {
        Self::load_from(&data_dir().join("settings.json"))
    }

    pub(super) fn load_from(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        Self::from_saved(&text, installed).0
    }

    /// Reads settings for operations that must preserve existing data on any read failure.
    pub(crate) fn load_checked() -> Result<Self, String> {
        let path = data_dir().join("settings.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !path.is_symlink() => {
                return Ok(Self {
                    instance_versions_migrated: true,
                    ..Self::default()
                });
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Self::parse_checked(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The saved settings when an install of a version just ended, `before` being what was
    /// installed before it. A file from before `following` is judged against `before`, as the
    /// update rule of launchers before it judged it before moving the selection: against the list
    /// with the new version in it, a follower would read as a pick. [`Settings::apply_install`]
    /// then decides, and the caller's save writes `following` from then on.
    pub fn load_for_install(before: &[String]) -> Self {
        Self::load_for_install_from(&data_dir().join("settings.json"), before)
    }

    pub(super) fn load_for_install_from(path: &Path, before: &[String]) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        Self::from_saved(&text, || before.to_vec()).0
    }

    /// The settings saved as `text`, and whether it is a file from before `following`, whose
    /// selection is then judged against `installed` (read only for such a file).
    fn from_saved(text: &str, installed: impl FnOnce() -> Vec<String>) -> (Self, bool) {
        let mut settings = Self::parse(text);
        if text.is_empty() {
            // A fresh launcher has no global pick to absorb.
            settings.instance_versions_migrated = true;
        }
        let legacy = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v.as_object().map(|o| !o.contains_key("following")))
            .unwrap_or(false);
        if legacy
            && settings
                .selected
                .as_deref()
                .is_some_and(|s| crate::nightly::date_of(s).is_some())
        {
            settings.adopt_legacy_selection(&installed());
        }
        (settings, legacy)
    }

    /// Settles the selection after `tag` was installed for `cause`, `installed` being every
    /// installed version newest first and `before` the same without `tag`
    /// ([`selection_after_install`]). True when it changed; the caller saves.
    pub fn apply_install(
        &mut self,
        tag: &str,
        before: &[String],
        installed: &[String],
        cause: InstallCause,
    ) -> bool {
        let Some(follow) = selection_after_install(
            self.selected.as_deref(),
            self.following.as_deref(),
            tag,
            before,
            installed,
            cause,
        ) else {
            return false;
        };
        self.selected = Some(tag.to_owned());
        self.following = follow.then(|| tag.to_owned());
        true
    }

    /// A settings file from before `following`: its selection follows when it is the newest
    /// installed Nightly. That is exactly when the update rule of launchers before it moved the
    /// selection on ([`select_after_update`]), so those players keep the behaviour they had; it
    /// can't tell a pick of the newest from following, and doesn't need to. An older one stays a
    /// pick.
    pub(crate) fn adopt_legacy_selection(&mut self, installed: &[String]) {
        let newest = installed
            .iter()
            .find(|t| crate::nightly::date_of(t).is_some());
        if self.following.is_none() && newest.is_some() && newest == self.selected.as_ref() {
            self.following = self.selected.clone();
        }
    }

    /// Keeps the selection if `installed` (newest first) still has it; else the newest, which
    /// the launcher chose and so follows. Both fields change together, so a save in between never
    /// writes one without the other.
    pub fn keep_installed_selection(&mut self, installed: &[String]) {
        if self
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            self.selected =
                crate::nightly::latest_for(crate::instances::Channel::Nightly, installed, None);
            self.following = self.selected.clone();
        }
    }
    pub fn parse(text: &str) -> Self {
        Self::parse_checked(text).unwrap_or_default()
    }

    fn parse_checked(text: &str) -> serde_json::Result<Self> {
        let mut value = serde_json::from_str::<serde_json::Value>(text)?;
        if let Some(fields) = value.as_object_mut() {
            // An appearance from a newer launcher is Dark, not a reason to lose every setting.
            if fields
                .get("appearance")
                .is_some_and(|a| serde_json::from_value::<Appearance>(a.clone()).is_err())
            {
                fields.remove("appearance");
            }
            if fields
                .get("while_playing")
                .is_some_and(|w| serde_json::from_value::<WhilePlaying>(w.clone()).is_err())
            {
                fields.remove("while_playing");
            }
            if fields
                .get("game_architecture")
                .is_some_and(|a| serde_json::from_value::<GameArchitecture>(a.clone()).is_err())
            {
                fields.remove("game_architecture");
            }
            if fields.get("accent").is_some_and(|a| {
                serde_json::from_value::<crate::themes::Accent>(a.clone()).is_err()
            }) {
                fields.remove("accent");
            }
            // A glass or blur that isn't a number is the default, not a reason to lose the rest.
            for key in ["glass", "blur"] {
                if fields.get(key).is_some_and(|v| !v.is_number()) {
                    fields.remove(key);
                }
            }
            look_settings::tidy(fields);
            // A language this launcher doesn't have yet follows the system's.
            if fields.get("language").is_some_and(|l| {
                serde_json::from_value::<Option<crate::i18n::Language>>(l.clone()).is_err()
            }) {
                fields.remove("language");
            }
        }
        let mut settings: Self = serde_json::from_value(value)?;
        look_settings::settle(&mut settings);
        Ok(settings)
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// The window's language: the one picked, else the system's.
    pub fn language(&self) -> crate::i18n::Language {
        self.language.unwrap_or_else(crate::i18n::Language::system)
    }
    /// Whether new Horizon versions are installed automatically.
    pub fn auto_install(&self) -> bool {
        self.install_new_versions.unwrap_or(true)
    }
    /// Whether old versions are deleted after an install.
    pub fn auto_prune(&self) -> bool {
        self.prune_old_versions.unwrap_or(true)
    }
    /// The plates' white as a fraction (0 to 0.16): [`glass`](Self::glass) in percent.
    pub fn glass_tint(&self) -> f32 {
        self.glass
            .filter(|v| v.is_finite())
            .map_or(horizon_theme::glass::TINT, |v| v.clamp(0., 16.) / 100.)
    }
    /// The plates' blur in px (0 to 48).
    pub fn glass_blur(&self) -> f32 {
        self.blur
            .filter(|v| v.is_finite())
            .map_or(horizon_theme::glass::BLUR, |v| v.clamp(0., 48.))
    }
    /// Whether the cheaper scene is on ([`performance`](Self::performance)).
    pub fn performance(&self) -> bool {
        self.performance.unwrap_or(false)
    }
    /// Registry mods update by themselves ([`Settings::mods_auto_update`]): on unless turned off.
    pub fn auto_updates_mods(&self) -> bool {
        self.mods_auto_update.unwrap_or(true)
    }
    /// How many of the newest release versions pruning keeps: at least the newest.
    pub fn keep_newest(&self) -> usize {
        self.keep_versions.unwrap_or(KEEP_NEWEST).max(1)
    }
    pub fn save(&self) -> Result<(), String> {
        // The window and background jobs share the process's temporary file.
        static SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A write that stops halfway leaves settings.json unreadable, and `load` answers
        // defaults for it: the file is replaced only once it is whole.
        let dir = data_dir();
        let path = dir.join("settings.json");
        let tmp = dir.join(format!("settings.{}.tmp", std::process::id()));
        let json =
            serde_json::to_vec_pretty(self).map_err(|e| format!("{}: {e}", path.display()))?;
        let written = fs::create_dir_all(&dir)
            .and_then(|()| fs::write(&tmp, json))
            .and_then(|()| fs::rename(&tmp, &path));
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(format!("{}: {e}", path.display()));
        }
        Ok(())
    }
}
