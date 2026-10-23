//! Horizon shader packs (docs/shader-packs.md): folders in `shaderpacks/`
//! holding a `pack.toml` and WGSL stages, plus Java (OptiFine/Iris) shader
//! packs in folders or `.zip` archives. Loading reads the manifest and the
//! player's option values, puts the prelude before each stage's source and
//! has naga parse and validate it, so a broken pack is reported (file, line,
//! column) and never reaches the GPU.
use horizon_shaderpack::{JavaPack, preprocess::Environment, properties::MenuElement};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// The stage and uniform contract this build provides.
pub const API: u32 = 2;
/// The folder beside `options.txt` that holds the packs.
pub const DIR: &str = "shaderpacks";
pub const PRELUDE: &str = include_str!("shader_pack_prelude.wgsl");
/// What api 2 adds after [`PRELUDE`] (`api = 2` packs only).
pub const PRELUDE_API2: &str = include_str!("shader_pack_prelude_api2.wgsl");
/// [`PRELUDE_API2`]'s bindings that cost a pass when a stage reads them.
const BINDING_SCENE_NORMAL: u32 = 7;
const BINDING_SCENE_MATERIAL: u32 = 8;
const BINDING_OPAQUE_DEPTH: u32 = 9;
const BINDING_SHADOW_DEPTH: u32 = 10;
/// The prelude's vertex entry point, shared by every stage.
pub const VERTEX_ENTRY: &str = "horizon_stage_vertex";
const MAX_STAGES: usize = 16;
const MAX_OPTIONS: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    pack: Info,
    #[serde(default)]
    stage: Vec<StageEntry>,
    #[serde(default)]
    options: BTreeMap<String, OptionEntry>,
    #[serde(default)]
    profiles: BTreeMap<String, BTreeMap<String, toml::Value>>,
    #[serde(default)]
    textures: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub license: String,
    pub api: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StageEntry {
    kind: String,
    shader: String,
    entry: Option<String>,
    scale: Option<f32>,
    format: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionEntry {
    label: Option<String>,
    #[serde(default)]
    description: String,
    default: toml::Value,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    values: Option<Vec<i64>>,
}

/// Where a stage runs: `composite` only so far (docs/shader-packs.md, Stages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StageKind {
    /// After the world pass, before the hand: reads the scene's colour and depth.
    Composite,
}
#[derive(Clone, Debug)]
pub struct Stage {
    pub kind: StageKind,
    /// The stage's file, relative to the pack.
    pub file: String,
    /// The prelude followed by the stage's source.
    pub source: String,
    pub entry: String,
    /// Target size relative to the world's.
    pub scale: f32,
    /// The target's format ([`STAGE_FORMATS`]); `None`: the screen's.
    pub format: Option<String>,
    /// The prelude bindings (group 0) the stage reads.
    pub bindings: BTreeSet<u32>,
}
/// The `format`s a stage's target can have.
pub const STAGE_FORMATS: [&str; 3] = ["rgba8unorm", "rgba16float", "rgb10a2unorm"];
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OptionKind {
    Bool,
    /// An `i32`, cycling through `values`.
    Int,
    /// An `f32` slider.
    Float {
        min: f64,
        max: f64,
        step: f64,
    },
    /// A Java `#define` / `const` string option cycling through `choices`
    /// (or drawn as a slider when `slider` is true and choices are numeric).
    Choice {
        slider: bool,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub struct PackOption {
    /// The WGSL `override` or GLSL `#define`/`const` name it sets.
    pub key: String,
    pub label: String,
    pub description: String,
    pub kind: OptionKind,
    pub values: Vec<i64>,
    pub default: f64,
    pub value: f64,
    /// Allowed `(raw_value, display_label)` pairs for `OptionKind::Choice`.
    pub choices: Vec<(String, String)>,
    pub default_choice: String,
    pub current_choice: String,
}

impl PackOption {
    /// Formatted current value for UI buttons and labels.
    pub fn display_value(&self) -> String {
        match self.kind {
            OptionKind::Bool => {
                if self.value != 0.0 {
                    "ON".into()
                } else {
                    "OFF".into()
                }
            }
            OptionKind::Int => format!("{}", self.value.round() as i64),
            OptionKind::Float { step, .. } => {
                if step >= 1.0 {
                    format!("{:.0}", self.value)
                } else if step >= 0.1 {
                    format!("{:.1}", self.value)
                } else {
                    format!("{:.2}", self.value)
                }
            }
            OptionKind::Choice { .. } => self
                .choices
                .iter()
                .find(|(raw, _)| *raw == self.current_choice)
                .map(|(_, label)| label.clone())
                .unwrap_or_else(|| self.current_choice.clone()),
        }
    }

    /// Normalized `0.0..=1.0` slider position if this option is a slider.
    pub fn slider_pos(&self) -> Option<f32> {
        match self.kind {
            OptionKind::Float { min, max, .. } if max > min => {
                Some(((self.value - min) / (max - min)).clamp(0.0, 1.0) as f32)
            }
            OptionKind::Choice { slider: true } if self.choices.len() > 1 => {
                let idx = self
                    .choices
                    .iter()
                    .position(|(raw, _)| *raw == self.current_choice)
                    .unwrap_or(0);
                Some(idx as f32 / (self.choices.len() - 1) as f32)
            }
            _ => None,
        }
    }

    /// Update the option from a normalized `0.0..=1.0` slider drag.
    pub fn set_slider_pos(&mut self, pos: f32) {
        let pos = f64::from(pos.clamp(0.0, 1.0));
        match self.kind {
            OptionKind::Float { min, max, step } => {
                let mut v = min + pos * (max - min);
                if step > 0.0 {
                    v = (v / step).round() * step;
                }
                self.value = v.clamp(min, max);
            }
            OptionKind::Choice { slider: true } if !self.choices.is_empty() => {
                let idx = (pos * (self.choices.len() - 1) as f64).round() as usize;
                if let Some((raw, _)) = self.choices.get(idx.min(self.choices.len() - 1)) {
                    self.current_choice = raw.clone();
                }
            }
            _ => {}
        }
    }

    /// Cycle or toggle the option (`reverse`: right-click steps backwards).
    pub fn cycle(&mut self, reverse: bool) {
        match self.kind {
            OptionKind::Bool => {
                self.value = if self.value == 0.0 { 1.0 } else { 0.0 };
            }
            OptionKind::Int if !self.values.is_empty() => {
                let cur = self.value.round() as i64;
                let idx = self.values.iter().position(|&v| v == cur).unwrap_or(0);
                let next = if reverse {
                    (idx + self.values.len() - 1) % self.values.len()
                } else {
                    (idx + 1) % self.values.len()
                };
                self.value = self.values[next] as f64;
            }
            OptionKind::Float { min, max, step } => {
                let delta = if step > 0.0 { step } else { (max - min) * 0.05 };
                let next = if reverse {
                    self.value - delta
                } else {
                    self.value + delta
                };
                self.value = next.clamp(min, max);
            }
            OptionKind::Choice { .. } if !self.choices.is_empty() => {
                let idx = self
                    .choices
                    .iter()
                    .position(|(raw, _)| *raw == self.current_choice)
                    .unwrap_or(0);
                let next = if reverse {
                    (idx + self.choices.len() - 1) % self.choices.len()
                } else {
                    (idx + 1) % self.choices.len()
                };
                self.current_choice = self.choices[next].0.clone();
            }
            _ => {}
        }
    }

    /// Serialized string value for `shaderpacks/<id>.txt`, or `None` if at default.
    pub fn saved_string(&self) -> Option<String> {
        match self.kind {
            OptionKind::Bool => {
                let b = self.value != 0.0;
                let def = self.default != 0.0;
                (b != def).then(|| b.to_string())
            }
            OptionKind::Int => {
                let i = self.value.round() as i64;
                let def = self.default.round() as i64;
                (i != def).then(|| i.to_string())
            }
            OptionKind::Float { .. } => {
                ((self.value - self.default).abs() > 1e-6).then(|| format!("{}", self.value))
            }
            OptionKind::Choice { .. } => {
                (self.current_choice != self.default_choice).then(|| self.current_choice.clone())
            }
        }
    }

    /// Reset to the pack's default value.
    pub fn reset(&mut self) {
        self.value = self.default;
        self.current_choice = self.default_choice.clone();
    }
}

/// A named preset profile in a Horizon or Java shader pack (`Low`, `High`, `Ultra`).
#[derive(Clone, Debug, PartialEq)]
pub struct PackProfile {
    pub name: String,
    pub label: String,
    pub values: BTreeMap<String, String>,
}

/// An item on a shader pack option screen (`shaders.properties` `screen`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenItem {
    Empty,
    Profile,
    Subscreen(String, String),
    Option(usize),
}

/// A laid-out option screen (main screen or sub-screen).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackScreen {
    pub name: String,
    pub label: String,
    pub columns: u32,
    pub items: Vec<ScreenItem>,
}

