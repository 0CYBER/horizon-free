//! Datapack registries a native mod declares: a registry
//! of the mod's own, whose entries are JSON files in data packs. The mod
//! declares a flat record schema once at startup; the game decodes every
//! entry against it, so a decoder never runs mod code and a native and a wasm
//! mod see the same bounds. See `docs/design/mod-api/mod-datapack-registries.md`.
use horizon_core::identifier::Identifier;
use serde_json::Value;

/// How a registry's entries reach joining clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DatapackRegistrySync {
    /// Entries stay on the server; a client needs nothing of the mod for them.
    #[default]
    ServerOnly,
    /// Clients receive the entries, and a client without the mod is refused.
    Required,
    /// Clients with the mod receive the entries; others join without them.
    Optional,
}

/// What a schema field holds, with the bounds the game enforces.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DatapackFieldKind {
    /// `true` or `false`.
    Bool,
    /// An integral JSON number in `[min, max]`.
    Int {
        /// The lowest accepted value.
        min: i64,
        /// The highest accepted value.
        max: i64,
    },
    /// A finite JSON number in `[min, max]`; an integral number is accepted.
    Float {
        /// The lowest accepted value.
        min: f64,
        /// The highest accepted value.
        max: f64,
    },
    /// A string of at most `max_len` bytes.
    Text {
        /// The most bytes accepted.
        max_len: u32,
    },
    /// A resource id (`minecraft:` when it names no namespace).
    Identifier,
    /// An id naming an entry of the registry `registry`; the game resolves it
    /// against the whole candidate set, so it may name an entry no pack
    /// has defined yet.
    Reference {
        /// The registry the id names an entry of (`example:spell`).
        registry: String,
    },
    /// A JSON array of at most `max_len` values, each of the `element` kind.
    List {
        /// The kind every element must have.
        element: Box<DatapackFieldKind>,
        /// The most elements accepted.
        max_len: u32,
    },
    /// A JSON object with the given field schema, of at most `max_len` bytes.
    Object {
        /// The fields the object accepts, each named once.
        fields: Vec<DatapackField>,
        /// The most bytes of the decoded value's compact JSON accepted, not
        /// of the source file (ids are normalized before this is measured).
        max_len: u32,
    },
}

/// A decoded field value. Ids and references are `Text`, normalised to
/// `namespace:path`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DatapackValue {
    /// A [`DatapackFieldKind::Bool`].
    Bool(bool),
    /// A [`DatapackFieldKind::Int`].
    Int(i64),
    /// A [`DatapackFieldKind::Float`].
    Float(f64),
    /// A [`DatapackFieldKind::Text`], [`DatapackFieldKind::Identifier`] or
    /// [`DatapackFieldKind::Reference`].
    Text(String),
    /// A [`DatapackFieldKind::List`]: the elements in order.
    List(Vec<DatapackValue>),
    /// A [`DatapackFieldKind::Object`]: `(field name, value)` in schema order.
    Object(Vec<(String, DatapackValue)>),
}

/// One field of a registry's entry record.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: a field is its name, kind and default; a mod builds it directly
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackField {
    /// The JSON key (`power`).
    pub name: String,
    /// What the field holds.
    pub kind: DatapackFieldKind,
    /// The value of an absent field; `None` makes the field required.
    pub default: Option<DatapackValue>,
}

impl DatapackField {
    /// A required field.
    pub fn required(name: &str, kind: DatapackFieldKind) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            default: None,
        }
    }

    /// A field that is `default` when an entry leaves it out.
    pub fn with_default(name: &str, kind: DatapackFieldKind, default: DatapackValue) -> Self {
        Self {
            name: name.to_owned(),
            kind,
            default: Some(default),
        }
    }
}

/// Why a mod's datapack registry declaration was refused
/// ([`Registrar::declare_datapack_registry`](crate::DatapackRegistriesRegistrar::declare_datapack_registry)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DatapackRegistrationError {
    /// Only both-side packages may declare datapack registries.
    WrongSide,
    /// The registry's key is malformed, empty or in a namespace the
    /// declaring mod does not own (a vanilla registry's included).
    InvalidKey,
    /// Another declaration already covers the registry key.
    Duplicate,
    /// A field name is empty or repeated.
    InvalidFieldName,
    /// A field's kind is unusable: bounds that are not finite or `min > max`,
    /// a `max_len` of 0, or a reference to a malformed registry key.
    InvalidFieldKind,
    /// A default is not a value of its field's kind, or is out of its bounds.
    InvalidDefault,
}

/// A registry the mod declared, as the game reads it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct DatapackRegistryDeclaration {
    /// The declaring mod's id.
    pub mod_id: String,
    /// The registry's namespaced key (`mymod:spell`).
    pub key: String,
    /// The entry record, in the order the decoded entry lists it.
    pub fields: Vec<DatapackField>,
    /// How clients receive the entries.
    pub sync: DatapackRegistrySync,
}

