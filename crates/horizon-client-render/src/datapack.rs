//! Data packs (`<world>/datapacks`: folders and zips with `pack.mcmeta`) and
//! the parts of them commands use: `.mcfunction` functions (with macros and
//! line continuation), function tags and predicates.
use crate::resource_source::ResourceSource;
mod slot_sources;
use crate::zip::Zip;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use {
    horizon_core::nbt::Tag,
    horizon_world::worldgen::{
        overrides,
        overrides::{PackFile, WorldgenOverrides},
    },
};

/// A pack on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct DataPack {
    /// `file/<name>` in `/datapack` and level.dat.
    pub id: String,
    source: ResourceSource,
    /// Presentation and selection supplied by an additional repository source.
    pub selection: Option<crate::pack_screen::PackInfo>,
}
impl DataPack {
    /// Whether this repository entry comes from the feature-pack source.
    pub fn is_feature(&self) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|info| info.source == crate::pack_screen::Source::Feature)
    }
    /// A named resource layer, retaining its open source for lazy reads.
    pub fn from_source(id: String, source: ResourceSource) -> Self {
        Self {
            id,
            source,
            selection: None,
        }
    }
    pub fn source(&self) -> &ResourceSource {
        &self.source
    }
    /// Every file path under `prefix` (relative to the pack root).
    pub fn list(&self, prefix: &str) -> Vec<String> {
        self.source.list(prefix)
    }
    /// The files of `<top>/<namespace>/<kind>/` for each kind, in the order
    /// `list("<top>/")` gives them. A folder pack walks only those folders:
    /// the built-in pack holds about 10,000 other data files.
    pub fn list_under(&self, top: &str, kinds: &[&str]) -> Vec<String> {
        self.source.list_under(top, kinds)
    }
    /// The files of `data/<namespace>/<kind>/` for each kind.
    pub fn list_kinds(&self, kinds: &[&str]) -> Vec<String> {
        self.list_under("data", kinds)
    }

    /// The files of `data/<namespace>/<folder>/` in every namespace, as
    /// `list` names them; a folder pack walks only those folders.
    pub fn list_folder(&self, folder: &str) -> Vec<String> {
        self.list_kinds(&[folder])
    }
    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.source.read(path).ok()
    }

    /// The `description` of `pack.mcmeta`, as text.
    pub fn description(&self) -> String {
        let Some(bytes) = self.read("pack.mcmeta") else {
            return String::new();
        };
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        match &v["pack"]["description"] {
            serde_json::Value::String(s) => s.clone(),
            other => other["text"].as_str().unwrap_or_default().to_owned(),
        }
    }
    /// The `features.enabled` flag ids of `pack.mcmeta`
    /// (`FeatureFlagsMetadataSection`): the flags the pack asks to turn on.
    pub fn features(&self) -> Vec<String> {
        self.mcmeta()["features"]["enabled"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// The `pack.mcmeta` as JSON, empty when it is missing or unreadable.
    fn mcmeta(&self) -> serde_json::Value {
        self.read("pack.mcmeta")
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}
impl DataPack {
    /// A mod's passive data layer, without reading the archive's pack metadata.
    pub fn mod_layer(id: String, source: &crate::packs::Source) -> std::io::Result<Self> {
        Ok(Self {
            id,
            source: source.open()?,
            selection: None,
        })
    }
    /// The built-in `vanilla` pack: the game's own data directory.
    pub fn builtin(root: &Path) -> Self {
        DataPack {
            id: "vanilla".into(),
            source: ResourceSource::directory(root.to_path_buf()),
            selection: None,
        }
    }
}
/// The packs in `<world>/datapacks`, sorted by name.
pub fn discover(world: &Path) -> Vec<DataPack> {
    discover_folder(&world.join("datapacks"))
}
/// `FolderRepositorySource.loadPacks`: packs in Create World's temporary
/// folder or a world's data-pack directory, with required folder metadata.
pub fn discover_folder(dir: &Path) -> Vec<DataPack> {
    discover_folder_with(dir, &crate::packs::DirectoryValidator::game())
}
/// [`discover_folder`] under an explicit `allowed_symlinks.txt` validator.
pub(crate) fn discover_folder_with(
    dir: &Path,
    validator: &crate::packs::DirectoryValidator,
) -> Vec<DataPack> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        // `PackDetector.detectPackResources`: a link, or a pack holding one, is
        // not a pack unless the validator allows the target; the id stays the
        // entry's name while the contents come from the target.
        let path = match crate::packs::detected_pack_path(&e.path(), validator) {
            Ok(Some(path)) => path,
            Ok(None) => continue,
            Err(err) => {
                log::warn!(
                    "Failed to read properties of '{}', ignoring: {err}",
                    e.path().display()
                );
                continue;
            }
        };
        if path.is_dir() && folder_metadata_present(&path) {
            packs.push(DataPack {
                id: format!("file/{name}"),
                source: ResourceSource::directory(path),
                selection: None,
            });
        } else if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".zip"))
            && let Ok(z) = Zip::open(&path)
            && z.contains("pack.mcmeta")
        {
            packs.push(DataPack {
                id: format!("file/{name}"),
                source: ResourceSource::opened_archive(z),
                selection: None,
            });
        }
    }
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    packs
}
/// `Pack.readPackMetadata`: even its fallback codec requires a pack section
/// and description. Keep this folder gate separate from format compatibility
/// and the existing interpretation of the description component.
fn folder_metadata_present(path: &Path) -> bool {
    let Some(meta) = std::fs::read(path.join("pack.mcmeta"))
        .ok()
        .and_then(|bytes| crate::packs::parse_meta(&bytes))
    else {
        return false;
    };
    meta.as_object()
        .and_then(|root| root.get("pack"))
        .and_then(serde_json::Value::as_object)
        .and_then(|pack| pack.get("description"))
        .is_some_and(|description| !description.is_null())
}
/// The resources root's built-in feature packs (`ServerPacksSource`
/// scanning the game's `data/minecraft/datapacks`): every folder with a
/// `pack.mcmeta`, a feature pack (`PackSource.FEATURE`). Off until a world
/// selects them.
pub fn feature_packs(resources: &Path) -> Vec<DataPack> {
    let Ok(entries) = std::fs::read_dir(resources.join("data").join("minecraft").join("datapacks"))
    else {
        return Vec::new();
    };
    let mut packs = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.join("pack.mcmeta").exists() {
            let mut pack = DataPack {
                id: e.file_name().to_string_lossy().into_owned(),
                source: ResourceSource::directory(path),
                selection: None,
            };
            let meta = pack.mcmeta();
            let (description, description_style) =
                crate::packs::component_root(&meta["pack"]["description"]);
            let name = format!("dataPack.{}.name", pack.id);
            pack.selection = Some(crate::pack_screen::PackInfo {
                id: pack.id.clone(),
                title: if crate::lang::has(&name) {
                    crate::text::Text::translate(&name, &[])
                } else {
                    crate::text::Text::literal(pack.id.clone())
                },
                description,
                description_style,
                source: crate::pack_screen::Source::Feature,
                compat: crate::packs::compatibility_for(
                    &meta,
                    horizon_core::version::DATA_PACK_FORMAT,
                ),
                icon: None,
                required: false,
                position: crate::packs::Position::Bottom,
                fixed: false,
                hidden: false,
            });
            packs.push(pack);
        }
    }
    packs.sort_by(|a, b| a.id.cmp(&b.id));
    packs
}
/// The packs of `<world>/datapacks` plus the resources root's feature packs:
/// the world's whole repository, as `PackRepository.reload` sees it.
pub fn discover_with_features(world_root: &Path, resources: &Path) -> Vec<DataPack> {
    let mut packs = discover(world_root);
    packs.extend(feature_packs(resources));
    packs
}

/// Built-in and contributed feature packs offered as world experiments.
pub fn feature_repository(
    resources: &Path,
    additional: &[std::sync::Arc<DataPack>],
) -> Vec<DataPack> {
    let mut packs = feature_packs(resources);
    packs.extend(
        additional
            .iter()
            .filter(|pack| {
                pack.selection
                    .as_ref()
                    .is_some_and(|info| info.source == crate::pack_screen::Source::Feature)
            })
            .map(|pack| (**pack).clone()),
    );
    packs
}

// ---- Create World ------------------------------------------------------------