/// Interactive configuration metadata and current values for a shader pack,
/// usable directly by the Options UI without a GPU device.
#[derive(Clone, Debug, Default)]
pub struct PackConfig {
    pub id: String,
    pub options: Vec<PackOption>,
    pub profiles: Vec<PackProfile>,
    pub main_screen: PackScreen,
    pub subscreens: Vec<PackScreen>,
    /// Active subscreen name (`None` = main screen).
    pub active_screen: Option<String>,
}

impl PackConfig {
    /// Load the configurable options, profiles, and screens for pack `id` in `dir`.
    pub fn load(dir: &Path, id: &str) -> Result<Self, String> {
        if id.is_empty() {
            return Err("no shader pack selected".into());
        }
        let path = dir.join(id);
        let saved = load_values(dir, id);
        if path.join("pack.toml").is_file() {
            let manifest = manifest(&path)?;
            let mut options = Vec::new();
            for (key, entry) in manifest.options {
                options.push(option(key, entry, &saved)?);
            }
            let mut profiles = Vec::new();
            for (name, map) in manifest.profiles {
                let mut values = BTreeMap::new();
                for (k, v) in map {
                    let s = match v {
                        toml::Value::Boolean(b) => b.to_string(),
                        toml::Value::Integer(i) => i.to_string(),
                        toml::Value::Float(f) => f.to_string(),
                        toml::Value::String(s) => s,
                        _ => continue,
                    };
                    values.insert(k, s);
                }
                profiles.push(PackProfile {
                    label: humanize_identifier(&name),
                    name,
                    values,
                });
            }
            let mut items = Vec::new();
            if !profiles.is_empty() {
                items.push(ScreenItem::Profile);
            }
            items.extend((0..options.len()).map(ScreenItem::Option));
            return Ok(Self {
                id: id.to_owned(),
                options,
                profiles,
                main_screen: PackScreen {
                    name: String::new(),
                    label: manifest.pack.name,
                    columns: 2,
                    items,
                },
                subscreens: Vec::new(),
                active_screen: None,
            });
        }
        if crate::java_pack::is_java_pack(&path) {
            return Self::load_java(&path, id, &saved);
        }
        Err("not a valid shader pack".into())
    }

