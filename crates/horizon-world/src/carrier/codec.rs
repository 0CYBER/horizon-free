//! Bounded NBT codec for the carrier data model.
//!
//! Slice 1 of the moving structures design (board issue 1, "Carrier data
//! model and bounded codec", in
//! `docs/design/mod-api/moving-structures.md`): versioned
//! persistence of [`Carrier`] using the crate's existing Java Edition
//! NBT (`horizon_core::nbt`), with no gameplay integration. The design's
//! save contract applies: "Persist versioned cells, canonical registry
//! names, block-entity data ... UUID, transform and simulation revision
//! ... Unsupported schema fails load explicitly."
//!
//! Every bound is checked before memory is committed: the NBT reader
//! refuses declared lengths past the bytes actually present (so a
//! declared 2³¹ list never allocates) and this module caps the input,
//! node and depth budgets at [`limits`] before parsing, then checks the
//! cell count before the cells vector is allocated and the payload
//! budget cell by cell while decoding. [`encode`] writes the transform's
//! exact doubles, and [`decode`] rebuilds them without renormalizing, so
//! `decode(encode(carrier))` is bitwise exact.

use super::contact::RigidTransform;
use super::model::{
    Carrier, Cell, Invalid, LocalPos, MAX_CARRIER_DATA_BYTES, MAX_CELLS, MAX_LOCAL_COORD,
    MIN_LOCAL_COORD, stored_transform,
};
use horizon_core::nbt::{self, Compound, JavaString, Limits, NamedTag, Tag};
use std::fmt;

/// The schema version this build writes and reads. Any other version
/// fails load explicitly with [`CodecError::UnknownSchemaVersion`].
pub const SCHEMA_VERSION: i32 = 1;
/// The file root's name.
const ROOT_NAME: &str = "horizon_carrier";
/// Input envelope: this bounds the wire bytes on the way in, not the
/// decoded heap — a full-size carrier decodes to noticeably more than
/// this once the tree exists. Past each string's own NBT limit (see
/// [`super::model::MAX_NBT_STRING_BYTES`]), it is the only bound on state
/// names and properties: a carrier whose encoded form passes it refuses at
/// the door, before the tree is
/// parsed.
pub const MAX_INPUT_BYTES: usize = 24 * 1024 * 1024;
/// Node ceiling, a chosen bound rather than one the byte budget implies:
/// an unnamed element of a byte list costs one wire byte, so hostile input
/// could pack far more nodes into 24 MiB, and this ceiling, not the byte
/// budget, is what bounds them. A legal carrier's smallest repeated node is
/// a property entry (tag byte, u16 name length, a name byte, u16 value
/// length, a value byte: seven bytes), so a legal carrier within the input
/// budget has under 3.6 million nodes and fits with margin. The reader
/// allocates the complete tree before the carrier schema is checked, so
/// neither bound is a heap bound.
const MAX_NODES: usize = 5 * 1024 * 1024;
/// The schema nests three deep under the root; eight refuses anything
/// deeper before it is parsed.
const MAX_DEPTH: usize = 8;
/// The exact root keys.
const ROOT_KEYS: [&str; 7] = [
    "version",
    "uuid_msb",
    "uuid_lsb",
    "generation",
    "revision",
    "transform",
    "cells",
];
/// The exact transform keys.
const TRANSFORM_KEYS: [&str; 3] = ["translation", "pivot", "rotation"];