/// Create World's Data Packs screen (`PackSelectionScreen` on vanilla's
/// `mcworld-` folder): the folder's packs (`PackSource.WORLD`), the
/// built-in one (`resources` holds its `pack.png`), and the feature packs
/// of `resources` (`PackSource.FEATURE`), available at the bottom and off
/// until picked — the Experiments.
pub fn screen_packs(dir: &Path, resources: &Path) -> Vec<crate::pack_screen::PackInfo> {
    use crate::{
        pack_screen::{Compat, PackInfo, Source, load_icon},
        packs::Position,
        text::{Style, Text},
    };
    let format = horizon_core::version::DATA_PACK_FORMAT;
    let mut infos: Vec<PackInfo> = crate::packs::folder(dir, format)
        .into_iter()
        .map(|p| PackInfo {
            icon: p.icon_png().as_deref().and_then(load_icon),
            id: p.id,
            title: p.title,
            description: p.raw_description,
            description_style: p.description_style,
            source: Source::World,
            compat: p.compatibility,
            required: false,
            position: Position::Top,
            fixed: false,
            hidden: false,
        })
        .collect();
    infos.push(PackInfo {
        id: "vanilla".into(),
        title: Text::translate("dataPack.vanilla.name", &[]),
        description: Text::translate("dataPack.vanilla.description", &[]),
        description_style: Style::default(),
        source: Source::BuiltIn,
        compat: Compat::Compatible,
        icon: std::fs::read(resources.join("pack.png"))
            .ok()
            .as_deref()
            .and_then(load_icon),
        required: false,
        position: Position::Bottom,
        fixed: false,
        hidden: false,
    });
    for p in feature_packs(resources) {
        infos.extend(p.selection);
    }
    infos
}
/// The folder of a game folder whose packs every new world there starts with: a modpack's profile
/// keeps the packs of its data-only jars here (`horizon-port-pack`). Each world gets its own copy,
/// as Create World's packs do, so a world keeps working when the folder changes. Vanilla has no
/// such folder, and without one nothing here happens.
pub const NEW_WORLD_PACKS: &str = "new-world-datapacks";

/// Create World's temporary pack folder holding `game_dir`'s [`NEW_WORLD_PACKS`], and their ids,
/// to show them selected in the Data Packs screen. `None` when there are none.
pub fn seed_create_world(game_dir: &Path) -> Option<(PathBuf, Vec<String>)> {
    let from = game_dir.join(NEW_WORLD_PACKS);
    let ids: Vec<String> = discover_folder(&from).into_iter().map(|p| p.id).collect();
    if ids.is_empty() {
        return None;
    }
    let seeded = temp_folder()
        .map_err(|e| e.to_string())
        .and_then(|dir| copy(&from, &dir).map(|()| dir));
    match seeded {
        Ok(dir) => Some((dir, ids)),
        Err(e) => {
            log::warn!("Horizon: data packs for new worlds: {e}");
            None
        }
    }
}

/// A world about to be created without Create World (`--world`, the control socket) gets
/// `game_dir`'s [`NEW_WORLD_PACKS`]. The game turns on every pack it finds in a world.
pub fn install_new_world_packs(game_dir: &Path, world: &Path) -> Result<(), String> {
    let from = game_dir.join(NEW_WORLD_PACKS);
    let exists = |name: &str| world.join(name).exists();
    if !from.is_dir() || exists("level.dat") || exists("level.dat_old") {
        return Ok(());
    }
    copy(&from, &world.join("datapacks"))
}