    fn load_java(path: &Path, id: &str, saved: &BTreeMap<String, String>) -> Result<Self, String> {
        let pack = JavaPack::open(path)?;
        let values = pack.values(saved);
        // The menus don't know the GPU: `shaders.properties` is read as on an
        // unknown one (the screens and profiles of the packs tested don't
        // depend on it).
        let env = Environment::iris("Unknown", "", 4);
        let props = pack.properties(&values, &env).unwrap_or_default();
        let lang = pack.translations("en_us");
        let sliders: BTreeSet<&str> = props.sliders().into_iter().collect();

        // Iris's names: the `.lang` entry, else the name itself; the tooltip
        // from `.lang`, else the option's comment.
        let named = |key: String, name: &str| lang.get(&key).cloned().unwrap_or(name.to_owned());
        let option_label = |name: &str| named(format!("option.{name}"), name);
        let option_desc = |name: &str, comment: Option<&str>| -> String {
            lang.get(&format!("option.{name}.comment"))
                .cloned()
                .or_else(|| comment.map(str::to_owned))
                .unwrap_or_default()
        };

        let mut options = Vec::new();
        let mut by_name = BTreeMap::new();
        for (name, b) in &pack.options.bools {
            let current = values.bools.get(name).copied().unwrap_or(b.default);
            let idx = options.len();
            options.push(PackOption {
                key: name.clone(),
                label: option_label(name),
                description: option_desc(name, b.comment.as_deref()),
                kind: OptionKind::Bool,
                values: Vec::new(),
                default: f64::from(u8::from(b.default)),
                value: f64::from(u8::from(current)),
                choices: Vec::new(),
                default_choice: b.default.to_string(),
                current_choice: current.to_string(),
            });
            by_name.insert(name.clone(), idx);
        }
        for (name, s) in &pack.options.strings {
            let current = values
                .strings
                .get(name)
                .cloned()
                .unwrap_or_else(|| s.default.clone());
            let choices: Vec<(String, String)> = s
                .allowed
                .iter()
                .map(|raw| (raw.clone(), named(format!("value.{name}.{raw}"), raw)))
                .collect();
            let idx = options.len();
            options.push(PackOption {
                key: name.clone(),
                label: option_label(name),
                description: option_desc(name, s.comment.as_deref()),
                kind: OptionKind::Choice {
                    slider: sliders.contains(name.as_str()),
                },
                values: Vec::new(),
                default: 0.0,
                value: 0.0,
                choices,
                default_choice: s.default.clone(),
                current_choice: current,
            });
            by_name.insert(name.clone(), idx);
        }

        let profiles: Vec<PackProfile> = props
            .profiles(&pack.options)
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                let label = named(format!("profile.{}", p.name), &p.name);
                PackProfile {
                    name: p.name,
                    label,
                    values: p.values,
                }
            })
            .collect();

        let referenced: BTreeSet<String> = std::iter::once(None)
            .chain(props.screens().into_iter().map(Some))
            .flat_map(|s| props.screen(s).unwrap_or_default())
            .filter_map(|e| match e {
                MenuElement::Option(o) => Some(o),
                _ => None,
            })
            .collect();

        let build_screen = |screen_name: Option<&str>| -> PackScreen {
            let elements = props
                .screen(screen_name)
                .unwrap_or_else(|| vec![MenuElement::Rest]);
            let mut items = Vec::new();
            if screen_name.is_none()
                && !profiles.is_empty()
                && !elements.contains(&MenuElement::Profile)
            {
                items.push(ScreenItem::Profile);
            }
            for el in elements {
                match el {
                    MenuElement::Empty => items.push(ScreenItem::Empty),
                    MenuElement::Profile => {
                        if !profiles.is_empty() {
                            items.push(ScreenItem::Profile);
                        }
                    }
                    MenuElement::Screen(sub) => {
                        let label = named(format!("screen.{sub}"), &sub);
                        items.push(ScreenItem::Subscreen(sub, label));
                    }
                    MenuElement::Option(opt) => {
                        if let Some(&idx) = by_name.get(&opt) {
                            items.push(ScreenItem::Option(idx));
                        }
                    }
                    MenuElement::Rest => {
                        for (idx, opt) in options.iter().enumerate() {
                            if !referenced.contains(&opt.key) {
                                items.push(ScreenItem::Option(idx));
                            }
                        }
                    }
                }
            }
            let name_str = screen_name.unwrap_or("").to_owned();
            let label = match screen_name {
                Some(n) => named(format!("screen.{n}"), n),
                None => id.to_owned(),
            };
            PackScreen {
                name: name_str,
                label,
                columns: props.columns(screen_name).unwrap_or(2),
                items,
            }
        };

        let main_screen = build_screen(None);
        let subscreens = props
            .screens()
            .into_iter()
            .map(|s| build_screen(Some(s)))
            .collect();

        Ok(Self {
            id: id.to_owned(),
            options,
            profiles,
            main_screen,
            subscreens,
            active_screen: None,
        })
    }

    /// Currently active screen (main or a sub-screen).
    pub fn current_screen(&self) -> &PackScreen {
        if let Some(name) = &self.active_screen
            && let Some(sub) = self.subscreens.iter().find(|s| s.name == *name)
        {
            return sub;
        }
        &self.main_screen
    }

    /// Name of the profile matching current option values, or `"Custom"`.
    pub fn active_profile_label(&self) -> String {
        let map = self.to_map();
        for profile in &self.profiles {
            let matches = profile.values.iter().all(|(k, v)| {
                if let Some(cur) = map.get(k) {
                    return cur == v;
                }
                if let Some(opt) = self.options.iter().find(|o| o.key == *k) {
                    return match opt.kind {
                        OptionKind::Bool => (opt.default != 0.0).to_string() == *v,
                        OptionKind::Int => (opt.default.round() as i64).to_string() == *v,
                        OptionKind::Float { .. } => format!("{}", opt.default) == *v,
                        OptionKind::Choice { .. } => opt.default_choice == *v,
                    };
                }
                true
            });
            if matches {
                return profile.label.clone();
            }
        }
        "Custom".into()
    }

    /// Cycle to the next (or previous) profile and apply its option values.
    pub fn cycle_profile(&mut self, reverse: bool) {
        if self.profiles.is_empty() {
            return;
        }
        let current = self.active_profile_label();
        let idx = self
            .profiles
            .iter()
            .position(|p| p.label == current)
            .map(|i| {
                if reverse {
                    (i + self.profiles.len() - 1) % self.profiles.len()
                } else {
                    (i + 1) % self.profiles.len()
                }
            })
            .unwrap_or(0);
        let values = self.profiles[idx].values.clone();
        for opt in &mut self.options {
            if let Some(v) = values.get(&opt.key) {
                match opt.kind {
                    OptionKind::Bool => {
                        if let Ok(b) = v.parse::<bool>() {
                            opt.value = f64::from(u8::from(b));
                        }
                    }
                    OptionKind::Int => {
                        if let Ok(i) = v.parse::<i64>() {
                            opt.value = i as f64;
                        }
                    }
                    OptionKind::Float { min, max, .. } => {
                        if let Ok(f) = v.parse::<f64>() {
                            opt.value = f.clamp(min, max);
                        }
                    }
                    OptionKind::Choice { .. } => {
                        opt.current_choice = v.clone();
                    }
                }
            }
        }
    }

    /// Reset all options to their defaults.
    pub fn reset_all(&mut self) {
        for opt in &mut self.options {
            opt.reset();
        }
    }

    /// Export non-default option values as a `key=value` map.
    pub fn to_map(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for opt in &self.options {
            if let Some(val) = opt.saved_string() {
                out.insert(opt.key.clone(), val);
            }
        }
        out
    }

    /// Save the current non-default option values to `shaderpacks/<id>.txt`.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        save_values(dir, &self.id, &self.to_map())
    }
}

