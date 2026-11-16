//! The debug screen's entries (26.3 `DebugScreenEntries`, `DebugScreenEntryList`):
//! what each F3 line block and debug renderer is, whether it shows always,
//! only in the overlay or never, the two built-in profiles, and
//! `debug-profile.json` beside `options.txt`, which keeps them.
use mod_api::{DebugEntry, DebugGroup, DebugRenderer, Identifier};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `DebugScreenEntryStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    AlwaysOn,
    InOverlay,
    Never,
}

impl Status {
    /// `getSerializedName`.
    pub fn name(self) -> &'static str {
        match self {
            Status::AlwaysOn => "alwaysOn",
            Status::InOverlay => "inOverlay",
            Status::Never => "never",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        [Status::AlwaysOn, Status::InOverlay, Status::Never]
            .into_iter()
            .find(|s| s.name() == name)
    }
}

/// `DebugScreenProfile`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Default,
    Performance,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::Default => "default",
            Profile::Performance => "performance",
        }
    }
    pub fn translation_key(self) -> &'static str {
        match self {
            Profile::Default => "debug.options.profile.default",
            Profile::Performance => "debug.options.profile.performance",
        }
    }
    fn parse(name: &str) -> Option<Self> {
        [Profile::Default, Profile::Performance]
            .into_iter()
            .find(|p| p.name() == name)
    }
    /// `DebugScreenEntries.PROFILES`.
    fn statuses(self) -> &'static [(&'static str, Status)] {
        use Status::*;
        match self {
            Profile::Default => &[
                ("3d_crosshair", InOverlay),
                ("game_version", InOverlay),
                ("tps", InOverlay),
                ("fps", InOverlay),
                ("memory", InOverlay),
                ("system_specs", InOverlay),
                ("player_position", InOverlay),
                ("player_section_position", InOverlay),
                ("simple_performance_impactors", InOverlay),
            ],
            Profile::Performance => &[
                ("tps", InOverlay),
                ("fps", AlwaysOn),
                ("gpu_utilization", InOverlay),
                ("memory", InOverlay),
                ("simple_performance_impactors", InOverlay),
            ],
        }
    }
}

/// `DebugEntryCategory`: the screen's two sections, by `sortKey`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// `SCREEN_TEXT`, `debug.options.category.text`.
    Text,
    /// `RENDERER` (`DebugEntryNoop`), `debug.options.category.renderer`.
    Renderer,
}

impl Category {
    pub fn translation_key(self) -> &'static str {
        match self {
            Category::Text => "debug.options.category.text",
            Category::Renderer => "debug.options.category.renderer",
        }
    }
}

/// One registered entry: its path in the `minecraft` namespace, its section,
/// and whether it shows while debug info is reduced (`isAllowed(true)`).
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub path: &'static str,
    pub category: Category,
    pub reduced: bool,
}

const fn text(path: &'static str, reduced: bool) -> Entry {
    Entry {
        path,
        category: Category::Text,
        reduced,
    }
}
const fn renderer(path: &'static str) -> Entry {
    Entry {
        path,
        category: Category::Renderer,
        reduced: false,
    }
}

/// `DebugScreenEntries`' registrations, in their order.
pub const ENTRIES: [Entry; 46] = [
    text("game_version", true),
    text("fps", true),
    text("tps", true),
    text("memory", true),
    text("detailed_memory", true),
    text("system_specs", true),
    text("looking_at_block_state", false),
    text("looking_at_block_tags", false),
    text("looking_at_fluid_state", false),
    text("looking_at_fluid_tags", false),
    text("looking_at_entity", false),
    text("looking_at_entity_tags", false),
    text("chunk_render_stats", true),
    text("chunk_generation_stats", false),
    text("entity_render_stats", true),
    text("particle_render_stats", false),
    text("chunk_source_stats", true),
    text("player_position", false),
    text("player_section_position", true),
    text("player_speed", false),
    text("light_levels", false),
    text("heightmap", false),
    text("biome", false),
    text("local_difficulty", false),
    text("day_count", false),
    text("entity_spawn_counts", false),
    text("sound_mood", false),
    text("sound_cache", true),
    text("post_effects", false),
    renderer("entity_hitboxes"),
    renderer("chunk_borders"),
    renderer("3d_crosshair"),
    renderer("chunk_section_paths"),
    text("gpu_utilization", true),
    text("simple_performance_impactors", true),
    renderer("chunk_section_octree"),
    renderer("visualize_water_levels"),
    renderer("visualize_heightmap"),
    renderer("visualize_collision_boxes"),
    renderer("visualize_entity_supporting_blocks"),
    renderer("visualize_block_light_levels"),
    renderer("visualize_sky_light_levels"),
    renderer("visualize_solid_faces"),
    renderer("visualize_chunks_on_server"),
    renderer("visualize_sky_light_sections"),
    renderer("chunk_section_visibility"),
];

/// The entries native mods registered (`register_debug_entries`), kept once at start.
static REGISTERED: OnceLock<Vec<DebugEntry>> = OnceLock::new();

/// Keeps the entries native mods registered, once at start. One whose id is not
/// `<namespace>:<path>` in a namespace other than vanilla's, or that another entry has
/// already, is logged and left out.
pub fn register(entries: Vec<DebugEntry>) {
    if REGISTERED.set(checked(entries)).is_err() {
        log::warn!("debug entries were registered already; these are left out");
    }
}

/// The entries native mods registered, in registration order.
pub fn registered() -> &'static [DebugEntry] {
    match REGISTERED.get() {
        Some(entries) => entries.as_slice(),
        None => &[],
    }
}

fn checked(entries: Vec<DebugEntry>) -> Vec<DebugEntry> {
    let mut kept: Vec<DebugEntry> = Vec::new();
    for entry in entries {
        let own =
            Identifier::parse_namespaced(&entry.id).is_some_and(|id| id.namespace() != "minecraft");
        if !own {
            log::warn!(
                "debug entry {:?} is not <namespace>:<path> in a mod's namespace",
                entry.id
            );
        } else if kept.iter().any(|k| k.id == entry.id) {
            log::warn!(
                "debug entry {} is registered twice; the first is kept",
                entry.id
            );
        } else {
            kept.push(entry);
        }
    }
    kept
}

/// A native mod's entry as the list shows it: a text entry, or with `DebugGroup::Renderer` a
/// renderer entry (`DebugEntryNoop`), hidden while debug info is reduced, as the vanilla entries
/// that are not allowed then are.
fn as_entry(entry: &'static DebugEntry) -> Entry {
    Entry {
        path: entry.id.as_str(),
        category: match entry.group {
            DebugGroup::Renderer => Category::Renderer,
            _ => Category::Text,
        },
        reduced: false,
    }
}

/// Every entry: vanilla's, then the native mods' (`registered`).
pub fn all() -> impl Iterator<Item = Entry> {
    ENTRIES.into_iter().chain(registered().iter().map(as_entry))
}

/// `DebugScreenEntries.getEntry` by path.
pub fn entry(path: &str) -> Option<Entry> {
    all().find(|e| e.path == path)
}

/// `DebugOptionsScreen.OptionList.updateSearch`: the entries whose path
/// contains `search`, by category then identifier.
pub fn search(search: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = all().filter(|e| e.path.contains(search)).collect();
    entries.sort_by(|a, b| a.category.cmp(&b.category).then(a.path.cmp(b.path)));
    entries
}

/// The `DebugScreenDisplayer` group key of a vanilla group.
pub fn group_key(group: DebugGroup) -> &'static str {
    match group {
        DebugGroup::Position => "position",
        DebugGroup::Light => "light",
        DebugGroup::Memory => "memory",
        DebugGroup::LookingAtBlock => "looking_at_block",
        DebugGroup::LookingAtEntity => "looking_at_entity",
        DebugGroup::LookingAtFluid => "looking_at_fluid",
        DebugGroup::ChunkGeneration => "chunk_generation",
        // Has no lines: `mod_lines` leaves it out.
        DebugGroup::Renderer => "renderer",
    }
}

/// The lines of the enabled native-mod entries by group, in registration order. `lines` is
/// asked only for the entries `enabled` names, so a disabled one costs nothing; an entry with
/// no lines joins no group, and a renderer entry has none.
pub fn mod_lines(
    entries: &[DebugEntry],
    enabled: impl Fn(&str) -> bool,
    mut lines: impl FnMut(&str, &mut Vec<String>),
) -> Vec<(DebugGroup, Vec<String>)> {
    let mut groups = Vec::new();
    let text = |e: &&DebugEntry| e.group != DebugGroup::Renderer;
    for entry in entries.iter().filter(text).filter(|e| enabled(&e.id)) {
        let mut text = Vec::new();
        lines(&entry.id, &mut text);
        if !text.is_empty() {
            groups.push((entry.group, text));
        }
    }
    groups
}

/// `DebugScreenEntryList`: every entry's status (by full identifier, as the
/// file keeps them) and the profile they came from if unchanged. Whether the
/// F3 overlay is open (`isOverlayVisible`) is the game's `debug`.
#[derive(Clone, Debug)]
pub struct DebugEntries {
    statuses: BTreeMap<String, Status>,
    profile: Option<Profile>,
    file: Option<PathBuf>,
    /// Monotonic `rebuildCurrentList` revision, including unchanged selections.
    version: u64,
}

impl Default for DebugEntries {
    fn default() -> Self {
        let mut entries = Self {
            statuses: BTreeMap::new(),
            profile: None,
            file: None,
            version: 0,
        };
        entries.reset_to_profile(Profile::Default);
        entries
    }
}

/// `Identifier.CODEC`: a bare path is in the `minecraft` namespace, a native mod's id is its own.
fn identifier(path: &str) -> Cow<'_, str> {
    if path.contains(':') {
        Cow::Borrowed(path)
    } else {
        Cow::Owned(format!("minecraft:{path}"))
    }
}

impl DebugEntries {
    /// `DebugScreenEntryList.load` from `debug-profile.json` in `dir`: the
    /// default profile without a file; a file that does not parse is reset
    /// to it and saved again.
    pub fn load(dir: &Path) -> Self {
        let file = dir.join("debug-profile.json");
        let mut entries = Self {
            file: Some(file.clone()),
            ..Self::default()
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return entries;
        };
        match Self::parse(&text) {
            Some((Some(profile), _)) => entries.reset_to_profile(profile),
            Some((None, custom)) => {
                entries.statuses = custom;
                entries.profile = None;
            }
            None => {
                log::error!(
                    "Couldn't read debug profile file {}, resetting to default",
                    file.display()
                );
                entries.save();
            }
        }
        entries
    }
    /// `SerializedOptions.CODEC`: an optional `profile` name and an optional
    /// `custom` map of identifiers to statuses.
    fn parse(text: &str) -> Option<(Option<Profile>, BTreeMap<String, Status>)> {
        let mut json: serde_json::Value = serde_json::from_str(text).ok()?;
        upgrade(&mut json);
        let object = json.as_object()?;
        let profile = match object.get("profile") {
            Some(name) => Some(Profile::parse(name.as_str()?)?),
            None => None,
        };
        let mut custom = BTreeMap::new();
        if let Some(map) = object.get("custom") {
            for (key, status) in map.as_object()? {
                // Identifier.CODEC: a bare path is in the minecraft namespace.
                let key = if key.contains(':') {
                    key.clone()
                } else {
                    identifier(key).into_owned()
                };
                custom.insert(key, Status::parse(status.as_str()?)?);
            }
        }
        Some((profile, custom))
    }
    /// `DebugScreenEntryList.save`: the profile's name while it is unchanged,
    /// else every status, with the data version the codec wraps it in.
    pub fn save(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let mut json = serde_json::Map::new();
        json.insert(
            "DataVersion".into(),
            horizon_core::version::DATA_VERSION.into(),
        );
        match self.profile {
            Some(profile) => {
                json.insert("profile".into(), profile.name().into());
            }
            None => {
                let custom: serde_json::Map<String, serde_json::Value> = self
                    .statuses
                    .iter()
                    .map(|(k, s)| (k.clone(), s.name().into()))
                    .collect();
                json.insert("custom".into(), custom.into());
            }
        }
        if let Err(e) = std::fs::write(file, serde_json::Value::Object(json).to_string()) {
            log::error!("Failed to save debug profile file {}: {e}", file.display());
        }
    }
    fn reset_to_profile(&mut self, profile: Profile) {
        self.profile = Some(profile);
        self.statuses = profile
            .statuses()
            .iter()
            .map(|(path, status)| (identifier(path).into_owned(), *status))
            .collect();
    }
    /// `loadProfile` then `save` (the screen's profile buttons).
    pub fn load_profile(&mut self, profile: Profile) {
        self.reset_to_profile(profile);
        self.invalidate();
        self.save();
    }
    pub fn is_using_profile(&self, profile: Profile) -> bool {
        self.profile == Some(profile)
    }
    /// `getStatus`: never for an entry the statuses leave out. The debug renderers ask every
    /// frame, so no identifier is built for the lookup; a path with a namespace (a native mod's entry) is
    /// matched as it is.
    pub fn status(&self, path: &str) -> Status {
        self.statuses
            .iter()
            .find(|(id, _)| id.as_str() == path || id.strip_prefix("minecraft:") == Some(path))
            .map_or(Status::Never, |(_, status)| *status)
    }
    /// `setStatus`: the statuses are custom from now on, and saved.
    pub fn set_status(&mut self, path: &str, status: Status) {
        self.profile = None;
        self.statuses.insert(identifier(path).into_owned(), status);
        self.invalidate();
        self.save();
    }
    /// `toggleStatus` (F3+B, F3+G) with the overlay open or not: whether the
    /// entry is on afterwards.
    pub fn toggle_status(&mut self, path: &str, overlay: bool) -> bool {
        let (status, on) = match self.statuses.get(identifier(path).as_ref()) {
            Some(Status::AlwaysOn) => (Status::Never, false),
            Some(Status::InOverlay) if overlay => (Status::Never, false),
            Some(Status::Never) if overlay => (Status::InOverlay, true),
            _ => (Status::AlwaysOn, true),
        };
        self.set_status(path, status);
        on
    }
    /// `isCurrentlyEnabled` (`rebuildCurrentList`): on always, or in the
    /// `overlay` while it is open, and allowed with the debug info `reduced`.
    pub fn enabled(&self, path: &str, overlay: bool, reduced: bool) -> bool {
        let shown = match self.status(path) {
            Status::AlwaysOn => true,
            Status::InOverlay => overlay,
            Status::Never => false,
        };
        shown && entry(path).is_some_and(|e| e.reduced || !reduced)
    }
    /// `DebugScreenEntryList.rebuildCurrentList`: invalidate even if the enabled set is unchanged.
    pub fn invalidate(&mut self) {
        self.version += 1;
    }
    /// `getCurrentlyEnabledVersion`: every rebuild, not a fingerprint of the final selection.
    pub fn enabled_version(&self) -> u64 {
        self.version
    }
    /// `DebugRenderer.refreshRendererList`'s new list: the ids of the `renderers` mods pushed
    /// whose renderer entry is on, in their order. An id with no entry a mod declared as a
    /// renderer is logged and left out, and an id pushed twice counts once.
    pub fn active_renderers(
        &self,
        renderers: Vec<DebugRenderer>,
        overlay: bool,
        reduced: bool,
    ) -> Vec<String> {
        let mut active: Vec<String> = Vec::new();
        for DebugRenderer { id } in renderers {
            let declared =
                id.contains(':') && entry(&id).is_some_and(|e| e.category == Category::Renderer);
            if !declared {
                log::warn!("debug renderer {id} has no renderer entry; it is left out");
            } else if self.enabled(&id, overlay, reduced) && !active.contains(&id) {
                active.push(id);
            }
        }
        active
    }
    /// `getCurrentlyEnabled().isEmpty()` negated: something to draw. Asked every frame, F3 closed
    /// or not, so it walks the few statuses kept rather than every entry.
    pub fn any_enabled(&self, overlay: bool, reduced: bool) -> bool {
        self.statuses
            .keys()
            .map(|id| id.strip_prefix("minecraft:").unwrap_or(id))
            .any(|path| self.enabled(path, overlay, reduced))
    }
}

/// `DataFixTypes.DEBUG_PROFILE.wrapCodec(..., 4649)`: a file without a
/// `DataVersion` is from 4649, the version the file first had.
const FIRST_VERSION: i64 = 4649;

/// The `DataFixers` rules for `References.DEBUG_PROFILE`, each run when the
/// file is older than its schema, on the `custom` map.
fn upgrade(json: &mut serde_json::Value) {
    let version = json
        .get("DataVersion")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(FIRST_VERSION);
    let Some(custom) = json.get_mut("custom").and_then(|c| c.as_object_mut()) else {
        return;
    };
    // 4650 DebugProfileOverlayReferenceFix: the status `inF3` is `inOverlay`.
    if version < 4650 {
        for status in custom.values_mut() {
            if status.as_str() == Some("inF3") {
                *status = "inOverlay".into();
            }
        }
    }
    // 4763 DebugProfileLookingAtSplitFix: the targeted block and fluid
    // entries became their `_state` halves; a key already there wins.
    if version < 4763 {
        for (old, new) in [
            (
                "minecraft:looking_at_block",
                "minecraft:looking_at_block_state",
            ),
            (
                "minecraft:looking_at_fluid",
                "minecraft:looking_at_fluid_state",
            ),
        ] {
            if let Some(status) = custom.remove(old) {
                custom.entry(new).or_insert(status);
            }
        }
    }
    // 5001 DebugProfileRenameEntryFix: `post_effect` is `post_effects`.
    if version < 5001
        && let Some(status) = custom.remove("minecraft:post_effect")
    {
        custom.insert("minecraft:post_effects".into(), status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggling_follows_the_overlay() {
        let mut e = DebugEntries::default();
        // Not in the default profile: on always while F3 is closed.
        assert!(e.toggle_status("entity_hitboxes", false));
        assert_eq!(e.status("entity_hitboxes"), Status::AlwaysOn);
        assert!(!e.toggle_status("entity_hitboxes", false));
        assert_eq!(e.status("entity_hitboxes"), Status::Never);
        // With F3 open, a hidden entry goes in the overlay only.
        assert!(e.toggle_status("chunk_borders", true));
        assert_eq!(e.status("chunk_borders"), Status::AlwaysOn);
        assert!(!e.toggle_status("chunk_borders", true));
        assert_eq!(e.status("chunk_borders"), Status::Never);
        assert!(e.toggle_status("chunk_borders", true));
        assert_eq!(e.status("chunk_borders"), Status::InOverlay);
        assert!(!e.toggle_status("chunk_borders", true));
        assert!(!e.is_using_profile(Profile::Default));
    }

    #[test]
    fn enabled_needs_the_overlay_and_full_debug_info() {
        let mut e = DebugEntries::default();
        assert!(!e.any_enabled(false, false));
        assert!(e.enabled("player_position", true, false));
        // Reduced debug info hides the position, not the section.
        assert!(!e.enabled("player_position", true, true));
        assert!(e.enabled("player_section_position", true, true));
        e.load_profile(Profile::Performance);
        assert!(e.enabled("fps", false, false));
        assert!(!e.enabled("memory", false, false));
    }

    #[test]
    fn the_file_keeps_a_profile_or_the_statuses() {
        let dir =
            std::env::temp_dir().join(format!("horizon-debug-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = DebugEntries::load(&dir);
        assert!(e.is_using_profile(Profile::Default));
        e.load_profile(Profile::Performance);
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("debug-profile.json")).unwrap())
                .unwrap();
        assert_eq!(saved["profile"], "performance");
        assert!(DebugEntries::load(&dir).is_using_profile(Profile::Performance));
        e.set_status("biome", Status::AlwaysOn);
        let loaded = DebugEntries::load(&dir);
        assert_eq!(loaded.status("biome"), Status::AlwaysOn);
        assert_eq!(loaded.status("fps"), Status::AlwaysOn);
        assert!(!loaded.is_using_profile(Profile::Performance));
        // A broken file is reset to the default profile.
        std::fs::write(dir.join("debug-profile.json"), "{\"profile\":\"nope\"}").unwrap();
        assert!(DebugEntries::load(&dir).is_using_profile(Profile::Default));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_files_are_upgraded_by_their_data_version() {
        // No DataVersion: every fix runs.
        let (_, custom) = DebugEntries::parse(
            r#"{"custom":{"minecraft:fps":"inF3","minecraft:looking_at_block":"alwaysOn",
            "minecraft:looking_at_fluid":"inF3","minecraft:looking_at_fluid_state":"never",
            "minecraft:post_effect":"alwaysOn"}}"#,
        )
        .unwrap();
        assert_eq!(custom["minecraft:fps"], Status::InOverlay);
        assert_eq!(custom["minecraft:looking_at_block_state"], Status::AlwaysOn);
        assert_eq!(custom["minecraft:looking_at_fluid_state"], Status::Never);
        assert_eq!(custom["minecraft:post_effects"], Status::AlwaysOn);
        assert!(!custom.contains_key("minecraft:looking_at_block"));
        assert!(!custom.contains_key("minecraft:post_effect"));
        // From 4763 on, only the post effects rename is left.
        let (_, custom) = DebugEntries::parse(
            r#"{"DataVersion":4763,"custom":{"minecraft:looking_at_block":"alwaysOn",
            "minecraft:post_effect":"alwaysOn"}}"#,
        )
        .unwrap();
        assert_eq!(custom["minecraft:looking_at_block"], Status::AlwaysOn);
        assert_eq!(custom["minecraft:post_effects"], Status::AlwaysOn);
        // `inF3` is no status any more once the file is past 4650.
        assert!(DebugEntries::parse(r#"{"DataVersion":4650,"custom":{"fps":"inF3"}}"#).is_none());
    }

    #[test]
    fn the_list_sorts_text_before_renderers() {
        let all = search("");
        assert_eq!(all.len(), 46);
        assert_eq!(all[0].path, "biome");
        let first_renderer = all
            .iter()
            .position(|e| e.category == Category::Renderer)
            .unwrap();
        assert_eq!(all[first_renderer].path, "3d_crosshair");
        assert!(
            all[first_renderer..]
                .iter()
                .all(|e| e.category == Category::Renderer)
        );
        assert_eq!(search("chunk_b").len(), 1);
    }

    #[test]
    fn a_native_mods_entries_keep_their_full_id_in_the_file() {
        let mut e = DebugEntries::default();
        e.set_status("mymod:tick_cost", Status::AlwaysOn);
        assert_eq!(e.status("mymod:tick_cost"), Status::AlwaysOn);
        assert_eq!(e.status("tick_cost"), Status::Never);
    }

    #[test]
    fn a_native_mods_entries_need_their_own_namespace_and_a_free_id() {
        let kept = checked(vec![
            DebugEntry {
                id: "mymod:tick_cost".into(),
                group: DebugGroup::Memory,
            },
            DebugEntry {
                id: "minecraft:fps".into(),
                group: DebugGroup::Memory,
            },
            DebugEntry {
                id: "tick_cost".into(),
                group: DebugGroup::Memory,
            },
            DebugEntry {
                id: "mymod:tick_cost".into(),
                group: DebugGroup::Light,
            },
            DebugEntry {
                id: "other:queue".into(),
                group: DebugGroup::Position,
            },
        ]);
        let ids: Vec<&str> = kept.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["mymod:tick_cost", "other:queue"]);
        assert_eq!(kept[0].group, DebugGroup::Memory);
    }

    #[test]
    fn a_native_mods_lines_join_their_group_and_only_enabled_entries_are_asked() {
        let entries = [
            DebugEntry {
                id: "mymod:tick_cost".into(),
                group: DebugGroup::Memory,
            },
            DebugEntry {
                id: "mymod:queue".into(),
                group: DebugGroup::Position,
            },
        ];
        let mut asked = Vec::new();
        let groups = mod_lines(
            &entries,
            |id| id == "mymod:tick_cost",
            |id, lines| {
                asked.push(id.to_owned());
                lines.push(format!("{id}: 3 ms"));
            },
        );
        // The disabled entry's lines hook is never asked.
        assert_eq!(asked, ["mymod:tick_cost"]);
        assert_eq!(
            groups,
            [(DebugGroup::Memory, vec!["mymod:tick_cost: 3 ms".to_owned()])]
        );
        assert_eq!(group_key(groups[0].0), "memory");
    }

    #[test]
    fn a_native_mods_renderer_entry_is_listed_as_a_renderer_and_has_no_lines() {
        let entry: &'static DebugEntry = Box::leak(Box::new(DebugEntry {
            id: "mymod:pipes".into(),
            group: DebugGroup::Renderer,
        }));
        assert_eq!(as_entry(entry).category, Category::Renderer);
        assert!(!as_entry(entry).reduced);
        let asked = mod_lines(
            std::slice::from_ref(entry),
            |_| true,
            |_, _| panic!("a renderer entry has no lines"),
        );
        assert!(asked.is_empty());
    }

    #[test]
    fn the_enabled_version_changes_with_every_rebuild_of_the_list() {
        let mut e = DebugEntries::default();
        let first = e.enabled_version();
        assert_eq!(e.status("mymod:pipes"), Status::Never);
        assert_eq!(e.enabled_version(), first);

        e.invalidate();
        assert_eq!(e.enabled_version(), first + 1);
        e.invalidate();
        assert_eq!(e.enabled_version(), first + 2);

        e.set_status("mymod:pipes", Status::AlwaysOn);
        let set = e.enabled_version();
        assert_ne!(set, first);

        e.load_profile(Profile::Performance);
        assert_ne!(e.enabled_version(), set);
    }

    #[test]
    fn an_entry_with_no_lines_joins_no_group() {
        let entries = [DebugEntry {
            id: "mymod:quiet".into(),
            group: DebugGroup::Light,
        }];
        assert!(mod_lines(&entries, |_| true, |_, _| {}).is_empty());
    }
}