/// Why an entry file did not decode. `field` names the offending JSON key.
#[derive(Clone, Debug, PartialEq, Eq)]
// no wire: decoding runs in the game; mods read decoded values, never failures
#[non_exhaustive]
pub enum DatapackDecodeError {
    /// The file is not JSON.
    Malformed(String),
    /// The JSON is not an object.
    NotAnObject,
    /// A required field is absent.
    MissingField {
        /// The absent key.
        field: String,
    },
    /// A key the schema does not declare.
    UnknownField {
        /// The undeclared key.
        field: String,
    },
    /// A value of the wrong JSON type.
    WrongType {
        /// The key.
        field: String,
        /// What the schema asked for (`integer`, `string`...).
        expected: &'static str,
    },
    /// A number outside the field's bounds.
    OutOfRange {
        /// The key.
        field: String,
    },
    /// A string over the field's `max_len`.
    TooLong {
        /// The key.
        field: String,
    },
    /// An id that does not parse.
    InvalidIdentifier {
        /// The key.
        field: String,
    },
}

/// A decoded entry: one value per schema field, in the schema's order.
#[derive(Clone, Debug, PartialEq)]
// no wire: the game's decoded candidate; a mod reads it through registry handles
// exhaustive: the game builds it with a literal; mods only read it
pub struct DatapackEntry {
    /// `(field name, value)` in schema order.
    pub values: Vec<(String, DatapackValue)>,
}

impl DatapackEntry {
    /// The value of the field `name`.
    pub fn get(&self, name: &str) -> Option<&DatapackValue> {
        self.values
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value)
    }
}

/// Whether `text` is a lowercase `namespace:path` id the way registrations name theirs.
pub(crate) fn valid_namespaced(text: &str) -> bool {
    let Some((namespace, path)) = text.split_once(':') else {
        return false;
    };
    let valid = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c);
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.bytes().all(valid)
        && path.bytes().all(|c| valid(c) || c == b'/')
}

fn finite_bounds(min: f64, max: f64) -> bool {
    min.is_finite() && max.is_finite() && min <= max
}

/// How deep a nested field schema may go. The JSON parser has its own limit;
/// this bounds the schema a mod declares at registration instead.
const MAX_FIELD_DEPTH: u8 = 8;
/// How many fields one object schema may declare.
const MAX_OBJECT_FIELDS: usize = 64;

impl DatapackFieldKind {
    /// Whether the kind itself is usable at this nesting depth.
    fn check_at(&self, depth: u8) -> bool {
        if depth > MAX_FIELD_DEPTH {
            return false;
        }
        match self {
            Self::Bool | Self::Identifier => true,
            Self::Int { min, max } => min <= max,
            Self::Float { min, max } => finite_bounds(*min, *max),
            Self::Text { max_len } => *max_len > 0,
            Self::Reference { registry } => valid_namespaced(registry),
            Self::List { element, max_len } => *max_len > 0 && element.check_at(depth + 1),
            Self::Object { fields, max_len } => {
                *max_len > 0
                    && !fields.is_empty()
                    && fields.len() <= MAX_OBJECT_FIELDS
                    && check_schema_at(fields, depth + 1).is_ok()
            }
        }
    }

    /// Whether `value` is a value of this kind inside its bounds.
    pub(super) fn accepts(&self, value: &DatapackValue) -> bool {
        match (self, value) {
            (Self::Bool, DatapackValue::Bool(_)) => true,
            (Self::Int { min, max }, DatapackValue::Int(v)) => (min..=max).contains(&v),
            (Self::Float { min, max }, DatapackValue::Float(v)) => {
                v.is_finite() && (min..=max).contains(&v)
            }
            (Self::Text { max_len }, DatapackValue::Text(v)) => v.len() <= *max_len as usize,
            (Self::Identifier | Self::Reference { .. }, DatapackValue::Text(v)) => {
                Identifier::parse(v).is_ok_and(|id| id.to_string() == *v)
            }
            (Self::List { element, max_len }, DatapackValue::List(values)) => {
                values.len() <= *max_len as usize && values.iter().all(|value| element.accepts(value))
            }
            // A decoded object holds every field in schema order, so that is the only form
            // accepted: a partial or reordered default would differ from the explicit `{}`.
            (Self::Object { fields, max_len }, DatapackValue::Object(values)) => {
                values.len() == fields.len()
                    && fields
                        .iter()
                        .zip(values)
                        .all(|(field, (name, value))| {
                            field.name == *name && field.kind.accepts(value)
                        })
                    && object_json_len(values) <= *max_len as usize
            }
            _ => false,
        }
    }

    fn expected(&self) -> &'static str {
        match self {
            Self::Bool => "boolean",
            Self::Int { .. } => "integer",
            Self::Float { .. } => "number",
            Self::Text { .. } => "string",
            Self::Identifier | Self::Reference { .. } => "identifier string",
            Self::List { .. } => "array",
            Self::Object { .. } => "object",
        }
    }
}