fn humanize_identifier(raw: &str) -> String {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in raw.chars() {
        if ch == '_' || ch == '-' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else if ch.is_uppercase()
            && !current.is_empty()
            && current.chars().last().is_some_and(char::is_lowercase)
        {
            words.push(std::mem::take(&mut current));
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .map(|w| {
            let lower = w.to_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct ShaderPack {
    /// The pack's folder name.
    pub id: String,
    pub info: Info,
    pub stages: Vec<Stage>,
    pub options: Vec<PackOption>,
    /// Optional custom noise/LUT texture bytes (RGBA8, width, height) from `[textures]`.
    pub custom_noise: Option<Rgba>,
    /// A Java (OptiFine/Iris) pack's translated passes, instead of `stages`.
    pub java: Option<crate::java_pack::JavaPasses>,
    dir: PathBuf,
    stamp: Stamp,
}

/// Every file's path, length and modification time: a changed stamp means
/// the pack or its `<id>.txt` option file was edited (hot reload).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stamp(Vec<(PathBuf, u64, Option<SystemTime>)>);
impl Stamp {
    pub fn of(dir: &Path) -> Self {
        let mut files = Vec::new();
        if let Ok(meta) = fs::metadata(dir)
            && meta.is_file()
        {
            files.push((dir.to_path_buf(), meta.len(), meta.modified().ok()));
        }
        // Also watch the sibling `<id>.txt` options file for live option reloads.
        if let Some(name) = dir.file_name()
            && let Some(parent) = dir.parent()
        {
            let txt = parent.join(format!("{}.txt", name.to_string_lossy()));
            if let Ok(meta) = fs::metadata(&txt)
                && meta.is_file()
            {
                files.push((txt, meta.len(), meta.modified().ok()));
            }
        }
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = fs::read_dir(&d) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    stack.push(path);
                } else {
                    files.push((path, meta.len(), meta.modified().ok()));
                }
            }
        }
        files.sort();
        Self(files)
    }
}

/// The packs in `dir` (`shaderpacks/`): folders holding a `pack.toml`, by
/// folder name, with their manifest or why it can't be read, and Java packs
/// (folders holding `shaders/`, zips).
pub fn list(dir: &Path) -> Vec<(String, Result<Info, String>)> {
    let mut packs: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if path.join("pack.toml").is_file() {
                Some((id, manifest(&path).map(|m| m.pack)))
            } else if crate::java_pack::is_java_pack(&path) {
                Some((id.clone(), Ok(java_info(id))))
            } else {
                None
            }
        })
        .collect();
    packs.sort_by(|a, b| a.0.cmp(&b.0));
    packs
}

/// `shaderpacks/` beside `options.txt` (the game's working folder: the game
/// home, not the launcher's version folder).
pub fn folder() -> PathBuf {
    std::env::current_dir().unwrap_or_default().join(DIR)
}

