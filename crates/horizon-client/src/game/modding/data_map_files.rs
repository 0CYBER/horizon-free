//! Decode a whole data-map file before applying any replace, values or removals.
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, value::RawValue};
use std::collections::{BTreeMap, HashMap, HashSet};

type Tags = HashMap<String, HashSet<String>>;

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    values: BTreeMap<String, Value>,
    #[serde(default)]
    remove: Vec<Removal>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Removal {
    Id(String),
    Object { key: String },
}

enum Key {
    Entry(String),
    Tag(String),
}

/// A resource location decoded at the file/value boundary.
pub(super) fn resource_id(raw: &str) -> Result<String, String> {
    horizon_core::identifier::Identifier::parse(raw)
        .map(|id| id.to_string())
        .map_err(|error| format!("invalid resource location {raw:?}: {error}"))
}

impl Key {
    fn decode(raw: &str) -> Result<Self, String> {
        match raw.strip_prefix('#') {
            Some(tag) => resource_id(tag).map(Self::Tag),
            None => resource_id(raw).map(Self::Entry),
        }
    }
    fn visit(&self, tags: &Tags, mut visit: impl FnMut(&str)) {
        match self {
            Self::Entry(id) => visit(id),
            Self::Tag(tag) => {
                if let Some(members) = tags.get(tag) {
                    for id in members {
                        visit(id);
                    }
                }
            }
        }
    }
}

/// A validated file: callers cannot apply a half-decoded file.
pub(super) struct File<T> {
    replace: bool,
    values: Vec<(Key, T)>,
    remove: Vec<Key>,
}

impl<T: Clone> File<T> {
    pub(super) fn apply(
        self,
        map: &mut BTreeMap<String, T>,
        tags: &Tags,
        known: impl Fn(&str) -> bool,
        context: &str,
    ) {
        if self.replace {
            map.clear();
        }
        for (key, value) in self.values {
            key.visit(tags, |id| {
                if known(id) {
                    map.insert(id.to_owned(), value.clone());
                } else {
                    log::warn!("Horizon: {context}: unknown target {id}, entry skipped");
                }
            });
        }
        for key in self.remove {
            key.visit(tags, |id| {
                map.remove(id);
            });
        }
    }
}

pub(super) fn entry_value(raw: &Value) -> Result<&Value, String> {
    if let Some(object) = raw.as_object()
        && let Some(value) = object.get("value")
    {
        if let Some(replace) = object.get("replace")
            && !replace.is_boolean()
        {
            return Err("entry replace must be a boolean".into());
        }
        return Ok(value);
    }
    Ok(raw)
}

fn decode<T>(
    file: Value,
    conditions: &horizon_world::conditions::LoadConditions,
    context: &str,
    value: impl Fn(&Value) -> Result<Option<T>, String>,
) -> Result<File<T>, String> {
    let raw: RawFile = serde_json::from_value(file).map_err(|error| error.to_string())?;
    let mut values = Vec::new();
    for (key, raw) in raw.values {
        let key = Key::decode(&key)?;
        if !conditions.allows_file(&raw, context) {
            continue;
        }
        if let Some(value) = value(&raw)? {
            values.push((key, value));
        }
    }
    let remove = raw
        .remove
        .into_iter()
        .map(|removal| {
            let key = match removal {
                Removal::Id(key) | Removal::Object { key } => key,
            };
            Key::decode(&key)
        })
        .collect::<Result<_, _>>()?;
    Ok(File {
        replace: raw.replace,
        values,
        remove,
    })
}

/// Pack-order loading shared by raw mod maps and typed builtin codecs. Each
/// value decodes before target membership, so even an unknown target cannot
/// conceal a bad value and allow the rest of that file to apply.
fn merge_versions<T: Clone>(
    id: &str,
    directory: &str,
    mut map: BTreeMap<String, T>,
    overlay: &horizon_world::loot::PackOverlay<'_>,
    tags: &Tags,
    prepare: impl Fn(&[u8], &str) -> Result<Option<File<T>>, String>,
    known: impl Fn(&str) -> bool,
) -> (BTreeMap<String, T>, bool) {
    let (namespace, path) = id.split_once(':').expect("validated data-map type ID");
    let wanted = format!("data_maps/{namespace}:{directory}/{path}");
    let mut applied = false;
    if let Some((_, versions)) = overlay.files.iter().find(|(key, _)| *key == wanted) {
        for bytes in versions {
            let prepared = prepare(bytes, &wanted);
            match prepared {
                Ok(Some(file)) => {
                    file.apply(&mut map, tags, &known, &wanted);
                    applied = true;
                }
                Ok(None) => {}
                Err(error) => log::warn!("Horizon: {id} ({directory}): file skipped: {error}"),
            }
        }
    }
    (map, applied)
}

