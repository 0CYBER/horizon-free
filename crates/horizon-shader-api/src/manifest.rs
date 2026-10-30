//! `horizon-shaders.toml`, a shader pack's manifest (schema 1, docs/shaders.md).
use serde::Deserialize;

/// The manifest's file name at the root of a pack.
pub const MANIFEST: &str = "horizon-shaders.toml";
/// The manifest schema this build reads.
pub const SCHEMA: u32 = 1;
/// The shader API line this build serves: the stages, their signatures and
/// the inputs a pack's WGSL may read.
pub const API: &str = "0.1";
/// Full-screen passes a pack may chain.
pub const MAX_POST_PASSES: usize = 8;
/// Options a pack may declare.
pub const MAX_OPTIONS: usize = 64;
/// The most GPU time a pack may declare for itself, in milliseconds.
pub const MAX_GPU_MS: f32 = 16.0;

/// A checked manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// `[a-z0-9_]{1,64}`: the pack's settings key and registry id.
    pub id: String,
    pub name: String,
    pub version: String,
    /// The API line the pack is written for.
    pub api: String,
    pub description: String,
    pub authors: Vec<String>,
    pub license: Option<String>,
    /// WGSL files put before every stage's own file, in order.
    pub shared: Vec<String>,
    pub sky: Option<Stage>,
    pub material: Option<Stage>,
    /// Full-screen passes, in the order they run.
    pub post: Vec<Stage>,
    pub options: Vec<PackOption>,
    pub budget: Budget,
}

/// One stage: the WGSL file that defines its function.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub file: String,
    /// Shown in the GPU time overlay; the file name when absent.
    #[serde(default)]
    pub name: Option<String>,
}

/// An option the player can change: a WGSL `override` constant of the
/// same name, set when the pipelines are built.
#[derive(Clone, Debug, PartialEq)]
pub struct PackOption {
    /// The `override`'s name in the pack's WGSL.
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: OptionKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OptionKind {
    /// `override <id>: bool`.
    Bool { default: bool },
    /// `override <id>: f32`; the slider moves by `step`.
    Float {
        default: f32,
        min: f32,
        max: f32,
        step: f32,
    },
    /// `override <id>: i32`.
    Int { default: i32, min: i32, max: i32 },
    /// `override <id>: u32`, the index of the chosen value.
    Choice { values: Vec<String>, default: u32 },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Budget {
    /// The GPU time the pack aims to add to a frame, in milliseconds; the
    /// overlay shows the pack over budget past it.
    pub gpu_ms: Option<f32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    schema: u32,
    id: String,
    name: String,
    version: String,
    api: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    authors: Vec<String>,
    license: Option<String>,
    #[serde(default)]
    shared: Vec<String>,
    sky: Option<Stage>,
    material: Option<Stage>,
    #[serde(default)]
    post: Vec<Stage>,
    #[serde(default)]
    options: Vec<RawOption>,
    #[serde(default)]
    budget: RawBudget,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOption {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "type")]
    kind: String,
    default: toml::Value,
    min: Option<toml::Value>,
    max: Option<toml::Value>,
    step: Option<f64>,
    values: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    gpu_ms: Option<f64>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let raw: Raw = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        let err = |e: String| format!("{MANIFEST}: {e}");
        if raw.schema != SCHEMA {
            return Err(err(format!(
                "schema {}; this Horizon reads {SCHEMA}",
                raw.schema
            )));
        }
        if raw.api != API {
            return Err(err(format!(
                "written for shader API {}; this Horizon serves {API}",
                raw.api
            )));
        }
        let id_ok = (1..=64).contains(&raw.id.len())
            && raw
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !id_ok {
            return Err(err(format!("id {:?} is not [a-z0-9_]{{1,64}}", raw.id)));
        }
        if raw.name.trim().is_empty() || raw.version.trim().is_empty() {
            return Err(err("name and version can't be empty".into()));
        }
        if raw.sky.is_none() && raw.material.is_none() && raw.post.is_empty() {
            return Err(err(
                "no stage: give it a [sky], a [material] or a [[post]]".into()
            ));
        }
        if raw.post.len() > MAX_POST_PASSES {
            return Err(err(format!(
                "{} post passes; a pack may chain {MAX_POST_PASSES}",
                raw.post.len()
            )));
        }
        let stages = raw.sky.iter().chain(&raw.material).chain(&raw.post);
        for file in raw.shared.iter().chain(stages.map(|s| &s.file)) {
            if !crate::package::plain_path(file) || !file.ends_with(".wgsl") {
                return Err(err(format!("{file:?} is not a plain path to a .wgsl file")));
            }
        }
        if raw.options.len() > MAX_OPTIONS {
            return Err(err(format!(
                "{} options; a pack may declare {MAX_OPTIONS}",
                raw.options.len()
            )));
        }
        let mut options = Vec::with_capacity(raw.options.len());
        for (i, o) in raw.options.into_iter().enumerate() {
            let id = o.id.clone();
            let option = o.check().map_err(|e| err(format!("option {id:?}: {e}")))?;
            if options[..i].iter().any(|p: &PackOption| p.id == option.id) {
                return Err(err(format!("option {id:?} is declared twice")));
            }
            options.push(option);
        }
        let gpu_ms = match raw.budget.gpu_ms {
            Some(ms) if !(ms > 0.0 && ms <= f64::from(MAX_GPU_MS)) => {
                return Err(err(format!(
                    "budget.gpu_ms {ms} is not in (0, {MAX_GPU_MS}]"
                )));
            }
            ms => ms.map(|ms| ms as f32),
        };
        Ok(Manifest {
            id: raw.id,
            name: raw.name,
            version: raw.version,
            api: raw.api,
            description: raw.description,
            authors: raw.authors,
            license: raw.license,
            shared: raw.shared,
            sky: raw.sky,
            material: raw.material,
            post: raw.post,
            options,
            budget: Budget { gpu_ms },
        })
    }