/// Why an encoded carrier could not be written or read.
#[derive(Clone, Debug, PartialEq)]
pub enum CodecError {
    /// The NBT container itself was malformed, truncated or over the
    /// input, node or depth budget.
    Nbt(nbt::Error),
    /// A schema version this build does not know; the carrier is refused
    /// whole rather than partially loaded.
    UnknownSchemaVersion {
        /// The version found in the file.
        found: i32,
    },
    /// A field was missing, had the wrong type, or the compound held
    /// unexpected keys.
    Malformed(&'static str),
    /// Structurally valid NBT violating the carrier model's bounds or
    /// invariants.
    Invalid(Invalid),
    /// Well-formed NBT with extra bytes after the root; the file is
    /// refused whole rather than silently trimmed.
    TrailingBytes {
        /// How many bytes followed the root tag.
        extra: usize,
    },
}
impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nbt(error) => write!(f, "carrier codec NBT error: {error}"),
            Self::UnknownSchemaVersion { found } => write!(
                f,
                "carrier schema version {found} is not supported (this build reads {SCHEMA_VERSION})"
            ),
            Self::Malformed(what) => write!(f, "carrier schema malformed: {what}"),
            Self::Invalid(invalid) => write!(f, "carrier refused: {invalid}"),
            Self::TrailingBytes { extra } => {
                write!(f, "{extra} bytes follow the carrier root")
            }
        }
    }
}
impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Nbt(error) => Some(error),
            Self::Invalid(invalid) => Some(invalid),
            _ => None,
        }
    }
}

/// The read and write budgets: the input bound, node ceiling and depth
/// ceiling shared by both directions so a written file always reads back.
fn limits() -> Limits {
    Limits {
        max_bytes: MAX_INPUT_BYTES,
        max_nodes: MAX_NODES,
        max_depth: MAX_DEPTH,
    }
}

/// Serializes a carrier to named-root NBT. The transform doubles are the
/// model's canonical values verbatim — no negative zeros, normalized
/// quaternion — so the reader's load-time normalizations cannot touch
/// them.
pub fn encode(carrier: &Carrier) -> Result<Vec<u8>, CodecError> {
    let transform = carrier.transform();
    let translation = transform.translation();
    let pivot = transform.pivot();
    let rotation = transform.rotation().components();
    let cells =
        carrier
            .cells()
            .iter()
            .map(|cell| {
                let pos = cell.pos();
                let mut entries = vec![
                    (
                        "pos",
                        Tag::IntArray(vec![
                            i32::from(pos.x()),
                            i32::from(pos.y()),
                            i32::from(pos.z()),
                        ]),
                    ),
                    ("state", Tag::String(cell.name().into())),
                ];
                if !cell.properties().is_empty() {
                    entries.push((
                        "properties",
                        Tag::compound(cell.properties().iter().map(|(key, value)| {
                            (key.as_ref(), Tag::String(value.as_ref().into()))
                        })),
                    ));
                }
                if let Some(data) = cell.data() {
                    entries.push(("data", Tag::ByteArray(data.to_vec())));
                }
                Tag::compound(entries)
            })
            .collect::<Vec<_>>();
    let root = Tag::compound([
        ("version", Tag::Int(SCHEMA_VERSION)),
        ("uuid_msb", Tag::Long((carrier.uuid() >> 64) as u64 as i64)),
        ("uuid_lsb", Tag::Long(carrier.uuid() as u64 as i64)),
        ("generation", Tag::Long(i64::from(carrier.generation()))),
        // A bit-cast long: the full u64 revision range round-trips.
        ("revision", Tag::Long(carrier.revision() as i64)),
        (
            "transform",
            Tag::compound([
                (
                    "translation",
                    double_list(&[translation.x, translation.y, translation.z]),
                ),
                ("pivot", double_list(&[pivot.x, pivot.y, pivot.z])),
                (
                    "rotation",
                    double_list(&[rotation[0], rotation[1], rotation[2], rotation[3]]),
                ),
            ]),
        ),
        ("cells", Tag::List(cells)),
    ]);
    nbt::write_named(
        &NamedTag {
            name: ROOT_NAME.into(),
            tag: root,
        },
        limits(),
    )
    .map_err(CodecError::Nbt)
}

/// A three- or four-double list from canonical transform components.
fn double_list(values: &[f64]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}