/// Import a `.zip` archive or shader pack folder from `source` into
/// `target_dir` (`shaderpacks/`), validating that it holds `pack.toml` or
/// `shaders/`. Returns the imported pack's folder/zip name (`id`).
pub fn import_pack(source: &Path, target_dir: &Path) -> Result<String, String> {
    let source = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    if !source.exists() {
        return Err(format!("{}: file or folder not found", source.display()));
    }
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| "invalid shader pack path".to_string())?;
    let is_horizon = source.is_dir() && source.join("pack.toml").is_file();
    let is_java = crate::java_pack::is_java_pack(&source);
    if !is_horizon && !is_java {
        return Err(format!(
            "{name}: not a shader pack (needs pack.toml or shaders/)"
        ));
    }
    if is_java && source.is_file() {
        // Verify the zip actually contains `shaders/` or `pack.toml` at its root.
        JavaPack::open(&source)?;
    } else if is_horizon {
        manifest(&source)?;
    }
    fs::create_dir_all(target_dir).map_err(|e| format!("{}: {e}", target_dir.display()))?;
    let dest = target_dir.join(&name);
    if dest.canonicalize().ok() == source.canonicalize().ok() {
        return Ok(name);
    }
    if dest.exists() {
        return Err(format!("{name} is already in {DIR}/"));
    }
    if source.is_file() {
        fs::copy(&source, &dest).map_err(|e| format!("failed to copy {name}: {e}"))?;
    } else {
        copy_dir_recursive(&source, &dest)?;
    }
    Ok(name)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    let entries = fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let target = dst.join(entry.file_name());
        // Links aren't followed: one pointing up the tree would never end.
        match entry.file_type() {
            Ok(t) if t.is_dir() => copy_dir_recursive(&path, &target)?,
            Ok(t) if t.is_file() => {
                fs::copy(&path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// `shaderpacks/` and its packs as the Shader Packs page lists them, read
/// again at most once a second while the page is open, so a pack dropped
/// into the folder shows up.
#[derive(Debug, Default)]
pub struct Folder {
    pub dir: PathBuf,
    pub packs: Vec<(String, Result<Info, String>)>,
    read: Option<std::time::Instant>,
}

impl Folder {
    pub fn poll(&mut self, dir: PathBuf) {
        let fresh = self
            .read
            .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1));
        if fresh && self.dir == dir {
            return;
        }
        self.packs = list(&dir);
        self.dir = dir;
        self.read = Some(std::time::Instant::now());
    }
}

/// A Java pack has no manifest: its name is its file name.
fn java_info(name: String) -> Info {
    Info {
        name,
        description: "Iris/OptiFine pack: its full-screen passes run.".into(),
        authors: Vec::new(),
        version: String::new(),
        license: String::new(),
        api: API,
    }
}

fn manifest(dir: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(dir.join("pack.toml")).map_err(|e| format!("pack.toml: {e}"))?;
    let manifest: Manifest = toml::from_str(&text).map_err(|e| format!("pack.toml: {e}"))?;
    if manifest.pack.api == 0 || manifest.pack.api > API {
        return Err(format!(
            "pack.toml: needs shader pack api {}, this Horizon has {API}",
            manifest.pack.api
        ));
    }
    Ok(manifest)
}

/// An RGBA8 image (`decode_png`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A PNG of any colour type as [`Rgba`].
pub(crate) fn decode_png(bytes: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width.max(1), info.height.max(1));
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => buf.into_iter().flat_map(|g| [g, g, g, 255]).collect(),
        _ => return Err("unsupported PNG color format".into()),
    };
    Ok(Rgba {
        width: w,
        height: h,
        pixels: rgba,
    })
}