fn to_json(value: &DatapackValue) -> Value {
    match value {
        DatapackValue::Bool(v) => Value::Bool(*v),
        DatapackValue::Int(v) => Value::Number((*v).into()),
        DatapackValue::Float(v) => serde_json::Number::from_f64(*v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DatapackValue::Text(v) => Value::String(v.clone()),
        DatapackValue::List(values) => Value::Array(values.iter().map(to_json).collect()),
        DatapackValue::Object(values) => Value::Object(
            values
                .iter()
                .map(|(name, value)| (name.clone(), to_json(value)))
                .collect(),
        ),
    }
}

/// The compact JSON length of a decoded value: the one measure used by both
/// the decoder and schema validation, so a default that passes registration
/// decodes too. This is the length of the decoded value, not of the source
/// file: an id such as `stone` is normalized to `minecraft:stone` first.
fn json_len(value: &DatapackValue) -> usize {
    serde_json::to_string(&to_json(value))
        .map(|text| {
            text.len()
        })
        .unwrap_or(usize::MAX)
}

/// The same measure for the fields of an object, without cloning the value
/// tree the caller still owns.
fn object_json_len(values: &[(String, DatapackValue)]) -> usize {
    let object: serde_json::Map<String, Value> = values
        .iter()
        .map(|(name, value)| (name.clone(), to_json(value)))
        .collect();
    serde_json::to_string(&Value::Object(object))
        .map(|text| {
            text.len()
        })
        .unwrap_or(usize::MAX)
}

/// Canonicalize shorthand ids in every default of a schema, including fields
/// nested in `List` and `Object` kinds. A top-level `stone` and a nested
/// `stone` must be treated the same way.
pub fn canonicalize_datapack_defaults(
    fields: &mut [DatapackField],
) -> Result<(), DatapackRegistrationError> {
    for field in fields.iter_mut() {
        // Children first: an object default takes omitted fields from them.
        canonicalize_child_defaults(&mut field.kind)?;
        if let Some(value) = field.default.as_mut() {
            canonicalize_value(&field.kind, value)?;
        }
    }
    Ok(())
}

fn canonicalize_child_defaults(kind: &mut DatapackFieldKind) -> Result<(), DatapackRegistrationError> {
    match kind {
        DatapackFieldKind::List { element, .. } => canonicalize_child_defaults(element),
        DatapackFieldKind::Object { fields, .. } => canonicalize_datapack_defaults(fields),
        _ => Ok(()),
    }
}

fn canonicalize_value(
    kind: &DatapackFieldKind,
    value: &mut DatapackValue,
) -> Result<(), DatapackRegistrationError> {
    match (kind, value) {
        (
            DatapackFieldKind::Identifier | DatapackFieldKind::Reference { .. },
            DatapackValue::Text(id),
        ) => {
            *id = Identifier::parse(id)
                .map_err(|_| DatapackRegistrationError::InvalidDefault)?
                .to_string();
            Ok(())
        }
        (DatapackFieldKind::List { element, .. }, DatapackValue::List(values)) => {
            for value in values {
                canonicalize_value(element, value)?;
            }
            Ok(())
        }
        (DatapackFieldKind::Object { fields, .. }, DatapackValue::Object(values)) => {
            // Materialize the default as the decoder would build it from the same object:
            // every field in schema order, an omitted one from its (already canonical) default.
            // An unknown or repeated name, or an omitted required field, is a bad default.
            if values
                .iter()
                .enumerate()
                .any(|(index, (name, _))| {
                    !fields.iter().any(|field| field.name == *name)
                        || values[..index].iter().any(|(seen, _)| seen == name)
                })
            {
                return Err(DatapackRegistrationError::InvalidDefault);
            }
            let mut given = std::mem::take(values);
            for field in fields {
                let value = match given.iter().position(|(name, _)| *name == field.name) {
                    Some(index) => {
                        let (_, mut value) = given.swap_remove(index);
                        canonicalize_value(&field.kind, &mut value)?;
                        value
                    }
                    None => field
                        .default
                        .clone()
                        .ok_or(DatapackRegistrationError::InvalidDefault)?,
                };
                values.push((field.name.clone(), value));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Checks a schema at registration: field count and names, kinds and defaults.
pub(crate) fn check_schema(fields: &[DatapackField]) -> Result<(), DatapackRegistrationError> {
    check_schema_at(fields, 0)
}

fn check_schema_at(
    fields: &[DatapackField],
    depth: u8,
) -> Result<(), DatapackRegistrationError> {
    if depth > MAX_FIELD_DEPTH {
        return Err(DatapackRegistrationError::InvalidFieldKind);
    }
    for (index, field) in fields.iter().enumerate() {
        if field.name.is_empty() || fields[..index].iter().any(|seen| seen.name == field.name) {
            return Err(DatapackRegistrationError::InvalidFieldName);
        }
        if !field.kind.check_at(depth) {
            return Err(DatapackRegistrationError::InvalidFieldKind);
        }
        if field
            .default
            .as_ref()
            .is_some_and(|default| !field.kind.accepts(default))
        {
            return Err(DatapackRegistrationError::InvalidDefault);
        }
    }
    Ok(())
}

impl DatapackRegistryDeclaration {
    /// The file an entry `entry` (`addon:fire`) of this registry (`example:spell`)
    /// sits in, inside a data pack: `data/addon/example/spell/fire.json`.
    pub fn entry_file(&self, entry: &str) -> Option<String> {
        let (entry_namespace, entry_path) = entry.split_once(':')?;
        let (registry_namespace, registry_path) = self.key.split_once(':')?;
        Some(format!(
            "data/{entry_namespace}/{registry_namespace}/{registry_path}/{entry_path}.json"
        ))
    }

    /// Decodes one entry file against the schema. A key the schema does not
    /// declare, a missing required key, a wrong type or an out-of-range value
    /// fails the whole entry, and no default stands in for invalid data.
    /// Nothing here runs mod code. Nesting is bounded by the JSON parser's
    /// recursion limit; the file's size is bounded before this, by
    /// [`DatapackCandidate::build`](super::datapack_registry_set::DatapackCandidate::build).
    pub fn decode_entry(&self, text: &str) -> Result<DatapackEntry, DatapackDecodeError> {
        // serde_json's own recursion limit (128) bounds nesting before any value is built.
        let json: Value = serde_json::from_str(text)
            .map_err(|error| DatapackDecodeError::Malformed(error.to_string()))?;
        let Value::Object(object) = json else {
            return Err(DatapackDecodeError::NotAnObject);
        };
        if let Some(unknown) = object
            .keys()
            .find(|key| !self.fields.iter().any(|field| &field.name == *key))
        {
            return Err(DatapackDecodeError::UnknownField {
                field: unknown.clone(),
            });
        }
        let mut values = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            let value = match object.get(&field.name) {
                Some(json) => decode_value(field, json)?,
                None => field
                    .default
                    .clone()
                    .ok_or_else(|| DatapackDecodeError::MissingField {
                        field: field.name.clone(),
                    })?,
            };
            values.push((field.name.clone(), value));
        }
        Ok(DatapackEntry { values })
    }
}

fn decode_value(field: &DatapackField, json: &Value) -> Result<DatapackValue, DatapackDecodeError> {
    decode_kind(&field.kind, json, &field.name)
}

/// Decode one JSON value against a kind. `path` is the field a failure names,
/// with child keys and array indexes appended for nested values.
fn decode_kind(
    kind: &DatapackFieldKind,
    json: &Value,
    path: &str,
) -> Result<DatapackValue, DatapackDecodeError> {
    let wrong = || DatapackDecodeError::WrongType {
        field: path.to_owned(),
        expected: kind.expected(),
    };
    let out_of_range = || DatapackDecodeError::OutOfRange {
        field: path.to_owned(),
    };
    let too_long = || DatapackDecodeError::TooLong {
        field: path.to_owned(),
    };
    match kind {
        DatapackFieldKind::Bool => json.as_bool().map(DatapackValue::Bool).ok_or_else(wrong),
        DatapackFieldKind::Int { min, max } => {
            // `as_i64` refuses 3.0 and anything past i64: vanilla's integer codec does too.
            let v = json.as_i64().ok_or_else(wrong)?;
            if (min..=max).contains(&&v) {
                Ok(DatapackValue::Int(v))
            } else {
                Err(out_of_range())
            }
        }
        DatapackFieldKind::Float { min, max } => {
            let v = json.as_f64().filter(|v| v.is_finite()).ok_or_else(wrong)?;
            if (min..=max).contains(&&v) {
                Ok(DatapackValue::Float(v))
            } else {
                Err(out_of_range())
            }
        }
        DatapackFieldKind::Text { max_len } => {
            let v = json.as_str().ok_or_else(wrong)?;
            if v.len() > *max_len as usize {
                Err(too_long())
            } else {
                Ok(DatapackValue::Text(v.to_owned()))
            }
        }
        DatapackFieldKind::Identifier | DatapackFieldKind::Reference { .. } => {
            let v = json.as_str().ok_or_else(wrong)?;
            Identifier::parse(v)
                .map(|id| DatapackValue::Text(id.to_string()))
                .map_err(|_| DatapackDecodeError::InvalidIdentifier {
                    field: path.to_owned(),
                })
        }
        DatapackFieldKind::List { element, max_len } => {
            let array = json.as_array().ok_or_else(wrong)?;
            if array.len() > *max_len as usize {
                return Err(too_long());
            }
            let mut values = Vec::with_capacity(array.len());
            for (index, item) in array.iter().enumerate() {
                values.push(decode_kind(element, item, &format!("{path}[{index}]"))?);
            }
            Ok(DatapackValue::List(values))
        }
        DatapackFieldKind::Object { fields, max_len } => {
            let object = json.as_object().ok_or_else(wrong)?;
            if let Some(unknown) = object
                .keys()
                .find(|key| !fields.iter().any(|field| { field.name == **key }))
            {
                return Err(DatapackDecodeError::UnknownField {
                    field: format!("{path}.{unknown}"),
                });
            }
            let mut values = Vec::with_capacity(fields.len());
            for field in fields {
                let child_path = format!("{path}.{}", field.name);
                let value = match object.get(&field.name) {
                    Some(json) => decode_kind(&field.kind, json, &child_path)?,
                    None => field
                        .default
                        .clone()
                        .ok_or(DatapackDecodeError::MissingField { field: child_path })?,
                };
                values.push((field.name.clone(), value));
            }
            let value = DatapackValue::Object(values);
            if json_len(&value) > *max_len as usize {
                return Err(too_long());
            }
            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `example:spell`: `{power: integer 0..100, element: id, chain: reference, silent: bool = false}`.
    fn spell() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: vec![
                DatapackField::required("power", DatapackFieldKind::Int { min: 0, max: 100 }),
                DatapackField::with_default(
                    "element",
                    DatapackFieldKind::Identifier,
                    DatapackValue::Text("example:fire".into()),
                ),
                DatapackField::with_default(
                    "silent",
                    DatapackFieldKind::Bool,
                    DatapackValue::Bool(false),
                ),
                DatapackField::with_default(
                    "next",
                    DatapackFieldKind::Reference {
                        registry: "example:spell".into(),
                    },
                    DatapackValue::Text("example:fire".into()),
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        }
    }

    /// NeoForge's rule for a datapack registry outside `minecraft` (its Registries guide, "Datapack
    /// Registries"): `data/<entry namespace>/<registry namespace>/<registry path>/<entry>.json`.
    /// It is not vanilla's, which uses the registry's path only; registration refuses a
    /// `minecraft` key, so no vanilla-path case exists.
    #[test]
    fn entry_files_follow_neoforges_namespaced_registry_folder() {
        assert_eq!(
            spell().entry_file("addon:fire").as_deref(),
            Some("data/addon/example/spell/fire.json")
        );
        assert_eq!(spell().entry_file("nocolon"), None);
    }

    #[test]
    fn a_record_decodes_in_schema_order_with_defaults() {
        let entry = spell()
            .decode_entry(r#"{"power": 7, "next": "ice"}"#)
            .unwrap();
        assert_eq!(
            entry.values,
            vec![
                ("power".to_owned(), DatapackValue::Int(7)),
                (
                    "element".to_owned(),
                    DatapackValue::Text("example:fire".into())
                ),
                ("silent".to_owned(), DatapackValue::Bool(false)),
                // An id with no namespace is vanilla's, as everywhere else.
                (
                    "next".to_owned(),
                    DatapackValue::Text("minecraft:ice".into())
                ),
            ]
        );
        assert_eq!(entry.get("power"), Some(&DatapackValue::Int(7)));
    }

    #[test]
    fn invalid_data_fails_the_entry_and_names_the_key() {
        let decl = spell();
        let err = |text: &str| decl.decode_entry(text).unwrap_err();
        assert_eq!(
            err(r#"{"power": 101}"#),
            DatapackDecodeError::OutOfRange {
                field: "power".into()
            }
        );
        assert_eq!(
            err(r#"{"power": -1}"#),
            DatapackDecodeError::OutOfRange {
                field: "power".into()
            }
        );
        // 3.0 is a float, not the integer the schema asks for.
        assert_eq!(
            err(r#"{"power": 3.0}"#),
            DatapackDecodeError::WrongType {
                field: "power".into(),
                expected: "integer"
            }
        );
        assert_eq!(
            err(r#"{"power": "7"}"#),
            DatapackDecodeError::WrongType {
                field: "power".into(),
                expected: "integer"
            }
        );
        assert_eq!(
            err(r#"{"silent": true}"#),
            DatapackDecodeError::MissingField {
                field: "power".into()
            }
        );
        assert_eq!(
            err(r#"{"power": 1, "powr": 2}"#),
            DatapackDecodeError::UnknownField {
                field: "powr".into()
            }
        );
        assert_eq!(
            err(r#"{"power": 1, "element": "Not An Id"}"#),
            DatapackDecodeError::InvalidIdentifier {
                field: "element".into()
            }
        );
        // A boolean field takes only `true` or `false`.
        for wrong in [r#""yes""#, "1", "null"] {
            assert_eq!(
                err(&format!(r#"{{"power": 1, "silent": {wrong}}}"#)),
                DatapackDecodeError::WrongType {
                    field: "silent".into(),
                    expected: "boolean"
                }
            );
        }
        assert_eq!(err("[1]"), DatapackDecodeError::NotAnObject);
        assert!(matches!(err("{"), DatapackDecodeError::Malformed(_)));
        // A nested value is never a scalar field.
        assert!(matches!(
            err(r#"{"power": {"a": 1}}"#),
            DatapackDecodeError::WrongType { .. }
        ));
    }

    #[test]
    fn nesting_is_bounded_by_the_json_parser() {
        // Size is the candidate's limit (`datapack_registry_set` tests), not the decoder's.
        let decl = spell();
        let deep = format!("{}1{}", "[".repeat(65), "]".repeat(65));
        assert!(matches!(
            decl.decode_entry(&deep),
            Err(DatapackDecodeError::NotAnObject)
        ));
        let very_deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
        assert!(matches!(
            decl.decode_entry(&very_deep),
            Err(DatapackDecodeError::Malformed(_))
        ));
    }

    #[test]
    fn floats_and_text_obey_their_bounds() {
        let decl = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:aura".into(),
            fields: vec![
                DatapackField::required("scale", DatapackFieldKind::Float { min: 0.0, max: 2.0 }),
                DatapackField::required("name", DatapackFieldKind::Text { max_len: 4 }),
            ],
            sync: DatapackRegistrySync::Optional,
        };
        // An integral number is a fine float.
        assert!(decl.decode_entry(r#"{"scale": 2, "name": "abcd"}"#).is_ok());
        assert_eq!(
            decl.decode_entry(r#"{"scale": 2.5, "name": "a"}"#)
                .unwrap_err(),
            DatapackDecodeError::OutOfRange {
                field: "scale".into()
            }
        );
        // 1e999 is not a JSON number serde_json yields: refused as malformed, never infinity.
        assert!(matches!(
            decl.decode_entry(r#"{"scale": 1e999, "name": "a"}"#),
            Err(DatapackDecodeError::Malformed(_))
        ));
        assert_eq!(
            decl.decode_entry(r#"{"scale": 1, "name": "abcde"}"#)
                .unwrap_err(),
            DatapackDecodeError::TooLong {
                field: "name".into()
            }
        );
    }

    #[test]
    fn schemas_are_checked_at_registration() {
        use DatapackRegistrationError as E;
        let int = || DatapackFieldKind::Int { min: 0, max: 1 };
        assert_eq!(check_schema(&spell().fields), Ok(()));
        let named = |name: &str| vec![DatapackField::required(name, int())];
        assert_eq!(check_schema(&named("")), Err(E::InvalidFieldName));
        // Any other key is a field, as vanilla's codecs take any JSON key.
        assert_eq!(check_schema(&named("Power")), Ok(()));
        assert_eq!(check_schema(&named("a b")), Ok(()));
        assert_eq!(
            check_schema(&[
                DatapackField::required("a", int()),
                DatapackField::required("a", int())
            ]),
            Err(E::InvalidFieldName)
        );
        let many: Vec<_> = (0..200)
            .map(|i| DatapackField::required(&format!("f{i}"), int()))
            .collect();
        assert_eq!(check_schema(&many), Ok(()));
        let kind = |kind| vec![DatapackField::required("a", kind)];
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Int { min: 2, max: 1 })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Float {
                min: 0.0,
                max: f64::NAN
            })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Float {
                min: f64::NEG_INFINITY,
                max: 1.0
            })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Text { max_len: 0 })),
            Err(E::InvalidFieldKind)
        );
        assert_eq!(
            check_schema(&kind(DatapackFieldKind::Reference {
                registry: "Bad Key".into()
            })),
            Err(E::InvalidFieldKind)
        );
        // A default must be a value of the kind, inside its bounds.
        let default = |value| vec![DatapackField::with_default("a", int(), value)];
        assert_eq!(check_schema(&default(DatapackValue::Int(1))), Ok(()));
        assert_eq!(
            check_schema(&default(DatapackValue::Int(2))),
            Err(E::InvalidDefault)
        );
        assert_eq!(
            check_schema(&default(DatapackValue::Bool(true))),
            Err(E::InvalidDefault)
        );
    }

    /// A slot whose definition has a list of ids and an object of display settings.
    fn nested() -> DatapackRegistryDeclaration {
        DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:slot".into(),
            fields: vec![
                DatapackField::required(
                    "tags",
                    DatapackFieldKind::List {
                        element: Box::new(DatapackFieldKind::Identifier),
                        max_len: 3,
                    },
                ),
                DatapackField::required(
                    "display",
                    DatapackFieldKind::Object {
                        fields: vec![
                            DatapackField::required(
                                "label",
                                DatapackFieldKind::Text { max_len: 16 },
                            ),
                            DatapackField::with_default(
                                "glow",
                                DatapackFieldKind::Bool,
                                DatapackValue::Bool(false),
                            ),
                        ],
                        max_len: 256,
                    },
                ),
            ],
            sync: DatapackRegistrySync::ServerOnly,
        }
    }

    #[test]
    fn nested_fields_decode_lists_and_objects() {
        let decl = nested();
        assert_eq!(check_schema(&decl.fields), Ok(()));
        let entry = decl
            .decode_entry(
                r#"{"tags":["curios:ring","example:face"],"display":{"label":"Ring"}}"#,
            )
            .expect("nested fields decode");
        assert_eq!(
            entry.get("tags"),
            Some(&DatapackValue::List(vec![
                DatapackValue::Text("curios:ring".into()),
                DatapackValue::Text("example:face".into()),
            ]))
        );
        assert_eq!(
            entry.get("display"),
            Some(&DatapackValue::Object(vec![
                ("label".into(), DatapackValue::Text("Ring".into())),
                ("glow".into(), DatapackValue::Bool(false)),
            ]))
        );
    }

    #[test]
    fn nested_fields_refuse_bad_children_instead_of_partial_values() {
        let decl = nested();
        assert_eq!(
            decl.decode_entry(r#"{"tags":["Bad Key"],"display":{"label":"R"}}"#)
                .unwrap_err(),
            DatapackDecodeError::InvalidIdentifier {
                field: "tags[0]".into()
            }
        );
        assert_eq!(
            decl.decode_entry(r#"{"tags":[],"display":{"label":"R","extra":1}}"#)
                .unwrap_err(),
            DatapackDecodeError::UnknownField {
                field: "display.extra".into()
            }
        );
        assert_eq!(
            decl.decode_entry(r#"{"tags":[],"display":{}}"#).unwrap_err(),
            DatapackDecodeError::MissingField {
                field: "display.label".into()
            }
        );
        assert_eq!(
            decl.decode_entry(
                r#"{"tags":["a:b","c:d","e:f","g:h"],"display":{"label":"R"}}"#
            )
            .unwrap_err(),
            DatapackDecodeError::TooLong {
                field: "tags".into()
            }
        );
        assert_eq!(
            decl.decode_entry(r#"{"tags":[],"display":"x"}"#).unwrap_err(),
            DatapackDecodeError::WrongType {
                field: "display".into(),
                expected: "object"
            }
        );
    }

    #[test]
    fn nested_schemas_are_bounded_at_registration() {
        let list = |kind| {
            vec![DatapackField::required(
                "a",
                DatapackFieldKind::List {
                    element: Box::new(kind),
                    max_len: 2,
                },
            )]
        };
        assert_eq!(check_schema(&list(DatapackFieldKind::Bool)), Ok(()));
        assert_eq!(
            check_schema(&list(DatapackFieldKind::List {
                element: Box::new(DatapackFieldKind::Bool),
                max_len: 2
            })),
            Ok(())
        );
        let mut too_deep = DatapackFieldKind::Bool;
        for _ in 0..=MAX_FIELD_DEPTH {
            too_deep = DatapackFieldKind::List {
                element: Box::new(too_deep),
                max_len: 2,
            };
        }
        assert_eq!(
            check_schema(&list(too_deep)),
            Err(DatapackRegistrationError::InvalidFieldKind)
        );
        let duplicate = DatapackFieldKind::Object {
            fields: vec![
                DatapackField::required("a", DatapackFieldKind::Bool),
                DatapackField::required("a", DatapackFieldKind::Bool),
            ],
            max_len: 64,
        };
        assert_eq!(
            check_schema(&list(duplicate)),
            Err(DatapackRegistrationError::InvalidFieldKind)
        );
        let bad_default = DatapackFieldKind::Object {
            fields: vec![DatapackField::with_default(
                "a",
                DatapackFieldKind::Int { min: 0, max: 1 },
                DatapackValue::Int(5),
            )],
            max_len: 64,
        };
        // A bad default inside a nested object is reported as a bad nested field kind:
        // the outer kind check only needs to know the object schema is unusable.
        assert_eq!(
            check_schema(&list(bad_default)),
            Err(DatapackRegistrationError::InvalidFieldKind)
        );
    }

    #[test]
    fn containers_decode_inside_each_other_and_object_length_is_exact() {
        let mut decl = nested();
        decl.fields = vec![DatapackField::required(
            "layers",
            DatapackFieldKind::List {
                element: Box::new(DatapackFieldKind::Object {
                    fields: vec![DatapackField::required(
                        "id",
                        DatapackFieldKind::Identifier,
                    )],
                    max_len: 128,
                }),
                max_len: 2,
            },
        )];
        let entry = decl
            .decode_entry(r#"{"layers":[{"id":"curios:ring"},{"id":"curios:charm"}]}"#)
            .expect("list of objects decodes");
        assert_eq!(
            entry.get("layers"),
            Some(&DatapackValue::List(vec![
                DatapackValue::Object(vec![(
                    "id".into(),
                    DatapackValue::Text("curios:ring".into())
                )]),
                DatapackValue::Object(vec![(
                    "id".into(),
                    DatapackValue::Text("curios:charm".into())
                )]),
            ]))
        );

        // max_len is the compact JSON length, the same measure a default is
        // checked with at registration. `{"label":"abcdefgh"}` is 19 bytes.
        decl.fields = vec![DatapackField::required(
            "display",
            DatapackFieldKind::Object {
                fields: vec![DatapackField::required(
                    "label",
                    DatapackFieldKind::Text { max_len: 64 },
                )],
                max_len: 8,
            },
        )];
        assert_eq!(
            decl.decode_entry(r#"{"display":{"label":"abcdefgh"}}"#)
                .unwrap_err(),
            DatapackDecodeError::TooLong {
                field: "display".into()
            }
        );
    }

    #[test]
    fn nested_id_defaults_canonicalize_and_duplicate_names_are_refused() {
        let mut fields = vec![DatapackField::required(
            "choice",
            DatapackFieldKind::Object {
                fields: vec![DatapackField::with_default(
                    "id",
                    DatapackFieldKind::Identifier,
                    DatapackValue::Text("stone".into()),
                )],
                max_len: 256,
            },
        )];
        canonicalize_datapack_defaults(&mut fields).expect("shorthand normalizes");
        assert_eq!(check_schema(&fields), Ok(()));
        let DatapackFieldKind::Object { fields: children, .. } = &fields[0].kind else {
            panic!("object kind");
        };
        assert_eq!(
            children[0].default,
            Some(DatapackValue::Text("minecraft:stone".into()))
        );

        let object = DatapackFieldKind::Object {
            fields: vec![
                DatapackField::required("a", DatapackFieldKind::Bool),
                DatapackField::with_default(
                    "b",
                    DatapackFieldKind::Bool,
                    DatapackValue::Bool(false),
                ),
            ],
            max_len: 256,
        };
        let duplicate = DatapackValue::Object(vec![
            ("a".into(), DatapackValue::Bool(true)),
            ("a".into(), DatapackValue::Text("bad".into())),
        ]);
        assert!(!object.accepts(&duplicate));
    }

    /// A partial, reordered object default is completed at registration into the value an
    /// explicit `{}`-plus-those-fields decodes to; one missing a required field is refused.
    #[test]
    fn object_defaults_materialize_like_a_decoded_object() {
        let display = |default| {
            vec![DatapackField::with_default(
                "display",
                DatapackFieldKind::Object {
                    fields: vec![
                        DatapackField::with_default(
                            "glow",
                            DatapackFieldKind::Bool,
                            DatapackValue::Bool(false),
                        ),
                        DatapackField::with_default(
                            "icon",
                            DatapackFieldKind::Identifier,
                            DatapackValue::Text("minecraft:stone".into()),
                        ),
                        DatapackField::required("label", DatapackFieldKind::Text { max_len: 16 }),
                    ],
                    max_len: 256,
                },
                default,
            )]
        };
        let partial = DatapackValue::Object(vec![
            ("label".into(), DatapackValue::Text("Ring".into())),
            ("glow".into(), DatapackValue::Bool(true)),
        ]);
        // Unmaterialized, it is not a value of the kind.
        assert_eq!(
            check_schema(&display(partial.clone())),
            Err(DatapackRegistrationError::InvalidDefault)
        );
        let mut fields = display(partial);
        canonicalize_datapack_defaults(&mut fields).expect("partial default completes");
        assert_eq!(check_schema(&fields), Ok(()));
        let decl = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:slot".into(),
            fields: fields.clone(),
            sync: DatapackRegistrySync::ServerOnly,
        };
        let explicit = decl
            .decode_entry(r#"{"display":{"label":"Ring","glow":true}}"#)
            .expect("explicit object decodes");
        let omitted = decl.decode_entry("{}").expect("default fills the object");
        assert_eq!(omitted.get("display"), explicit.get("display"));

        let mut missing = display(DatapackValue::Object(vec![(
            "glow".into(),
            DatapackValue::Bool(true),
        )]));
        assert_eq!(
            canonicalize_datapack_defaults(&mut missing),
            Err(DatapackRegistrationError::InvalidDefault)
        );
    }

    #[test]
    fn existing_kinds_and_values_keep_their_wire_tags() {
        use crate::wire::{Reader, Writer};
        fn tag(bytes: &[u8]) -> u32 {
            Reader::new(bytes).take::<u32>().expect("variant tag")
        }
        for (index, kind) in [
            DatapackFieldKind::Bool,
            DatapackFieldKind::Int { min: 0, max: 1 },
            DatapackFieldKind::Float {
                min: 0.0,
                max: 1.0,
            },
            DatapackFieldKind::Text { max_len: 4 },
            DatapackFieldKind::Identifier,
            DatapackFieldKind::Reference {
                registry: "example:spell".into(),
            },
        ]
        .into_iter()
        .enumerate()
        {
            let mut out = Writer::new();
            out.put(&kind);
            assert_eq!(out.error(), None);
            // The byte tag is the variant index: appending List/Object must
            // not move any of the six existing tags.
            assert_eq!(tag(out.bytes()), index as u32);
            assert_eq!(Reader::decode::<DatapackFieldKind>(out.bytes()), Ok(kind));
        }
        for (index, value) in [
            DatapackValue::Bool(true),
            DatapackValue::Int(7),
            DatapackValue::Float(0.5),
            DatapackValue::Text("example:fire".into()),
        ]
        .into_iter()
        .enumerate()
        {
            let mut out = Writer::new();
            out.put(&value);
            assert_eq!(out.error(), None);
            assert_eq!(tag(out.bytes()), index as u32);
            assert_eq!(Reader::decode::<DatapackValue>(out.bytes()), Ok(value));
        }
    }
}