/// Parses named-root NBT into a carrier, checking every bound before the
/// memory behind it is committed. Unknown schema versions, duplicate
/// cells, out-of-range coordinates, over-budget payloads and
/// non-canonical transforms are refused whole; nothing is clamped and no
/// refusal depends on catching a panic.
pub fn decode(bytes: &[u8]) -> Result<Carrier, CodecError> {
    let (root, used) = nbt::read_named(bytes, limits()).map_err(CodecError::Nbt)?;
    if used != bytes.len() {
        return Err(CodecError::TrailingBytes {
            extra: bytes.len() - used,
        });
    }
    let Tag::Compound(mut map) = root.tag else {
        return Err(malformed("root is not a compound"));
    };
    // The version gate is the first carrier field checked: the NBT
    // reader has already built the whole tree by now, but among the
    // carrier's own fields the version is read before the key set or any
    // value, so a file from another schema reports its version whatever
    // the rest of the tree holds.
    let version = int_field(&map, "version")?;
    if version != SCHEMA_VERSION {
        return Err(CodecError::UnknownSchemaVersion { found: version });
    }
    expect_exact_keys(&map, &ROOT_KEYS)?;
    let uuid = (((long_field(&map, "uuid_msb")? as u64) as u128) << 64)
        | (long_field(&map, "uuid_lsb")? as u64) as u128;
    let generation = u32::try_from(long_field(&map, "generation")?)
        .map_err(|_| malformed("negative generation"))?;
    // Bit-cast back: revisions at and above 2^63 round-trip exactly.
    let revision = long_field(&map, "revision")? as u64;
    let transform = decode_transform(&map)?;
    let cells_tag = map
        .remove(&JavaString::from("cells"))
        .ok_or_else(|| malformed("missing cells"))?;
    let Tag::List(items) = cells_tag else {
        return Err(malformed("cells is not a list"));
    };
    // The cell count is checked before the cells vector is allocated;
    // the list length itself was already bounded by the node budget and
    // the bytes actually present.
    if items.len() > MAX_CELLS {
        return Err(CodecError::Invalid(Invalid::CellCount {
            count: items.len(),
        }));
    }
    let mut cells = Vec::with_capacity(items.len());
    let mut total = 0_usize;
    for item in items {
        let Tag::Compound(mut cell_map) = item else {
            return Err(malformed("cell is not a compound"));
        };
        check_cell_keys(&cell_map)?;
        let pos = position_field(&cell_map)?;
        let mut axes = [0_i8; 3];
        for (axis, &value) in pos.iter().enumerate() {
            if !(MIN_LOCAL_COORD..=MAX_LOCAL_COORD).contains(&value) {
                return Err(CodecError::Invalid(Invalid::Coordinate { value }));
            }
            axes[axis] = value as i8;
        }
        let name = string_field(&cell_map, "state")?;
        let properties = decode_properties(&mut cell_map)?;
        let data = match cell_map.remove(&JavaString::from("data")) {
            Some(Tag::ByteArray(bytes)) => {
                // Checked arithmetic on the running budget: overflow and
                // the 16 MiB bound both refuse before the bytes move.
                total = total
                    .checked_add(bytes.len())
                    .filter(|total| *total <= MAX_CARRIER_DATA_BYTES)
                    .ok_or(CodecError::Invalid(Invalid::CarrierData {
                        total: MAX_CARRIER_DATA_BYTES + 1,
                    }))?;
                Some(bytes)
            }
            Some(_) => return Err(malformed("data is not a byte array")),
            None => None,
        };
        cells.push(
            Cell::new(
                LocalPos::new(axes[0], axes[1], axes[2]),
                &name,
                properties,
                data,
            )
            .map_err(CodecError::Invalid)?,
        );
    }
    // Sorting and duplicate detection happen once, in the model; the
    // file may list cells in any order.
    Carrier::new(uuid, generation, revision, transform, cells).map_err(CodecError::Invalid)
}

/// Reads and validates the transform compound.
fn decode_transform(map: &Compound) -> Result<RigidTransform, CodecError> {
    let transform_tag = field(map, "transform").ok_or_else(|| malformed("missing transform"))?;
    let Tag::Compound(transform_map) = transform_tag else {
        return Err(malformed("transform is not a compound"));
    };
    expect_exact_keys(transform_map, &TRANSFORM_KEYS)?;
    let translation = doubles_field(transform_map, "translation")?;
    let pivot = doubles_field(transform_map, "pivot")?;
    let rotation = doubles_field(transform_map, "rotation")?;
    stored_transform(translation, pivot, rotation).map_err(CodecError::Invalid)
}

