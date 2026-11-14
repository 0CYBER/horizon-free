//! Resource packs (PackRepository / FolderRepositorySource): discovery of
//! folder and zip packs in `resourcepacks/` plus the built-in Programmer Art
//! and High Contrast packs, `pack.mcmeta` metadata and format compatibility,
//! and an overlay resource root that layers the enabled packs over the
//! vanilla resources (later packs win), so every loader sees one tree.
mod cache;
mod import;
use crate::resource_source::stays_inside;
use crate::text::Style;
use crate::text::Text;
pub use horizon_client_core::pattern;
pub use import::copy_pack;
use pattern::Pattern;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

/// SharedConstants resource pack format of 26.3 (major, minor).
pub const RESOURCE_FORMAT: (i64, i64) = horizon_core::version::RESOURCE_PACK_FORMAT;

#[derive(Clone, Debug)]
pub enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
    Opened(crate::resource_source::ResourceSource),
}
impl Source {
    /// Open this layer once for a load/reload, sharing it across resource reads.
    pub fn open(&self) -> io::Result<crate::resource_source::ResourceSource> {
        match self {
            Self::Dir(root) => Ok(crate::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            Self::Zip(path) => crate::resource_source::ResourceSource::archive(path),
            Self::Opened(source) => Ok(source.clone()),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Pack {
    /// `file/<name>` for user packs, the bare name for built-in ones.
    pub id: String,
    pub source: Source,
    pub title: Text,
    pub description: Text,
    /// The description with its legacy codes, and its root style (the pack screen's).
    pub raw_description: Text,
    pub description_style: Style,
    pub compatible: bool,
    pub compatibility: crate::pack_screen::Compat,
    /// Overlay directories active for this game version, in `pack.mcmeta` order: also those a
    /// Polytone `polytone_condition` turns on.
    pub overlays: Vec<String>,
    /// The values of the pack's Polytone config entries.
    pub polytone_configs: horizon_polytone::pack::Configs,
    /// `filter.block` entries (namespace and path patterns) hiding resources
    /// of the packs below this one.
    pub filter: Vec<Filter>,
    /// Presentation and selection supplied by an additional repository source.
    pub selection: Option<crate::pack_screen::PackInfo>,
}
#[derive(Clone, Debug, Default)]
pub struct Filter {
    namespace: Option<Pattern>,
    path: Option<Pattern>,
}
impl Filter {
    fn blocks(&self, file: &str) -> bool {
        let Some(rest) = file.strip_prefix("assets/") else {
            return false;
        };
        let Some((namespace, path)) = rest.split_once('/') else {
            return false;
        };
        self.namespace
            .as_ref()
            .is_none_or(|p| p.is_match(namespace))
            && self.path.as_ref().is_none_or(|p| p.is_match(path))
    }
}
/// A text component from JSON (strings, `text`, `translate`, arrays, `extra`
/// and the style fields), legacy `§` codes resolved as the font does.
pub fn component(v: &serde_json::Value) -> Text {
    component_root(v).0.resolve_legacy()
}
/// The component, its legacy codes left in place (each span is a
/// component's contents and style), and its root style (`Component.getStyle`).
pub fn component_root(v: &serde_json::Value) -> (Text, Style) {
    let mut out = Text::empty();
    let root = build(v, &Style::default(), &mut out);
    (out, root)
}
fn style_of(o: &serde_json::Map<String, serde_json::Value>) -> Style {
    let flag = |k: &str| o.get(k).and_then(|b| b.as_bool());
    Style {
        color: o.get("color").and_then(|c| c.as_str()).and_then(|c| {
            crate::text::named_color(c).or_else(|| {
                c.strip_prefix('#')
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
            })
        }),
        bold: flag("bold"),
        italic: flag("italic"),
        underlined: flag("underlined"),
        strikethrough: flag("strikethrough"),
        obfuscated: flag("obfuscated"),
        font: o.get("font").and_then(crate::text::font_id),
        ..Style::default()
    }
}
/// Appends `v` under `parent`'s style; returns the component's own style.
fn build(v: &serde_json::Value, parent: &Style, out: &mut Text) -> Style {
    match v {
        serde_json::Value::Array(a) => {
            let Some(first) = a.first() else {
                return Style::default();
            };
            let own = build(first, parent, out);
            let mut style = own.clone();
            style.inherit(parent);
            for e in &a[1..] {
                build(e, &style, out);
            }
            own
        }
        serde_json::Value::Object(o) => {
            let own = style_of(o);
            let mut style = own.clone();
            style.inherit(parent);
            if let Some(s) = o.get("text").and_then(|s| s.as_str()) {
                out.push(Text::styled(s, style.clone()));
            } else if let Some(k) = o.get("translate").and_then(|s| s.as_str()) {
                let args: Vec<Text> = o
                    .get("with")
                    .and_then(|w| w.as_array())
                    .map(|a| a.iter().map(component).collect())
                    .unwrap_or_default();
                out.push(Text::translate(k, &args).with_style(&style));
            }
            match o.get("extra") {
                Some(serde_json::Value::Array(extra)) => {
                    for e in extra {
                        build(e, &style, out);
                    }
                }
                Some(e) => {
                    build(e, &style, out);
                }
                None => {}
            }
            own
        }
        serde_json::Value::String(s) => {
            out.push(Text::styled(s.as_str(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Number(n) => {
            out.push(Text::styled(n.to_string(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Bool(b) => {
            out.push(Text::styled(b.to_string(), parent.clone()));
            Style::default()
        }
        serde_json::Value::Null => Style::default(),
    }
}
fn format_of(v: &serde_json::Value) -> Option<(i64, i64)> {
    match v {
        serde_json::Value::Number(n) => Some((n.as_i64()?, 0)),
        serde_json::Value::Array(a) => Some((
            a.first()?.as_i64()?,
            a.get(1).and_then(|m| m.as_i64()).unwrap_or(0),
        )),
        _ => None,
    }
}
/// `PackFormat`: major and minor.
type Format = (i64, i64);
/// `PackFormat.lastPreMinorVersion`: the last major without minor versions,
/// for the resource pack format (64) or the data pack one (81).
fn last_pre_minor(current: Format) -> i64 {
    if current.0 == horizon_core::version::DATA_PACK_FORMAT.0 {
        81
    } else {
        64
    }
}
/// `PackFormat.fullCodec(default_minor)`: an int, or a list of 1 to 256
/// non-negative ints whose second is the minor. None: it does not parse.
fn format_codec(v: &serde_json::Value, default_minor: i64) -> Option<Format> {
    // `Codec.INT` reads any JSON number through `Number.intValue`: a decimal such as `97.1`
    // counts as its whole part (97), as it does for the game.
    let non_negative = |v: &serde_json::Value| {
        v.as_i64()
            .or_else(|| {
                v.as_f64()
                    .filter(|f| f.is_finite())
                    .map(|f| f.trunc() as i64)
            })
            .filter(|n| *n >= 0 && *n <= i64::from(i32::MAX))
    };
    match v {
        serde_json::Value::Array(a) if (1..=256).contains(&a.len()) => {
            let list: Option<Vec<i64>> = a.iter().map(non_negative).collect();
            let list = list?;
            Some((list[0], list.get(1).copied().unwrap_or(default_minor)))
        }
        serde_json::Value::Array(_) => None,
        v => Some((non_negative(v)?, default_minor)),
    }
}
/// `InclusiveRange.codec(Codec.INT)`: `[min, max]` or
/// `{min_inclusive, max_inclusive}`, min not above max.
fn int_range_codec(v: &serde_json::Value) -> Option<(i64, i64)> {
    let int = |v: &serde_json::Value| v.as_i64().filter(|n| i32::try_from(*n).is_ok());
    let (min, max) = match v {
        serde_json::Value::Array(a) if a.len() == 2 => (int(&a[0])?, int(&a[1])?),
        serde_json::Value::Object(o) => {
            (int(o.get("min_inclusive")?)?, int(o.get("max_inclusive")?)?)
        }
        _ => return None,
    };
    (min <= max).then_some((min, max))
}
/// An `optionalFieldOf`: absent is `Ok(None)`, one that does not parse an error.
fn optional<T>(
    v: &serde_json::Value,
    parse: impl Fn(&serde_json::Value) -> Option<T>,
) -> Result<Option<T>, ()> {
    if v.is_null() {
        Ok(None)
    } else {
        parse(v).map(Some).ok_or(())
    }
}
/// `PackFormat.IntermediaryFormat.validatePackFormatForRange`.
fn validate_pack_format_for_range(main_format: i64, min: i64, max: i64) -> Option<String> {
    if main_format < min || main_format > max {
        Some(format!(
            "Pack declared support for versions {min} to {max} but declared main format is {main_format}"
        ))
    } else if main_format < 15 {
        Some("Multi-version packs cannot support minimum version of less than 15, since this will leave versions in range unable to load pack.".into())
    } else {
        None
    }
}
/// `PackFormat` as `toString` prints it.
fn format_text(f: Format) -> String {
    if f.1 == i64::from(i32::MAX) {
        format!("{}.*", f.0)
    } else {
        format!("{}.{}", f.0, f.1)
    }
}
/// `PackFormat.IntermediaryFormat.validate` for a pack's `pack` section
/// (`hasPackFormatField`, no `requireOldField`): the supported range, or why
/// vanilla refuses the metadata (it then falls back to a pack of unknown
/// format).
pub fn validate_pack_range(
    pack: &serde_json::Value,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    const CONTEXT: &str = "Pack";
    const OLD: &str = "supported_formats";
    let bad = |field: &str| format!("{CONTEXT} field {field} does not parse");
    let min =
        optional(&pack["min_format"], |v| format_codec(v, 0)).map_err(|()| bad("min_format"))?;
    let max = optional(&pack["max_format"], |v| {
        format_codec(v, i64::from(i32::MAX))
    })
    .map_err(|()| bad("max_format"))?;
    let format = optional(&pack["pack_format"], |v| {
        v.as_i64().filter(|n| i32::try_from(*n).is_ok())
    })
    .map_err(|()| bad("pack_format"))?;
    let supported = optional(&pack[OLD], int_range_codec).map_err(|()| bad(OLD))?;
    if min.is_some() != max.is_some() {
        return Err(format!(
            "{CONTEXT} missing field, must declare both min_format and max_format"
        ));
    }
    if let (Some(min), Some(max)) = (min, max) {
        return validate_new_format(min, max, format, supported, last_pre_minor);
    }
    if let Some(supported) = supported {
        return validate_old_format(supported, format, last_pre_minor);
    }
    match format {
        Some(main) if main > last_pre_minor => Err(format!(
            "{CONTEXT} declares support for version newer than {last_pre_minor}, but is missing mandatory fields min_format and max_format"
        )),
        Some(main) => Ok(((main, 0), (main, 0))),
        None => Err(format!(
            "{CONTEXT} could not be parsed, missing format version information"
        )),
    }
}
/// `PackFormat.IntermediaryFormat.validateNewFormat`.
fn validate_new_format(
    min: Format,
    max: Format,
    format: Option<i64>,
    supported: Option<(i64, i64)>,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    const OLD: &str = "supported_formats";
    let (major_min, major_max) = (min.0, max.0);
    if min > max {
        return Err(format!(
            "Pack min_format ({}) is greater than max_format ({})",
            format_text(min),
            format_text(max)
        ));
    }
    if major_min > last_pre_minor {
        if supported.is_some() {
            return Err(format!(
                "Pack key {OLD} is deprecated starting from pack format {}. Remove {OLD} from your pack.mcmeta.",
                last_pre_minor + 1
            ));
        }
        if let Some(main) = format
            && let Some(error) = validate_pack_format_for_range(main, major_min, major_max)
        {
            return Err(error);
        }
    } else {
        let Some(old) = supported else {
            return Err(format!(
                "Pack declares support for format {major_min}, but game versions supporting formats 15 to {last_pre_minor} require a {OLD} field. Add \"{OLD}\": [{major_min}, {last_pre_minor}] or require a version greater or equal to {}.0.",
                last_pre_minor + 1
            ));
        };
        if old.0 != major_min {
            return Err(format!(
                "Pack version declaration mismatch between {OLD} (from {}) and min_format ({})",
                old.0,
                format_text(min)
            ));
        }
        if old.1 != major_max && old.1 != last_pre_minor {
            return Err(format!(
                "Pack version declaration mismatch between {OLD} (up to {}) and max_format ({})",
                old.1,
                format_text(max)
            ));
        }
        let Some(main) = format else {
            return Err(format!(
                "Pack declares support for formats up to {last_pre_minor}, but game versions supporting formats 15 to {last_pre_minor} require a pack_format field. Add \"pack_format\": {major_min} or require a version greater or equal to {}.0.",
                last_pre_minor + 1
            ));
        };
        if let Some(error) = validate_pack_format_for_range(main, major_min, major_max) {
            return Err(error);
        }
    }
    Ok((min, max))
}
/// `PackFormat.IntermediaryFormat.validateOldFormat`.
fn validate_old_format(
    supported: (i64, i64),
    format: Option<i64>,
    last_pre_minor: i64,
) -> Result<(Format, Format), String> {
    let (min, max) = supported;
    if max > last_pre_minor {
        return Err(format!(
            "Pack declares support for version newer than {last_pre_minor}, but is missing mandatory fields min_format and max_format"
        ));
    }
    let Some(main) = format else {
        return Err(format!(
            "Pack declares support for formats up to {last_pre_minor}, but game versions supporting formats 15 to {last_pre_minor} require a pack_format field. Add \"pack_format\": {min} or require a version greater or equal to {}.0.",
            last_pre_minor + 1
        ));
    };
    if let Some(error) = validate_pack_format_for_range(main, min, max) {
        return Err(error);
    }
    Ok(((min, 0), (max, 0)))
}
/// PackCompatibility: inside the pack's declared range.
pub fn compatible(meta: &serde_json::Value) -> bool {
    compatibility(meta) == crate::pack_screen::Compat::Compatible
}
/// `PackCompatibility.forVersion`: too old, too new, compatible, or unknown
/// when the metadata names no (valid) format.
pub fn compatibility(meta: &serde_json::Value) -> crate::pack_screen::Compat {
    compatibility_for(meta, RESOURCE_FORMAT)
}
/// `compatibility` against `current`, the resource or the data pack format.
/// Metadata that fails `PackFormat.packCodec` parses as the fallback type,
/// whose format is `Integer.MAX_VALUE`: unknown.
pub fn compatibility_for(meta: &serde_json::Value, current: Format) -> crate::pack_screen::Compat {
    use crate::pack_screen::Compat;
    match validate_pack_range(&meta["pack"], last_pre_minor(current)) {
        Err(_) => Compat::Unknown,
        Ok((_, max)) if max < current => Compat::TooOld,
        Ok((min, _)) if current < min => Compat::TooNew,
        Ok(_) => Compat::Compatible,
    }
}
/// Whether an `overlays.entries[]` item applies to the current format
/// (OverlayMetadataSection: `formats` as an int, `[min, max]` or
/// `{min_inclusive, max_inclusive}`, or `min_format`/`max_format`).
fn overlay_applies(entry: &serde_json::Value) -> bool {
    let current = RESOURCE_FORMAT;
    if let (Some(min), Some(max)) = (
        format_of(&entry["min_format"]),
        format_of(&entry["max_format"]),
    ) {
        return min <= current && current <= (max.0, if max.1 == 0 { i64::MAX } else { max.1 });
    }
    let formats = &entry["formats"];
    if let Some(n) = formats.as_i64() {
        return n == current.0;
    }
    if let Some(a) = formats.as_array() {
        return match (
            a.first().and_then(|v| v.as_i64()),
            a.get(1).and_then(|v| v.as_i64()),
        ) {
            (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
            _ => false,
        };
    }
    match (
        formats["min_inclusive"].as_i64(),
        formats["max_inclusive"].as_i64(),
    ) {
        (Some(lo), Some(hi)) => lo <= current.0 && current.0 <= hi,
        _ => false,
    }
}
fn overlays_of(meta: &serde_json::Value, configs: &horizon_polytone::pack::Configs) -> Vec<String> {
    meta["overlays"]["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| horizon_polytone::pack::overlay_active(e, configs, overlay_applies(e)))
        .filter_map(|e| e["directory"].as_str().map(str::to_owned))
        // OverlayMetadataSection's DIR_VALIDATOR: one folder of the pack, `[-_a-zA-Z0-9.]+`.
        .filter(|d| {
            stays_inside(d)
                && !d.contains('/')
                && d.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
        .collect()
}
fn filter_of(meta: &serde_json::Value) -> Vec<Filter> {
    meta["filter"]["block"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| Filter {
            namespace: b["namespace"].as_str().and_then(Pattern::new),
            path: b["path"].as_str().and_then(Pattern::new),
        })
        .collect()
}
impl Pack {
    /// A mod's asset layer, which has no resource-pack metadata.
    pub fn mod_layer(id: String, source: Source) -> Self {
        let polytone_configs = polytone_configs(&source);
        Self {
            title: Text::literal(&id),
            id,
            source,
            description: Text::empty(),
            raw_description: Text::empty(),
            description_style: Style::default(),
            compatible: true,
            compatibility: crate::pack_screen::Compat::Compatible,
            overlays: Vec::new(),
            polytone_configs,
            filter: Vec::new(),
            selection: None,
        }
    }

    /// The pack's `pack.png` (`PackResources.getRootResource`), for its icon.
    pub fn icon_png(&self) -> Option<Vec<u8>> {
        self.source.open().ok()?.read("pack.png").ok()
    }
}
fn read_meta(source: &Source) -> Option<serde_json::Value> {
    let bytes = source.open().ok()?.read("pack.mcmeta").ok()?;
    parse_meta(&bytes)
}
/// The first JSON value of a `pack.mcmeta`: what follows it is ignored, as some packs sign the
/// file with a comment after the closing brace. Gson reads past a UTF-8 byte order mark;
/// serde_json rejects it.
pub(crate) fn parse_meta(bytes: &[u8]) -> Option<serde_json::Value> {
    first_json(bytes).ok()
}
/// The first JSON value of a `.mcmeta` file, as `ResourceMetadata.fromJsonStream` reads it:
/// Gson's `TypeAdapter.read` stops after the value, so text after it (packs leave `//` notes
/// there) is never read, and a UTF-8 byte order mark is skipped.
pub(crate) fn first_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match serde_json::Deserializer::from_slice(bytes)
        .into_iter()
        .next()
    {
        Some(value) => value,
        None => Err(serde::de::Error::custom("empty metadata")),
    }
}
/// A pack's Polytone config entries (`assets/<namespace>/polytone/config_entries/<name>.json`,
/// in the pack's base, not its overlays) at their default values.
pub fn polytone_configs(source: &Source) -> horizon_polytone::pack::Configs {
    let mut configs = horizon_polytone::pack::Configs::default();
    let id_of = |name: &str| -> Option<String> {
        let rest = name.strip_prefix("assets/")?;
        let (namespace, rest) = rest.split_once('/')?;
        let entry = rest
            .strip_prefix("polytone/config_entries/")?
            .strip_suffix(".json")?;
        Some(format!("{namespace}:{entry}"))
    };
    let Ok(source) = source.open() else {
        return configs;
    };
    for name in source.list("assets/") {
        if let Some(id) = id_of(&name)
            && let Ok(bytes) = source.read(&name)
            && let Ok(json) = serde_json::from_slice(&bytes)
        {
            configs.add_entry(id, &json);
        }
    }
    for (id, value) in polytone_choices() {
        if configs.entries().contains_key(&id) {
            configs.set(&id, value);
        }
    }
    configs
}

/// A pack's Polytone config entries and one's value, for the game's Pack Options.
pub use horizon_polytone::pack::{ConfigValue, Configs};

/// The player's Polytone config choices (#11486): what they changed from the packs' defaults,
/// by entry id, for every pack. `polytone_configs` lays them over each pack's defaults.
static POLYTONE_CHOICES: std::sync::RwLock<BTreeMap<String, horizon_polytone::pack::ConfigValue>> =
    std::sync::RwLock::new(BTreeMap::new());

pub fn polytone_choices() -> BTreeMap<String, horizon_polytone::pack::ConfigValue> {
    POLYTONE_CHOICES
        .read()
        .map(|c| c.clone())
        .unwrap_or_default()
}

/// Replaces the player's Polytone config choices (packs read them when they load again).
pub fn set_polytone_choices(choices: BTreeMap<String, horizon_polytone::pack::ConfigValue>) {
    if let Ok(mut c) = POLYTONE_CHOICES.write() {
        *c = choices;
    }
}

/// Where the choices are kept: `polytone-configs.json` beside `options.txt`.
pub fn polytone_choices_file() -> std::path::PathBuf {
    crate::logging::player_dir().join("polytone-configs.json")
}

/// Reads the choices from `path` (none when it is missing or broken).
pub fn load_polytone_choices(path: &Path) -> BTreeMap<String, horizon_polytone::pack::ConfigValue> {
    use horizon_polytone::pack::ConfigValue;
    let Ok(bytes) = fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_slice(&bytes) else {
        log::warn!("{}: not a JSON object", path.display());
        return BTreeMap::new();
    };
    map.into_iter()
        .filter_map(|(id, v)| {
            let value = match v {
                serde_json::Value::Number(n) => ConfigValue::Number(n.as_f64()?),
                serde_json::Value::String(s) => ConfigValue::Text(s),
                serde_json::Value::Bool(b) => ConfigValue::Bool(b),
                _ => return None,
            };
            Some((id, value))
        })
        .collect()
}

/// Writes the choices to `path`.
pub fn save_polytone_choices(
    path: &Path,
    choices: &BTreeMap<String, horizon_polytone::pack::ConfigValue>,
) -> std::io::Result<()> {
    use horizon_polytone::pack::ConfigValue;
    let map: serde_json::Map<String, serde_json::Value> = choices
        .iter()
        .map(|(id, v)| {
            let json = match v {
                ConfigValue::Number(n) => serde_json::json!(n),
                ConfigValue::Text(s) => serde_json::json!(s),
                ConfigValue::Bool(b) => serde_json::json!(b),
            };
            (id.clone(), json)
        })
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .map_err(std::io::Error::other)?;
    fs::write(path, text)
}
fn pack(id: String, title: Text, source: Source) -> Option<Pack> {
    pack_for(id, title, source, RESOURCE_FORMAT)
}
/// A pack whose compatibility is against `format`.
pub fn pack_for(id: String, title: Text, source: Source, format: (i64, i64)) -> Option<Pack> {
    let meta = read_meta(&source)?;
    let polytone_configs = polytone_configs(&source);
    let (raw_description, description_style) = component_root(&meta["pack"]["description"]);
    Some(Pack {
        id,
        title,
        description: raw_description.resolve_legacy(),
        raw_description,
        description_style,
        compatible: compatible(&meta),
        compatibility: compatibility_for(&meta, format),
        overlays: overlays_of(&meta, &polytone_configs),
        polytone_configs,
        filter: filter_of(&meta),
        source,
        selection: None,
    })
}
/// Built-in and `resourcepacks/` packs (vanilla itself excluded), sorted like
/// the pack screen (built-ins first, then by name).
pub fn discover(resources: &Path, packs_dir: &Path) -> Vec<Pack> {
    let mut out = Vec::new();
    for (id, key) in [
        ("programmer_art", "resourcePack.programmer_art.name"),
        ("high_contrast", "resourcePack.high_contrast.name"),
    ] {
        let z = resources.join(format!("assets/minecraft/resourcepacks/{id}.zip"));
        if z.exists()
            && let Some(p) = pack(id.into(), Text::translate(key, &[]), Source::Zip(z))
        {
            out.push(p);
        }
    }
    out.extend(folder(packs_dir, RESOURCE_FORMAT));
    out
}
/// The packs (folders and zips) in `dir`, by name, as `FolderRepositorySource`
/// finds them; their compatibility is against `format`.
pub fn folder(dir: &Path, format: (i64, i64)) -> Vec<Pack> {
    folder_with(dir, format, &DirectoryValidator::game())
}
/// [`folder`] under an explicit `allowed_symlinks.txt` validator.
pub(crate) fn folder_with(
    dir: &Path,
    format: (i64, i64),
    validator: &DirectoryValidator,
) -> Vec<Pack> {
    let mut user: Vec<Pack> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = match detected_pack_path(&e.path(), validator) {
                Ok(Some(path)) => path,
                Ok(None) => return None,
                Err(err) => {
                    log::warn!(
                        "Failed to read properties of '{}', ignoring: {err}",
                        e.path().display()
                    );
                    return None;
                }
            };
            let source = if path.is_dir() {
                Source::Dir(path)
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
            {
                Source::Zip(path)
            } else {
                return None;
            };
            pack_for(format!("file/{name}"), Text::literal(name), source, format)
        })
        .collect();
    user.sort_by(|a, b| a.id.cmp(&b.id));
    user
}

/// `LevelStorageSource.ALLOWED_SYMLINKS_CONFIG_NAME`: the game folder's list of
/// symbolic link targets the game accepts.
pub const ALLOWED_SYMLINKS_CONFIG_NAME: &str = "allowed_symlinks.txt";

/// `DirectoryValidator` over vanilla's `PathAllowList`: which symbolic link
/// targets the game accepts. With no entry, no link is, so a link planted in a
/// pack folder cannot point the game at the rest of the disk.
#[derive(Default)]
pub struct DirectoryValidator {
    /// `EntryType.PREFIX`: a target whose text starts with the entry matches.
    prefixes: Vec<String>,
}
impl DirectoryValidator {
    /// `LevelStorageSource.parseValidator`: the entries of `config`, none when
    /// it is missing, unreadable or unparseable. `[glob]` and `[regex]` entries
    /// are skipped with a warning: Horizon has no Java path matcher to compile
    /// them with, so a skipped entry allows less than vanilla's, never more,
    /// except that vanilla denies every link when an entry fails to compile.
    pub fn parse(config: &Path) -> Self {
        let Ok(text) = fs::read_to_string(config) else {
            return Self::default();
        };
        let mut prefixes = Vec::new();
        for line in text.lines() {
            // `ConfigEntry.parse`: blank lines and comments, untrimmed otherwise.
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(rest) = line.strip_prefix('[') else {
                prefixes.push(line.to_owned());
                continue;
            };
            let Some((kind, pattern)) = rest.split_once(']') else {
                // `ConfigEntry.parse` throws on an unterminated type, and
                // `parseValidator` then keeps no entry at all.
                log::error!(
                    "Horizon: {}: unterminated type in {line:?}",
                    config.display()
                );
                return Self::default();
            };
            match kind {
                "prefix" => prefixes.push(pattern.to_owned()),
                "glob" | "regex" => log::warn!(
                    "Horizon: {}: {kind} entry {pattern:?} is not supported and allows nothing",
                    config.display()
                ),
                _ => {
                    // An unsupported type throws in `parse`, as an unterminated
                    // one does: the whole file then allows no link.
                    log::error!(
                        "Horizon: {}: unsupported type in {line:?}",
                        config.display()
                    );
                    return Self::default();
                }
            }
        }
        Self { prefixes }
    }
    /// The game folder's list, as `Minecraft` reads it from
    /// `gameDirPath.resolve("allowed_symlinks.txt")`; the game folder is the
    /// working directory, as for `options.txt`.
    pub fn game() -> Self {
        Self::parse(
            &std::env::current_dir()
                .unwrap_or_default()
                .join(ALLOWED_SYMLINKS_CONFIG_NAME),
        )
    }
    /// `PathAllowList.matches`, on the target as written, not resolved.
    fn allows(&self, target: &Path) -> bool {
        target
            .to_str()
            .is_some_and(|t| self.prefixes.iter().any(|p| t.starts_with(p.as_str())))
    }
}
/// `DirectoryValidator.validateKnownDirectory`: every link under `dir`, never
/// following one, added to `issues` with its target unless the validator
/// allows the target.
fn forbidden_links(
    dir: &Path,
    validator: &DirectoryValidator,
    issues: &mut Vec<(PathBuf, PathBuf)>,
) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            let target = fs::read_link(&path)?;
            if !validator.allows(&target) {
                issues.push((path, target));
            }
        } else if kind.is_dir() {
            forbidden_links(&path, validator, issues)?;
        }
    }
    Ok(())
}
/// `FolderRepositorySource.discoverPacks`'s warning for a refused entry: the
/// entry, as vanilla names it, and every forbidden link in it.
fn refused(entry: &Path, issues: &[(PathBuf, PathBuf)]) {
    let list: Vec<String> = issues
        .iter()
        .map(|(link, target)| format!("{}->{}", link.display(), target.display()))
        .collect();
    log::warn!(
        "Ignoring potential pack entry: Failed to validate '{}'. Found forbidden symlinks: {}",
        entry.display(),
        list.join(", ")
    );
}
/// `PackDetector.detectPackResources`: the path a pack entry names, once the
/// validator accepts the entry and every link inside it; `None` when it is
/// missing or not a pack, `Err` when it cannot be read. A link is followed one
/// level, to its target as written rather than resolved, as vanilla does.
pub(crate) fn detected_pack_path(
    path: &Path,
    validator: &DirectoryValidator,
) -> io::Result<Option<PathBuf>> {
    let mut kind = match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None), // `NoSuchFileException`
        Err(err) => return Err(err),
    };
    let mut target = path.to_path_buf();
    if kind.is_symlink() {
        let link = fs::read_link(path)?;
        if !validator.allows(&link) {
            refused(path, &[(path.to_path_buf(), link)]);
            return Ok(None);
        }
        target = link;
        // One level only: the attributes read on the target do not follow a
        // second link, and such an entry is neither a directory nor a file.
        kind = fs::symlink_metadata(&target)?.file_type();
        if kind.is_symlink() {
            return Ok(None);
        }
    }
    if kind.is_dir() {
        let mut issues = Vec::new();
        forbidden_links(&target, validator, &mut issues)?;
        if !issues.is_empty() {
            refused(path, &issues);
            return Ok(None);
        }
    }
    Ok(Some(target))
}
/// A file a pack provides: its path (folder packs) and its name inside the pack.
type Provided = (
    Option<PathBuf>,
    crate::resource_source::ResourceSource,
    String,
);
/// Files a pack provides under `assets/`, keyed by their resource name; active
/// overlay directories are applied after the base, in order, later wins.
fn pack_files(
    pack: &Pack,
    source: &crate::resource_source::ResourceSource,
) -> io::Result<BTreeMap<String, Provided>> {
    let mut out = BTreeMap::new();
    for root in std::iter::once(String::new()).chain(
        pack.overlays
            .iter()
            .map(|directory| format!("{directory}/")),
    ) {
        let source = source.subdirectory(root.trim_end_matches('/'))?;
        for inner in source.list("assets/") {
            let path = match &pack.source {
                Source::Dir(directory) => Some(directory.join(&root).join(&inner)),
                Source::Zip(_) | Source::Opened(_) => None,
            };
            out.insert(inner.clone(), (path, source.clone(), inner));
        }
    }
    Ok(out)
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    // Relative sources are relative to the process, not the overlay directory.
    std::os::unix::fs::symlink(std::path::absolute(target)?, at)
}
#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    if target.is_dir() {
        Ok(())
    } else {
        // The mirror only reads these files, and unix links them: a hardlink gives
        // the same view without writing the 639 MB prepared root again, which was
        // 33 s of a cold Windows launch with a pack on. Another volume (or a
        // filesystem without hardlinks) still copies.
        fs::hard_link(target, at).or_else(|_| fs::copy(target, at).map(|_| ()))
    }
}
/// A directory junction (`horizon_threads::junction`); the mirror falls back to copying the
/// tree when it fails.
#[cfg(windows)]
fn junction(target: &Path, at: &Path) -> io::Result<()> {
    horizon_threads::junction(target, at)
}
#[cfg(not(windows))]
fn junction(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}
/// Build `out` as `vanilla` with the packs' files layered on top (packs in
/// load order, last wins). Untouched directories are links into vanilla.
pub fn build_overlay(vanilla: &Path, packs: &[Pack], out: &Path) -> io::Result<()> {
    cache::build(vanilla, packs, out)
}

struct ResolvedPacks {
    files: BTreeMap<String, (usize, Provided)>,
    stacks: BTreeMap<String, Vec<(usize, Provided)>>,
    hidden: BTreeSet<String>,
}

fn resolve(packs: &[Pack]) -> io::Result<ResolvedPacks> {
    let sources: Vec<_> = packs
        .iter()
        .map(|pack| pack.source.open())
        .collect::<io::Result<_>>()?;
    // Final owner of every overridden file.
    let mut files: BTreeMap<String, (usize, Provided)> = BTreeMap::new();
    // Every pack's copy of the files vanilla reads as a resource stack.
    let mut stacks: BTreeMap<String, Vec<(usize, Provided)>> = BTreeMap::new();
    for (i, (p, source)) in packs.iter().zip(&sources).enumerate() {
        // A pack's filter hides what the packs below it provide.
        files.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        stacks.retain(|name, _| !p.filter.iter().any(|f| f.blocks(name)));
        for (name, provided) in pack_files(p, source)? {
            if merge_kind(&name).is_some() {
                stacks
                    .entry(name.clone())
                    .or_default()
                    .push((i, provided.clone()));
            }
            files.insert(name, (i, provided));
        }
    }
    // FallbackResourceManager: a file's `.mcmeta` counts only from the pack that
    // supplies the file or one above it; vanilla's own sits below every pack.
    let mut hidden: BTreeSet<String> = files
        .iter()
        .filter(|(name, _)| !name.ends_with(".mcmeta"))
        .filter_map(|(name, (owner, _))| {
            let meta = format!("{name}.mcmeta");
            match files.get(&meta) {
                Some((meta_owner, _)) if meta_owner >= owner => None,
                _ => Some(meta),
            }
        })
        .collect();
    // A companion from below the winning colour would describe another surface.
    // A map-only pack above it is intentional and remains valid.
    for (name, (owner, _)) in &files {
        let Some(stem) = name.strip_suffix(".png") else {
            continue;
        };
        if stem.ends_with("_n") || stem.ends_with("_s") {
            continue;
        }
        for suffix in ["_n.png", "_s.png"] {
            let map = format!("{stem}{suffix}");
            if files
                .get(&map)
                .is_none_or(|(map_owner, _)| map_owner < owner)
            {
                hidden.insert(map);
            }
        }
    }
    for name in &hidden {
        files.remove(name);
    }
    Ok(ResolvedPacks {
        files,
        stacks,
        hidden,
    })
}

/// Resolve a pack selection without creating an overlay directory. Files keep
/// their open source; only vanilla's resource-stack merges are held as bytes.
/// Diverges from upstream: jars and zip packs remain in place during loading.
pub fn resources(
    base: crate::resource_source::ResourceSource,
    packs: &[Pack],
) -> io::Result<crate::resource_source::ResourceSource> {
    use crate::resource_source::{Resource, ResourceSource};
    if packs.is_empty() {
        return Ok(base);
    }
    let ResolvedPacks {
        mut files,
        stacks,
        mut hidden,
    } = resolve(packs)?;
    let filters: Vec<_> = packs.iter().flat_map(|pack| &pack.filter).collect();
    let merged = merged_resources(&base, packs, &stacks);
    add_sprite_renames(&base, &merged, &filters, &mut files, &mut hidden);
    let mut index: BTreeMap<_, _> = base
        .list("")
        .into_iter()
        .filter(|name| {
            !hidden.contains(name)
                && !filters.iter().any(|filter| filter.blocks(name))
                && name != horizon_core::prepared_files::FILE_NAME
        })
        .map(|name| (name.clone(), Resource::file(base.clone(), name)))
        .collect();
    for (name, (_, (_, source, inner))) in files {
        index.insert(name, Resource::file(source, inner));
    }
    for (name, bytes) in merged {
        index.insert(name, Resource::bytes(bytes));
    }
    Ok(ResourceSource::indexed(index))
}

fn merged_resources(
    base: &crate::resource_source::ResourceSource,
    packs: &[Pack],
    stacks: &BTreeMap<String, Vec<(usize, Provided)>>,
) -> BTreeMap<String, Vec<u8>> {
    let filters: Vec<_> = packs.iter().flat_map(|pack| &pack.filter).collect();
    let mut merged = BTreeMap::new();
    for (name, stack) in stacks {
        let mut layers = Vec::new();
        if !filters.iter().any(|filter| filter.blocks(name)) {
            layers.extend(base.read(name).ok());
        }
        for (owner, (_, source, inner)) in stack {
            let bytes = source.read(inner).ok();
            let bytes = match (bytes, merge_kind(name)) {
                (Some(bytes), Some(Merge::Sounds)) => Some(
                    packs
                        .get(*owner)
                        .and_then(|pack| {
                            horizon_polytone::pack::sounds_without_unmet(
                                &bytes,
                                &pack.polytone_configs,
                            )
                        })
                        .unwrap_or(bytes),
                ),
                (bytes, _) => bytes,
            };
            layers.extend(bytes);
        }
        if let Some(kind) = merge_kind(name)
            && let Some(bytes) = merge(kind, &layers)
        {
            merged.insert(name.clone(), bytes);
        }
    }
    merged
}

fn build_overlay_uncached(
    vanilla: &Path,
    packs: &[Pack],
    out: &Path,
    dependencies: &mut cache::Dependencies,
) -> io::Result<()> {
    if out.exists() {
        fs::remove_dir_all(out)?;
    }
    fs::create_dir_all(out)?;
    let ResolvedPacks {
        mut files,
        stacks,
        mut hidden,
    } = resolve(packs)?;
    // Vanilla sits below every pack, so filters hide its files too.
    let filters: Vec<&Filter> = packs.iter().flat_map(|p| &p.filter).collect();
    for name in stacks.keys() {
        dependencies.file(&vanilla.join(name));
    }
    let merged = merged_resources(
        &crate::resource_source::ResourceSource::directory(vanilla.to_owned()),
        packs,
        &stacks,
    );
    add_sprite_renames(
        &crate::resource_source::ResourceSource::directory(vanilla.to_owned()),
        &merged,
        &filters,
        &mut files,
        &mut hidden,
    );
    let touched_dirs: BTreeSet<String> = files
        .keys()
        .flat_map(|f| {
            let parts: Vec<&str> = f.split('/').collect();
            (1..parts.len()).map(move |n| parts[..n].join("/"))
        })
        .collect();
    /// A zip entry to write: its pack, its name in the zip and where it goes.
    type Extract<'a> = (&'a crate::resource_source::ResourceSource, &'a str, PathBuf);
    #[allow(clippy::too_many_arguments)]
    fn mirror<'a>(
        rel: &str,
        vanilla: &Path,
        out: &Path,
        touched: &BTreeSet<String>,
        hidden: &BTreeSet<String>,
        files: &'a BTreeMap<String, (usize, Provided)>,
        merged: &BTreeMap<String, Vec<u8>>,
        filters: &[&Filter],
        dependencies: &mut cache::Dependencies,
        extract: &mut Vec<Extract<'a>>,
    ) -> io::Result<()> {
        let src = if rel.is_empty() {
            vanilla.to_path_buf()
        } else {
            vanilla.join(rel)
        };
        let dst = if rel.is_empty() {
            out.to_path_buf()
        } else {
            out.join(rel)
        };
        fs::create_dir_all(&dst)?;
        dependencies.directory(&src);
        let mut names: BTreeSet<String> = BTreeSet::new();
        if let Ok(entries) = fs::read_dir(&src) {
            names.extend(
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned()),
            );
        }
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        // The files under this directory are one run of the sorted map: walking only that run
        // keeps a pack of tens of thousands of files in thousands of folders from being
        // scanned once per folder.
        for f in files
            .range(prefix.clone()..)
            .map_while(|(f, _)| f.strip_prefix(&prefix))
        {
            names.insert(f.split('/').next().unwrap_or("").to_owned());
        }
        for name in names {
            let child = format!("{prefix}{name}");
            // Vanilla's files in one file: it describes vanilla's root, not this mirror.
            if hidden.contains(&child) || child == horizon_core::prepared_files::FILE_NAME {
                continue;
            }
            if let Some(bytes) = merged.get(&child) {
                fs::write(out.join(&child), bytes)?;
            } else if let Some((_, (path, source, inner))) = files.get(&child) {
                let target = out.join(&child);
                match path {
                    Some(p) => link(p, &target)?,
                    // Inflated and written after the walk, on every thread.
                    None => extract.push((source, inner.as_str(), target)),
                }
            } else if filters.is_empty()
                && !touched.contains(&child)
                && vanilla.join(&child).is_dir()
                && junction(&vanilla.join(&child), &out.join(&child)).is_ok()
            {
                // Linked whole, like the unix symlink: no file in it is visited or copied.
            } else if touched.contains(&child)
                // Without a junction Windows cannot link a directory the way unix does (`link`
                // above), so an untouched one has to be mirrored: skipping it hid textures/font
                // and the title panorama from the overlay, and a font json that names those
                // sheets then failed to load (#1688).
                || (vanilla.join(&child).is_dir() && (cfg!(not(unix)) || !filters.is_empty()))
            {
                mirror(
                    &child,
                    vanilla,
                    out,
                    touched,
                    hidden,
                    files,
                    merged,
                    filters,
                    dependencies,
                    extract,
                )?;
            } else if filters.iter().any(|f| f.blocks(&child)) {
                // Hidden by a pack's filter.
            } else {
                #[cfg(not(unix))]
                if vanilla.join(&child).is_file() {
                    dependencies.file(&vanilla.join(&child));
                }
                link(&vanilla.join(&child), &out.join(&child))?;
            }
        }
        Ok(())
    }
    let mut extract = Vec::new();
    mirror(
        "",
        vanilla,
        out,
        &touched_dirs,
        &hidden,
        &files,
        &merged,
        &filters,
        dependencies,
        &mut extract,
    )?;
    // What the zips may unpack to, all threads together: a server's pack is a download the
    // player only accepts, and a gigabyte of zeros deflates to a megabyte, so without it a pack
    // at the download limit wrote hundreds of gigabytes here. Vanilla never unpacks a pack.
    let budget = std::sync::atomic::AtomicU64::new(unpack_budget(packs));
    // Inflating and writing a big pack's files one by one was most of a cold build; its
    // folders all exist by now.
    let extracted: io::Result<()> = crate::par::par_map(&extract, |(source, inner, target)| {
        let copied = fs::File::create(target).and_then(|file| {
            let mut file = io::BufWriter::new(file);
            source.copy_to(inner, &mut file, &budget)?;
            io::Write::flush(&mut file)
        });
        match copied {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::FileTooLarge => Err(e),
            Err(e) => {
                log::warn!("Horizon: skipping unreadable pack entry {inner}: {e}");
                let _ = fs::remove_file(target);
                Ok(())
            }
        }
    })
    .into_iter()
    .collect();
    if extracted.is_err() {
        // What was unpacked before the budget ran out goes too, not only the marker.
        let _ = fs::remove_dir_all(out);
    }
    extracted
}
/// `SingleFile` with a `sprite`: the atlas holds the resource's pixels, and the resource's
/// `.mcmeta`, under the sprite's id. Every other sprite of the blocks and items atlases is its
/// texture's own id, and the loaders read a model's sprite from `textures/<sprite>.png`, so the
/// overlay gives a renamed sprite that file. ItemsAdder and Nexo packs name their models'
/// textures only through these renames (#14485).
///
/// A sprite gets its resource's file only when the whole source list, in order, ends on the
/// rename (`SpriteSourceList.list`: a later source of the same id replaces it), when the other
/// atlas, which reads the same file, lists it from that resource too or not at all, and when it
/// has no texture of its own. A texture that is there may be read as itself (another source's
/// resource, an unstitched or paletted sprite's input), so no alias ever replaces one; such a
/// sprite draws its own texture, as before aliases.
fn add_sprite_renames(
    vanilla: &crate::resource_source::ResourceSource,
    merged: &BTreeMap<String, Vec<u8>>,
    filters: &[&Filter],
    files: &mut BTreeMap<String, (usize, Provided)>,
    hidden: &mut BTreeSet<String>,
) {
    use crate::atlas_sources::{
        SpriteOrigin, SpriteSource, decode, list, list_textures, texture_path,
    };
    let png = |id: &str| {
        texture_path(Path::new(""), id, "textures")
            .to_string_lossy()
            .replace('\\', "/")
    };
    let atlases: Vec<Vec<SpriteSource>> = ["blocks", "items"]
        .into_iter()
        .filter_map(|atlas| merged.get(&format!("assets/minecraft/atlases/{atlas}.json")))
        .filter_map(|bytes| decode(std::str::from_utf8(bytes).ok()?).ok())
        .collect();
    let renamed = |source: &SpriteSource| matches!(source, SpriteSource::Single { resource, sprite: Some(sprite) } if sprite != resource);
    if !atlases.iter().flatten().any(renamed) {
        return;
    }
    // The textures a source can list: the packs' and vanilla's the packs leave visible.
    let mut textures: Vec<String> = list_textures(vanilla)
        .into_iter()
        .filter(|id| {
            let name = png(id);
            !hidden.contains(&name) && !filters.iter().any(|f| f.blocks(&name))
        })
        .chain(files.keys().filter_map(|name| {
            let (namespace, rest) = name.strip_prefix("assets/")?.split_once('/')?;
            let path = rest.strip_prefix("textures/")?.strip_suffix(".png")?;
            Some(format!("{namespace}:{path}"))
        }))
        .collect();
    textures.sort();
    textures.dedup();
    // Each sprite's resource, `None` where an atlas lists it any other way.
    let mut renames: BTreeMap<String, Option<String>> = BTreeMap::new();
    for sources in &atlases {
        for (sprite, origin) in list(sources, &textures, &mut crate::atlas_sources::unasked) {
            let from = match origin {
                SpriteOrigin::File(resource) if resource != sprite => Some(resource),
                _ => None,
            };
            renames
                .entry(sprite)
                .and_modify(|seen| {
                    if *seen != from {
                        *seen = None;
                    }
                })
                .or_insert(from);
        }
    }
    let renames = renames
        .into_iter()
        .filter(|(sprite, _)| textures.binary_search(sprite).is_err())
        .filter_map(|(sprite, resource)| Some((png(&resource?), png(&sprite))));
    // Each resource is the file the packs give, never another rename's sprite.
    let mut found = Vec::new();
    for (from, to) in renames {
        // `texture_path` names an id that would leave the root as a file that is never there.
        if !stays_inside(&from) || !stays_inside(&to) {
            continue;
        }
        let from_meta = format!("{from}.mcmeta");
        // A pack's texture with its own `.mcmeta`, or vanilla's below every pack.
        let (file, meta) = match files.get(&from) {
            Some(file) => (file.clone(), files.get(&from_meta).cloned()),
            None => {
                if !vanilla.contains(&from) || filters.iter().any(|f| f.blocks(&from)) {
                    continue;
                }
                let meta = files.get(&from_meta).cloned().or_else(|| {
                    (vanilla.contains(&from_meta) && !filters.iter().any(|f| f.blocks(&from_meta)))
                        .then(|| (0, (None, vanilla.clone(), from_meta.clone())))
                });
                ((0, (None, vanilla.clone(), from.clone())), meta)
            }
        };
        found.push((to, file, meta));
    }
    for (to, file, meta) in found {
        let to_meta = format!("{to}.mcmeta");
        files.insert(to, file);
        match meta {
            Some(meta) => {
                hidden.remove(&to_meta);
                files.insert(to_meta, meta);
            }
            None => {
                files.remove(&to_meta);
                hidden.insert(to_meta);
            }
        }
    }
}
/// The most the zip packs in `packs` may unpack to in one overlay: 16 times their size on disk
/// (real packs are mostly PNGs and JSON, a few times smaller zipped at most), at least 64 MiB
/// for a small pack of text, and never past 8 GiB.
fn unpack_budget(packs: &[Pack]) -> u64 {
    packs
        .iter()
        .filter_map(|p| match &p.source {
            Source::Zip(z) => fs::metadata(z).ok().map(|m| m.len()),
            // Open resource layers need not have a filesystem archive. They
            // share the absolute ceiling, not a guessed compressed size.
            Source::Opened(_) => Some(8 << 30),
            Source::Dir(_) => None,
        })
        .sum::<u64>()
        .saturating_mul(16)
        .clamp(64 << 20, 8 << 30)
}
/// Files vanilla loads from every pack in turn (`getResourceStack`) rather
/// than from the top pack only.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Merge {
    /// `ClientLanguage.appendFrom`: later packs override keys.
    Lang,
    /// `SoundManager.Preparations.handleRegistration`: sounds add up unless
    /// an event says `replace`.
    Sounds,
    /// `FontManager.loadResourceStack`: the top pack's providers come first.
    Font,
    /// `SpriteSourceList.load`: sources from the bottom pack up.
    Atlas,
    /// `BlockStateModelLoader.loadBlockStateDefinitionStack`: every pack's
    /// definition binds on its own, and a higher one replaces only the states
    /// it binds. Kept as its layers ([`BLOCKSTATE_LAYERS`]) for the catalog.
    BlockStates,
}
/// The key of a stacked blockstate file: its definitions, bottom first, each
/// as the pack wrote it.
pub const BLOCKSTATE_LAYERS: &str = "horizon:layers";
/// Whether the packs merger stacks this file's every layer (atlases, lang,
/// sounds, blockstates) instead of letting the topmost one win.
pub fn merge_stacks(name: &str) -> bool {
    merge_kind(name).is_some()
}

fn merge_kind(name: &str) -> Option<Merge> {
    let (_, rest) = name.strip_prefix("assets/")?.split_once('/')?;
    if rest == "sounds.json" {
        return Some(Merge::Sounds);
    }
    let dir = rest.strip_suffix(".json")?.split_once('/')?.0;
    match dir {
        "lang" => Some(Merge::Lang),
        "font" => Some(Merge::Font),
        "atlases" => Some(Merge::Atlas),
        "blockstates" => Some(Merge::BlockStates),
        _ => None,
    }
}
/// One file with the effect of loading `layers` (bottom first) as vanilla
/// does. Layers that do not parse are skipped, as vanilla logs and skips them.
fn merge(kind: Merge, layers: &[Vec<u8>]) -> Option<Vec<u8>> {
    use serde_json::{Map, Value};
    let mut parsed: Vec<Map<String, Value>> = layers
        .iter()
        // SoundManager skips a sounds.json Gson rejects, whole.
        .filter(|b| kind != Merge::Sounds || crate::sound_events::is_valid_file(b))
        .filter_map(|b| {
            match serde_json::from_slice(b) {
                Ok(Value::Object(m)) => Some(m),
                // `Language.loadFromJson` reads with Gson's lenient reader, so
                // a lang file with `#` comments still counts; a file that
                // strict JSON accepts keeps its bytes as they are.
                _ if kind == Merge::Lang => {
                    let mut strings = std::collections::HashMap::new();
                    horizon_client_core::lang::read_lang_file(b, &mut strings);
                    Some(
                        strings
                            .into_iter()
                            .map(|(k, v)| (k, Value::String(v)))
                            .collect(),
                    )
                }
                _ => None,
            }
        })
        .collect();
    let list = |key: &str, layers: &mut dyn Iterator<Item = &Map<String, Value>>| {
        let items: Vec<Value> = layers
            .filter_map(|l| l.get(key).and_then(Value::as_array))
            .flatten()
            .cloned()
            .collect();
        Value::Object(Map::from_iter([(key.to_owned(), Value::Array(items))]))
    };
    let out = match kind {
        Merge::Lang => Value::Object(parsed.into_iter().flatten().collect()),
        Merge::Sounds => {
            let mut events = Map::new();
            for (event, def) in parsed.into_iter().flatten() {
                if !events.contains_key(&event)
                    || crate::sound_events::json_bool(&def["replace"], false)
                {
                    let mut fresh = Map::new();
                    if let Some(subtitle) = def.get("subtitle") {
                        fresh.insert("subtitle".into(), subtitle.clone());
                    }
                    fresh.insert("sounds".into(), Value::Array(Vec::new()));
                    events.insert(event.clone(), Value::Object(fresh));
                }
                if let (Some(Value::Array(sounds)), Some(Value::Array(more))) = (
                    events.get_mut(&event).and_then(|e| e.get_mut("sounds")),
                    def.get("sounds"),
                ) {
                    sounds.extend(more.iter().cloned());
                }
            }
            Value::Object(events)
        }
        Merge::Font => list("providers", &mut parsed.iter().rev()),
        // SpriteSourceList: the bottom file whole, each layer's sources
        // appended to its own, from the bottom up. A layer whose sources
        // are not a list is skipped, as vanilla logs and skips the file.
        Merge::Atlas => {
            let mut layers = parsed
                .into_iter()
                .filter(|l| l.get("sources").is_none_or(Value::is_array));
            let mut doc = layers.next()?;
            let added = layers
                .filter_map(|l| l.get("sources").and_then(Value::as_array).cloned())
                .flatten()
                .collect();
            atlas_append(&mut doc, added)?;
            Value::Object(doc)
        }
        // One definition is the file as it is.
        Merge::BlockStates if parsed.len() < 2 => Value::Object(parsed.pop()?),
        Merge::BlockStates => Value::Object(Map::from_iter([(
            BLOCKSTATE_LAYERS.to_owned(),
            Value::Array(parsed.into_iter().map(Value::Object).collect()),
        )])),
    };
    serde_json::to_vec(&out).ok()
}
/// `SpriteSourceList.load` on one document: it keeps its own fields, and
/// `sources` gains `added` after its own entries. None when `sources` is
/// not a list.
fn atlas_append(
    doc: &mut serde_json::Map<String, serde_json::Value>,
    added: Vec<serde_json::Value>,
) -> Option<()> {
    let sources = doc
        .entry("sources".to_owned())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    sources.as_array_mut()?.extend(added);
    Some(())
}
/// The atlas file `base` (vanilla's own) plus `added` sources make, as a
/// stack of them loads: the base document whole with the added sources
/// after its own. `what` names the file in errors.
pub fn append_atlas_sources(
    base: Option<&str>,
    added: &[serde_json::Value],
    what: &str,
) -> Result<Vec<u8>, String> {
    let mut doc: serde_json::Map<String, serde_json::Value> = match base {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("{what}: {e}"))?,
        None => serde_json::Map::new(),
    };
    atlas_append(&mut doc, added.to_vec()).ok_or_else(|| format!("{what}: no sources"))?;
    serde_json::to_vec(&doc).map_err(|e| format!("{what}: {e}"))
}
/// `DownloadedPackSource.loadRequestedPacks`: a server's downloaded pack,
/// "World Specific Resources"; None when its `pack.mcmeta` can't be read. Its
/// id names the file, which is named after its hash.
pub fn server_pack(id: u128, path: &Path) -> Option<Pack> {
    let file = path.file_name()?.to_string_lossy();
    pack(
        format!("server/{id:032x}/{file}"),
        Text::translate("resourcePack.server.name", &[]),
        Source::Zip(path.to_path_buf()),
    )
}
/// The packs to apply for an options.txt `resourcePacks` list (bottom first).
pub fn enabled(all: &[Pack], ids: &[String]) -> Vec<Pack> {
    let available = all
        .iter()
        .map(|pack| {
            pack.selection.as_ref().map_or_else(
                || Selectable::optional(&pack.id, pack.compatible),
                crate::pack_screen::PackInfo::selectable,
            )
        })
        .collect();
    SelectionModel::new(available, ids)
        .selected
        .into_iter()
        .rev()
        .filter_map(|selected| all.iter().find(|p| p.id == selected.id).cloned())
        .collect()
}
/// `PackRepository.isAbleToClearAnyPack`: clearing the selection
/// (`rebuildSelected([])`, which keeps only the required vanilla pack)
/// changes it, that is, some selected pack still exists.
pub fn able_to_clear_any_pack(all: &[Pack], selected: &[String]) -> bool {
    let current = enabled(all, selected);
    let required = enabled(all, &[]);
    current
        .iter()
        .map(|pack| &pack.id)
        .ne(required.iter().map(|pack| &pack.id))
}
/// Resource root for a pack selection: vanilla itself when no pack is on.
pub fn resource_root(vanilla: &Path, packs: &[Pack], cache: &Path) -> io::Result<PathBuf> {
    if packs.is_empty() {
        return Ok(vanilla.to_path_buf());
    }
    let key: String = packs
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join("|");
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    });
    let out = cache.join(format!("{hash:016x}"));
    cache::hold(&out)?;
    build_overlay(vanilla, packs, &out)?;
    // Old selections' overlays would otherwise stay for good, each a whole unpacked copy.
    cache::prune(cache);
    Ok(out)
}

/// `Pack.Position`: where a pack goes when it is added to a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Top,
    Bottom,
}
/// A pack as the selection sees it (`PackSelectionConfig` and its
/// compatibility).
#[derive(Clone, Debug, PartialEq)]
pub struct Selectable {
    pub id: String,
    pub required: bool,
    pub position: Position,
    pub fixed: bool,
    pub compatible: bool,
}
impl Selectable {
    /// A pack from `resourcepacks/` or a built-in optional one: not
    /// required, added on top, free to move (`FolderRepositorySource`).
    pub fn optional(id: &str, compatible: bool) -> Self {
        Self {
            id: id.to_owned(),
            required: false,
            position: Position::Top,
            fixed: false,
            compatible,
        }
    }
    /// The vanilla pack: required, at the bottom (`BuiltInPackSource`).
    pub fn vanilla() -> Self {
        Self {
            id: "vanilla".into(),
            required: true,
            position: Position::Bottom,
            fixed: false,
            compatible: true,
        }
    }
    /// A server's downloaded pack (`DownloadedPackSource.DOWNLOADED_PACK_SELECTION`):
    /// required, fixed at the top, so the pack screen cannot remove or reorder it.
    pub fn downloaded(id: &str, compatible: bool) -> Self {
        Self {
            id: id.to_owned(),
            required: true,
            position: Position::Top,
            fixed: true,
            compatible,
        }
    }
}