    /// Every WGSL file the manifest names, shared files first, each once.
    pub fn files(&self) -> Vec<&str> {
        let mut files: Vec<&str> = Vec::new();
        let stages = self.sky.iter().chain(&self.material).chain(&self.post);
        for file in self.shared.iter().chain(stages.map(|s| &s.file)) {
            if !files.contains(&file.as_str()) {
                files.push(file);
            }
        }
        files
    }
}

/// A WGSL identifier the renderer leaves to packs: ASCII, not starting
/// with a digit, `_` or the renderer's own `horizon_` prefix.
pub fn pack_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && name.len() <= 64
        && !name.starts_with("horizon_")
}

impl RawOption {
    fn check(self) -> Result<PackOption, String> {
        if !pack_identifier(&self.id) {
            return Err(
                "the id must be a WGSL identifier: a letter, then letters, digits or _, up to 64, not starting with horizon_".into(),
            );
        }
        let int = |v: &toml::Value| {
            v.as_integer()
                .and_then(|i| i32::try_from(i).ok())
                .ok_or_else(|| format!("{v} is not a 32-bit integer"))
        };
        let float = |v: &toml::Value| {
            v.as_float()
                .or_else(|| v.as_integer().map(|i| i as f64))
                .filter(|f| f.is_finite())
                .map(|f| f as f32)
                .ok_or_else(|| format!("{v} is not a number"))
        };
        let need = |v: Option<toml::Value>, key: &str| v.ok_or_else(|| format!("needs {key}"));
        let unused = |present: bool, key: &str| {
            if present {
                Err(format!("{key} doesn't apply to type = {:?}", self.kind))
            } else {
                Ok(())
            }
        };
        let kind = match self.kind.as_str() {
            "bool" => {
                unused(self.min.is_some() || self.max.is_some(), "min/max")?;
                unused(self.step.is_some(), "step")?;
                unused(self.values.is_some(), "values")?;
                let default = self
                    .default
                    .as_bool()
                    .ok_or("default must be true or false")?;
                OptionKind::Bool { default }
            }
            "float" => {
                unused(self.values.is_some(), "values")?;
                let (min, max) = (
                    float(&need(self.min, "min")?)?,
                    float(&need(self.max, "max")?)?,
                );
                let default = float(&self.default)?;
                let step = self.step.map_or((max - min) / 100.0, |s| s as f32);
                if !(min < max && (min..=max).contains(&default)) {
                    return Err(format!("needs min < max and default in [{min}, {max}]"));
                }
                if !(step > 0.0 && step <= max - min) {
                    return Err(format!("step {step} is not in (0, max - min]"));
                }
                OptionKind::Float {
                    default,
                    min,
                    max,
                    step,
                }
            }
            "int" => {
                unused(self.step.is_some(), "step")?;
                unused(self.values.is_some(), "values")?;
                let (min, max) = (int(&need(self.min, "min")?)?, int(&need(self.max, "max")?)?);
                let default = int(&self.default)?;
                if !(min < max && (min..=max).contains(&default)) {
                    return Err(format!("needs min < max and default in [{min}, {max}]"));
                }
                OptionKind::Int { default, min, max }
            }
            "choice" => {
                unused(self.min.is_some() || self.max.is_some(), "min/max")?;
                unused(self.step.is_some(), "step")?;
                let values = self.values.ok_or("needs values")?;
                if !(2..=32).contains(&values.len()) {
                    return Err("needs 2 to 32 values".into());
                }
                if values
                    .iter()
                    .enumerate()
                    .any(|(i, v)| v.trim().is_empty() || values[..i].contains(v))
                {
                    return Err("values must be distinct and not empty".into());
                }
                let default = self
                    .default
                    .as_str()
                    .and_then(|d| values.iter().position(|v| v == d))
                    .ok_or("default must be one of the values")?;
                OptionKind::Choice {
                    values,
                    default: default as u32,
                }
            }
            other => {
                return Err(format!("type {other:?} is not bool, float, int or choice"));
            }
        };
        Ok(PackOption {
            id: self.id,
            name: self.name,
            description: self.description,
            kind,
        })
    }
}

impl OptionKind {
    /// The WGSL type of the option's `override`.
    pub fn wgsl_type(&self) -> &'static str {
        match self {
            OptionKind::Bool { .. } => "bool",
            OptionKind::Float { .. } => "f32",
            OptionKind::Int { .. } => "i32",
            OptionKind::Choice { .. } => "u32",
        }
    }

    /// The default as the pipeline constant wgpu takes.
    pub fn default_constant(&self) -> f64 {
        match *self {
            OptionKind::Bool { default } => f64::from(u8::from(default)),
            OptionKind::Float { default, .. } => f64::from(default),
            OptionKind::Int { default, .. } => f64::from(default),
            OptionKind::Choice { default, .. } => f64::from(default),
        }
    }
}