impl ShaderPack {
    /// Load and validate the pack in `dir` with the option values in
    /// `values` (missing ones take their default).
    pub fn load(dir: &Path, values: &BTreeMap<String, String>) -> Result<Self, String> {
        let stamp = Stamp::of(dir);
        let manifest = manifest(dir)?;
        let pack_api = manifest.pack.api;
        if manifest.stage.is_empty() || manifest.stage.len() > MAX_STAGES {
            return Err(format!("pack.toml: 1 to {MAX_STAGES} stages"));
        }
        if manifest.options.len() > MAX_OPTIONS {
            return Err(format!("pack.toml: at most {MAX_OPTIONS} options"));
        }
        let options = manifest
            .options
            .into_iter()
            .map(|(key, entry)| option(key, entry, values))
            .collect::<Result<Vec<_>, _>>()?;
        let mut stages = Vec::new();
        for entry in manifest.stage {
            let kind = match entry.kind.as_str() {
                "composite" => StageKind::Composite,
                other => {
                    return Err(format!(
                        "pack.toml: stage kind `{other}` isn't supported (api {API} has `composite`)"
                    ));
                }
            };
            if pack_api == 1 && entry.format.is_some() {
                return Err(format!("pack.toml: {}: `format` needs api 2", entry.shader));
            }
            if let Some(format) = entry.format.as_deref()
                && !STAGE_FORMATS.contains(&format)
            {
                return Err(format!(
                    "pack.toml: stage format `{format}` isn't one of {}",
                    STAGE_FORMATS.join(", ")
                ));
            }
            let scale = entry.scale.unwrap_or(1.0);
            if !(0.25..=2.0).contains(&scale) {
                return Err(format!(
                    "pack.toml: {}: scale must be 0.25 to 2",
                    entry.shader
                ));
            }
            let path = dir.join(&entry.shader);
            if !path.starts_with(dir) || entry.shader.contains("..") {
                return Err(format!("pack.toml: {} is outside the pack", entry.shader));
            }
            let code = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", entry.shader))?;
            let entry_point = entry.entry.unwrap_or_else(|| "fs_main".into());
            let prelude = match pack_api {
                1 => PRELUDE.to_owned(),
                _ => format!("{PRELUDE}\n{PRELUDE_API2}"),
            };
            let source = format!("{prelude}\n{code}");
            let bindings = validate(
                &source,
                prelude.lines().count(),
                &entry.shader,
                &entry_point,
            )?;
            stages.push(Stage {
                kind,
                file: entry.shader,
                source,
                entry: entry_point,
                scale,
                format: entry.format,
                bindings,
            });
        }
        let mut custom_noise = None;
        for (name, rel) in &manifest.textures {
            if name != "noise" {
                return Err(format!(
                    "pack.toml: texture `{name}` isn't supported (api {API} has `noise`)"
                ));
            }
            if rel.contains("..") {
                return Err(format!("pack.toml: texture {rel} is outside the pack"));
            }
            let bytes = fs::read(dir.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
            custom_noise = Some(decode_png(&bytes).map_err(|e| format!("{rel}: {e}"))?);
        }
        Ok(Self {
            id: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            info: manifest.pack,
            stages,
            options,
            custom_noise,
            java: None,
            dir: dir.to_path_buf(),
            stamp,
        })
    }
    /// Load the pack at `path` (a folder or a zip): a Horizon pack, or a
    /// Java pack translated for the GPU `vendor` and `renderer` names.
    pub fn load_any(
        path: &Path,
        values: &BTreeMap<String, String>,
        vendor: &str,
        renderer: &str,
    ) -> Result<Self, String> {
        if path.join("pack.toml").is_file() || !crate::java_pack::is_java_pack(path) {
            return Self::load(path, values);
        }
        let stamp = Stamp::of(path);
        let java = crate::java_pack::load(path, values, vendor, renderer)?;
        let id = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Self {
            info: java_info(id.clone()),
            id,
            stages: Vec::new(),
            options: Vec::new(),
            custom_noise: None,
            java: Some(java),
            dir: path.to_path_buf(),
            stamp,
        })
    }
    /// Whether a stage reads one of the prelude's `bindings`.
    fn reads(&self, bindings: &[u32]) -> bool {
        self.stages
            .iter()
            .any(|s| bindings.iter().any(|b| s.bindings.contains(b)))
    }
    /// Whether a stage reads the shadow map (`shadow_depth`, `sample_shadow`).
    pub fn needs_shadow(&self) -> bool {
        self.reads(&[BINDING_SHADOW_DEPTH])
    }
    /// Whether a stage reads the G-buffer (`scene_normal`, `scene_material`).
    pub fn needs_gbuffer(&self) -> bool {
        self.reads(&[BINDING_SCENE_NORMAL, BINDING_SCENE_MATERIAL])
    }
    /// Whether the world's depth before translucent geometry is kept
    /// (`opaque_depth`, which the G-buffer is also built from).
    pub fn needs_opaque_depth(&self) -> bool {
        self.needs_gbuffer() || self.reads(&[BINDING_OPAQUE_DEPTH])
    }
    /// The shadow map's resolution and distance in blocks.
    pub fn shadow_config(&self) -> (u32, f32) {
        let res = self
            .options
            .iter()
            .find(|o| o.key == "shadow_resolution" || o.key == "shadowMapResolution")
            .map(|o| (o.value.round() as u32).clamp(256, 4096))
            .unwrap_or(2048);
        let dist = self
            .options
            .iter()
            .find(|o| o.key == "shadow_distance" || o.key == "shadowDistance")
            .map(|o| (o.value as f32).clamp(16.0, 512.0))
            .unwrap_or(128.0);
        (res, dist)
    }
    /// The pipeline-overridable constants every stage gets: the option values.
    pub fn constants(&self) -> Vec<(&str, f64)> {
        self.options
            .iter()
            .map(|o| (o.key.as_str(), o.value))
            .collect()
    }
    /// The pack's files changed since it loaded.
    pub fn changed(&self) -> bool {
        Stamp::of(&self.dir) != self.stamp
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn option(
    key: String,
    entry: OptionEntry,
    values: &BTreeMap<String, String>,
) -> Result<PackOption, String> {
    let bad = |what: &str| format!("pack.toml: option `{key}`: {what}");
    let (kind, default) = match entry.default {
        toml::Value::Boolean(b) => (OptionKind::Bool, f64::from(u8::from(b))),
        toml::Value::Integer(i) => (OptionKind::Int, i as f64),
        toml::Value::Float(f) => (
            OptionKind::Float {
                min: entry.min.ok_or_else(|| bad("a number needs min"))?,
                max: entry.max.ok_or_else(|| bad("a number needs max"))?,
                step: entry.step.unwrap_or(0.0),
            },
            f,
        ),
        _ => return Err(bad("default must be true/false, an integer or a number")),
    };
    let values_list = entry.values.unwrap_or_default();
    if kind == OptionKind::Int && !values_list.contains(&(default as i64)) {
        return Err(bad("an integer needs `values` holding its default"));
    }
    let value = values
        .get(&key)
        .and_then(|v| match kind {
            OptionKind::Bool => v.parse::<bool>().ok().map(|b| f64::from(u8::from(b))),
            OptionKind::Int => v
                .parse::<i64>()
                .ok()
                .filter(|i| values_list.contains(i))
                .map(|i| i as f64),
            OptionKind::Float { min, max, .. } => v
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(|f| f.clamp(min, max)),
            OptionKind::Choice { .. } => None,
        })
        .unwrap_or(default);
    Ok(PackOption {
        label: entry.label.unwrap_or_else(|| humanize_identifier(&key)),
        description: entry.description,
        key,
        kind,
        values: values_list,
        default,
        value,
        choices: Vec::new(),
        default_choice: String::new(),
        current_choice: String::new(),
    })
}

/// naga parses and validates the stage (prelude included) and checks its
/// fragment entry point; errors point into the pack's own file.
/// Parse and validate a stage: the prelude's bindings (group 0) its entry
/// point uses, or the error at the stage's own line.
fn validate(
    source: &str,
    prelude_lines: usize,
    file: &str,
    entry: &str,
) -> Result<BTreeSet<u32>, String> {
    let prelude_lines = prelude_lines as u32 + 1;
    let at = |location: Option<naga::SourceLocation>| match location {
        Some(l) if l.line_number > prelude_lines => {
            format!(
                "{file}:{}:{}",
                l.line_number - prelude_lines,
                l.line_position
            )
        }
        _ => file.to_string(),
    };
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| format!("{}: {}", at(e.location(source)), e.message()))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| format!("{}: {}", at(e.location(source)), e.as_inner()))?;
    let Some(index) = module
        .entry_points
        .iter()
        .position(|e| e.name == entry && e.stage == naga::ShaderStage::Fragment)
    else {
        return Err(format!("{file}: no fragment entry point `{entry}`"));
    };
    let uses = info.get_entry_point(index);
    Ok(module
        .global_variables
        .iter()
        .filter(|(handle, _)| !uses[*handle].is_empty())
        .filter_map(|(_, var)| var.binding.as_ref())
        .filter(|binding| binding.group == 0)
        .map(|binding| binding.binding)
        .collect())
}