/// Ids from `DownloadedPackSource` (`server/…`); they must not go into
/// `options.txt` `resourcePacks`.
pub fn is_downloaded_pack_id(id: &str) -> bool {
    id.starts_with("server/")
}

/// The player's own selection: drop every downloaded server/world pack id.
pub fn player_pack_ids(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| !is_downloaded_pack_id(id))
        .collect()
}

/// `Pack.Position.insert`: below or above the packs fixed at that end.
fn insert(list: &mut Vec<Selectable>, pack: Selectable, reverse: bool) {
    let own = pack.position;
    let side = match (own, reverse) {
        (Position::Top, false) | (Position::Bottom, true) => Position::Top,
        _ => Position::Bottom,
    };
    let pinned = |p: &Selectable| p.fixed && p.position == own;
    if side == Position::Bottom {
        let i = list.iter().take_while(|p| pinned(p)).count();
        list.insert(i, pack);
    } else {
        let i = list.len() - list.iter().rev().take_while(|p| pinned(p)).count();
        list.insert(i, pack);
    }
}

/// `PackSelectionModel` over a `PackRepository`: the packs not selected
/// (in id order) and the selected ones, top first, as the pack screen's two
/// lists show them.
#[derive(Clone, Debug, Default)]
pub struct SelectionModel {
    available: Vec<Selectable>,
    pub unselected: Vec<Selectable>,
    pub selected: Vec<Selectable>,
}
impl SelectionModel {
    /// `available` in any order; `selected` as options.txt lists them,
    /// bottom first.
    pub fn new(mut available: Vec<Selectable>, selected: &[String]) -> Self {
        // PackRepository.discoverAvailable: every source's packs go into one
        // TreeMap by id (String.compareTo), so `file/` packs sort among the
        // built-in ones.
        available.sort_by(|a, b| a.id.encode_utf16().cmp(b.id.encode_utf16()));
        available.dedup_by(|a, b| a.id == b.id);
        let mut m = Self {
            available,
            ..Self::default()
        };
        let mut chosen = m.rebuild(selected);
        chosen.reverse();
        m.unselected = m
            .available
            .iter()
            .filter(|p| !chosen.iter().any(|c| c.id == p.id))
            .cloned()
            .collect();
        m.selected = chosen;
        m
    }
    /// `PackRepository.rebuildSelected`: the named packs that exist, and
    /// every required one.
    fn rebuild(&self, ids: &[String]) -> Vec<Selectable> {
        let mut out: Vec<Selectable> = ids
            .iter()
            .filter_map(|id| self.available.iter().find(|p| p.id == *id).cloned())
            .collect();
        for p in &self.available {
            if p.required && !out.iter().any(|o| o.id == p.id) {
                insert(&mut out, p.clone(), false);
            }
        }
        out
    }
    fn list(&self, selected: bool) -> &Vec<Selectable> {
        if selected {
            &self.selected
        } else {
            &self.unselected
        }
    }
    pub fn can_select(&self, selected: bool, _: usize) -> bool {
        !selected
    }
    pub fn can_unselect(&self, selected: bool, i: usize) -> bool {
        selected && !self.selected[i].required
    }
    pub fn can_move_up(&self, selected: bool, i: usize) -> bool {
        i > 0 && !self.list(selected)[i - 1].fixed
    }
    pub fn can_move_down(&self, selected: bool, i: usize) -> bool {
        let list = self.list(selected);
        i + 1 < list.len() && !list[i + 1].fixed
    }
    /// `EntryBase.toggleSelection`: to the other list, at its default
    /// position (from the other end).
    fn toggle(&mut self, selected: bool, i: usize) {
        let (from, to) = if selected {
            (&mut self.selected, &mut self.unselected)
        } else {
            (&mut self.unselected, &mut self.selected)
        };
        let p = from.remove(i);
        insert(to, p, true);
    }
    /// `PackEntry`'s activation (a click on the icon, or the select key):
    /// select, an incompatible pack only once its confirmation was
    /// accepted, or else unselect. True when the lists changed.
    pub fn activate(&mut self, selected: bool, i: usize, confirmed: bool) -> bool {
        if self.can_select(selected, i) {
            if self.unselected[i].compatible || confirmed {
                self.toggle(false, i);
                return true;
            }
            false
        } else if self.can_unselect(selected, i) {
            self.toggle(true, i);
            true
        } else {
            false
        }
    }
    /// `moveUp` / `moveDown` when allowed.
    pub fn move_by(&mut self, selected: bool, i: usize, up: bool) -> bool {
        let ok = if up {
            self.can_move_up(selected, i)
        } else {
            self.can_move_down(selected, i)
        };
        if ok {
            let list = if selected {
                &mut self.selected
            } else {
                &mut self.unselected
            };
            let p = list.remove(i);
            list.insert(if up { i - 1 } else { i + 1 }, p);
        }
        ok
    }
    /// `commit`: the repository's selection, bottom first, as options.txt
    /// stores it (downloaded `server/…` packs stay out of that file).
    pub fn commit(&self) -> Vec<String> {
        let ids: Vec<String> = self.selected.iter().rev().map(|p| p.id.clone()).collect();
        player_pack_ids(self.rebuild(&ids).into_iter().map(|p| p.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_layers_keep_priority_filters_metadata_and_reload_without_unpacking() {
        use crate::resource_source::ResourceSource;
        use std::io::Write;
        let root =
            std::env::temp_dir().join(format!("horizon-indexed-packs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let vanilla = root.join("vanilla");
        let low = root.join("low");
        let maps = root.join("maps");
        let write = |base: &Path, name: &str, bytes: &[u8]| {
            let path = base.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        };
        for (name, bytes) in [
            ("assets/demo/common.txt", b"vanilla".as_slice()),
            ("assets/demo/hidden.txt", b"vanilla"),
            ("assets/demo/only_base.txt", b"base-only"),
            ("assets/demo/textures/stone.png", b"base-colour"),
            ("assets/demo/textures/stone_n.png", b"base-normal"),
            ("assets/demo/textures/stone_s.png", b"base-specular"),
            ("assets/demo/textures/stone.png.mcmeta", b"old-metadata"),
            (
                "assets/demo/lang/en_us.json",
                br#"{"vanilla":"v","key":"v"}"#,
            ),
            ("tables.json", b"tables"),
        ] {
            write(&vanilla, name, bytes);
        }
        write(&low, "assets/demo/common.txt", b"folder");
        write(&low, "assets/demo/hidden.txt", b"folder");
        write(&low, "assets/demo/lang/en_us.json", br#"{"key":"folder"}"#);
        write(&maps, "assets/demo/textures/stone_n.png", b"new-normal");
        let archive_path = root.join("passive.jar");
        let archive = |path: &Path, replacement: &[u8]| {
            let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
            for (name, bytes) in [
                ("assets/demo/common.txt", b"jar".as_slice()),
                ("new/assets/demo/common.txt", replacement),
                ("assets/demo/hidden.txt", b"own-filtered-name"),
                ("assets/demo/textures/stone.png", b"jar-colour"),
                ("assets/demo/lang/en_us.json", br#"{"jar":"j"}"#),
                ("assets/../../escape.txt", b"unsafe"),
                ("Ignored.class", b"passive bytes"),
            ] {
                writer
                    .start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        };
        archive(&archive_path, b"overlay");
        let low = Pack::mod_layer("folder".into(), Source::Dir(low));
        let mut jar = Pack::mod_layer("jar".into(), Source::Zip(archive_path.clone()));
        jar.overlays = vec!["new".into()];
        jar.filter = filter_of(
            &serde_json::json!({"filter":{"block":[{"namespace":"demo","path":"hidden\\.txt"}]}}),
        );
        let base = ResourceSource::directory(vanilla);
        let before = ResourceSource::directory(root.clone()).list("");
        let loaded = resources(base.clone(), &[low.clone(), jar.clone()]).unwrap();
        assert_eq!(loaded.read("assets/demo/common.txt").unwrap(), b"overlay");
        // A pack's own files survive its filter; only lower copies are hidden.
        assert_eq!(
            loaded.read("assets/demo/hidden.txt").unwrap(),
            b"own-filtered-name"
        );
        assert_eq!(
            loaded.read("assets/demo/only_base.txt").unwrap(),
            b"base-only"
        );
        assert_eq!(loaded.read("tables.json").unwrap(), b"tables");
        let lang: serde_json::Value =
            serde_json::from_slice(&loaded.read("assets/demo/lang/en_us.json").unwrap()).unwrap();
        assert_eq!(
            lang,
            serde_json::json!({"vanilla":"v","key":"folder","jar":"j"})
        );
        for hidden in [
            "assets/demo/textures/stone_n.png",
            "assets/demo/textures/stone_s.png",
            "assets/demo/textures/stone.png.mcmeta",
            "Ignored.class",
        ] {
            assert_eq!(
                loaded.read(hidden).unwrap_err().kind(),
                io::ErrorKind::NotFound
            );
            assert!(!loaded.list("").iter().any(|name| name == hidden));
        }
        assert_eq!(ResourceSource::directory(root.clone()).list(""), before);
        assert!(!root.join("assets").exists());
        assert!(!root.join("escape.txt").exists());
        let maps = Pack::mod_layer("maps".into(), Source::Dir(maps));
        let with_maps = resources(base.clone(), &[low.clone(), jar.clone(), maps]).unwrap();
        assert_eq!(
            with_maps.read("assets/demo/textures/stone_n.png").unwrap(),
            b"new-normal"
        );
        let mut filter_only = jar.clone();
        filter_only.source = Source::Dir(root.join("empty"));
        filter_only.overlays.clear();
        let filtered = resources(base.clone(), &[low.clone(), filter_only]).unwrap();
        assert_eq!(
            filtered.read("assets/demo/hidden.txt").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(
            !filtered
                .list("assets/demo/")
                .iter()
                .any(|name| name.ends_with("hidden.txt"))
        );
        let replacement = root.join("replacement.jar");
        archive(&replacement, b"reloaded");
        fs::rename(&archive_path, root.join("old.jar")).unwrap();
        fs::rename(replacement, &archive_path).unwrap();
        let reloaded = resources(base, &[low, jar]).unwrap();
        assert_eq!(
            reloaded.read("assets/demo/common.txt").unwrap(),
            b"reloaded"
        );
        assert_eq!(loaded.read("assets/demo/common.txt").unwrap(), b"overlay");
        let _ = fs::remove_dir_all(root);
    }

    /// `FolderRepositorySource.discoverPacks` + `PackDetector.detectPackResources`:
    /// an entry that is a link, or holds one, is not a pack the game lists.
    #[cfg(unix)]
    #[test]
    fn pack_discovery_refuses_symlinks() {
        use std::os::unix::fs::symlink;
        let base = std::env::temp_dir().join(format!("horizon-pack-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        let pack = |dir: &Path| {
            fs::create_dir_all(dir.join("assets/deep")).unwrap();
            fs::write(dir.join("pack.mcmeta"), meta).unwrap();
        };
        // A linked pack's target, a file a pack may hold a link to, and a link
        // whose own target is a link.
        pack(&base.join("outside/pack"));
        fs::create_dir_all(base.join("outside")).unwrap();
        fs::write(base.join("outside/file.txt"), "f").unwrap();
        symlink(base.join("outside/pack"), base.join("outside/alias")).unwrap();
        let packs = base.join("packs");
        for name in ["clean", "nested", "broken"] {
            pack(&packs.join(name));
        }
        symlink(
            base.join("outside/file.txt"),
            packs.join("nested/assets/deep/link"),
        )
        .unwrap();
        symlink(base.join("missing"), packs.join("broken/link")).unwrap();
        symlink(base.join("outside/pack"), packs.join("linked")).unwrap();
        symlink(base.join("outside/alias"), packs.join("chained")).unwrap();
        let ids = |validator: &DirectoryValidator| {
            let mut ids: Vec<String> = folder_with(&packs, RESOURCE_FORMAT, validator)
                .into_iter()
                .map(|p| p.id)
                .collect();
            ids.extend(
                crate::datapack::discover_folder_with(&packs, validator)
                    .into_iter()
                    .map(|p| p.id),
            );
            ids
        };
        // Vanilla's default validator, with no `allowed_symlinks.txt`: no link.
        assert_eq!(
            ids(&DirectoryValidator::default()),
            ["file/clean", "file/clean"]
        );
        // An entry naming the linked pack's target lets that link, and only it,
        // through: the pack holding a link to a file, the broken link and the
        // link to a link stay out. A `[glob]` entry Horizon cannot compile
        // allows nothing.
        let allow = base.join(ALLOWED_SYMLINKS_CONFIG_NAME);
        fs::write(
            &allow,
            format!(
                "# packs shared between game folders\n[glob]**/outside/**\n[prefix]{}\n[prefix]{}\n",
                base.join("outside/pack").display(),
                base.join("outside/alias").display()
            ),
        )
        .unwrap();
        assert_eq!(
            ids(&DirectoryValidator::parse(&allow)),
            ["file/clean", "file/linked", "file/clean", "file/linked"]
        );
        // An unsupported entry type throws in `ConfigEntry.parse`, so the whole
        // file allows no link.
        fs::write(
            &allow,
            format!(
                "[bogus]{}\n[prefix]{}\n",
                base.join("outside/pack").display(),
                base.join("outside/pack").display()
            ),
        )
        .unwrap();
        assert_eq!(
            ids(&DirectoryValidator::parse(&allow)),
            ["file/clean", "file/clean"]
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// `DownloadedPackSource.DOWNLOADED_PACK_SELECTION`: the pack is selected,
    /// fixed at the top, cannot be removed, and stays out of options.txt.
    #[test]
    fn a_downloaded_server_pack_is_fixed_selected_and_not_committed() {
        let server = "server/00000000000000000000000000000001/hash.zip";
        let available = vec![
            Selectable::optional("file/Mine", true),
            Selectable::downloaded(server, true),
            Selectable::vanilla(),
        ];
        let model = SelectionModel::new(available, &["file/Mine".into()]);
        assert_eq!(
            model
                .selected
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            [server, "file/Mine", "vanilla"]
        );
        assert!(!model.can_unselect(true, 0));
        // A neighbour cannot move up past the fixed server pack.
        assert!(!model.can_move_up(true, 1));
        assert_eq!(
            model.commit(),
            vec!["vanilla".to_owned(), "file/Mine".to_owned()]
        );
        assert!(is_downloaded_pack_id(server));
        assert!(!is_downloaded_pack_id("file/Mine"));
    }

    #[test]
    fn a_comment_after_the_metadata_does_not_hide_a_pack() {
        let meta = br#"{"pack":{"description":"d","pack_format":97}}
// Signed by the pack's author"#;
        assert!(parse_meta(meta).is_some());
        assert!(parse_meta(b"not json").is_none());
    }

    #[test]
    fn mcmeta_with_utf8_bom_is_read() {
        let dir = std::env::temp_dir().join(format!("horizon-bom-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut meta = b"\xEF\xBB\xBF".to_vec();
        meta.extend_from_slice(br#"{"pack":{"description":"d","pack_format":97}}"#);
        fs::write(dir.join("pack.mcmeta"), &meta).unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_some());
        // Without a BOM the same JSON still reads; other leading bytes still fail.
        fs::write(dir.join("pack.mcmeta"), &meta[3..]).unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_some());
        fs::write(
            dir.join("pack.mcmeta"),
            [b"\xEF\xBB".as_slice(), &meta[3..]].concat(),
        )
        .unwrap();
        assert!(read_meta(&Source::Dir(dir.clone())).is_none());
        fs::remove_dir_all(&dir).ok();
    }
    /// `PackFormat.IntermediaryFormat.validate` and its three helpers, with
    /// the resource pack's last pre-minor version, 64.
    #[test]
    fn pack_format_validation() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let check = |s: &str| validate_pack_range(&v(s), 64);
        // validate: both min_format and max_format, or neither.
        assert!(
            check(r#"{"min_format":97}"#)
                .unwrap_err()
                .contains("both min_format and max_format")
        );
        assert!(check(r#"{"max_format":97}"#).is_err());
        assert!(check("{}").unwrap_err().contains("missing format version"));
        // A pack_format alone: fine up to 64, not above.
        assert_eq!(check(r#"{"pack_format":34}"#), Ok(((34, 0), (34, 0))));
        assert!(
            check(r#"{"pack_format":97}"#)
                .unwrap_err()
                .contains("newer than 64")
        );
        // validateNewFormat: min above max, the deprecated key above 64.
        assert!(
            check(r#"{"min_format":98,"max_format":97}"#)
                .unwrap_err()
                .contains("greater than max_format")
        );
        assert!(check(r#"{"min_format":[97,2],"max_format":[97,1]}"#).is_err());
        assert!(
            check(r#"{"min_format":97,"max_format":98,"supported_formats":[97,98]}"#)
                .unwrap_err()
                .contains("deprecated")
        );
        // A decimal reads as its whole part, in a list or alone: `[97.1]` is format 97 (a pack
        // that meant `[97, 1]` still loads and counts as compatible).
        assert_eq!(
            check(r#"{"min_format":[97.1],"max_format":[97.1]}"#),
            Ok(((97, 0), (97, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":97.9,"max_format":98.5}"#),
            Ok(((97, 0), (98, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":97,"max_format":98}"#),
            Ok(((97, 0), (98, i64::from(i32::MAX))))
        );
        assert_eq!(
            check(r#"{"min_format":[97,0],"max_format":[97,1]}"#),
            Ok(((97, 0), (97, 1)))
        );
        // validatePackFormatForRange: a pack_format outside the range.
        assert!(
            check(r#"{"min_format":97,"max_format":98,"pack_format":99}"#)
                .unwrap_err()
                .contains("declared main format is 99")
        );
        assert!(check(r#"{"min_format":97,"max_format":98,"pack_format":98}"#).is_ok());
        // Below 65 the old fields are required and must agree.
        assert!(
            check(r#"{"min_format":34,"max_format":70}"#)
                .unwrap_err()
                .contains("require a supported_formats")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[35,64],"pack_format":35}"#
            )
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,60],"pack_format":35}"#
            )
            .unwrap_err()
            .contains("mismatch")
        );
        assert!(
            check(r#"{"min_format":34,"max_format":70,"supported_formats":[34,64]}"#)
                .unwrap_err()
                .contains("require a pack_format")
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,64],"pack_format":33}"#
            )
            .is_err()
        );
        assert!(
            check(
                r#"{"min_format":34,"max_format":70,"supported_formats":[34,64],"pack_format":34}"#
            )
            .is_ok()
        );
        // Multi-version packs below 15 cannot work.
        assert!(
            check(
                r#"{"min_format":10,"max_format":70,"supported_formats":[10,64],"pack_format":10}"#
            )
            .unwrap_err()
            .contains("less than 15")
        );
        // validateOldFormat: a range above 64, or no pack_format.
        assert!(
            check(r#"{"supported_formats":[34,70],"pack_format":34}"#)
                .unwrap_err()
                .contains("newer than 64")
        );
        assert!(
            check(r#"{"supported_formats":[34,60]}"#)
                .unwrap_err()
                .contains("require a pack_format")
        );
        assert_eq!(
            check(r#"{"supported_formats":[34,60],"pack_format":40}"#),
            Ok(((34, 0), (60, 0)))
        );
        assert!(
            check(
                r#"{"supported_formats":{"min_inclusive":34,"max_inclusive":60},"pack_format":40}"#
            )
            .is_ok()
        );
        assert!(check(r#"{"supported_formats":[60,34],"pack_format":40}"#).is_err());
        // A refused pack is of unknown format.
        use crate::pack_screen::Compat;
        assert_eq!(
            compatibility(&v(r#"{"pack":{"pack_format":97}}"#)),
            Compat::Unknown
        );
        assert_eq!(
            compatibility(&v(
                r#"{"pack":{"min_format":97,"max_format":98,"pack_format":99}}"#
            )),
            Compat::Unknown
        );
    }
    #[test]
    fn compatibility_ranges() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        assert!(compatible(&v(
            r#"{"pack":{"min_format":97,"max_format":98}}"#
        )));
        assert!(compatible(&v(
            r#"{"pack":{"min_format":[97,0],"max_format":[97,1]}}"#
        )));
        assert!(!compatible(&v(
            r#"{"pack":{"min_format":[97,2],"max_format":99}}"#
        )));
        assert!(!compatible(&v(r#"{"pack":{"pack_format":34}}"#)));
        assert_eq!(
            component(&v(r#"["a",{"text":"b","color":"red"}]"#)).plain(),
            "ab"
        );
    }
    #[test]
    fn a_sound_entry_whose_polytone_condition_fails_is_left_out() {
        let base =
            std::env::temp_dir().join(format!("horizon-packs-sound-cond-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (vanilla, dir, out) = (base.join("vanilla"), base.join("pack"), base.join("out"));
        fs::create_dir_all(vanilla.join("assets/minecraft")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/sounds.json"),
            r#"{"block.anvil.land":{"sounds":["vanilla"]}}"#,
        )
        .unwrap();
        let configs = dir.join("assets/mypack/polytone/config_entries");
        fs::create_dir_all(&configs).unwrap();
        fs::create_dir_all(dir.join("assets/minecraft")).unwrap();
        fs::write(configs.join("quiet.json"), r#"{"default_value": "OFF"}"#).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"p","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("assets/minecraft/sounds.json"),
            r#"{"block.anvil.land":{"replace":true,"polytone_condition":"config('mypack:quiet') == 'ON'","sounds":["soft"]},
                "block.anvil.use":{"sounds":["kept"]}}"#,
        )
        .unwrap();
        let pack = pack("file/p".into(), Text::literal("p"), Source::Dir(dir)).unwrap();
        build_overlay(&vanilla, &[pack], &out).unwrap();
        let merged: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("assets/minecraft/sounds.json")).unwrap())
                .unwrap();
        // The condition is false: the pack's entry doesn't replace vanilla's.
        assert_eq!(
            merged["block.anvil.land"]["sounds"],
            serde_json::json!(["vanilla"])
        );
        assert_eq!(
            merged["block.anvil.use"]["sounds"],
            serde_json::json!(["kept"])
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn stacked_resources_merge_like_vanilla() {
        let json = |v: &str| v.as_bytes().to_vec();
        let parse = |b: Vec<u8>| serde_json::from_slice::<serde_json::Value>(&b).unwrap();
        // ClientLanguage: later packs override keys, the rest stays.
        let lang = merge(
            Merge::Lang,
            &[
                json(r#"{"a":"vanilla a","b":"vanilla b"}"#),
                json(r#"{"b":"pack b"}"#),
            ],
        );
        assert_eq!(
            parse(lang.unwrap()),
            serde_json::json!({"a":"vanilla a","b":"pack b"})
        );
        // A layer with `#` comments is read leniently, not dropped.
        let lenient = merge(
            Merge::Lang,
            &[
                json(r#"{"a":"vanilla a"}"#),
                json("# a pack comment\n{\"b\":\"pack b\", # trailing\n\"a\":\"pack a\"}"),
            ],
        );
        assert_eq!(
            parse(lenient.unwrap()),
            serde_json::json!({"a":"pack a","b":"pack b"})
        );
        // SoundManager: sounds add up; `replace` starts the event over.
        let sounds = merge(
            Merge::Sounds,
            &[
                json(r#"{"x":{"subtitle":"s.x","sounds":["v1"]},"y":{"sounds":["v2"]}}"#),
                json(
                    r#"{"x":{"sounds":["p1"]},"y":{"replace":true,"subtitle":"s.y","sounds":["p2"]}}"#,
                ),
                b"not json".to_vec(),
            ],
        );
        assert_eq!(
            parse(sounds.unwrap()),
            serde_json::json!({
                "x":{"subtitle":"s.x","sounds":["v1","p1"]},
                "y":{"subtitle":"s.y","sounds":["p2"]}
            })
        );
        // FontManager: the top pack's providers come first.
        let font = merge(
            Merge::Font,
            &[json(r#"{"providers":[1,2]}"#), json(r#"{"providers":[3]}"#)],
        );
        assert_eq!(
            parse(font.unwrap()),
            serde_json::json!({"providers":[3,1,2]})
        );
        // SpriteSourceList: sources from the bottom pack up, and the bottom
        // file keeps the fields a merge must not drop.
        let atlas = merge(
            Merge::Atlas,
            &[
                json(r#"{"sources":[1],"unseen":true}"#),
                json(r#"{"sources":[2]}"#),
            ],
        );
        assert_eq!(
            parse(atlas.unwrap()),
            serde_json::json!({"sources":[1,2],"unseen":true})
        );
        // BlockStateModelLoader: every definition kept, bottom first, for the
        // catalog to bind one by one; one that does not parse is skipped.
        let gate = merge(
            Merge::BlockStates,
            &[
                json(r#"{"variants":{"":{"model":"v"}}}"#),
                b"not json".to_vec(),
                json(r#"{"variants":{"powered=true":{"model":"p"}}}"#),
            ],
        );
        assert_eq!(
            parse(gate.unwrap()),
            serde_json::json!({BLOCKSTATE_LAYERS:[
                {"variants":{"":{"model":"v"}}},
                {"variants":{"powered=true":{"model":"p"}}}
            ]})
        );
        // A single definition stays the file as it is.
        let alone = merge(
            Merge::BlockStates,
            &[json(r#"{"variants":{"":{"model":"p"}}}"#)],
        );
        assert_eq!(
            parse(alone.unwrap()),
            serde_json::json!({"variants":{"":{"model":"p"}}})
        );
        assert_eq!(
            merge_kind("assets/minecraft/blockstates/oak_fence_gate.json"),
            Some(Merge::BlockStates)
        );
        assert_eq!(
            merge_kind("assets/minecraft/lang/en_us.json"),
            Some(Merge::Lang)
        );
        assert_eq!(
            merge_kind("assets/minecraft/sounds.json"),
            Some(Merge::Sounds)
        );
        assert_eq!(
            merge_kind("assets/minecraft/font/default.json"),
            Some(Merge::Font)
        );
        assert_eq!(merge_kind("assets/minecraft/font/unifont.zip"), None);
        assert_eq!(
            merge_kind("assets/minecraft/atlases/blocks.json"),
            Some(Merge::Atlas)
        );
        assert_eq!(merge_kind("assets/minecraft/models/block/stone.json"), None);
    }
    #[test]
    fn metadata_free_mod_and_jar_layers_keep_pack_order() {
        use std::io::Write;
        let base = std::env::temp_dir().join(format!("horizon-mod-layers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        let path = "assets/shared/textures/block/test.png";
        fs::create_dir_all(vanilla.join(path).parent().unwrap()).unwrap();
        fs::write(vanilla.join(path), b"vanilla").unwrap();
        let jar = |name: &str, content: &[u8]| {
            let path = base.join(name);
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            for (entry, bytes) in [
                ("assets/shared/textures/block/test.png", content),
                ("data/shared/recipe/test.json", b"{}".as_slice()),
                ("Example.class", b"dummy".as_slice()),
            ] {
                zip.start_file(entry, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap();
            path
        };
        let jar_a = jar("a.jar", b"jar-a");
        let jar_b = jar("b.jar", b"jar-b");
        let mod_a = base.join("mod-a");
        let mod_b = base.join("mod-b");
        for (root, bytes) in [(&mod_a, b"mod-a".as_slice()), (&mod_b, b"mod-b".as_slice())] {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), bytes).unwrap();
        }
        let layers = [
            Pack::mod_layer("jar-a".into(), Source::Zip(jar_a)),
            Pack::mod_layer("mod-a".into(), Source::Dir(mod_a)),
            Pack::mod_layer("jar-b".into(), Source::Zip(jar_b)),
            Pack::mod_layer("mod-b".into(), Source::Dir(mod_b)),
        ];
        let out = base.join("overlay");
        build_overlay(&vanilla, &layers[..3], &out).unwrap();
        assert_eq!(fs::read(out.join(path)).unwrap(), b"jar-b");
        build_overlay(&vanilla, &layers, &out).unwrap();
        assert_eq!(fs::read(out.join(path)).unwrap(), b"mod-b");
        assert!(!out.join("data/shared/recipe/test.json").exists());
        assert!(!out.join("Example.class").exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn companion_maps_follow_the_winning_colour_or_a_higher_map_only_pack() {
        let root =
            std::env::temp_dir().join(format!("horizon-material-owner-{}", std::process::id()));
        let vanilla = root.join("vanilla");
        let relative = "assets/minecraft/textures/block";
        fs::create_dir_all(vanilla.join(relative)).unwrap();
        let names = ["stone.png", "stone_n.png", "stone_s.png"];
        for name in names {
            fs::write(vanilla.join(relative).join(name), b"base").unwrap();
        }
        for (pack, files) in [
            ("low", vec!["stone_n.png", "stone_s.png"]),
            ("colour", vec!["stone.png"]),
            ("maps", vec!["stone_n.png"]),
        ] {
            let dir = root.join("packs").join(pack);
            fs::create_dir_all(dir.join(relative)).unwrap();
            fs::write(
                dir.join("pack.mcmeta"),
                br#"{"pack":{"description":"fixture","pack_format":97}}"#,
            )
            .unwrap();
            for name in files {
                fs::write(dir.join(relative).join(name), pack.as_bytes()).unwrap();
            }
        }
        let all = discover(&vanilla, &root.join("packs"));
        let order = |names: &[&str]| {
            names
                .iter()
                .map(|name| {
                    all.iter()
                        .find(|p| p.id == format!("file/{name}"))
                        .unwrap()
                        .clone()
                })
                .collect::<Vec<_>>()
        };
        let out = root.join("out");
        build_overlay(&vanilla, &order(&["low", "colour"]), &out).unwrap();
        assert_eq!(
            fs::read(out.join(relative).join("stone.png")).unwrap(),
            b"colour"
        );
        assert!(!out.join(relative).join("stone_n.png").exists());
        assert!(!out.join(relative).join("stone_s.png").exists());
        build_overlay(&vanilla, &order(&["low", "colour", "maps"]), &out).unwrap();
        assert_eq!(
            fs::read(out.join(relative).join("stone_n.png")).unwrap(),
            b"maps"
        );
        assert!(!out.join(relative).join("stone_s.png").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn overlay_layers_packs_over_vanilla() {
        // `&` would split an unquoted `cmd /C mklink` line.
        let base = std::env::temp_dir().join(format!("horizon-packs-&{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(vanilla.join("assets/minecraft/lang")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"vanilla dirt",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/lang/en_us.json"),
            br#"{"a":"vanilla","b":"vanilla"}"#,
        )
        .unwrap();
        fs::write(vanilla.join("facts.json"), b"[]").unwrap();
        // A directory no pack touches (#1688: a font json names sheets in it).
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/font")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/font/ascii.png"),
            b"font",
        )
        .unwrap();
        let pack_dir = base.join("packs/Mine");
        fs::create_dir_all(pack_dir.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            pack_dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"mine","pack_format":97}}"#,
        )
        .unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/textures/block/stone.png"),
            b"pack",
        )
        .unwrap();
        fs::create_dir_all(pack_dir.join("assets/minecraft/lang")).unwrap();
        fs::write(
            pack_dir.join("assets/minecraft/lang/en_us.json"),
            br#"{"b":"pack"}"#,
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].id, "file/Mine");
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/dirt.png")).unwrap(),
            b"vanilla dirt"
        );
        // A pack's language file adds to vanilla's instead of replacing it.
        assert_eq!(
            fs::read(out.join("assets/minecraft/lang/en_us.json")).unwrap(),
            br#"{"a":"vanilla","b":"pack"}"#
        );
        assert_eq!(fs::read(out.join("facts.json")).unwrap(), b"[]");
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/font/ascii.png")).unwrap(),
            b"font"
        );
        // Linked, not mirrored: a junction on Windows, a symlink on unix.
        assert!(
            fs::symlink_metadata(out.join("assets/minecraft/textures/font"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn a_renamed_atlas_sprite_reads_its_resource() {
        let base =
            std::env::temp_dir().join(format!("horizon-sprite-rename-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let write = |path: PathBuf, bytes: &[u8]| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        };
        let vanilla = base.join("vanilla");
        let textures = vanilla.join("assets/minecraft/textures");
        let animation = b"{\"animation\":{}}";
        write(textures.join("block/sea_lantern.png"), b"lantern");
        write(textures.join("block/sea_lantern.png.mcmeta"), animation);
        write(textures.join("item/stick.png"), b"vanilla stick");
        write(textures.join("item/stick.png.mcmeta"), animation);
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        let pack = base.join("packs/IA");
        write(pack.join("pack.mcmeta"), meta);
        write(pack.join("assets/iageneric/textures/item/box.png"), b"box");
        // An ItemsAdder pack's items atlas: its models name only the `ia:` sprites.
        write(
            pack.join("assets/minecraft/atlases/items.json"),
            br#"{"sources":[
                {"type":"single","resource":"iageneric:item/box","sprite":"ia:0"},
                {"type":"single","resource":"minecraft:block/sea_lantern","sprite":"ia:1199"},
                {"type":"single","resource":"iageneric:item/box","sprite":"minecraft:item/stick"},
                {"type":"single","resource":"minecraft:item/stick","sprite":"ia:3"},
                {"type":"single","resource":"iageneric:item/gone","sprite":"ia:2"},
                {"type":"single","resource":"iageneric:item/box","sprite":"ia:../../../escape"},
                {"type":"single","resource":"iageneric:item/box","sprite":"ia:7"},
                {"type":"single","resource":"iageneric:item/box","sprite":"ia:8"},
                {"type":"single","resource":"iageneric:item/box","sprite":"ia:9"}
            ]}"#,
        );
        // A pack above it: later sources of three of those sprites, a blocks atlas that
        // disagrees about another, and a filter hiding vanilla's animation of the stick.
        let higher = base.join("packs/Top");
        write(
            higher.join("pack.mcmeta"),
            br#"{"pack":{"description":"d","pack_format":97},
                "filter":{"block":[{"path":"textures/item/stick\\.png\\.mcmeta"}]}}"#,
        );
        write(
            higher.join("assets/minecraft/atlases/items.json"),
            br#"{"sources":[
                {"type":"single","resource":"minecraft:block/sea_lantern","sprite":"ia:8"},
                {"type":"filter","pattern":{"namespace":"ia","path":"7"}}
            ]}"#,
        );
        write(
            higher.join("assets/minecraft/atlases/blocks.json"),
            br#"{"sources":[
                {"type":"single","resource":"minecraft:block/sea_lantern","sprite":"ia:9"}
            ]}"#,
        );
        let packs = discover(&vanilla, &base.join("packs"));
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        let read = |rel: &str| fs::read(out.join(rel)).ok();
        let texture = |id: &str| read(&format!("assets/ia/textures/{id}.png"));
        assert_eq!(texture("0").as_deref(), Some(&b"box"[..]));
        assert_eq!(read("assets/ia/textures/0.png.mcmeta"), None);
        // A vanilla resource comes with its animation.
        assert_eq!(texture("1199").as_deref(), Some(&b"lantern"[..]));
        assert_eq!(
            read("assets/ia/textures/1199.png.mcmeta").as_deref(),
            Some(&animation[..])
        );
        // A texture that is there stays itself: other sources may read it (here as `ia:3`'s
        // resource; an unstitched or paletted sprite's input alike).
        assert_eq!(
            read("assets/minecraft/textures/item/stick.png").as_deref(),
            Some(&b"vanilla stick"[..])
        );
        // ...with its `.mcmeta` only where no filter hides it.
        assert_eq!(texture("3").as_deref(), Some(&b"vanilla stick"[..]));
        assert_eq!(read("assets/ia/textures/3.png.mcmeta"), None);
        // A missing resource adds nothing, and no rename leaves the overlay.
        assert_eq!(texture("2"), None);
        assert!(!out.join("escape.png").exists());
        // The last source of a sprite wins: a filter drops it, a later rename replaces it.
        assert_eq!(texture("7"), None);
        assert_eq!(texture("8").as_deref(), Some(&b"lantern"[..]));
        // The two atlases read one file, and disagree about `ia:9`: it gets neither.
        assert_eq!(texture("9"), None);
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn every_file_of_a_big_stack_comes_from_its_top_pack() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("horizon-pack-stack-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        // Folders whose names start alike (`item`, `item_extra`, `item2`) and files beside
        // folders of almost their name: what a prefix lookup could mix up.
        let names: Vec<String> = (0..400)
            .map(|i| {
                let folder = ["item", "item_extra", "item2", "item/sub", "block"][i % 5];
                format!("assets/ns{}/textures/{folder}/f{i}.png", i % 3)
            })
            .chain(["assets/ns0/textures/item.png".to_owned()])
            .collect();
        let mut expected: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for name in names.iter().step_by(4) {
            let path = vanilla.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("vanilla {name}")).unwrap();
            expected.insert(name.clone(), format!("vanilla {name}").into_bytes());
        }
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        // Packs a and b are zips, c a folder; each provides a different part of the names.
        fs::create_dir_all(base.join("packs/c")).unwrap();
        fs::write(base.join("packs/c/pack.mcmeta"), meta).unwrap();
        for (pack, every) in [("a", 2), ("b", 3)] {
            let file = fs::File::create(base.join(format!("packs/{pack}.zip"))).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            zip.start_file("pack.mcmeta", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(meta).unwrap();
            for name in names.iter().step_by(every) {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(format!("{pack} {name}").as_bytes()).unwrap();
                expected.insert(name.clone(), format!("{pack} {name}").into_bytes());
            }
            zip.finish().unwrap();
        }
        for name in names.iter().skip(1).step_by(5) {
            let path = base.join("packs/c").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, format!("c {name}")).unwrap();
            expected.insert(name.clone(), format!("c {name}").into_bytes());
        }
        let all = folder(&base.join("packs"), RESOURCE_FORMAT);
        let ids = ["file/a.zip", "file/b.zip", "file/c"].map(String::from);
        let out = base.join("overlay");
        build_overlay(&vanilla, &enabled(&all, &ids), &out).unwrap();
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    let name = path.strip_prefix(root).unwrap().to_string_lossy();
                    out.insert(name.replace('\\', "/"), fs::read(&path).unwrap());
                }
            }
        }
        let mut built = BTreeMap::new();
        walk(&out.join("assets"), &out, &mut built);
        assert_eq!(built, expected);
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    #[ignore = "daily"]
    fn a_pack_that_unpacks_past_its_budget_fails_and_leaves_nothing() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("horizon-pack-bomb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::create_dir_all(base.join("packs")).unwrap();
        let pack = |name: &str, files: &[(&str, usize)]| {
            let path = base.join("packs").join(name);
            let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
            let deflated = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("pack.mcmeta", deflated).unwrap();
            zip.write_all(br#"{"pack":{"description":"d","pack_format":97}}"#)
                .unwrap();
            let zeros = vec![0u8; 1 << 20];
            for (file, mib) in files {
                zip.start_file(*file, deflated).unwrap();
                for _ in 0..*mib {
                    zip.write_all(&zeros).unwrap();
                }
            }
            zip.finish().unwrap();
            fs::metadata(&path).unwrap().len()
        };
        // Three 22 MiB files of zeros: well under the per-file limit, 66 MiB in all from a zip of
        // a few hundred KiB, past the 64 MiB a small pack may unpack to.
        let size = pack(
            "bomb.zip",
            &[
                ("assets/minecraft/textures/block/a.png", 22),
                ("assets/minecraft/textures/block/b.png", 22),
                ("assets/minecraft/textures/block/c.png", 22),
            ],
        );
        assert!(size < 1 << 20, "the bomb is small: {size} bytes");
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        let out = base.join("overlay");
        let error = build_overlay(&vanilla, &packs, &out).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge, "{error}");
        assert!(
            !out.exists(),
            "what was unpacked before the budget ran out stays"
        );
        // A file within the budget unpacks whole.
        fs::remove_file(base.join("packs/bomb.zip")).unwrap();
        pack("fine.zip", &[("assets/minecraft/textures/block/a.png", 8)]);
        let packs = discover(&vanilla, &base.join("packs"));
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::metadata(out.join("assets/minecraft/textures/block/a.png"))
                .unwrap()
                .len(),
            8 << 20
        );
        let _ = fs::remove_dir_all(&base);
    }
    #[test]
    fn zip_entries_never_leave_the_overlay() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("horizon-pack-escape-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"vanilla",
        )
        .unwrap();
        fs::create_dir_all(base.join("packs")).unwrap();
        let mut zip = zip::ZipWriter::new(fs::File::create(base.join("packs/evil.zip")).unwrap());
        for (name, data) in [
            (
                "pack.mcmeta",
                &br#"{"pack":{"description":"d","pack_format":97}}"#[..],
            ),
            ("assets/minecraft/textures/block/stone.png", b"pack"),
            ("assets/../../escaped.txt", b"out"),
            ("assets/minecraft/../../../escaped_lang.json", b"{}"),
            ("assets\\..\\..\\escaped_backslash.txt", b"out"),
            ("assets/./minecraft/dot.txt", b"in"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        assert_eq!(packs.len(), 1);
        let out = base.join("work/overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"pack"
        );
        for escaped in [
            "escaped.txt",
            "escaped_lang.json",
            "escaped_backslash.txt",
            "work/escaped.txt",
            "work/escaped_lang.json",
            "work/escaped_backslash.txt",
        ] {
            assert!(!base.join(escaped).exists(), "{escaped} was written");
        }
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn built_in_and_folder_packs_give_their_pack_png() {
        use std::io::Write as _;
        let base = std::env::temp_dir().join(format!("horizon-pack-icons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let meta = br#"{"pack":{"description":"d","pack_format":97}}"#;
        let built_in = base.join("vanilla/assets/minecraft/resourcepacks");
        fs::create_dir_all(&built_in).unwrap();
        let mut zip =
            zip::ZipWriter::new(fs::File::create(built_in.join("programmer_art.zip")).unwrap());
        for (name, data) in [("pack.mcmeta", &meta[..]), ("pack.png", b"dirt")] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
        for (name, icon) in [("Iconless", None), ("Mine", Some(&b"mine"[..]))] {
            let dir = base.join("packs").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("pack.mcmeta"), meta).unwrap();
            if let Some(icon) = icon {
                fs::write(dir.join("pack.png"), icon).unwrap();
            }
        }
        let packs = discover(&base.join("vanilla"), &base.join("packs"));
        let icons: Vec<_> = packs
            .iter()
            .map(|p| (p.id.as_str(), p.icon_png()))
            .collect();
        assert_eq!(
            icons,
            [
                ("programmer_art", Some(b"dirt".to_vec())),
                ("file/Iconless", None),
                ("file/Mine", Some(b"mine".to_vec())),
            ]
        );
        let _ = fs::remove_dir_all(base);
    }
    #[test]
    fn polytone_conditions_turn_overlays_on_from_the_packs_configs() {
        let dir =
            std::env::temp_dir().join(format!("horizon-packs-polytone-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let configs = dir.join("assets/mypack/polytone/config_entries");
        fs::create_dir_all(&configs).unwrap();
        fs::write(configs.join("fog.json"), r#"{"default_value": "ON"}"#).unwrap();
        fs::write(
            configs.join("birds.json"),
            r#"{"default_value": 0, "max": 300}"#,
        )
        .unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            r#"{"pack":{"description":"p","pack_format":97},"overlays":{"entries":[
                {"directory":"opt_fog","formats":[0,0],"polytone_condition":"config('mypack:fog') == 'ON'"},
                {"directory":"opt_birds","formats":[0,0],"polytone_condition":"config('mypack:birds') > 0"},
                {"directory":"opt_plain","formats":[0,0]}]}}"#,
        )
        .unwrap();
        let pack = pack(
            "file/p".into(),
            Text::literal("p"),
            Source::Dir(dir.clone()),
        )
        .unwrap();
        assert_eq!(pack.overlays, ["opt_fog"]);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn opened_resources_supply_base_polytone_configs() {
        use horizon_assets::resource_source::{Resource, ResourceSource};
        let source = Source::Opened(ResourceSource::indexed(BTreeMap::from([
            (
                "assets/example/polytone/config_entries/fog.json".into(),
                Resource::bytes(br#"{"default_value":"ON"}"#.to_vec()),
            ),
            (
                "overlay/assets/example/polytone/config_entries/fog.json".into(),
                Resource::bytes(br#"{"default_value":"OFF"}"#.to_vec()),
            ),
        ])));
        let layer = Pack::mod_layer("example".into(), source);
        let meta = serde_json::json!({"overlays":{"entries":[{
            "directory":"fog", "formats":[0,0],
            "polytone_condition":"config('example:fog') == 'ON'"
        }]}});
        assert_eq!(overlays_of(&meta, &layer.polytone_configs), ["fog"]);
    }

    #[test]
    fn overlays_and_filters() {
        let v = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":[90,96],"directory":"old"},
            {"formats":{"min_inclusive":97,"max_inclusive":97},"directory":"new"},
            {"min_format":97,"max_format":[97,5],"directory":"also"},
            {"formats":50,"directory":"no"}]}}"#);
        assert_eq!(overlays_of(&meta, &Default::default()), ["new", "also"]);
        // Only one plain folder of the pack, as vanilla's DIR_VALIDATOR has it.
        let meta = v(r#"{"overlays":{"entries":[
            {"formats":97,"directory":".."},
            {"formats":97,"directory":"../outside"},
            {"formats":97,"directory":"/etc"},
            {"formats":97,"directory":"a/b"},
            {"formats":97,"directory":"C:"},
            {"formats":97,"directory":"ok_1.21-x"}]}}"#);
        assert_eq!(overlays_of(&meta, &Default::default()), ["ok_1.21-x"]);
        let base = std::env::temp_dir().join(format!("horizon-packs-ov-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let vanilla = base.join("vanilla");
        fs::create_dir_all(vanilla.join("assets/minecraft/textures/block")).unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/stone.png"),
            b"v",
        )
        .unwrap();
        fs::write(
            vanilla.join("assets/minecraft/textures/block/dirt.png"),
            b"v",
        )
        .unwrap();
        let dir = base.join("packs/P");
        fs::create_dir_all(dir.join("new/assets/minecraft/textures/block")).unwrap();
        fs::create_dir_all(dir.join("old/assets/minecraft/textures/block")).unwrap();
        fs::write(
            dir.join("pack.mcmeta"),
            br#"{"pack":{"description":"p","pack_format":97},
            "overlays":{"entries":[{"formats":97,"directory":"new"},{"formats":1,"directory":"old"}]},
            "filter":{"block":[{"path":"textures/block/dirt\\.png"}]}}"#,
        )
        .unwrap();
        fs::write(
            dir.join("new/assets/minecraft/textures/block/stone.png"),
            b"overlay",
        )
        .unwrap();
        fs::write(
            dir.join("old/assets/minecraft/textures/block/stone.png"),
            b"old",
        )
        .unwrap();
        let packs = discover(&vanilla, &base.join("packs"));
        let out = base.join("overlay");
        build_overlay(&vanilla, &packs, &out).unwrap();
        assert_eq!(
            fs::read(out.join("assets/minecraft/textures/block/stone.png")).unwrap(),
            b"overlay"
        );
        assert!(
            !out.join("assets/minecraft/textures/block/dirt.png")
                .exists()
        );
        let _ = fs::remove_dir_all(base);
    }
}
