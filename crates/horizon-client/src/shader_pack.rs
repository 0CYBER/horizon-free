//! Horizon shader packs (docs/shader-packs.md): folders in `shaderpacks/`
//! holding a `pack.toml` and WGSL stages. Loading reads the manifest and the
//! player's option values, puts the prelude before each stage's source and
//! has naga parse and validate it, so a broken pack is reported (file, line,
//! column) and never reaches the GPU.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

/// The stage and uniform contract this build provides.
pub const API: u32 = 1;
/// The folder beside `options.txt` that holds the packs.
pub const DIR: &str = "shaderpacks";
pub const PRELUDE: &str = include_str!("shader_pack_prelude.wgsl");
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
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionEntry {
    label: Option<String>,
    default: toml::Value,
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    values: Option<Vec<i64>>,
}

/// Where a stage runs. Api 1 has `composite` only.
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
}
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
}
#[derive(Clone, Debug, PartialEq)]
pub struct PackOption {
    /// The WGSL `override` it sets.
    pub key: String,
    pub label: String,
    pub kind: OptionKind,
    pub values: Vec<i64>,
    pub default: f64,
    pub value: f64,
}

pub struct ShaderPack {
    /// The pack's folder name.
    pub id: String,
    pub info: Info,
    pub stages: Vec<Stage>,
    pub options: Vec<PackOption>,
    /// A Java (OptiFine/Iris) pack's translated passes, instead of `stages`.
    pub java: Option<crate::java_pack::JavaPasses>,
    dir: PathBuf,
    stamp: Stamp,
}

/// Every file's path, length and modification time: a changed stamp means
/// the pack was edited (hot reload).
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
    if manifest.pack.api > API {
        return Err(format!(
            "pack.toml: needs shader pack api {}, this Horizon has {API}",
            manifest.pack.api
        ));
    }
    Ok(manifest)
}

impl ShaderPack {
    /// Load and validate the pack in `dir` with the option values in
    /// `values` (missing ones take their default).
    pub fn load(dir: &Path, values: &BTreeMap<String, String>) -> Result<Self, String> {
        let stamp = Stamp::of(dir);
        let manifest = manifest(dir)?;
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
            let source = format!("{PRELUDE}\n{code}");
            validate(&source, &entry.shader, &entry_point)?;
            stages.push(Stage {
                kind,
                file: entry.shader,
                source,
                entry: entry_point,
                scale,
            });
        }
        Ok(Self {
            id: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            info: manifest.pack,
            stages,
            options,
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
            java: Some(java),
            dir: path.to_path_buf(),
            stamp,
        })
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
        })
        .unwrap_or(default);
    Ok(PackOption {
        label: entry.label.unwrap_or_else(|| key.clone()),
        key,
        kind,
        values: values_list,
        default,
        value,
    })
}

/// naga parses and validates the stage (prelude included) and checks its
/// fragment entry point; errors point into the pack's own file.
fn validate(source: &str, file: &str, entry: &str) -> Result<(), String> {
    let prelude_lines = PRELUDE.lines().count() as u32 + 1;
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
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .map_err(|e| format!("{}: {}", at(e.location(source)), e.as_inner()))?;
    if !module
        .entry_points
        .iter()
        .any(|e| e.name == entry && e.stage == naga::ShaderStage::Fragment)
    {
        return Err(format!("{file}: no fragment entry point `{entry}`"));
    }
    Ok(())
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
                MANIFEST.replace("api = 1", "api = 2"),
                "needs shader pack api 2",
            ),
            (
                MANIFEST.replace("\"composite\"", "\"shadow\""),
                "stage kind `shadow`",
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
}