/// Refuses a compound whose keys are not exactly `keys`.
fn expect_exact_keys(map: &Compound, keys: &[&str]) -> Result<(), CodecError> {
    if map.len() == keys.len() && keys.iter().all(|key| field(map, key).is_some()) {
        Ok(())
    } else {
        Err(malformed("unexpected or missing keys"))
    }
}

/// Cell compounds hold `pos` and `state` always, `properties` and
/// `data` optionally, and nothing else.
fn check_cell_keys(map: &Compound) -> Result<(), CodecError> {
    const OPTIONAL: [&str; 4] = ["pos", "state", "properties", "data"];
    let present = OPTIONAL
        .iter()
        .filter(|key| field(map, key).is_some())
        .count();
    if present == map.len() && field(map, "pos").is_some() && field(map, "state").is_some() {
        Ok(())
    } else {
        Err(malformed(
            "cell keys are not pos, state and optional properties/data",
        ))
    }
}

/// Takes the optional `properties` compound as string pairs; wrong value
/// types fail as malformed schema rather than model refusals.
fn decode_properties(cell_map: &mut Compound) -> Result<Vec<(String, String)>, CodecError> {
    match cell_map.remove(&JavaString::from("properties")) {
        Some(Tag::Compound(map)) => {
            let mut pairs = Vec::new();
            for (key, value) in map {
                let Tag::String(value) = value else {
                    return Err(malformed("property is not a string"));
                };
                let key = key
                    .to_string()
                    .map_err(|_| malformed("property name is not valid UTF-16"))?;
                let value = value
                    .to_string()
                    .map_err(|_| malformed("property value is not valid UTF-16"))?;
                pairs.push((key, value));
            }
            Ok(pairs)
        }
        Some(_) => Err(malformed("properties is not a compound")),
        None => Ok(Vec::new()),
    }
}

fn malformed(reason: &'static str) -> CodecError {
    CodecError::Malformed(reason)
}

fn field<'a>(map: &'a Compound, key: &str) -> Option<&'a Tag> {
    map.get(&JavaString::from(key))
}

fn long_field(map: &Compound, key: &str) -> Result<i64, CodecError> {
    match field(map, key) {
        Some(Tag::Long(value)) => Ok(*value),
        _ => Err(malformed("expected a long")),
    }
}

fn int_field(map: &Compound, key: &str) -> Result<i32, CodecError> {
    match field(map, key) {
        Some(Tag::Int(value)) => Ok(*value),
        _ => Err(malformed("expected an int")),
    }
}

/// The `pos` field: exactly three ints, taken without cloning the
/// stored vector — a longer array is refused before it is copied.
fn position_field(map: &Compound) -> Result<[i32; 3], CodecError> {
    match field(map, "pos") {
        Some(Tag::IntArray(values)) if values.len() == 3 => Ok([values[0], values[1], values[2]]),
        Some(Tag::IntArray(_)) => Err(malformed("pos is not three coordinates")),
        _ => Err(malformed("pos is not an int array")),
    }
}

fn string_field(map: &Compound, key: &str) -> Result<String, CodecError> {
    match field(map, key) {
        Some(Tag::String(value)) => value
            .to_string()
            .map_err(|_| malformed("string is not valid UTF-16")),
        _ => Err(malformed("expected a string")),
    }
}

fn doubles_field<const N: usize>(map: &Compound, key: &str) -> Result<[f64; N], CodecError> {
    let Tag::List(values) = field(map, key).ok_or_else(|| malformed("missing double list"))? else {
        return Err(malformed("expected a double list"));
    };
    if values.len() != N {
        return Err(malformed("double list has the wrong length"));
    }
    let mut doubles = [0_f64; N];
    for (slot, value) in doubles.iter_mut().zip(values) {
        let Tag::Double(bits) = value else {
            return Err(malformed("expected doubles"));
        };
        *slot = f64::from_bits(*bits);
    }
    Ok(doubles)
}

#[cfg(test)]
mod tests;