/// A new empty folder for Create World's packs
/// (`Files.createTempDirectory("mcworld-")`).
pub fn temp_folder() -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("mcworld-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
/// `PackSelectionScreen.onFilesDrop`'s `PackDetector`: a zip is a pack; a
/// folder is one when it holds a `pack.mcmeta`, or exactly one folder that
/// does (a folder zipped by the system, which adds a wrapper). Copied into
/// `dir`; false when `path` is no pack, or when part of it could not be copied.
pub fn copy_pack(path: &Path, dir: &Path) -> bool {
    fn copy_dir(from: &Path, at: &Path) -> bool {
        let mut whole = true;
        if let Err(e) = std::fs::create_dir_all(at) {
            log::warn!("Horizon: {} is not made: {e}", at.display());
            whole = false;
        }
        for entry in std::fs::read_dir(from).into_iter().flatten().flatten() {
            let path = entry.path();
            let to = at.join(entry.file_name());
            let copied = if path.is_dir() {
                copy_dir(&path, &to)
            } else {
                match std::fs::copy(&path, &to) {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!(
                            "Horizon: {} is not copied into the pack: {e}",
                            path.display()
                        );
                        false
                    }
                }
            };
            whole &= copied;
        }
        whole
    }
    let name = || {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    if path.is_file() {
        return path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            && {
                let _ = std::fs::create_dir_all(dir);
                std::fs::copy(path, dir.join(name())).is_ok()
            };
    }
    if !path.is_dir() {
        return false;
    }
    if path.join("pack.mcmeta").is_file() {
        return copy_dir(path, &dir.join(name()));
    }
    let own: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("pack.mcmeta").is_file())
        .collect();
    match own.as_slice() {
        [one] => copy_dir(one, &dir.join(one.file_name().unwrap_or_default())),
        _ => false,
    }
}
/// What's in a folder of packs, to notice packs added or removed while the
/// screen shows it (`PackSelectionScreen.Watcher`).
pub fn folder_stamp(dir: &Path) -> Vec<(String, Option<std::time::SystemTime>)> {
    let mut stamp: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| {
            let modified = e.metadata().and_then(|m| m.modified()).ok();
            (e.file_name().to_string_lossy().into_owned(), modified)
        })
        .collect();
    stamp.sort();
    stamp
}
/// Create World's packs into the new world before it opens
/// (`copyTempDataPackDirToNewWorld`): every pack of `dir` goes to
/// `<world>/datapacks`, and `datapacks.json` enables those `selected`
/// (lowest priority first) and disables the rest.
pub fn install(dir: &Path, world: &Path, selected: &[String]) -> Result<(), String> {
    let err = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let target = world.join("datapacks");
    std::fs::create_dir_all(&target).map_err(|e| err(&target, e))?;
    let mut disabled = Vec::new();
    for pack in crate::packs::folder(dir, horizon_core::version::DATA_PACK_FORMAT) {
        let from = match &pack.source {
            crate::packs::Source::Dir(from) | crate::packs::Source::Zip(from) => from,
            crate::packs::Source::Opened(_) => {
                return Err(format!(
                    "{}: an open resource layer cannot be installed as a file pack",
                    pack.id
                ));
            }
        };
        copy(from, &target.join(pack.id.trim_start_matches("file/")))?;
        if !selected.contains(&pack.id) {
            disabled.push(pack.id);
        }
    }
    if !selected.iter().any(|id| id == "vanilla") {
        disabled.push("vanilla".into());
    }
    let json = serde_json::json!({ "enabled": selected, "disabled": disabled });
    let file = world.join("datapacks.json");
    std::fs::write(&file, json.to_string()).map_err(|e| err(&file, e))
}
/// Re-Create World's packs from a saved world: a new folder holding a copy of
/// its `datapacks` (`CreateWorldScreen.createTempDataPackDirFromExistingWorld`; `None` when the
/// world has none), and the selection its `datapacks.json` makes, lowest
/// priority first (`WorldOpenFlows.recreateWorldData` keeps the world's `DataConfiguration`).
/// The built-in pack is selected unless the world turned it off.
pub fn for_recreate(world: &Path) -> (Option<PathBuf>, Vec<String>) {
    let packs = discover(world);
    // No flags here: the recreated world keeps the original's
    // `enabled_features`, and the flag-aware load re-adds the feature packs
    // it asks for (`for_recreate`'s selection only seeds the new world's
    // `datapacks.json`).
    let mut selected = enabled_ids(world, &packs, &[]);
    let vanilla_off = std::fs::read(world.join("datapacks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["disabled"].as_array().cloned())
        .is_some_and(|a| a.iter().any(|id| id.as_str() == Some("vanilla")));
    if !vanilla_off {
        selected.insert(0, "vanilla".into());
    }
    if packs.is_empty() {
        return (None, selected);
    }
    let dir = temp_folder().ok().and_then(|dir| {
        if copy(&world.join("datapacks"), &dir).is_ok() {
            Some(dir)
        } else {
            let _ = std::fs::remove_dir_all(&dir);
            None
        }
    });
    (dir, selected)
}
/// A file, or a folder and everything in it.
fn copy(from: &Path, to: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{} -> {}: {e}", from.display(), to.display());
    if !from.is_dir() {
        return std::fs::copy(from, to).map(drop).map_err(err);
    }
    std::fs::create_dir_all(to).map_err(err)?;
    for e in std::fs::read_dir(from).map_err(err)?.flatten() {
        copy(&e.path(), &to.join(e.file_name()))?;
    }
    Ok(())
}

// ---- functions ---------------------------------------------------------------

/// A `.mcfunction`: plain command lines and macro lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub id: String,
    pub lines: Vec<FunctionLine>,
    /// Macro parameter names in order of first use.
    pub parameters: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionLine {
    Command(String),
    /// A `$` line: text with `$(name)` placeholders.
    Macro(String),
}
impl Function {
    pub fn is_macro(&self) -> bool {
        !self.parameters.is_empty()
    }
    /// `MacroFunction.instantiate`: the command lines with the arguments
    /// substituted. The error is the translation key and its arguments.
    pub fn instantiate(
        &self,
        arguments: Option<&Tag>,
    ) -> Result<Vec<String>, (String, Vec<String>)> {
        let mut values: BTreeMap<&str, String> = BTreeMap::new();
        if self.is_macro() {
            let Some(args) = arguments else {
                return Err((
                    "commands.function.error.missing_arguments".into(),
                    vec![self.id.clone()],
                ));
            };
            for p in &self.parameters {
                let Some(v) = args.get(p) else {
                    return Err((
                        "commands.function.error.missing_argument".into(),
                        vec![self.id.clone(), p.clone()],
                    ));
                };
                values.insert(p, stringify(v));
            }
        }
        Ok(self
            .lines
            .iter()
            .map(|l| match l {
                FunctionLine::Command(c) => c.clone(),
                FunctionLine::Macro(t) => substitute(t, &values),
            })
            .collect())
    }
}
/// `MacroFunction.stringify`.
fn stringify(tag: &Tag) -> String {
    match tag {
        Tag::Float(v) => decimal(f64::from(f32::from_bits(*v))),
        Tag::Double(v) => decimal(f64::from_bits(*v)),
        Tag::Byte(v) => v.to_string(),
        Tag::Short(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::String(s) => s.to_string_lossy(),
        other => crate::snbt::to_snbt(other),
    }
}
/// `DecimalFormat("#")` with up to 15 fraction digits.
fn decimal(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    let s = format!("{v:.15}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_owned()
    }
}
fn substitute(template: &str, values: &BTreeMap<&str, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("$(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find(')') {
            Some(j) => {
                if let Some(v) = values.get(&after[..j]) {
                    out.push_str(v);
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}
fn macro_parameters(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(i) = rest.find("$(") {
        let after = &rest[i + 2..];
        let Some(j) = after.find(')') else {
            return Err(format!("Unterminated macro variable in '{line}'"));
        };
        let name = &after[..j];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("Invalid macro variable name '{name}' in '{line}'"));
        }
        out.push(name.to_owned());
        rest = &after[j + 1..];
    }
    Ok(out)
}
/// `CommandFunction.fromLines`: comments, blank lines, `\` continuation and
/// `$` macro lines. Command syntax is checked by the caller.
pub fn parse_function(id: &str, text: &str) -> Result<Function, String> {
    let raw: Vec<&str> = text.lines().collect();
    let mut lines = Vec::new();
    let mut parameters: Vec<String> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let number = i + 1;
        let mut line = raw[i].trim().to_owned();
        if line.ends_with('\\') {
            loop {
                i += 1;
                if i == raw.len() {
                    return Err("Line continuation at end of file".into());
                }
                line.pop();
                line.push_str(raw[i].trim());
                if !line.ends_with('\\') {
                    break;
                }
            }
        }
        i += 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('/') {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
                .collect();
            return Err(if rest.starts_with('/') {
                format!(
                    "Unknown or invalid command '{line}' on line {number} (if you intended to make a comment, use '#' not '//')"
                )
            } else {
                format!(
                    "Unknown or invalid command '{line}' on line {number} (did you mean '{name}'? Do not use a preceding forwards slash.)"
                )
            });
        }
        if let Some(m) = line.strip_prefix('$') {
            let names = macro_parameters(m)?;
            // `StringTemplate.fromString`: a `$` line with no `$(…)` is not a
            // macro, and fails the whole file rather than running unchanged.
            if names.is_empty() {
                return Err(format!(
                    "Can't parse function line {number}: '{m}': No variables in macro"
                ));
            }
            for p in names {
                if !parameters.contains(&p) {
                    parameters.push(p);
                }
            }
            lines.push(FunctionLine::Macro(m.to_owned()));
        } else {
            lines.push(FunctionLine::Command(line));
        }
    }
    Ok(Function {
        id: id.to_owned(),
        lines,
        parameters,
    })
}

/// The data of every enabled pack: functions, function tags, predicates.
#[derive(Default)]
pub struct DataStore {
    pub functions: BTreeMap<String, Function>,
    /// Function tag id -> entries (`ns:f`, `#ns:tag`), with `required` flags.
    pub function_tags: BTreeMap<String, Vec<TagEntryOf>>,
    pub predicates: BTreeMap<String, serde_json::Value>,
    /// `item_modifier` files, for `/item modify`.
    pub item_modifiers: BTreeMap<String, serde_json::Value>,
    /// `slot_source` definitions, evaluated against a command's live accessor.
    pub slot_sources: BTreeMap<String, serde_json::Value>,
    /// Slot-source tags belong to the same loaded snapshot as their definitions.
    pub slot_source_tags: BTreeMap<String, Vec<TagEntryOf>>,
    /// The content tables' raw files from the packs after the built-in one:
    /// Server JSON resources (recipes, loot, enchantments, songs, instruments,
    /// predicates, modifiers, advancements, registries, data maps and tags), by
    /// `<kind-dir>/<ns>:<path>`, each id's files in pack order. The tables
    /// merge them over what they read from the resource root.
    pub files: BTreeMap<String, Vec<Vec<u8>>>,
    /// Functions that failed to parse (id, message), for logging.
    pub errors: Vec<(String, String)>,
}
/// `<kind-dir>/<ns>:<path>` for a pack file the content tables read:
/// `recipe/`, `recipe_filter/`, `loot_table/`, `advancement/`, `trade_set/`,
/// `villager_trade/`, `dialog/`, `damage_type/`, `dimension_type/`, `data_maps/`, enchantments, songs,
/// instruments, predicates, item modifiers, trim materials and patterns, or
/// `tags/<sub>/`, where the id keeps the sub path so a nested tag kind stays addressable.
fn content_id(file: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let (kind, rest) = rest.split_once('/')?;
    let kind = match kind {
        "recipe" | "recipe_filter" | "loot_table" | "advancement" | "trade_set"
        | "villager_trade" | "dialog" | "damage_type" | "dimension_type" | "data_maps"
        | "enchantment" | "jukebox_song" | "instrument" | "predicate" | "item_modifier"
        | "trim_material" | "trim_pattern" => kind,
        // A tag id is `{ns}:{path-under-tags}` (`tags/worldgen/biome/
        // test:sub/mytag`), the whole path after the namespace, so nested
        // registries keep their directory (`TagLoader`'s id shape).
        "tags" => {
            return Some(format!("tags/{ns}:{}", rest.strip_suffix(".json")?));
        }
        _ => return None,
    };
    Some(format!("{kind}/{ns}:{}", rest.strip_suffix(".json")?))
}
/// `ns:path` for a file `data/<ns>/<kind>/<path>.<ext>`.
fn resource_id(file: &str, kind: &str, ext: &str) -> Option<String> {
    let rest = file.strip_prefix("data/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest
        .strip_prefix(kind)?
        .strip_prefix('/')?
        .strip_suffix(ext)?;
    Some(format!("{ns}:{path}"))
}
/// The entry files of mods' datapack registries in `packs`, lowest pack first: each file
/// `data/<entry namespace>/<registry namespace>/<registry path>/<entry>.json` of a registry
/// in `registries` (`(namespace, path)` of its key), by that path, each path's versions in
/// pack order, each with its pack's id. Only the highest accepted version is read into
/// the returned map. `fallback` permits trying lower versions after a condition refusal;
/// registered conditions disable fallback so losing callbacks never run. The folders
/// are the mods' own, so the fixed kinds `DataStore` lists never cover them.
pub fn registry_files(
    packs: &[&DataPack],
    registries: &[(&str, &str)],
    fallback: bool,
    mut accepts: impl FnMut(&str, &str, &[u8]) -> bool,
) -> Result<BTreeMap<String, Vec<(String, Vec<u8>)>>, Box<mod_api::DatapackLoadError>> {
    let mut kinds: Vec<&str> = registries.iter().map(|(namespace, _)| *namespace).collect();
    kinds.sort_unstable();
    kinds.dedup();
    let mut sources: BTreeMap<String, Vec<&DataPack>> = BTreeMap::new();
    for pack in packs {
        for file in pack.list_kinds(&kinds) {
            let in_registry = file
                .strip_prefix("data/")
                .and_then(|rest| rest.split_once('/'))
                .is_some_and(|(_, rest)| {
                    file.ends_with(".json")
                        && registries.iter().any(|(namespace, path)| {
                            rest.strip_prefix(namespace)
                                .and_then(|rest| rest.strip_prefix('/'))
                                .and_then(|rest| rest.strip_prefix(path))
                                .is_some_and(|rest| rest.starts_with('/'))
                        })
                });
            if in_registry {
                sources.entry(file).or_default().push(pack);
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    for (file, packs) in sources {
        for pack in packs.into_iter().rev() {
            // Read at most the custom-registry limit plus a refusal byte, including
            // decompressed output. No losing payload remains resident.
            let bytes = pack
                .source
                .read_head(&file, mod_api::MAX_DATAPACK_ENTRY_BYTES as u64 + 1)
                .unwrap_or_default();
            let oversized = bytes.len() > mod_api::MAX_DATAPACK_ENTRY_BYTES;
            if oversized || accepts(&file, &pack.id, &bytes) {
                total += bytes.len();
                if !oversized && total > mod_api::MAX_SNAPSHOT_BYTES {
                    let (namespace, path) = registries
                        .iter()
                        .find(|(namespace, path)| {
                            file.splitn(3, '/').nth(2).is_some_and(|rest| {
                                rest.starts_with(&format!("{namespace}/{path}/"))
                            })
                        })
                        .expect("listed custom registry file");
                    let rest = file.strip_prefix("data/").unwrap();
                    let (entry_namespace, rest) = rest.split_once('/').unwrap();
                    let entry = rest
                        .strip_prefix(&format!("{namespace}/{path}/"))
                        .unwrap()
                        .strip_suffix(".json")
                        .unwrap();
                    return Err(Box::new(mod_api::DatapackLoadError {
                        registry: format!("{namespace}:{path}"),
                        entry: format!("{entry_namespace}:{entry}"),
                        file,
                        pack: pack.id.clone(),
                        reason: mod_api::DatapackLoadReason::CandidateTooLarge,
                    }));
                }
                files.insert(file.clone(), vec![(pack.id.clone(), bytes)]);
                break;
            }
            if !fallback {
                break;
            }
        }
    }
    Ok(files)
}

impl DataStore {
    pub fn load(packs: &[&DataPack]) -> Self {
        Self::load_impl(packs, None)
    }
    /// Command resources and function tags share the owning server load's
    /// conditions; raw JSON stacks remain intact for world-table readers.
    pub fn load_with_conditions(
        packs: &[&DataPack],
        conditions: &horizon_world::conditions::LoadConditions,
    ) -> Self {
        Self::load_impl(packs, Some(conditions))
    }
    fn load_impl(
        packs: &[&DataPack],
        conditions: Option<&horizon_world::conditions::LoadConditions>,
    ) -> Self {
        let mut s = Self::default();
        for pack in packs {
            // `tags` walks the whole tags tree, so a `tags/function` file is
            // listed by both kinds: handle every file once.
            let mut seen = std::collections::BTreeSet::new();
            let builtin = pack.id == "vanilla";
            for f in pack.list_kinds(&[
                "advancement",
                "damage_type",
                "data_maps",
                "dialog",
                "dimension_type",
                "enchantment",
                "function",
                "instrument",
                "item_modifier",
                "jukebox_song",
                "loot_table",
                "predicate",
                "recipe",
                "recipe_filter",
                "slot_source",
                "tags",
                "trade_set",
                "trim_material",
                "trim_pattern",
                "villager_trade",
            ]) {
                if !seen.insert(f.clone()) {
                    continue;
                }
                // Preserve JSON stacks independently of the command views
                // below. World-table readers evaluate the winning resource's
                // conditions using their captured server inventory.
                if !builtin
                    && let Some(id) = content_id(&f)
                    && let Some(bytes) = pack.read(&f)
                {
                    s.files.entry(id).or_default().push(bytes);
                }
                if let Some(id) = resource_id(&f, "function", ".mcfunction")
                    && let Some(bytes) = pack.read(&f)
                {
                    match parse_function(&id, &String::from_utf8_lossy(&bytes)) {
                        Ok(func) => {
                            s.functions.insert(id, func);
                        }
                        Err(e) => s.errors.push((id, e)),
                    }
                } else if let Some(id) = resource_id(&f, "tags/function", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    let mut entries = tag_entries(&v);
                    if let Some(removes) = v["remove"].as_array() {
                        entries.extend(
                            tag_entries(&serde_json::json!({ "values": removes }))
                                .into_iter()
                                .map(|(id, required, _)| (id, required, true)),
                        );
                    }
                    let slot = s.function_tags.entry(id).or_default();
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    slot.extend(entries);
                } else if let Some(id) = resource_id(&f, "predicate", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice(&bytes)
                {
                    s.predicates.insert(id, v);
                } else if let Some(id) = resource_id(&f, "item_modifier", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice(&bytes)
                {
                    s.item_modifiers.insert(id, v);
                } else if let Some(id) = resource_id(&f, "tags/slot_source", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    let mut entries = tag_entries(&v);
                    if let Some(removes) = v["remove"].as_array() {
                        entries.extend(
                            tag_entries(&serde_json::json!({ "values": removes }))
                                .into_iter()
                                .map(|(id, required, _)| (id, required, true)),
                        );
                    }
                    let slot = s.slot_source_tags.entry(id).or_default();
                    if v["replace"].as_bool() == Some(true) {
                        slot.clear();
                    }
                    slot.extend(entries);
                } else if let Some(id) = resource_id(&f, "slot_source", ".json")
                    && let Some(bytes) = pack.read(&f)
                    && let Ok(v) = serde_json::from_slice(&bytes)
                {
                    s.slot_sources.insert(id, v);
                }
            }
        }
        if let Some(conditions) = conditions {
            s.predicates
                .retain(|id, value| conditions.allows_id(value, "predicate", id));
            s.item_modifiers
                .retain(|id, value| conditions.allows_id(value, "item_modifier", id));
        }
        s
    }
    /// `ServerFunctionLibrary.getTag`: only valid function tags expand
    /// (nested tags included, each function once).
    pub fn tag_functions(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen_fns = std::collections::HashSet::new();
        let mut seen_tags = std::collections::HashSet::new();
        let invalid = self.invalid_function_tags(id);
        self.expand(id, &mut out, &mut seen_fns, &mut seen_tags, &invalid);
        out
    }
    /// `TagLoader.tryBuildTag`: missing required members invalidate their tag and
    /// required parents. Optional invalid children contribute nothing.
    /// Resolve against the current accepted function map, without caching a
    /// candidate store or changing the existing expansion's cycle handling.
    fn invalid_function_tags<'a>(&'a self, id: &'a str) -> std::collections::HashSet<&'a str> {
        let mut invalid = std::collections::HashSet::new();
        let mut parents: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();
        let mut seen = std::collections::HashSet::new();
        let mut pending = vec![id];
        while let Some(tag) = pending.pop() {
            if !seen.insert(tag) {
                continue;
            }
            let Some(entries) = self.function_tags.get(tag) else {
                invalid.insert(tag);
                continue;
            };
            for (entry, required, remove) in entries {
                if *remove {
                    continue;
                }
                if let Some(child) = entry.strip_prefix('#') {
                    pending.push(child);
                    if *required {
                        parents.entry(child).or_default().push(tag);
                    }
                } else if *required && !self.functions.contains_key(entry) {
                    invalid.insert(tag);
                }
            }
        }
        pending.extend(invalid.iter().copied());
        while let Some(child) = pending.pop() {
            for &parent in parents.get(child).into_iter().flatten() {
                if invalid.insert(parent) {
                    pending.push(parent);
                }
            }
        }
        invalid
    }
    /// Depth-first walk keeping first-seen order. Each tag is expanded at most
    /// once, so reference cycles end and nesting depth is not capped.
    fn expand<'a>(
        &'a self,
        id: &'a str,
        out: &mut Vec<String>,
        seen_fns: &mut std::collections::HashSet<&'a str>,
        seen_tags: &mut std::collections::HashSet<&'a str>,
        invalid: &std::collections::HashSet<&'a str>,
    ) {
        if invalid.contains(id) || !seen_tags.insert(id) {
            return;
        }
        let Some(entries) = self.function_tags.get(id) else {
            return;
        };
        for (e, required, remove) in entries {
            // A removal (a `remove` list's entry, after the file's values)
            // takes an id or a tag's members out of the set built so far; it
            // is optional: an unknown id removes nothing and never
            // invalidates the tag.
            if *remove {
                match e.strip_prefix('#') {
                    Some(inner) => {
                        let mut members = Vec::new();
                        self.expand(
                            inner,
                            &mut members,
                            &mut Default::default(),
                            seen_tags,
                            invalid,
                        );
                        out.retain(|f| !members.contains(f));
                        seen_fns.retain(|f| !members.iter().any(|member| member.as_str() == *f));
                    }
                    None => {
                        out.retain(|f| f.as_str() != e.as_str());
                        seen_fns.remove(e.as_str());
                    }
                }
                continue;
            }
            let _ = required;
            match e.strip_prefix('#') {
                Some(inner) => {
                    let mut members = Vec::new();
                    let mut member_ids = Default::default();
                    self.expand(inner, &mut members, &mut member_ids, seen_tags, invalid);
                    for member in members {
                        if let Some(&id) = member_ids.get(member.as_str())
                            && seen_fns.insert(id)
                        {
                            out.push(member);
                        }
                    }
                }
                None => {
                    if self.functions.contains_key(e) && seen_fns.insert(e) {
                        out.push(e.clone());
                    }
                }
            }
        }
        seen_tags.remove(id);
    }
}
fn normalize(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
/// An entry of a tag file's `values` or `remove`: its id, whether it is
/// required, and whether it removes (`TagLoader.EntryWithSource.remove`).
type TagEntryOf = (String, bool, bool);

fn tag_entries(v: &serde_json::Value) -> Vec<TagEntryOf> {
    v["values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| match e {
            serde_json::Value::String(s) => Some((hash_normalize(s), true, false)),
            serde_json::Value::Object(o) => Some((
                hash_normalize(o.get("id")?.as_str()?),
                o.get("required")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                o.get("remove").and_then(serde_json::Value::as_bool) == Some(true),
            )),
            _ => None,
        })
        .collect()
}
fn hash_normalize(s: &str) -> String {
    match s.strip_prefix('#') {
        Some(rest) => format!("#{}", normalize(rest)),
        None => normalize(s),
    }
}

/// Whether every flag `pack` asks for (`Pack.getRequestedFeatures`) is among
/// the world's `enabled_features`.
fn features_enabled(pack: &DataPack, enabled_features: &[String]) -> bool {
    pack.features()
        .iter()
        .all(|f| enabled_features.iter().any(|e| e == f))
}

/// The ids of `packs` (a world's, from `discover`) that are enabled, lowest
/// priority first: those `<world>/datapacks.json` lists as enabled, in its
/// order, then every pack it lists neither way, in name order.
///
/// `enabled_features` are the world's feature flags
/// (`WorldDataConfiguration.enabledFeatures`): a pack a pack file selected
/// while asking for missing features comes off
/// (`MinecraftServer.configurePackRepository`), and a feature pack whose
/// features the world enables is forced on however the world's selection
/// feels about it (`MinecraftServer.enableForcedFeaturePacks`).
pub fn enabled_ids(world: &Path, packs: &[DataPack], enabled_features: &[String]) -> Vec<String> {
    let saved: Option<serde_json::Value> = std::fs::read(world.join("datapacks.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let list = |key: &str| -> Vec<String> {
        saved
            .as_ref()
            .and_then(|v| v[key].as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let disabled = list("disabled");
    let mut enabled: Vec<String> = list("enabled")
        .into_iter()
        .filter(|id| packs.iter().any(|p| &p.id == id))
        .filter(|id| {
            packs
                .iter()
                .find(|p| &p.id == id)
                .is_none_or(|p| features_enabled(p, enabled_features))
        })
        .collect();
    for p in packs {
        // A pack nobody mentions comes on (`PackRepository.reload` selects
        // every new pack), but a feature pack stays off until the world
        // enabled its flag (`PackSource.FEATURE` starts unselected).
        if !enabled.contains(&p.id)
            && !disabled.contains(&p.id)
            && p.id.starts_with("file/")
            && features_enabled(p, enabled_features)
        {
            enabled.push(p.id.clone());
        }
    }
    for p in packs {
        if p.is_feature()
            && !p.features().is_empty()
            && features_enabled(p, enabled_features)
            && !enabled.contains(&p.id)
        {
            enabled.push(p.id.clone());
        }
    }
    selected_ids(packs.iter(), &enabled)
}

/// Rebuild a data-pack selection, preserving its order and inserting required packs.
pub fn selected_ids<'a>(
    packs: impl Iterator<Item = &'a DataPack>,
    selected: &[String],
) -> Vec<String> {
    // Existing selections keep their order; mandatory additions use the same
    // insertion rules as the pack screen.
    let available = packs
        .map(|pack| {
            pack.selection.as_ref().map_or_else(
                || crate::packs::Selectable::optional(&pack.id, true),
                crate::pack_screen::PackInfo::selectable,
            )
        })
        .collect();
    crate::packs::SelectionModel::new(available, selected)
        .selected
        .into_iter()
        .rev()
        .map(|pack| pack.id)
        .collect()
}

/// The level stems of `packs` (`data/<namespace>/dimension/<path>.json`, lowest priority first,
/// as `enabled_ids` orders them): the id `namespace:path` and the parsed JSON, a later pack's
/// file for an id replacing an earlier one's. This is `context.datapackDimensions()` of
/// `WorldOpenFlows.loadWorldStem`.
pub fn dimension_stems(packs: &[&DataPack]) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for pack in packs {
        for file in pack.list_kinds(&["dimension"]) {
            let Some(rest) = file.strip_prefix("data/") else {
                continue;
            };
            let Some((namespace, rest)) = rest.split_once('/') else {
                continue;
            };
            let Some(path) = rest
                .strip_prefix("dimension/")
                .and_then(|p| p.strip_suffix(".json"))
            else {
                continue;
            };
            // An id that is no safe folder name (`..`, an empty segment) would open a level
            // outside the world's `dimensions` folder.
            let id = format!("{namespace}:{path}");
            if !horizon_core::level_id::valid_level_id(&id) {
                continue;
            }
            if let Some(json) = pack
                .read(&file)
                .and_then(|b| serde_json::from_slice(&b).ok())
            {
                out.insert(id, json);
            }
        }
    }
    out
}

// ---- worldgen ----------------------------------------------------------------

/// The worldgen entries of `packs` (lowest priority first, the built-in pack
/// left out: the prepared bundles are it) for the chunk generator, and what
/// it leaves out, for the log. `biomes` says which biomes the game has: a
/// pack's new biome is left out (the game has no colours, sounds or id for it).
pub fn worldgen_overrides(
    packs: &[&DataPack],
    biomes: impl Fn(&str) -> bool,
) -> (WorldgenOverrides, Vec<String>) {
    worldgen_overrides_over(overrides::mod_dimensions(), packs, biomes)
}

/// [`worldgen_overrides`] over the levels `mods` registered, which sit below
/// every pack.
pub fn worldgen_overrides_over(
    mods: &[overrides::ModDimension],
    packs: &[&DataPack],
    biomes: impl Fn(&str) -> bool,
) -> (WorldgenOverrides, Vec<String>) {
    let mut out = WorldgenOverrides::default();
    for dimension in mods {
        out.add_mod_dimension(dimension);
    }
    let mut not_read: BTreeMap<String, usize> = BTreeMap::new();
    let mut warnings = Vec::new();
    for pack in packs {
        for file in pack.list_kinds(&overrides::PACK_KINDS) {
            if let Some(id) = overrides::template_id(&file) {
                out.set_template(
                    id,
                    horizon_assets::resource_source::Resource::file(pack.source.clone(), file),
                );
                continue;
            }
            let read = || String::from_utf8(pack.read(&file)?).ok();
            match out.add_file(&pack.id, &file, read) {
                PackFile::Read | PackFile::Other => {}
                PackFile::NotRead(kind) => {
                    *not_read.entry(format!("{}: {kind}", pack.id)).or_default() += 1
                }
                PackFile::Unreadable => warnings.push(format!("{}: {file}: unreadable", pack.id)),
            }
        }
    }
    warnings.extend(out.finish(biomes));
    for (kind, files) in not_read {
        warnings.push(format!("{kind}: {files} files the generator does not read"));
    }
    (out, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_level_outside_the_worlds_folder_is_not_a_level() {
        let dir = std::env::temp_dir().join(format!("horizon-pack-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p");
        for path in [
            "data/test/dimension/pocket.json",
            "data/test/dimension/a/b.json",
            "data/test/dimension/my pocket.json",
            "data/test/dimension/Pocket.json",
            "data/Test/dimension/pocket.json",
        ] {
            let file = pack.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "{}").unwrap();
        }
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"","pack_format":15}}"#,
        )
        .unwrap();
        let packs = discover(&dir);
        let refs: Vec<&DataPack> = packs.iter().collect();
        let ids: Vec<String> = dimension_stems(&refs).into_keys().collect();
        assert_eq!(ids, ["test:a/b", "test:pocket"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pack_replaces_a_mods_level() {
        let root = std::env::temp_dir().join(format!("horizon-mod-level-{}", std::process::id()));
        let world = root.join("world");
        let pack = world.join("datapacks/over");
        let types = pack.join("data/test/dimension_type");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            pack.join("pack.mcmeta"),
            r#"{"pack":{"description":"","pack_format":15}}"#,
        )
        .unwrap();
        std::fs::write(
            types.join("pocket.json"),
            r#"{"min_y":0,"height":64,"logical_height":64}"#,
        )
        .unwrap();
        let packs = discover(&world);
        let mods = [overrides::ModDimension {
            id: "test:pocket".into(),
            stem: r#"{"type":"test:pocket"}"#.into(),
            dimension_type: r#"{"min_y":-128,"height":448,"logical_height":448}"#.into(),
            sea_level: None,
        }];
        let height = |packs: &[&DataPack]| {
            let (o, _) = worldgen_overrides_over(&mods, packs, |_| true);
            o.level_height("test:pocket").unwrap().height
        };
        assert_eq!(height(&[]), 448);
        assert_eq!(height(&[&packs[0]]), 64);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn structure_templates_read_archive_generations_without_extraction() {
        use std::io::Write;
        use {
            horizon_core::nbt::{Limits, NamedTag},
            horizon_world::{
                registry::BlockRegistry,
                worldgen::structure::{StateTransforms, StructureResources},
            },
        };
        let root =
            std::env::temp_dir().join(format!("horizon-template-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        struct Scratch(PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(root);
        let archive = scratch.0.join("templates.jar");
        let write_archive = |width| {
            let tag = Tag::compound([
                (
                    "size",
                    Tag::List(vec![Tag::Int(width), Tag::Int(3), Tag::Int(4)]),
                ),
                (
                    "palette",
                    Tag::List(vec![Tag::compound([(
                        "Name",
                        Tag::String("minecraft:stone".into()),
                    )])]),
                ),
                (
                    "blocks",
                    Tag::List(vec![Tag::compound([
                        (
                            "pos",
                            Tag::List(vec![Tag::Int(1), Tag::Int(0), Tag::Int(1)]),
                        ),
                        ("state", Tag::Int(0)),
                    ])]),
                ),
                ("entities", Tag::List(Vec::new())),
            ]);
            let bytes = horizon_core::nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag,
                },
                Limits::templates(),
            )
            .unwrap();
            let mut gzip =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gzip.write_all(&bytes).unwrap();
            let temporary = scratch.0.join("replacement.jar");
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&temporary).unwrap());
            zip.start_file(
                "data/example/structure/nested/piece.nbt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(&gzip.finish().unwrap()).unwrap();
            zip.finish().unwrap();
            std::fs::rename(temporary, &archive).unwrap();
        };
        write_archive(2);
        let old =
            DataPack::from_source("passive".into(), ResourceSource::archive(&archive).unwrap());
        let (before, warnings) = worldgen_overrides(&[&old], |_| true);
        assert!(warnings.is_empty(), "{warnings:?}");
        write_archive(5);
        let new =
            DataPack::from_source("passive".into(), ResourceSource::archive(&archive).unwrap());
        let (after, warnings) = worldgen_overrides(&[&new], |_| true);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_ne!(
            before, after,
            "a reopened archive invalidates the generator cache"
        );
        let registry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
                (
                    "states",
                    Tag::List(vec![
                        Tag::compound([("id", Tag::String("minecraft:air".into()))]),
                        Tag::compound([("id", Tag::String("minecraft:stone".into()))]),
                    ]),
                ),
                (
                    "defaults",
                    Tag::compound([
                        ("minecraft:air", Tag::Int(0)),
                        ("minecraft:stone", Tag::Int(1)),
                    ]),
                ),
            ]))
            .unwrap(),
        ));
        let manager = |overrides: &WorldgenOverrides| {
            let transforms = StateTransforms::from_json(
                r#"{"rotate":[[0,0,0],[1,1,1]],"mirror":[[0,0],[1,1]],"full_block":[false,true]}"#,
                2,
            )
            .unwrap();
            StructureResources::new(
                registry,
                transforms,
                &scratch.0.join("unpacked-templates"),
                BTreeMap::new(),
                &BTreeMap::new(),
            )
            .with_templates(overrides.templates().clone())
        };
        // Neither generation has read its NBT yet; the old one must still
        // read from the reader retained before the archive was replaced.
        let first = manager(&before).template("example:nested/piece").unwrap();
        let second = manager(&after).template("example:nested/piece").unwrap();
        assert_eq!(first.size, [2, 3, 4]);
        assert_eq!(second.size, [5, 3, 4]);
        let blocks = second.palettes()[0].blocks();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].pos, [1, 0, 1]);
        assert_eq!(blocks[0].state.0, 1);
        assert!(!scratch.0.join("data").exists());
        assert!(!scratch.0.join("unpacked-templates").exists());
    }

    #[test]
    fn folder_metadata_is_required_before_contents_are_discovered() {
        let root =
            std::env::temp_dir().join(format!("horizon-folder-metadata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, metadata) in [
            ("broken", Some("{")),
            ("root_empty", Some("{}")),
            ("root_array", Some("[]")),
            ("pack_empty", Some(r#"{"pack":{}}"#)),
            ("pack_string", Some(r#"{"pack":"bad"}"#)),
            ("missing", None),
            (
                "full",
                Some(
                    r#"{"pack":{"description":"full","pack_format":121,"min_format":[121,0],"max_format":[121,0]}}"#,
                ),
            ),
            (
                "description",
                Some(r#"{"pack":{"description":"description only"}}"#),
            ),
            ("numeric", Some(r#"{"pack":{"description":42}}"#)),
            ("null", Some(r#"{"pack":{"description":null}}"#)),
            ("bom", Some("\u{feff}{\"pack\":{\"description\":\"BOM\"}}")),
            (
                "comment",
                Some("{\"pack\":{\"description\":\"comment\"}}\n// trailing"),
            ),
        ] {
            let dir = root.join(name);
            std::fs::create_dir_all(dir.join("data/h146/function")).unwrap();
            std::fs::write(
                dir.join(format!("data/h146/function/{name}.mcfunction")),
                "scoreboard players set admitted h146 1\n",
            )
            .unwrap();
            if let Some(metadata) = metadata {
                std::fs::write(dir.join("pack.mcmeta"), metadata).unwrap();
            }
        }
        let packs = discover_folder(&root);
        assert_eq!(
            packs.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            [
                "file/bom",
                "file/comment",
                "file/description",
                "file/full",
                "file/numeric"
            ]
        );
        let store = DataStore::load(&packs.iter().collect::<Vec<_>>());
        assert_eq!(
            store
                .functions
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "h146:bom",
                "h146:comment",
                "h146:description",
                "h146:full",
                "h146:numeric"
            ]
        );
        let numeric = packs.iter().find(|p| p.id == "file/numeric").unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&numeric.read("pack.mcmeta").unwrap()).unwrap();
        assert_eq!(raw["pack"]["description"], 42);
        // Builtin and feature construction still have their independent paths.
        let resources = root.join("resources");
        let feature = resources.join("data/minecraft/datapacks/feature");
        std::fs::create_dir_all(&feature).unwrap();
        std::fs::write(
            feature.join("pack.mcmeta"),
            r#"{"pack":{"description":"feature"}}"#,
        )
        .unwrap();
        assert_eq!(
            feature_packs(&resources)
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ["feature"]
        );
        assert_eq!(DataPack::builtin(&resources).id, "vanilla");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn data_map_files_are_content_files_keyed_by_map_namespace_and_registry() {
        assert_eq!(
            content_id("data/mymod/data_maps/item/fuel.json").as_deref(),
            Some("data_maps/mymod:item/fuel")
        );
        assert_eq!(content_id("data/mymod/data_maps/item/fuel.txt"), None);
    }

    #[test]
    fn a_pack_that_cannot_be_copied_is_reported() {
        let root = std::env::temp_dir().join(format!("hz-datapack-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack = root.join("pack");
        std::fs::create_dir_all(pack.join("data")).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
        std::fs::write(pack.join("data/functions.mcfunction"), "").unwrap();
        // A file where the pack's folder goes: every copy under it fails.
        let blocked = root.join("blocked");
        std::fs::write(&blocked, "").unwrap();
        assert!(!copy_pack(&pack, &blocked));
        let dir = root.join("packs");
        assert!(copy_pack(&pack, &dir));
        assert!(dir.join("pack/pack.mcmeta").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_game_folder_s_new_world_packs_go_into_new_worlds_only() {
        let root = std::env::temp_dir().join(format!("hz-new-world-packs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (game, world, old) = (root.join("game"), root.join("new"), root.join("old"));
        assert!(seed_create_world(&game).is_none());
        assert_eq!(install_new_world_packs(&game, &world), Ok(()));
        assert!(!world.join("datapacks").exists());
        for pack in ["void", "islands"] {
            let dir = game.join(NEW_WORLD_PACKS).join(pack);
            std::fs::create_dir_all(dir.join("data/v/function")).unwrap();
            std::fs::write(dir.join("pack.mcmeta"), r#"{"pack":{"description":""}}"#).unwrap();
            std::fs::write(dir.join("data/v/function/a.mcfunction"), "say a").unwrap();
        }
        let (temp, ids) = seed_create_world(&game).unwrap();
        assert_eq!(ids, ["file/islands", "file/void"]);
        assert!(temp.join("void/data/v/function/a.mcfunction").is_file());
        let _ = std::fs::remove_dir_all(&temp);
        install_new_world_packs(&game, &world).unwrap();
        let ids: Vec<String> = discover(&world).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["file/islands", "file/void"]);
        // A world that exists already keeps the packs it has.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("level.dat"), "").unwrap();
        install_new_world_packs(&game, &old).unwrap();
        assert!(discover(&old).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enabled_packs_override_worldgen_in_order() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-worldgen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |pack: &str, p: &str, t: &str| {
            let path = dir.join("datapacks").join(pack).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        for pack in ["a", "b", "off"] {
            write(pack, "pack.mcmeta", r#"{"pack":{"description":"fixture"}}"#);
            write(pack, "data/minecraft/worldgen/noise/cave.json", pack);
            write(pack, "data/p/structure/room/a.nbt", pack);
            write(
                pack,
                "data/minecraft/tags/worldgen/biome/is_hill.json",
                "{\"values\":[]}",
            );
        }
        write("a", "data/minecraft/worldgen/biome/new_peaks.json", "{}");
        write("a", "data/minecraft/worldgen/biome/plains.json", "{}");
        write(
            "a",
            "data/minecraft/worldgen/world_preset/normal.json",
            "{}",
        );
        write(
            "b",
            "data/minecraft/dimension_type/overworld.json",
            "{\"min_y\":-64,\"height\":2096}",
        );
        // `b` is enabled first, so `a` (enabled after it) wins.
        std::fs::write(
            dir.join("datapacks.json"),
            "{\"enabled\":[\"file/b\",\"file/a\"],\"disabled\":[\"file/off\"]}",
        )
        .unwrap();
        let packs = discover(&dir);
        let enabled = enabled_ids(&dir, &packs, &[]);
        assert_eq!(enabled, ["file/b", "file/a"]);
        let enabled: Vec<&DataPack> = enabled
            .iter()
            .map(|id| packs.iter().find(|p| &p.id == id).unwrap())
            .collect();
        let (o, warnings) = worldgen_overrides(&enabled, |b| b == "minecraft:plains");
        assert_eq!(
            o.entries("noises").collect::<Vec<_>>(),
            [("minecraft:cave", "a")]
        );
        assert_eq!(o.templates().len(), 1);
        assert_eq!(o.templates()["p:room/a"].read().unwrap(), b"a");
        assert_eq!(
            o.entries("biomes").collect::<Vec<_>>(),
            [("minecraft:plains", "{}")]
        );
        let tag_packs: Vec<&str> = o.tags("biome_tags").map(|(_, p, _)| p).collect();
        assert_eq!(tag_packs, ["file/b", "file/a"]);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("minecraft:new_peaks"));
        assert!(warnings[1].contains("worldgen/world_preset"));
        assert_eq!(
            o.level_height("minecraft:overworld").map(|h| h.height),
            Some(2096)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    use horizon_core::nbt::parse_snbt;

    #[test]
    fn parses_comments_continuations_and_macros() {
        let f = parse_function(
            "a:b",
            "# a comment\n\nsay hi\n  say \\\n  there  \n$say $(who) is $(n)\n$tell $(who) $(who)\n",
        )
        .unwrap();
        assert_eq!(
            f.lines,
            vec![
                FunctionLine::Command("say hi".into()),
                FunctionLine::Command("say there".into()),
                FunctionLine::Macro("say $(who) is $(n)".into()),
                FunctionLine::Macro("tell $(who) $(who)".into()),
            ]
        );
        assert_eq!(f.parameters, ["who", "n"]);
        assert!(
            parse_function("a:b", "/say hi")
                .unwrap_err()
                .contains("did you mean 'say'")
        );
        assert!(
            parse_function("a:b", "//x")
                .unwrap_err()
                .contains("use '#'")
        );
        assert!(parse_function("a:b", "say \\").is_err());
        assert!(parse_function("a:b", "$say $(bad name)").is_err());
    }

    #[test]
    fn macros_substitute_vanilla_text() {
        let f = parse_function("a:b", "$say $(s) $(i) $(d) $(l) $(b) $(c)\nsay plain").unwrap();
        let args = parse_snbt("{s:\"hi\",i:5,d:1.5d,l:7L,b:1b,c:{x:1}}")
            .unwrap()
            .0;
        let out = f.instantiate(Some(&args)).unwrap();
        assert_eq!(out, ["say hi 5 1.5 7 1 {x:1}", "say plain"]);
        let args = parse_snbt("{s:\"x\",i:1,d:3.0d,l:1L,b:0b,c:{}}").unwrap().0;
        assert_eq!(f.instantiate(Some(&args)).unwrap()[0], "say x 1 3 1 0 {}");
        assert_eq!(
            f.instantiate(None).unwrap_err().0,
            "commands.function.error.missing_arguments"
        );
        let partial = parse_snbt("{s:\"x\"}").unwrap().0;
        assert_eq!(
            f.instantiate(Some(&partial)).unwrap_err().0,
            "commands.function.error.missing_argument"
        );
        // A plain function needs no arguments.
        let plain = parse_function("a:p", "say 1").unwrap();
        assert_eq!(plain.instantiate(None).unwrap(), ["say 1"]);
    }

    /// `StringTemplate.fromString` throws "No variables in macro" for a `$`
    /// line with no `$(…)`, so `FunctionBuilder.addMacro` fails and
    /// `ServerFunctionLibrary.reload` leaves the whole file out: `/function`
    /// reports it unknown instead of running the line unchanged.
    #[test]
    fn a_macro_line_without_a_variable_drops_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-macro-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/macros");
        let write = |p: &str, t: &str| {
            let path = pack.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "pack.mcmeta",
            "{\"pack\":{\"description\":\"Macros\",\"pack_format\":1}}",
        );
        // A plain line before and after does not save the file.
        write(
            "data/t/function/novar.mcfunction",
            "say before\n$say hello\nsay after\n",
        );
        write("data/t/function/lone.mcfunction", "$\n");
        // A `$` not followed by `(` is text, not a variable.
        write(
            "data/t/function/dollar.mcfunction",
            "$say $(who) costs $5\n",
        );
        write("data/t/function/plain.mcfunction", "say plain\n");
        let packs = discover(&dir);
        let store = DataStore::load(&[&packs[0]]);
        for id in ["t:novar", "t:lone"] {
            assert!(!store.functions.contains_key(id), "{id} was registered");
            assert!(
                store
                    .errors
                    .iter()
                    .any(|(e, m)| e == id && m.contains("No variables in macro")),
                "{id} has no load error: {:?}",
                store.errors
            );
        }
        let dollar = &store.functions["t:dollar"];
        assert_eq!(dollar.parameters, ["who"]);
        assert!(store.functions.contains_key("t:plain"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packs_provide_functions_and_tags() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/mypack");
        let write = |p: &str, t: &str| {
            let path = pack.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "pack.mcmeta",
            "{\"pack\":{\"description\":\"Hello\",\"pack_format\":1}}",
        );
        write(
            "data/mine/function/a.mcfunction",
            "say a\nfunction mine:sub/b\n",
        );
        write("data/mine/function/sub/b.mcfunction", "say b");
        write(
            "data/minecraft/tags/function/tick.json",
            "{\"values\":[\"mine:a\",\"#mine:more\",{\"id\":\"mine:gone\",\"required\":false}]}",
        );
        write(
            "data/mine/tags/function/more.json",
            "{\"values\":[\"mine:sub/b\",\"mine:a\"]}",
        );
        write(
            "data/mine/predicate/p.json",
            "{\"condition\":\"minecraft:random_chance\",\"chance\":1}",
        );
        write(
            "data/mine/dialog/config/points.json",
            "{\"type\":\"minecraft:notice\",\"title\":\"Points\"}",
        );
        write("data/mine/dimension_type/pocket.json", "{\"min_y\":-128}");
        let packs = discover(&dir);
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/mypack");
        assert_eq!(packs[0].description(), "Hello");
        let store = DataStore::load(&[&packs[0]]);
        assert_eq!(store.functions.len(), 2);
        assert!(store.functions.contains_key("mine:sub/b"));
        assert_eq!(
            store.tag_functions("minecraft:tick"),
            ["mine:a", "mine:sub/b"]
        );
        assert!(store.predicates.contains_key("mine:p"));
        // Dialogs reach the synchronized registry through the content files.
        assert_eq!(
            store.files["dialog/mine:config/points"],
            [b"{\"type\":\"minecraft:notice\",\"title\":\"Points\"}".to_vec()]
        );
        // So do dimension types: a client takes a level's build height from them.
        assert_eq!(
            store.files["dimension_type/mine:pocket"],
            [b"{\"min_y\":-128}".to_vec()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn function_removals_use_finished_members_and_ignore_missing_ids() {
        let mut store = DataStore::default();
        store.functions.insert(
            "t:f".into(),
            Function {
                id: "t:f".into(),
                lines: Vec::new(),
                parameters: Vec::new(),
            },
        );
        store.function_tags.insert(
            "t:child".into(),
            vec![("t:f".into(), true, false), ("t:f".into(), true, true)],
        );
        store.function_tags.insert(
            "t:parent".into(),
            vec![
                ("t:f".into(), true, false),
                ("#t:child".into(), true, true),
                ("t:missing".into(), true, true),
                ("#t:missing".into(), true, true),
            ],
        );
        assert_eq!(store.tag_functions("t:parent"), ["t:f"]);
        store
            .function_tags
            .insert("t:child".into(), vec![("t:f".into(), true, false)]);
        assert!(store.tag_functions("t:parent").is_empty());
    }

    #[test]
    fn tag_resolution_dedupes_fast_ends_cycles_and_nests_deeply() {
        let mut store = DataStore::default();
        // Every referenced function exists; missing required ones invalidate a tag.
        for id in (0..25_000)
            .map(|i| format!("t:f{i}"))
            .chain(["t:x", "t:y", "t:deep"].map(str::to_owned))
        {
            store.functions.insert(
                id.clone(),
                Function {
                    id,
                    lines: Vec::new(),
                    parameters: Vec::new(),
                },
            );
        }
        let big: Vec<TagEntryOf> = (0..50_000)
            .map(|i| (format!("t:f{}", i % 25_000), true, false))
            .collect();
        store.function_tags.insert("t:big".into(), big);
        let start = std::time::Instant::now();
        let got = store.tag_functions("t:big");
        assert_eq!(got.len(), 25_000);
        assert_eq!(got[0], "t:f0");
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        // A cycle terminates and keeps first-seen order.
        store.function_tags.insert(
            "t:a".into(),
            vec![("#t:b".into(), true, false), ("t:x".into(), true, false)],
        );
        store.function_tags.insert(
            "t:b".into(),
            vec![("#t:a".into(), true, false), ("t:y".into(), true, false)],
        );
        assert_eq!(store.tag_functions("t:a"), ["t:y", "t:x"]);
        // Nesting deeper than the old cap of 16 still resolves.
        for i in 0..40 {
            store.function_tags.insert(
                format!("d:{i}"),
                vec![(format!("#d:{}", i + 1), true, false)],
            );
        }
        store
            .function_tags
            .insert("d:40".into(), vec![("t:deep".into(), true, false)]);
        assert_eq!(store.tag_functions("d:0"), ["t:deep"]);
    }

    #[test]
    fn server_json_resources_keep_ordered_stacks_for_conditional_readers() {
        let root =
            std::env::temp_dir().join(format!("horizon-server-json-stacks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let kinds = [
            "enchantment",
            "jukebox_song",
            "instrument",
            "predicate",
            "item_modifier",
        ];
        let mut packs = Vec::new();
        for name in ["low", "high"] {
            let folder = root.join(name);
            for kind in kinds {
                let file = folder.join(format!("data/demo/{kind}/nested/entry.json"));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, serde_json::json!({"layer":name,"neoforge:conditions":[{"type":"unsupported"}]}).to_string()).unwrap();
            }
            packs.push(DataPack::from_source(
                name.into(),
                ResourceSource::directory(folder),
            ));
        }
        let store = DataStore::load(&packs.iter().collect::<Vec<_>>());
        for kind in kinds {
            let versions = &store.files[&format!("{kind}/demo:nested/entry")];
            assert_eq!(versions.len(), 2);
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&versions[0]).unwrap()["layer"],
                "low"
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&versions[1]).unwrap()["layer"],
                "high"
            );
        }
        assert!(store.predicates.contains_key("demo:nested/entry"));
        assert!(store.item_modifiers.contains_key("demo:nested/entry"));
        let filtered = DataStore::load_with_conditions(
            &packs.iter().collect::<Vec<_>>(),
            &horizon_world::conditions::LoadConditions::vanilla(),
        );
        assert!(filtered.predicates.is_empty());
        assert!(filtered.item_modifiers.is_empty());
        assert_eq!(filtered.files, store.files);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn command_json_selects_before_conditions_and_function_tags_gate_each_layer() {
        let root =
            std::env::temp_dir().join(format!("horizon-command-conditions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut packs = Vec::new();
        for name in ["low", "high"] {
            let folder = root.join(name);
            let conditions = if name == "high" {
                serde_json::json!([{"type":"neoforge:mod_loaded","modid":"provider"}])
            } else {
                serde_json::json!([])
            };
            for kind in ["predicate", "item_modifier"] {
                let file = folder.join(format!("data/demo/{kind}/entry.json"));
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(
                    file,
                    serde_json::json!({"layer":name,"neoforge:conditions":conditions}).to_string(),
                )
                .unwrap();
            }
            let dir = folder.join("data/demo/tags/function");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("load.json"), serde_json::json!({"replace":true,"values":[format!("demo:{name}")],"neoforge:conditions":conditions}).to_string()).unwrap();
            std::fs::write(
                dir.join("unknown.json"),
                r#"{"neoforge:conditions":[{"type":"unsupported"}],"replace":true}"#,
            )
            .unwrap();
            packs.push(DataPack::from_source(
                name.into(),
                ResourceSource::directory(folder),
            ));
        }
        let refs = packs.iter().collect::<Vec<_>>();
        let off = DataStore::load_with_conditions(
            &refs,
            &horizon_world::conditions::LoadConditions::vanilla(),
        );
        assert!(off.predicates.is_empty());
        assert!(off.item_modifiers.is_empty());
        assert_eq!(
            off.function_tags["demo:load"],
            [("demo:high".into(), true, false)]
        );
        assert!(off.function_tags["demo:unknown"].is_empty());
        let enabled = horizon_world::conditions::LoadConditions::new(["provider"]);
        let on = DataStore::load_with_conditions(&refs, &enabled);
        assert_eq!(on.predicates["demo:entry"]["layer"], "high");
        assert_eq!(on.item_modifiers["demo:entry"]["layer"], "high");
        assert_eq!(
            on.function_tags["demo:load"],
            [("demo:high".into(), true, false)]
        );
        assert_eq!(on.files, off.files);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn folder_packs_list_the_needed_kinds_in_walk_order() {
        let dir = std::env::temp_dir().join(format!("hz-datapack-kinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p");
        let files = [
            "data/b/function/z.mcfunction",
            "data/b/function/a/y.mcfunction",
            "data/b/loot_table/l.json",
            "data/b/tags/block/t.json",
            "data/b/tags/function/load.json",
            "data/b/predicate/p.json",
            "data/a/item_modifier/m.json",
            "data/a/function/f.mcfunction",
            "data/a/functions/old.mcfunction",
            "data/a/recipe/r.json",
            "data/a/tags/function/tick.json",
            "data/c/worldgen/noise/n.json",
        ];
        for f in files {
            let path = pack.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }
        std::fs::write(pack.join("pack.mcmeta"), "{}").unwrap();
        let p = DataPack::builtin(&pack);
        let kinds = ["function", "item_modifier", "predicate", "tags/function"];
        let wanted: Vec<String> = p
            .list("data/")
            .into_iter()
            .filter(|f| {
                let rest = f.splitn(3, '/').nth(2).unwrap_or_default();
                kinds.iter().any(|k| rest.starts_with(&format!("{k}/")))
            })
            .collect();
        assert_eq!(wanted.len(), 7);
        assert_eq!(p.list_kinds(&kinds), wanted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn create_world_installs_its_packs() {
        let root = std::env::temp_dir().join(format!("hz-datapack-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (dir, world) = (root.join("mcworld"), root.join("world"));
        let write = |pack: &str, p: &str, t: &str| {
            let path = dir.join(pack).join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, t).unwrap();
        };
        write(
            "fns",
            "pack.mcmeta",
            r#"{"pack":{"description":"fixture","min_format":121,"max_format":121}}"#,
        );
        write("fns", "data/x/function/f.mcfunction", "say hi");
        write(
            "off",
            "pack.mcmeta",
            r#"{"pack":{"description":"fixture","min_format":121,"max_format":121}}"#,
        );
        std::fs::create_dir_all(&dir).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(dir.join("old.zip")).unwrap());
        zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, br#"{"pack":{"pack_format":48}}"#).unwrap();
        zip.finish().unwrap();
        let infos = screen_packs(&dir, &root);
        let ids: Vec<&str> = infos.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["file/fns", "file/off", "file/old.zip", "vanilla"]);
        let compat: Vec<_> = infos.iter().map(|p| p.compat).collect();
        use crate::pack_screen::Compat;
        assert_eq!(
            compat,
            [
                Compat::Compatible,
                Compat::Compatible,
                Compat::TooOld,
                Compat::Compatible
            ]
        );
        let selected = ["vanilla", "file/old.zip", "file/fns"].map(String::from);
        install(&dir, &world, &selected).unwrap();
        let packs = discover(&world);
        let ids: Vec<&str> = packs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["file/fns", "file/off", "file/old.zip"]);
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(world.join("datapacks.json")).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({"enabled": selected, "disabled": ["file/off"]})
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A mod's registry folders are listed per pack, lowest pack first, and only the declared
    /// registries' `.json` files, in any entry namespace.
    #[test]
    fn registry_files_list_the_declared_folders_in_pack_order() {
        let root =
            std::env::temp_dir().join(format!("horizon-registry-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut packs = Vec::new();
        for name in ["low", "high"] {
            let folder = root.join(name);
            for file in [
                "data/example/example/spell/fire.json",
                "data/addon/example/spell/nested/ice.json",
                "data/addon/example/other/skip.json",
                "data/addon/example/spell/note.txt",
                "data/addon/example/spellbook/skip.json",
            ] {
                let path = folder.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, name).unwrap();
            }
            packs.push(DataPack::from_source(
                name.into(),
                ResourceSource::directory(folder),
            ));
        }
        let refs = packs.iter().collect::<Vec<_>>();
        let files = registry_files(&refs, &[("example", "spell")], true, |_, _, _| true).unwrap();
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "data/addon/example/spell/nested/ice.json",
                "data/example/example/spell/fire.json"
            ]
        );
        assert_eq!(
            files["data/example/example/spell/fire.json"],
            vec![("high".to_owned(), b"high".to_vec())]
        );
        assert!(
            registry_files(&refs, &[], true, |_, _, _| true)
                .unwrap()
                .is_empty()
        );
        // Oversized losing files are never read; an oversized winner is retained
        // only as the bounded refusal prefix, not as its full payload.
        let path = "data/example/example/spell/fire.json";
        std::fs::write(root.join("low").join(path), vec![b' '; 1_048_577]).unwrap();
        let files = registry_files(&refs, &[("example", "spell")], true, |_, _, _| true).unwrap();
        assert_eq!(files[path], vec![("high".to_owned(), b"high".to_vec())]);
        std::fs::write(root.join("high").join(path), vec![b' '; 1_048_578]).unwrap();
        let files = registry_files(&refs, &[("example", "spell")], true, |_, _, _| true).unwrap();
        assert_eq!(files[path].len(), 1);
        assert_eq!(files[path][0].1.len(), 1_048_577);
        let _ = std::fs::remove_dir_all(&root);
    }
}
