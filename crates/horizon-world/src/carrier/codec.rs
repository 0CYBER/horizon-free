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
mod tests {
    use super::*;
    use crate::carrier::contact::{RigidTransform, Rotation};
    use horizon_core::geometry::Vec3;

    /// Distinct positions for up to `count` cells, drawn from the i8
    /// cube in a deterministic spread.
    fn positions(count: usize) -> Vec<LocalPos> {
        (0..count)
            .map(|i| {
                LocalPos::new(
                    ((i % 256) as i32 - 128) as i8,
                    ((i / 256) % 64) as i8 - 32,
                    ((i / 16_384) % 64) as i8 - 32,
                )
            })
            .collect()
    }

    fn sample_carrier() -> Carrier {
        let transform = RigidTransform::new(
            Vec3::new(-1.5, 2.25, 1.0e300),
            Vec3::new(0.5, -0.5, 3.0),
            // A 4-radian turn: its raw quaternion has negative w, so this
            // exercises the canonical sign flip on construction.
            Rotation::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), 4.0).unwrap(),
        )
        .unwrap();
        Carrier::new(
            0x0011_2233_4455_6677_8899_aabb_ccdd_eeff,
            7,
            1 << 40,
            transform,
            vec![
                Cell::new(LocalPos::new(1, 2, 3), "minecraft:stone", Vec::new(), None).unwrap(),
                Cell::new(
                    LocalPos::new(-4, 5, -6),
                    "minecraft:furnace",
                    vec![("facing".to_string(), "south".to_string())],
                    Some(vec![7, 8, 9]),
                )
                .unwrap(),
                // An empty payload is Some, distinct from None.
                Cell::new(
                    LocalPos::new(9, 9, 9),
                    "minecraft:chest",
                    Vec::new(),
                    Some(vec![]),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn rewrite(bytes: &[u8], patch: impl FnOnce(&mut Compound)) -> Vec<u8> {
        let (root, _) = nbt::read_named(bytes, limits()).unwrap();
        let Tag::Compound(mut map) = root.tag else {
            panic!("test rewrite needs a compound root");
        };
        patch(&mut map);
        nbt::write_named(
            &NamedTag {
                name: ROOT_NAME.into(),
                tag: Tag::Compound(map),
            },
            limits(),
        )
        .unwrap()
    }

    /// Replaces the cell list with hand-built cell compounds.
    fn cells_tag(cells: Vec<Tag>) -> Tag {
        Tag::List(cells)
    }

    fn cell_tag(x: i32, y: i32, z: i32, name: &str, data: Option<Vec<u8>>) -> Tag {
        cell_tag_with(x, y, z, name, &[], data)
    }

    /// A cell compound with explicit `properties`, like vanilla structure
    /// NBT's `Properties` compound of strings.
    fn cell_tag_with(
        x: i32,
        y: i32,
        z: i32,
        name: &str,
        properties: &[(&str, &str)],
        data: Option<Vec<u8>>,
    ) -> Tag {
        let mut entries = vec![
            ("pos", Tag::IntArray(vec![x, y, z])),
            ("state", Tag::String(name.into())),
        ];
        if !properties.is_empty() {
            entries.push((
                "properties",
                Tag::compound(
                    properties
                        .iter()
                        .map(|(key, value)| (*key, Tag::String((*value).into()))),
                ),
            ));
        }
        if let Some(data) = data {
            entries.push(("data", Tag::ByteArray(data)));
        }
        Tag::compound(entries)
    }

    #[test]
    fn round_trip_is_bitwise_exact() {
        let carrier = sample_carrier();
        let decoded = decode(&encode(&carrier).unwrap()).unwrap();
        assert_eq!(decoded.uuid(), carrier.uuid());
        assert_eq!(decoded.generation(), carrier.generation());
        assert_eq!(decoded.revision(), carrier.revision());
        assert_eq!(decoded.cells(), carrier.cells());
        let got = decoded.transform();
        let want = carrier.transform();
        let doubles = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        assert_eq!(doubles(got.translation()), doubles(want.translation()));
        assert_eq!(doubles(got.pivot()), doubles(want.pivot()));
        assert_eq!(
            got.rotation().components().map(f64::to_bits),
            want.rotation().components().map(f64::to_bits)
        );
        // A second pass is stable: re-encoding the decode is byte-identical.
        assert_eq!(encode(&decoded).unwrap(), encode(&carrier).unwrap());
    }

    #[test]
    fn round_trip_identity_transform() {
        let carrier = Carrier::new(
            42,
            u32::MAX,
            u64::MAX,
            RigidTransform::identity_at(Vec3::new(0.0, 0.0, 0.0)).unwrap(),
            vec![],
        )
        .unwrap();
        let decoded = decode(&encode(&carrier).unwrap()).unwrap();
        assert_eq!(decoded, carrier);
        assert_eq!(
            decoded.transform().rotation().components(),
            Rotation::IDENTITY.components()
        );
    }

    #[test]
    fn empty_and_bare_payloads_round_trip_distinctly() {
        let carrier = sample_carrier();
        let decoded = decode(&encode(&carrier).unwrap()).unwrap();
        // Sorted by position: (-4,5,-6), (1,2,3), (9,9,9).
        let cells = decoded.cells();
        assert_eq!(cells[0].data(), Some(&[7, 8, 9][..]));
        assert_eq!(cells[1].data(), None);
        assert_eq!(cells[2].data(), Some(&[][..]));
    }

    #[test]
    fn accepts_exactly_the_cell_limit_and_refuses_one_more() {
        let at_limit = Carrier::new(
            1,
            0,
            0,
            RigidTransform::identity_at(Vec3::new(1.0, 1.0, 1.0)).unwrap(),
            positions(4096)
                .into_iter()
                .map(|pos| Cell::new(pos, "minecraft:stone", Vec::new(), None).unwrap())
                .collect(),
        )
        .unwrap();
        assert_eq!(at_limit.cells().len(), 4096);
        assert_eq!(decode(&encode(&at_limit).unwrap()).unwrap(), at_limit);

        let over = positions(4097)
            .into_iter()
            .map(|pos| {
                cell_tag(
                    pos.x() as i32,
                    pos.y() as i32,
                    pos.z() as i32,
                    "minecraft:stone",
                    None,
                )
            })
            .collect::<Vec<_>>();
        let bytes = rewrite(&encode(&at_limit).unwrap(), |map| {
            map.insert("cells".into(), cells_tag(over));
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::Invalid(Invalid::CellCount { count: 4097 }))
        );
    }

    #[test]
    fn accepts_boundary_coordinates_and_refuses_past_them() {
        let base = sample_carrier();
        let boundary = rewrite(&encode(&base).unwrap(), |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![
                    cell_tag(127, -128, 127, "minecraft:stone", None),
                    cell_tag(-128, 127, -128, "minecraft:stone", None),
                ]),
            );
        });
        let decoded = decode(&boundary).unwrap();
        // Cells come back in canonical sorted order, not file order.
        assert_eq!(
            decoded.cells().iter().map(Cell::pos).collect::<Vec<_>>(),
            vec![
                LocalPos::new(-128, 127, -128),
                LocalPos::new(127, -128, 127)
            ]
        );
        for value in [128_i32, -129] {
            let bytes = rewrite(&boundary, |map| {
                map.insert(
                    "cells".into(),
                    cells_tag(vec![cell_tag(value, 0, 0, "minecraft:stone", None)]),
                );
            });
            assert_eq!(
                decode(&bytes),
                Err(CodecError::Invalid(Invalid::Coordinate { value }))
            );
        }
    }

    #[test]
    fn accepts_exactly_64k_per_cell_and_refuses_one_more() {
        let at_limit = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![cell_tag(
                    0,
                    0,
                    0,
                    "minecraft:stone",
                    Some(vec![0; 64 * 1024]),
                )]),
            );
        });
        assert_eq!(
            decode(&at_limit).unwrap().cells()[0].data().unwrap().len(),
            64 * 1024
        );
        let over = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![cell_tag(
                    0,
                    0,
                    0,
                    "minecraft:stone",
                    Some(vec![0; 64 * 1024 + 1]),
                )]),
            );
        });
        assert_eq!(
            decode(&over),
            Err(CodecError::Invalid(Invalid::CellData {
                len: 64 * 1024 + 1
            }))
        );
    }

    #[test]
    fn accepts_exactly_16mib_and_refuses_one_more_byte() {
        // 256 cells of 64 KiB each fill the 16 MiB carrier budget exactly.
        let full = (0..256)
            .map(|i| {
                cell_tag(
                    i % 40,
                    i / 40,
                    0,
                    "minecraft:stone",
                    Some(vec![0; 64 * 1024]),
                )
            })
            .collect();
        let at_limit = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            map.insert("cells".into(), cells_tag(full));
        });
        let decoded = decode(&at_limit).unwrap();
        assert_eq!(
            decoded.cells().iter().map(Cell::data_len).sum::<usize>(),
            16777216
        );
        let over = rewrite(&at_limit, |map| {
            let Tag::List(mut items) = map.get(&JavaString::from("cells")).unwrap().clone() else {
                panic!("cells must be a list");
            };
            items.push(cell_tag(120, 120, 120, "minecraft:stone", Some(vec![1])));
            map.insert("cells".into(), cells_tag(items));
        });
        assert_eq!(
            decode(&over).unwrap_err(),
            CodecError::Invalid(Invalid::CarrierData { total: 16777217 })
        );
    }

    #[test]
    fn truncated_input_is_refused() {
        let bytes = encode(&sample_carrier()).unwrap();
        for cut in [0_usize, 1, bytes.len() / 2, bytes.len() - 1] {
            let result = decode(&bytes[..cut]);
            assert!(
                matches!(result, Err(CodecError::Nbt(_))),
                "cut at {cut} must refuse"
            );
        }
    }

    #[test]
    fn unknown_schema_version_is_refused_explicitly() {
        let bytes = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            map.insert("version".into(), Tag::Int(SCHEMA_VERSION + 1));
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::UnknownSchemaVersion {
                found: SCHEMA_VERSION + 1
            })
        );
        // A missing version is malformed, not a silent default.
        let bytes = rewrite(bytes.as_slice(), |map| {
            map.remove(&JavaString::from("version"));
        });
        assert!(matches!(decode(&bytes), Err(CodecError::Malformed(_))));
    }

    #[test]
    fn duplicate_cells_in_file_are_refused() {
        let bytes = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![
                    cell_tag(1, 1, 1, "minecraft:stone", None),
                    cell_tag(1, 1, 1, "minecraft:dirt", None),
                ]),
            );
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::Invalid(Invalid::DuplicateCell))
        );
    }

    #[test]
    fn non_finite_transform_is_refused() {
        for poisoned in [
            f64::NAN.to_bits(),
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
        ] {
            let bytes = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
                let Tag::Compound(transform) = map.get_mut(&JavaString::from("transform")).unwrap()
                else {
                    panic!("transform must be a compound");
                };
                transform.insert(
                    "translation".into(),
                    Tag::List(vec![
                        Tag::Double(poisoned),
                        Tag::Double(0.0f64.to_bits()),
                        Tag::Double(0.0f64.to_bits()),
                    ]),
                );
            });
            assert_eq!(
                decode(&bytes),
                Err(CodecError::Invalid(Invalid::NonFiniteTransform))
            );
        }
    }

    #[test]
    fn zero_quaternion_is_refused() {
        let bytes = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            let Tag::Compound(transform) = map.get_mut(&JavaString::from("transform")).unwrap()
            else {
                panic!("transform must be a compound");
            };
            transform.insert(
                "rotation".into(),
                Tag::List(vec![Tag::Double(0.0f64.to_bits()); 4]),
            );
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::Invalid(Invalid::ZeroQuaternion))
        );
    }

    #[test]
    fn non_canonical_quaternion_is_refused() {
        let carrier = sample_carrier();
        let components = carrier.transform().rotation().components();
        // The sign-flipped twin: same rotation, but not the canonical
        // form the encoder writes.
        let negated = rewrite(&encode(&carrier).unwrap(), |map| {
            let Tag::Compound(transform) = map.get_mut(&JavaString::from("transform")).unwrap()
            else {
                panic!("transform must be a compound");
            };
            transform.insert(
                "rotation".into(),
                Tag::List(
                    components
                        .iter()
                        .map(|v| Tag::Double((-v).to_bits()))
                        .collect(),
                ),
            );
        });
        assert_eq!(
            decode(&negated),
            Err(CodecError::Invalid(Invalid::QuaternionNotCanonical))
        );
        // A finite nonzero quaternion with the wrong norm is not what
        // normalization writes.
        let unnormalized = rewrite(&encode(&carrier).unwrap(), |map| {
            let Tag::Compound(transform) = map.get_mut(&JavaString::from("transform")).unwrap()
            else {
                panic!("transform must be a compound");
            };
            transform.insert(
                "rotation".into(),
                Tag::List(vec![
                    Tag::Double(2.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
            );
        });
        assert_eq!(
            decode(&unnormalized),
            Err(CodecError::Invalid(Invalid::QuaternionNotCanonical))
        );
    }

    #[test]
    fn declared_length_of_two_pow_31_is_refused_without_allocation() {
        // A byte-array root declaring 2^31 bytes: the reader compares the
        // declared length with the bytes present before allocating.
        let mut bytes = vec![0x07, 0x00, 0x01, b'x'];
        bytes.extend_from_slice(&i32::MAX.to_be_bytes());
        match decode(&bytes) {
            Err(CodecError::Nbt(error)) => {
                assert_eq!(
                    error.reason, "collection exceeds available bytes",
                    "{error}"
                );
            }
            other => panic!("expected the byte length refusal, got {other:?}"),
        }
        // A cells list declaring 2^31 compounds inside a valid root. The
        // key is complete, so the refusal comes from the list length
        // check, not from parsing the key.
        let mut bytes = vec![0x0a, 0x00, 0x00]; // compound root, empty name
        bytes.extend_from_slice(&[0x09]); // list
        bytes.extend_from_slice(&[0x00, 0x05]); // key length: "cells"
        bytes.extend_from_slice(b"cells");
        bytes.extend_from_slice(&[0x0a]); // element type compound
        bytes.extend_from_slice(&i32::MAX.to_be_bytes());
        match decode(&bytes) {
            Err(CodecError::Nbt(error)) => {
                assert_eq!(error.reason, "node limit exceeded", "{error}");
            }
            other => panic!("expected the list length refusal, got {other:?}"),
        }
    }

    #[test]
    fn long_legal_property_names_round_trip() {
        // The declaration boundary sets no token length, so a property
        // name past every vanilla maximum (65 letters against the vanilla
        // 20) and a long value are ordinary legal state data.
        let carrier = Carrier::new(
            9,
            0,
            0,
            RigidTransform::identity_at(Vec3::ZERO).unwrap(),
            vec![
                Cell::new(
                    LocalPos::new(0, 0, 0),
                    "minecraft:stone",
                    vec![
                        ("a".repeat(65), "b".repeat(66)),
                        ("facing".to_string(), "north".to_string()),
                    ],
                    None,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let decoded = decode(&encode(&carrier).unwrap()).unwrap();
        assert_eq!(decoded, carrier);
        assert_eq!(
            decoded.cells()[0].properties()[0].0.as_ref(),
            "a".repeat(65)
        );
    }

    #[test]
    fn state_strings_round_trip_at_the_nbt_limit_and_refuse_one_past() {
        // One NBT string holds at most 65,535 bytes of modified UTF-8; the
        // disk writer would write anything longer as empty, so 65,535 must
        // round-trip exactly and 65,536 must refuse before any save.
        let at_limit = Carrier::new(
            10,
            0,
            0,
            RigidTransform::identity_at(Vec3::ZERO).unwrap(),
            vec![
                Cell::new(
                    LocalPos::new(0, 0, 0),
                    &format!("m:{}", "p".repeat(65_533)),
                    vec![("k".repeat(65_535), "v".repeat(65_535))],
                    None,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let decoded = decode(&encode(&at_limit).unwrap()).unwrap();
        assert_eq!(decoded, at_limit);
        assert_eq!(decoded.cells()[0].name().len(), 65_535);
        assert_eq!(decoded.cells()[0].properties()[0].0.len(), 65_535);
        assert_eq!(decoded.cells()[0].properties()[0].1.len(), 65_535);

        let cell = |name: &str, key: String, value: String| {
            Cell::new(LocalPos::new(0, 0, 0), name, vec![(key, value)], None)
        };
        assert_eq!(
            cell("minecraft:stone", "k".to_string(), "v".repeat(65_536)),
            Err(Invalid::StringTooLong { len: 65_536 })
        );
        assert_eq!(
            cell("minecraft:stone", "k".repeat(65_536), "v".to_string()),
            Err(Invalid::StringTooLong { len: 65_536 })
        );
        assert_eq!(
            cell(
                &format!("m:{}", "p".repeat(65_534)),
                "k".to_string(),
                "v".to_string()
            ),
            Err(Invalid::StringTooLong { len: 65_536 })
        );
    }

    #[test]
    fn an_unknown_version_beats_broken_keys_and_values() {
        // The version is the first carrier field checked: a file from
        // another schema reports its version even when its keys or other
        // fields are also wrong. (The NBT reader has still built the
        // whole tree first — the gate is first among the carrier's
        // fields, not before parsing.)
        let bytes = rewrite(&encode(&sample_carrier()).unwrap(), |map| {
            map.insert("version".into(), Tag::Int(SCHEMA_VERSION + 7));
            map.insert("surprise".into(), Tag::Int(0));
            map.insert("uuid_msb".into(), Tag::Int(1));
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::UnknownSchemaVersion {
                found: SCHEMA_VERSION + 7
            })
        );
    }

    #[test]
    fn trailing_bytes_after_the_root_are_refused() {
        let mut bytes = encode(&sample_carrier()).unwrap();
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(CodecError::TrailingBytes { extra: 1 }));
        bytes.extend_from_slice(&[1, 2, 3]);
        assert_eq!(decode(&bytes), Err(CodecError::TrailingBytes { extra: 4 }));
    }

    #[test]
    fn properties_round_trip_sorted_regardless_of_file_order() {
        // The canonical fixture: vanilla oak_stairs with four properties,
        // handed to the model in a shuffled order.
        let shuffled = vec![
            ("waterlogged".to_string(), "false".to_string()),
            ("half".to_string(), "bottom".to_string()),
            ("facing".to_string(), "north".to_string()),
            ("shape".to_string(), "straight".to_string()),
        ];
        let carrier = Carrier::new(
            5,
            0,
            0,
            RigidTransform::identity_at(Vec3::ZERO).unwrap(),
            vec![
                Cell::new(
                    LocalPos::new(0, 0, 0),
                    "minecraft:oak_stairs",
                    shuffled,
                    None,
                )
                .unwrap(),
            ],
        )
        .unwrap();
        let decoded = decode(&encode(&carrier).unwrap()).unwrap();
        assert_eq!(decoded, carrier);
        let cell = &decoded.cells()[0];
        assert_eq!(cell.name(), "minecraft:oak_stairs");
        let flat: Vec<(&str, &str)> = cell
            .properties()
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
            .collect();
        assert_eq!(
            flat,
            [
                ("facing", "north"),
                ("half", "bottom"),
                ("shape", "straight"),
                ("waterlogged", "false")
            ]
        );
        // The same state written with properties in reverse file order
        // decodes to the same sorted model.
        let bytes = rewrite(&encode(&carrier).unwrap(), |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![cell_tag_with(
                    0,
                    0,
                    0,
                    "minecraft:oak_stairs",
                    &[
                        ("waterlogged", "false"),
                        ("shape", "straight"),
                        ("half", "bottom"),
                        ("facing", "north"),
                    ],
                    None,
                )]),
            );
        });
        assert_eq!(decode(&bytes).unwrap(), carrier);
    }

    #[test]
    fn bad_properties_in_file_are_refused() {
        let base = encode(&sample_carrier()).unwrap();
        // An empty property value.
        let bytes = rewrite(&base, |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![cell_tag_with(
                    0,
                    0,
                    0,
                    "minecraft:oak_stairs",
                    &[("facing", "")],
                    None,
                )]),
            );
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::Invalid(Invalid::InvalidProperty))
        );
        // A property value with an uppercase character.
        let bytes = rewrite(&base, |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![cell_tag_with(
                    0,
                    0,
                    0,
                    "minecraft:oak_stairs",
                    &[("facing", "North")],
                    None,
                )]),
            );
        });
        assert_eq!(
            decode(&bytes),
            Err(CodecError::Invalid(Invalid::InvalidProperty))
        );
        // A property value that is not a string is malformed schema.
        let bytes = rewrite(&base, |map| {
            map.insert(
                "cells".into(),
                cells_tag(vec![Tag::compound([
                    ("pos", Tag::IntArray(vec![0, 0, 0])),
                    ("state", Tag::String("minecraft:oak_stairs".into())),
                    ("properties", Tag::compound([("facing", Tag::Int(1))])),
                ])]),
            );
        });
        assert!(matches!(decode(&bytes), Err(CodecError::Malformed(_))));
    }

    #[test]
    fn cell_order_is_stable_regardless_of_file_order() {
        let cells = positions(500);
        let forward = cells_tag(
            cells
                .iter()
                .map(|pos| {
                    cell_tag(
                        pos.x() as i32,
                        pos.y() as i32,
                        pos.z() as i32,
                        "minecraft:stone",
                        None,
                    )
                })
                .collect(),
        );
        let reversed = {
            let Tag::List(mut items) = forward.clone() else {
                unreachable!()
            };
            items.reverse();
            cells_tag(items)
        };
        let model = Carrier::new(
            3,
            0,
            0,
            RigidTransform::identity_at(Vec3::ZERO).unwrap(),
            cells
                .iter()
                .map(|pos| Cell::new(*pos, "minecraft:stone", Vec::new(), None).unwrap())
                .collect(),
        )
        .unwrap();
        for order in [forward, reversed] {
            let bytes = rewrite(&encode(&model).unwrap(), |map| {
                map.insert("cells".into(), order);
            });
            assert_eq!(decode(&bytes).unwrap().cells(), model.cells());
        }
    }

    #[test]
    fn mistyped_or_unexpected_fields_are_refused() {
        let base = encode(&sample_carrier()).unwrap();
        let wrongs: [(&str, fn(&mut Compound)); 5] = [
            ("uuid_msb is not a long", |map: &mut Compound| {
                map.insert("uuid_msb".into(), Tag::Int(1));
            }),
            ("state is not a string", |map: &mut Compound| {
                map.insert(
                    "cells".into(),
                    cells_tag(vec![Tag::compound([
                        ("pos", Tag::IntArray(vec![0, 0, 0])),
                        ("state", Tag::Int(1)),
                    ])]),
                );
            }),
            ("extra root key", |map: &mut Compound| {
                map.insert("surprise".into(), Tag::Int(0));
            }),
            ("extra cell key", |map: &mut Compound| {
                map.insert(
                    "cells".into(),
                    cells_tag(vec![Tag::compound([
                        ("pos", Tag::IntArray(vec![0, 0, 0])),
                        ("state", Tag::String("minecraft:stone".into())),
                        ("extra", Tag::Int(0)),
                    ])]),
                );
            }),
            ("pos is not an int array", |map: &mut Compound| {
                map.insert(
                    "cells".into(),
                    cells_tag(vec![cell_at(
                        Tag::Int(0),
                        Tag::String("minecraft:stone".into()),
                    )]),
                );
            }),
        ];
        for (name, patch) in wrongs {
            let bytes = rewrite(&base, patch);
            assert!(
                matches!(decode(&bytes), Err(CodecError::Malformed(_))),
                "{name} must be malformed"
            );
        }
    }

    /// A raw cell compound with prebuilt tags, for wrong-typed fields.
    fn cell_at(pos: Tag, state: Tag) -> Tag {
        Tag::compound([("pos", pos), ("state", state)])
    }
}