/// The option values saved for a pack (`shaderpacks/<id>.txt`, `key=value`).
pub fn load_values(dir: &Path, id: &str) -> BTreeMap<String, String> {
    fs::read_to_string(dir.join(format!("{id}.txt")))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

/// Save option values for a pack (`shaderpacks/<id>.txt`, `key=value`).
/// When `values` is empty, removes `<id>.txt` if present.
pub fn save_values(dir: &Path, id: &str, values: &BTreeMap<String, String>) -> Result<(), String> {
    if id.is_empty() {
        return Ok(());
    }
    let path = dir.join(format!("{id}.txt"));
    if values.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut text = String::new();
    for (k, v) in values {
        text.push_str(&format!("{k}={v}\n"));
    }
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("horizon-shader-pack-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn pack(files: &[(&str, &str)]) -> PathBuf {
        let dir = scratch();
        for (name, text) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }
    const MANIFEST: &str = r#"
[pack]
name = "Test"
api = 1

[[stage]]
kind = "composite"
shader = "shaders/grade.wgsl"

[options.strength]
label = "Strength"
default = 0.5
min = 0.0
max = 1.0
step = 0.1

[options.invert]
default = false

[options.mode]
default = 1
values = [0, 1, 2]
"#;
    const GRADE: &str = r#"
override strength: f32 = 0.5;
override invert: bool = false;
override mode: i32 = 1;
@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(previous, linear_sampler, in.uv, 0.0).rgb;
    let d = linear_depth(depth_at(in.uv));
    c = mix(c, vec3(luminance(c)), strength) * f32(mode) + d * frame.fog_color.rgb * 0.0;
    if invert { c = 1.0 - c; }
    return vec4(c, 1.0);
}
"#;

    #[test]
    fn the_sample_pack_loads() {
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shaderpacks/horizon-sample");
        let pack = ShaderPack::load(&dir, &BTreeMap::new()).unwrap();
        assert_eq!(pack.id, "horizon-sample");
        assert!(!pack.stages.is_empty());
        assert!(!pack.changed());
    }

    #[test]
    fn options_take_saved_values_within_their_range() {
        let dir = pack(&[("pack.toml", MANIFEST), ("shaders/grade.wgsl", GRADE)]);
        let values = BTreeMap::from([
            ("strength".to_string(), "7".to_string()),
            ("invert".to_string(), "true".to_string()),
            ("mode".to_string(), "5".to_string()),
        ]);
        let pack = ShaderPack::load(&dir, &values).unwrap();
        assert_eq!(
            pack.constants(),
            [("invert", 1.0), ("mode", 1.0), ("strength", 1.0)]
        );
        assert_eq!(pack.stages[0].entry, "fs_main");
        assert_eq!(pack.stages[0].kind, StageKind::Composite);
    }

    #[test]
    fn errors_point_into_the_pack_file() {
        let broken = "\n@fragment fn fs_main(in: StageIn) -> @location(0) vec4<f32> {\n    return vec4(nope, 1.0);\n}\n";
        let dir = pack(&[("pack.toml", MANIFEST), ("shaders/grade.wgsl", broken)]);
        let error = ShaderPack::load(&dir, &BTreeMap::new()).err().unwrap();
        assert!(error.starts_with("shaders/grade.wgsl:3:"), "{error}");
        let wrong_entry = GRADE.replace("fs_main", "other");
        let dir = pack(&[
            ("pack.toml", MANIFEST),
            ("shaders/grade.wgsl", &wrong_entry),
        ]);
        let error = ShaderPack::load(&dir, &BTreeMap::new()).err().unwrap();
        assert_eq!(
            error,
            "shaders/grade.wgsl: no fragment entry point `fs_main`"
        );
    }

    #[test]
    fn manifests_are_checked() {
        for (manifest, expected) in [
            (
                MANIFEST.replace("api = 1", "api = 3"),
                "needs shader pack api 3",
            ),
            (
                MANIFEST.replace("\"composite\"", "\"shadow\""),
                "stage kind `shadow`",
            ),
            (
                MANIFEST.replace(
                    "kind = \"composite\"",
                    "kind = \"composite\"\nformat = \"rgba16float\"",
                ),
                "`format` needs api 2",
            ),
            (
                MANIFEST.replace("api = 1", "api = 2").replace(
                    "kind = \"composite\"",
                    "kind = \"composite\"\nformat = \"rgba32float\"",
                ),
                "stage format `rgba32float`",
            ),
            (
                MANIFEST.replace("grade.wgsl", "../grade.wgsl"),
                "outside the pack",
            ),
            (MANIFEST.replace("min = 0.0\n", ""), "a number needs min"),
            (
                MANIFEST.replace("values = [0, 1, 2]", "values = [0]"),
                "holding its default",
            ),
            (
                MANIFEST.replace("name = \"Test\"", "name = \"Test\"\ncolour = 1"),
                "unknown field",
            ),
        ] {
            let dir = pack(&[("pack.toml", &manifest), ("shaders/grade.wgsl", GRADE)]);
            let error = ShaderPack::load(&dir, &BTreeMap::new()).err().unwrap();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn api_2_stages_and_pack_config_work() {
        let manifest_v2 = MANIFEST.replace("api = 1", "api = 2").replace(
            "kind = \"composite\"",
            "kind = \"composite\"\nformat = \"rgba16float\"",
        );
        let dir = pack(&[("pack.toml", &manifest_v2), ("shaders/grade.wgsl", GRADE)]);
        let loaded = ShaderPack::load(&dir, &BTreeMap::new()).unwrap();
        assert_eq!(loaded.stages[0].format.as_deref(), Some("rgba16float"));
        assert!(
            !loaded.needs_shadow() && !loaded.needs_gbuffer() && !loaded.needs_opaque_depth(),
            "reading none of them costs no pass"
        );
        let opaque = GRADE.replace("depth_at(in.uv)", "opaque_depth_at(in.uv)");
        let reads = pack(&[("pack.toml", &manifest_v2), ("shaders/grade.wgsl", &opaque)]);
        let reads = ShaderPack::load(&reads, &BTreeMap::new()).unwrap();
        assert!(reads.needs_opaque_depth() && !reads.needs_gbuffer());
        let api_1 = pack(&[("pack.toml", MANIFEST), ("shaders/grade.wgsl", &opaque)]);
        let error = ShaderPack::load(&api_1, &BTreeMap::new()).err().unwrap();
        assert!(
            error.contains("opaque_depth_at"),
            "api 1 stages don't see api 2's names: {error}"
        );
        let parent = dir.parent().unwrap();
        let id = dir.file_name().unwrap().to_str().unwrap();
        let mut cfg = PackConfig::load(parent, id).unwrap();
        assert_eq!(cfg.options.len(), 3);
        cfg.options[0].cycle(false);
        cfg.save(parent).unwrap();
        assert!(loaded.changed(), "saving <id>.txt updates the stamp");
    }

    #[test]
    fn edits_change_the_stamp_and_the_list_shows_every_pack() {
        let root = scratch();
        let dir = root.join("mine");
        fs::create_dir_all(dir.join("shaders")).unwrap();
        fs::write(dir.join("pack.toml"), MANIFEST).unwrap();
        fs::write(dir.join("shaders/grade.wgsl"), GRADE).unwrap();
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken/pack.toml"), "[pack]\n").unwrap();
        fs::create_dir_all(root.join("not-a-pack")).unwrap();
        let pack = ShaderPack::load(&dir, &BTreeMap::new()).unwrap();
        assert!(!pack.changed());
        fs::write(dir.join("shaders/grade.wgsl"), format!("{GRADE}\n")).unwrap();
        assert!(pack.changed());
        let listed = list(&root);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].0, "broken");
        assert!(listed[0].1.is_err());
        assert_eq!(listed[1].1.as_ref().unwrap().name, "Test");
    }

    #[test]
    fn saved_values_are_read_per_pack() {
        let dir = scratch();
        fs::write(dir.join("mine.txt"), "strength = 0.25\ninvert=true\n").unwrap();
        let values = load_values(&dir, "mine");
        assert_eq!(values["strength"], "0.25");
        assert_eq!(values["invert"], "true");
        assert!(load_values(&dir, "other").is_empty());
    }

    /// Dropping a pack on the Shader Packs page: zips and folders, Java
    /// and Horizon, and the things that aren't packs.
    #[test]
    fn imports_zips_and_folders_and_refuses_the_rest() {
        let shaderpacks = scratch();
        let java = pack(&[("shaders/composite.fsh", "void main() {}\n")]);
        let zip_path = scratch().join("Java Pack.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        zip.start_file(
            "Java Pack/shaders/final.fsh",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"void main() {}\n").unwrap();
        zip.finish().unwrap();

        assert_eq!(
            import_pack(&zip_path, &shaderpacks).unwrap(),
            "Java Pack.zip"
        );
        assert!(shaderpacks.join("Java Pack.zip").is_file());
        let folder = import_pack(&java, &shaderpacks).unwrap();
        assert!(
            shaderpacks
                .join(&folder)
                .join("shaders/composite.fsh")
                .is_file()
        );
        // One already there is refused; one dropped from shaderpacks/ itself
        // is just picked.
        let again = import_pack(&zip_path, &shaderpacks).unwrap_err();
        assert!(again.contains("already"), "{again}");
        assert_eq!(
            import_pack(&shaderpacks.join("Java Pack.zip"), &shaderpacks).unwrap(),
            "Java Pack.zip"
        );

        let not_a_pack = pack(&[("readme.txt", "hello")]);
        assert!(
            import_pack(&not_a_pack, &shaderpacks)
                .unwrap_err()
                .contains("not a shader pack")
        );
        let fake_zip = scratch().join("broken.zip");
        fs::write(&fake_zip, "not a zip").unwrap();
        assert!(import_pack(&fake_zip, &shaderpacks).is_err());
        let broken_horizon = pack(&[("pack.toml", "name = ")]);
        assert!(import_pack(&broken_horizon, &shaderpacks).is_err());
        assert!(import_pack(&shaderpacks.join("missing"), &shaderpacks).is_err());
        let left: Vec<_> = fs::read_dir(&shaderpacks)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 2, "nothing copied for refused packs: {left:?}");
    }
}