pub(super) fn load<T: Clone>(
    id: &str,
    directory: &str,
    map: BTreeMap<String, T>,
    overlay: &horizon_world::loot::PackOverlay<'_>,
    tags: &Tags,
    value: impl Fn(&Value) -> Result<Option<T>, String>,
    known: impl Fn(&str) -> bool,
) -> (BTreeMap<String, T>, bool) {
    merge_versions(
        id,
        directory,
        map,
        overlay,
        tags,
        |bytes, context| {
            let file = serde_json::from_slice::<Value>(bytes).map_err(|error| error.to_string())?;
            if !overlay.conditions.allows_file(&file, context) {
                return Ok(None);
            }
            decode(file, overlay.conditions, context, &value).map(Some)
        },
        known,
    )
}

type RawObject = BTreeMap<String, Box<RawValue>>;
pub(super) struct NumberCodec<F> {
    pub field: &'static str,
    pub decode: F,
}
fn read_field<T: DeserializeOwned + Default>(
    object: &mut RawObject,
    field: &str,
) -> Result<T, String> {
    object
        .remove(field)
        .map(|raw| serde_json::from_str(raw.get()).map_err(|error| error.to_string()))
        .transpose()
        .map(|value| value.unwrap_or_default())
}
fn allows_raw(
    object: &mut RawObject,
    conditions: &horizon_world::conditions::LoadConditions,
    context: &str,
) -> Result<bool, String> {
    let Some(raw) = object.remove("neoforge:conditions") else {
        return Ok(true);
    };
    let value: Value = serde_json::from_str(raw.get()).map_err(|error| error.to_string())?;
    Ok(conditions.allows_file(&serde_json::json!({"neoforge:conditions": value}), context))
}
fn scalar(raw: Box<RawValue>, field: &str) -> Result<Box<RawValue>, String> {
    if !raw.get().trim_start().starts_with('{') {
        return Ok(raw);
    }
    let mut object: RawObject =
        serde_json::from_str(raw.get()).map_err(|error| error.to_string())?;
    object
        .remove(field)
        .ok_or_else(|| format!("missing {field}"))
}
/// Only the numeric codecs use RawValue. Their decimal text never takes an
/// f64 round trip, including ignored extra fields and conditional entries.
pub(super) fn load_numeric<T: Clone>(
    id: &str,
    directory: &str,
    map: BTreeMap<String, T>,
    overlay: &horizon_world::loot::PackOverlay<'_>,
    tags: &Tags,
    codec: NumberCodec<impl Fn(&RawValue) -> Result<T, String>>,
    known: impl Fn(&str) -> bool,
) -> (BTreeMap<String, T>, bool) {
    merge_versions(
        id,
        directory,
        map,
        overlay,
        tags,
        |bytes, context| {
            let mut object: RawObject =
                serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
            if !allows_raw(&mut object, overlay.conditions, context)? {
                return Ok(None);
            }
            let replace = read_field(&mut object, "replace")?;
            let entries: RawObject = read_field(&mut object, "values")?;
            let removals: Vec<Removal> = read_field(&mut object, "remove")?;
            let mut values = Vec::new();
            for (key, raw) in entries {
                let key = Key::decode(&key)?;
                let payload = if raw.get().trim_start().starts_with('{') {
                    let mut entry: RawObject =
                        serde_json::from_str(raw.get()).map_err(|error| error.to_string())?;
                    if !allows_raw(&mut entry, overlay.conditions, context)? {
                        continue;
                    }
                    if let Some(value) = entry.remove("value") {
                        let _: bool = read_field(&mut entry, "replace")?;
                        scalar(value, codec.field)?
                    } else {
                        entry
                            .remove(codec.field)
                            .ok_or_else(|| format!("missing {}", codec.field))?
                    }
                } else {
                    raw
                };
                values.push((key, (codec.decode)(&payload)?));
            }
            let remove = removals
                .into_iter()
                .map(|removal| {
                    let key = match removal {
                        Removal::Id(key) | Removal::Object { key } => key,
                    };
                    Key::decode(&key)
                })
                .collect::<Result<_, _>>()?;
            Ok(Some(File {
                replace,
                values,
                remove,
            }))
        },
        known,
    )
}
