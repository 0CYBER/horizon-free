//! Carrier data model: identity, transform and local cells.
//!
//! Slice 1 of the moving structures design
//! (`docs/design/mod-api/moving-structures.md`, board issue 1:
//! "Carrier data model and bounded codec"): the persisted shape of a
//! carrier — UUID, generation, simulation revision, transform and local
//! cells — with no gameplay integration. The transform reuses
//! [`RigidTransform`] and [`Rotation`] from slice 5's contact kernel;
//! cells are stored sorted by position, without duplicates, and air is
//! never stored.
//!
//! Trust-boundary limits (design section "Proposed mod_api surface and
//! trust boundary"), all checked before allocation and refusing rather
//! than clamping:
//! - at most [`MAX_CELLS`] non-air cells;
//! - local coordinates −128..127, inherent in the `i8` axes of
//!   [`LocalPos`] (the codec checks the range when narrowing stored `i32`
//!   values);
//! - at most [`MAX_CELL_DATA_BYTES`] decoded block-entity payload bytes
//!   per cell and [`MAX_CARRIER_DATA_BYTES`] per carrier;
//! - no duplicate cells, no NaN/infinity, no zero quaternion, no
//!   arithmetic overflow, and no empty or malformed block-state names —
//!   while property names and values carry no length limits of their
//!   own, matching the registry's declaration contract.

use super::contact::{RigidTransform, Rotation};
use horizon_core::geometry::Vec3;
use horizon_core::identifier::Identifier;
use std::fmt;

/// Design trust-boundary limit: at most 4,096 non-air cells per carrier.
pub const MAX_CELLS: usize = 4096;
/// Design trust-boundary limit: at most 64 KiB of decoded block-entity
/// payload bytes per cell.
pub const MAX_CELL_DATA_BYTES: usize = 64 * 1024;
/// Design trust-boundary limit: at most 16 MiB of decoded block-entity
/// payload bytes per carrier.
pub const MAX_CARRIER_DATA_BYTES: usize = 16 * 1024 * 1024;
/// Inclusive local coordinate bounds; equal to the `i8` range carried by
/// [`LocalPos`].
pub const MIN_LOCAL_COORD: i32 = -128;
/// See [`MIN_LOCAL_COORD`].
pub const MAX_LOCAL_COORD: i32 = 127;
/// The block-declaration boundary (`BlockDefinition::new`,
/// `horizon-mod-runtime/src/block_definitions.rs`) bounds neither the
/// number of properties nor the length of any name or value, and neither
/// does the identifier charset — so the model sets no length of its own
/// either. What does bound a state is its storage: each block id,
/// property name and value is one NBT string, and an NBT string holds at
/// most [`MAX_NBT_STRING_BYTES`] of modified UTF-8 (vanilla's chunk
/// palette stores states the same way). Past that the disk writer would
/// silently write an empty string, so such a state is refused here, never
/// saved in a form that cannot load. Beyond that, a carrier whose encoded
/// form does not fit the codec's input budget does not decode.
///
/// The NBT string limit: `DataOutput.writeUTF`'s u16 length prefix. Every
/// accepted token is ASCII, where modified UTF-8 length equals byte length.
pub const MAX_NBT_STRING_BYTES: usize = 65_535;
/// How far a stored quaternion's squared norm may sit from one and still
/// count as the encoder's normalized output: renormalizing rounds each
/// component, leaving the squared norm within a few units in the last
/// place (~2.3e-15); anything farther is not what the encoder wrote.
const QUATERNION_NORM_TOLERANCE: f64 = 1.0e-9;
/// The vanilla air block states; air cells are never stored.
const AIR_STATES: [&str; 3] = ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"];

/// Why a carrier model value was refused. Refusals never clamp, truncate
/// or partially apply: the caller keeps the previous state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// A NaN or infinite transform component.
    NonFiniteTransform,
    /// A quaternion whose components are all zero.
    ZeroQuaternion,
    /// A stored quaternion that is not the canonical normalized form the
    /// encoder writes: its first nonzero component is negative, or its
    /// norm sits farther from one than normalization can leave it.
    QuaternionNotCanonical,
    /// More cells than [`MAX_CELLS`].
    CellCount {
        /// The offending cell count.
        count: usize,
    },
    /// A stored local coordinate outside −128..127.
    Coordinate {
        /// The offending value before narrowing to `i8`.
        value: i32,
    },
    /// Two cells at the same local position.
    DuplicateCell,
    /// More block-entity payload bytes in one cell than
    /// [`MAX_CELL_DATA_BYTES`].
    CellData {
        /// The offending payload length.
        len: usize,
    },
    /// More block-entity payload bytes in total than
    /// [`MAX_CARRIER_DATA_BYTES`].
    CarrierData {
        /// The offending total.
        total: usize,
    },
    /// An air block state; air is never stored.
    AirCell,
    /// An empty, malformed block identifier.
    BlockName,
    /// A property name or value that is empty or holds a character
    /// outside `[a-z0-9_]` (every vanilla registry token matches).
    InvalidProperty,
    /// Two properties with the same name.
    DuplicateProperty,
    /// A block id, property name or value longer than one NBT string can
    /// hold ([`MAX_NBT_STRING_BYTES`]).
    StringTooLong {
        /// The string's length in bytes.
        len: usize,
    },
}
impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteTransform => write!(f, "non-finite transform value"),
            Self::ZeroQuaternion => write!(f, "zero-length quaternion"),
            Self::QuaternionNotCanonical => write!(f, "quaternion is not canonical"),
            Self::CellCount { count } => write!(f, "{count} cells exceeds {MAX_CELLS}"),
            Self::Coordinate { value } => {
                write!(f, "local coordinate {value} outside -128..127")
            }
            Self::DuplicateCell => write!(f, "duplicate local cell"),
            Self::CellData { len } => write!(
                f,
                "cell payload of {len} bytes exceeds {MAX_CELL_DATA_BYTES}"
            ),
            Self::CarrierData { total } => write!(
                f,
                "carrier payload of {total} bytes exceeds {MAX_CARRIER_DATA_BYTES}"
            ),
            Self::AirCell => write!(f, "air cells are not stored"),
            Self::BlockName => write!(f, "invalid block name"),
            Self::InvalidProperty => write!(f, "invalid property name or value"),
            Self::DuplicateProperty => write!(f, "duplicate property name"),
            Self::StringTooLong { len } => write!(
                f,
                "state string of {len} bytes exceeds the NBT string limit {MAX_NBT_STRING_BYTES}"
            ),
        }
    }
}
impl std::error::Error for Invalid {}

/// A local cell position. The `i8` axes make the design's −128..127 local
/// coordinate limit inherent to the type; ordered `(x, y, z)` for the
/// carrier's sorted cell storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalPos {
    x: i8,
    y: i8,
    z: i8,
}
impl LocalPos {
    /// A position from `i8` axes.
    pub const fn new(x: i8, y: i8, z: i8) -> Self {
        Self { x, y, z }
    }
    /// The local X axis.
    pub const fn x(self) -> i8 {
        self.x
    }
    /// The local Y axis.
    pub const fn y(self) -> i8 {
        self.y
    }
    /// The local Z axis.
    pub const fn z(self) -> i8 {
        self.z
    }
}
impl fmt::Display for LocalPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{},{})", self.x, self.y, self.z)
    }
}

/// One local cell of a carrier: an integer local position, the block's
/// canonical namespaced identifier, its state properties (like vanilla
/// structure NBT's `Name` plus `Properties`), and an optional
/// block-entity payload. Properties are stored sorted by name, without
/// duplicates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pos: LocalPos,
    name: Box<str>,
    properties: Box<[(Box<str>, Box<str>)]>,
    data: Option<Box<[u8]>>,
}
impl Cell {
    /// Validates and builds one cell. The block name must be a canonical
    /// namespaced identifier (explicit namespace, nonempty path) and must
    /// not be one of the vanilla air states. Each property name and value
    /// must be nonempty and drawn from `[a-z0-9_]` — vanilla's registry
    /// tokens all match — with no additional length or count limit beyond
    /// the shared NBT string limit; names must not repeat. The payload
    /// must fit [`MAX_CELL_DATA_BYTES`]. Every bound is checked as it is
    /// reached — properties are consumed lazily and nothing is
    /// pre-allocated from an unchecked count.
    pub fn new(
        pos: LocalPos,
        name: &str,
        properties: impl IntoIterator<Item = (String, String)>,
        data: Option<Vec<u8>>,
    ) -> Result<Self, Invalid> {
        if name.len() > MAX_NBT_STRING_BYTES {
            return Err(Invalid::StringTooLong { len: name.len() });
        }
        if Identifier::parse_namespaced(name).is_none() {
            return Err(Invalid::BlockName);
        }
        if AIR_STATES.contains(&name) {
            return Err(Invalid::AirCell);
        }
        let mut sorted: Vec<(Box<str>, Box<str>)> = Vec::new();
        for (key, value) in properties {
            if let Some(len) = [key.len(), value.len()]
                .into_iter()
                .find(|len| *len > MAX_NBT_STRING_BYTES)
            {
                return Err(Invalid::StringTooLong { len });
            }
            if !valid_property_token(&key) || !valid_property_token(&value) {
                return Err(Invalid::InvalidProperty);
            }
            sorted.push((key.into_boxed_str(), value.into_boxed_str()));
        }
        sorted.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(Invalid::DuplicateProperty);
        }
        if let Some(bytes) = &data
            && bytes.len() > MAX_CELL_DATA_BYTES
        {
            return Err(Invalid::CellData { len: bytes.len() });
        }
        Ok(Self {
            pos,
            name: name.into(),
            properties: sorted.into_boxed_slice(),
            data: data.map(Vec::into_boxed_slice),
        })
    }

    /// The cell's local position.
    pub fn pos(&self) -> LocalPos {
        self.pos
    }
    /// The canonical namespaced block identifier, without properties.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The state properties, sorted by name without duplicates.
    pub fn properties(&self) -> &[(Box<str>, Box<str>)] {
        &self.properties
    }
    /// The block-entity payload, if the block has one. `Some(&[])` and
    /// `None` are distinct and both round-trip.
    pub fn data(&self) -> Option<&[u8]> {
        self.data.as_deref()
    }
    /// The decoded byte count this cell contributes to the carrier
    /// payload budget: its block-entity payload alone.
    pub fn data_len(&self) -> usize {
        self.data.as_ref().map_or(0, |data| data.len())
    }
}

/// A nonempty property name or value drawn from `[a-z0-9_]`, the pattern
/// every vanilla registry token matches. This syntax check adds no length
/// bound; the shared NBT string limit is checked at the owning boundary.
fn valid_property_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A level-owned carrier: UUID, generation, simulation revision, rigid
/// transform and sorted duplicate-free local cells. The transform is the
/// contact kernel's [`RigidTransform`], stored in canonical form —
/// normalized quaternion, and no negative zeros anywhere, so the exact
/// bits survive a codec round trip.
#[derive(Clone, Debug, PartialEq)]
pub struct Carrier {
    uuid: u128,
    generation: u32,
    revision: u64,
    transform: RigidTransform,
    cells: Vec<Cell>,
}
impl Carrier {
    /// Validates and builds a carrier. The cell vector is checked against
    /// [`MAX_CELLS`] before anything is allocated, then sorted by position
    /// (`sort_unstable` allocates nothing); duplicate positions and a
    /// total payload past [`MAX_CARRIER_DATA_BYTES`] are refused. The
    /// transform is rebuilt in canonical form: finite, with the
    /// quaternion's exact bits kept and negative zeros normalized to
    /// positive zero the way the NBT reader does on load.
    pub fn new(
        uuid: u128,
        generation: u32,
        revision: u64,
        transform: RigidTransform,
        cells: Vec<Cell>,
    ) -> Result<Self, Invalid> {
        let transform = canonical_transform(transform)?;
        if cells.len() > MAX_CELLS {
            return Err(Invalid::CellCount { count: cells.len() });
        }
        let mut cells = cells;
        cells.sort_unstable_by_key(|cell| cell.pos);
        if cells.windows(2).any(|pair| pair[0].pos == pair[1].pos) {
            return Err(Invalid::DuplicateCell);
        }
        let total = cells.iter().map(Cell::data_len).sum();
        if total > MAX_CARRIER_DATA_BYTES {
            return Err(Invalid::CarrierData { total });
        }
        Ok(Self {
            uuid,
            generation,
            revision,
            transform,
            cells,
        })
    }

    /// The carrier's UUID.
    pub fn uuid(&self) -> u128 {
        self.uuid
    }
    /// The carrier's generation; stale generations refuse replayed handles.
    pub fn generation(&self) -> u32 {
        self.generation
    }
    /// The simulation revision at the time of the snapshot.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// The canonical rigid transform.
    pub fn transform(&self) -> RigidTransform {
        self.transform
    }
    /// The sorted, duplicate-free local cells.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }
}

/// Rebuilds a valid in-memory transform in canonical form. A
/// [`RigidTransform`] from the contact kernel is already finite with a
/// normalized quaternion, but its doubles may hold negative zeros
/// (`-0.0 + 0.0` is `0.0`; every other finite value is unchanged), which
/// the NBT reader canonicalizes on load — normalizing them here keeps
/// `decode(encode(carrier))` bitwise exact for every carrier.
fn canonical_transform(transform: RigidTransform) -> Result<RigidTransform, Invalid> {
    let translation = transform.translation();
    let pivot = transform.pivot();
    stored_transform(
        [translation.x, translation.y, translation.z],
        [pivot.x, pivot.y, pivot.z],
        transform.rotation().components(),
    )
}

/// Validates ten transform doubles from an untrusted stored form and
/// rebuilds a [`RigidTransform`] without renormalizing the rotation:
/// renormalizing a quaternion the encoder already normalized can move its
/// last bit, breaking the exact round-trip contract, so instead the
/// quaternion must already be the canonical normalized form — first
/// nonzero component nonnegative, norm within
/// [`QUATERNION_NORM_TOLERANCE`] of one. A stored quaternion merely close
/// to unit (within the tolerance) is accepted as-is; it round-trips to
/// its own exact bits, and its effect on rotation stays within rounding.
pub(crate) fn stored_transform(
    translation: [f64; 3],
    pivot: [f64; 3],
    rotation: [f64; 4],
) -> Result<RigidTransform, Invalid> {
    if !translation
        .iter()
        .chain(pivot.iter())
        .chain(rotation.iter())
        .all(|v| v.is_finite())
    {
        return Err(Invalid::NonFiniteTransform);
    }
    let [w, x, y, z] = rotation;
    if w == 0.0 && x == 0.0 && y == 0.0 && z == 0.0 {
        return Err(Invalid::ZeroQuaternion);
    }
    if let Some(first) = rotation.iter().find(|v| **v != 0.0)
        && *first < 0.0
    {
        return Err(Invalid::QuaternionNotCanonical);
    }
    let norm_squared = w * w + x * x + y * y + z * z;
    if (norm_squared - 1.0).abs() > QUATERNION_NORM_TOLERANCE {
        return Err(Invalid::QuaternionNotCanonical);
    }
    // Untouched bits, except that `from_exact_components` canonicalizes
    // negative zeros; the finite and nonzero checks above cannot fail
    // here, so the error mapping is unreachable belt-and-braces.
    let rotation =
        Rotation::from_exact_components(w, x, y, z).ok_or(Invalid::NonFiniteTransform)?;
    let zero_canonical = |v: [f64; 3]| Vec3::new(v[0] + 0.0, v[1] + 0.0, v[2] + 0.0);
    RigidTransform::new(zero_canonical(translation), zero_canonical(pivot), rotation)
        .map_err(|_| Invalid::NonFiniteTransform)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_1_SQRT_2;

    fn identity_transform() -> RigidTransform {
        RigidTransform::identity_at(Vec3::new(10.0, 64.0, 10.0)).unwrap()
    }

    fn stone(pos: LocalPos) -> Cell {
        Cell::new(pos, "minecraft:stone", Vec::new(), None).unwrap()
    }

    #[test]
    fn stores_cells_sorted_without_duplicates() {
        let carrier = Carrier::new(
            1,
            0,
            0,
            identity_transform(),
            vec![
                stone(LocalPos::new(5, 0, 0)),
                stone(LocalPos::new(-5, 3, 2)),
                stone(LocalPos::new(0, 0, 0)),
            ],
        )
        .unwrap();
        let positions: Vec<LocalPos> = carrier.cells().iter().map(Cell::pos).collect();
        assert_eq!(
            positions,
            vec![
                LocalPos::new(-5, 3, 2),
                LocalPos::new(0, 0, 0),
                LocalPos::new(5, 0, 0)
            ]
        );
        assert_eq!(
            Carrier::new(
                1,
                0,
                0,
                identity_transform(),
                vec![stone(LocalPos::new(1, 1, 1)), stone(LocalPos::new(1, 1, 1))],
            ),
            Err(Invalid::DuplicateCell)
        );
    }

    #[test]
    fn refuses_empty_malformed_oversized_and_air_names() {
        for state in [
            "",
            "stone",           // not namespaced
            "minecraft:",      // empty path
            ":stone",          // empty namespace
            "Minecraft:Stone", // uppercase
        ] {
            assert_eq!(
                Cell::new(LocalPos::new(0, 0, 0), state, Vec::new(), None),
                Err(Invalid::BlockName),
                "{state:?}"
            );
        }
        for air in AIR_STATES {
            assert_eq!(
                Cell::new(LocalPos::new(0, 0, 0), air, Vec::new(), None),
                Err(Invalid::AirCell)
            );
            // Properties do not smuggle air in either.
            assert_eq!(
                Cell::new(
                    LocalPos::new(0, 0, 0),
                    air,
                    vec![("facing".to_string(), "north".to_string())],
                    None
                ),
                Err(Invalid::AirCell)
            );
        }
        // A long block identifier is legal: neither the identifier
        // charset nor the declaration boundary bounds its length.
        let long = format!("minecraft:{}", "a".repeat(1015));
        assert_eq!(long.len(), 1025);
        assert!(Cell::new(LocalPos::new(0, 0, 0), &long, Vec::new(), None).is_ok());
    }

    #[test]
    fn cell_payload_limit_is_checked_at_construction() {
        // Literal bounds: changing the constant must fail this test.
        assert!(
            Cell::new(
                LocalPos::new(0, 0, 0),
                "minecraft:stone",
                Vec::new(),
                Some(vec![0; 65536])
            )
            .is_ok()
        );
        assert_eq!(
            Cell::new(
                LocalPos::new(0, 0, 0),
                "minecraft:stone",
                Vec::new(),
                Some(vec![0; 65537])
            ),
            Err(Invalid::CellData { len: 65537 })
        );
    }

    #[test]
    fn carrier_cell_count_limit_is_checked_before_allocation() {
        let cells = (0..4096)
            .map(|i| {
                stone(LocalPos::new(
                    (i % 256 - 128) as i8,
                    (i / 256 - 8) as i8,
                    (i / 1024 - 4) as i8,
                ))
            })
            .collect();
        assert!(Carrier::new(1, 0, 0, identity_transform(), cells).is_ok());
        let over = (0..4097)
            .map(|i| {
                stone(LocalPos::new(
                    (i % 256 - 128) as i8,
                    (i / 256 - 8) as i8,
                    (i / 1024 - 4) as i8,
                ))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            Carrier::new(1, 0, 0, identity_transform(), over),
            Err(Invalid::CellCount { count: 4097 })
        );
    }

    #[test]
    fn carrier_payload_budget_is_checked() {
        // 256 cells of 65,536 bytes fill the 16,777,216-byte budget.
        let full = (0..256)
            .map(|i| {
                Cell::new(
                    LocalPos::new(i as i8, 0, 0),
                    "minecraft:stone",
                    Vec::new(),
                    Some(vec![0; 65536]),
                )
                .unwrap()
            })
            .collect::<Vec<_>>();
        assert!(Carrier::new(1, 0, 0, identity_transform(), full.clone()).is_ok());
        let mut over = full;
        let mut last = stone(LocalPos::new(127, 127, 127));
        last.data = Some(vec![0; 1].into());
        over.push(last);
        assert_eq!(
            Carrier::new(1, 0, 0, identity_transform(), over),
            Err(Invalid::CarrierData { total: 16777217 })
        );
    }

    fn oak_stairs_properties() -> Vec<(String, String)> {
        vec![
            ("waterlogged".to_string(), "false".to_string()),
            ("half".to_string(), "bottom".to_string()),
            ("facing".to_string(), "north".to_string()),
            ("shape".to_string(), "straight".to_string()),
        ]
    }

    #[test]
    fn properties_are_stored_sorted_regardless_of_input_order() {
        let cell = Cell::new(
            LocalPos::new(0, 0, 0),
            "minecraft:oak_stairs",
            oak_stairs_properties(),
            None,
        )
        .unwrap();
        let names: Vec<&str> = cell
            .properties()
            .iter()
            .map(|(key, _)| key.as_ref())
            .collect();
        assert_eq!(names, ["facing", "half", "shape", "waterlogged"]);
        // The same state built in canonical order is equal.
        let sorted = Cell::new(
            LocalPos::new(0, 0, 0),
            "minecraft:oak_stairs",
            vec![
                ("facing".to_string(), "north".to_string()),
                ("half".to_string(), "bottom".to_string()),
                ("shape".to_string(), "straight".to_string()),
                ("waterlogged".to_string(), "false".to_string()),
            ],
            None,
        )
        .unwrap();
        assert_eq!(cell, sorted);
    }

    #[test]
    fn refuses_bad_property_lists() {
        let cell = |properties: Vec<(String, String)>| {
            Cell::new(
                LocalPos::new(0, 0, 0),
                "minecraft:oak_stairs",
                properties,
                None,
            )
        };
        assert_eq!(
            cell(vec![
                ("facing".to_string(), "north".to_string()),
                ("facing".to_string(), "south".to_string()),
            ]),
            Err(Invalid::DuplicateProperty)
        );
        // Empty value and empty name.
        assert_eq!(
            cell(vec![("facing".to_string(), String::new())]),
            Err(Invalid::InvalidProperty)
        );
        assert_eq!(
            cell(vec![(String::new(), "north".to_string())]),
            Err(Invalid::InvalidProperty)
        );
        // Uppercase in the value, dash in the name.
        assert_eq!(
            cell(vec![("facing".to_string(), "North".to_string())]),
            Err(Invalid::InvalidProperty)
        );
        assert_eq!(
            cell(vec![("not-a-key".to_string(), "north".to_string())]),
            Err(Invalid::InvalidProperty)
        );
        // Long names and values are legal, and so are many properties:
        // the declaration boundary sets no count or length, so a
        // 65-letter name (past every vanilla maximum) is just a state.
        assert!(
            cell(vec![
                ("a".repeat(65), "b".repeat(66)),
                (
                    "creaking_heart_state".to_string(),
                    "waiting_for_reward_ejection".to_string()
                ),
            ])
            .is_ok()
        );
        assert!(
            cell(
                (0..200)
                    .map(|i| (format!("p{i:03}"), "1".to_string()))
                    .collect()
            )
            .is_ok()
        );
    }

    #[test]
    fn negated_quaternion_components_collapse_to_one_rotation() {
        let (w, x, y, z) = (0.6, 0.3, 0.2, 0.1);
        let positive = Rotation::from_components(w, x, y, z).unwrap();
        let negative = Rotation::from_components(-w, -x, -y, -z).unwrap();
        assert_eq!(positive, negative);
        // The stored form is canonical: first nonzero nonnegative.
        assert!(negative.components()[0] > 0.0);
        let carrier = |rotation: Rotation| {
            Carrier::new(
                1,
                0,
                0,
                RigidTransform::new(Vec3::ZERO, Vec3::ZERO, rotation).unwrap(),
                vec![],
            )
            .unwrap()
        };
        assert_eq!(carrier(positive), carrier(negative));
    }

    #[test]
    fn negative_zeros_are_canonicalized_on_construction() {
        // from_components keeps a negative zero in a non-leading slot, and
        // RigidTransform::new accepts it; the carrier canonicalizes both.
        let rotation = Rotation::from_components(1.0, -0.0, 0.0, 0.0).unwrap();
        assert!(rotation.components()[1].is_sign_negative());
        let transform = RigidTransform::new(
            Vec3::new(-0.0, 1.0, -0.0),
            Vec3::new(0.0, -0.0, 2.0),
            rotation,
        )
        .unwrap();
        let carrier = Carrier::new(1, 0, 0, transform, vec![]).unwrap();
        let stored = carrier.transform();
        for value in [
            stored.translation().x,
            stored.translation().z,
            stored.pivot().y,
            stored.rotation().components()[1],
        ] {
            assert!(value == 0.0 && !value.is_sign_negative());
        }
    }

    #[test]
    fn stored_transform_refuses_untrusted_doubles() {
        let translation = [1.0, 2.0, 3.0];
        let pivot = [0.0; 3];
        assert_eq!(
            stored_transform([f64::NAN, 2.0, 3.0], pivot, [1.0, 0.0, 0.0, 0.0]),
            Err(Invalid::NonFiniteTransform)
        );
        assert_eq!(
            stored_transform(translation, [f64::INFINITY, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]),
            Err(Invalid::NonFiniteTransform)
        );
        assert_eq!(
            stored_transform(translation, pivot, [f64::NAN, 0.0, 0.0, 0.0]),
            Err(Invalid::NonFiniteTransform)
        );
        assert_eq!(
            stored_transform(translation, pivot, [0.0; 4]),
            Err(Invalid::ZeroQuaternion)
        );
        // Sign-flipped canonical form.
        assert_eq!(
            stored_transform(
                translation,
                pivot,
                [-FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2, 0.0]
            ),
            Err(Invalid::QuaternionNotCanonical)
        );
        // Finite, nonzero, sign-canonical, but not unit.
        assert_eq!(
            stored_transform(translation, pivot, [2.0, 0.0, 0.0, 0.0]),
            Err(Invalid::QuaternionNotCanonical)
        );
    }

    #[test]
    fn stored_transform_keeps_exact_bits() {
        // The encoder's own output always passes: a quaternion normalized
        // by Rotation::from_components, with no negative zeros.
        let rotation = Rotation::from_axis_angle(Vec3::new(1.0, 2.0, 3.0), 0.7).unwrap();
        let components = rotation.components();
        let rebuilt = stored_transform([1.5, -2.5, 3.5], [0.25, -0.25, 0.75], components).unwrap();
        assert_eq!(
            rebuilt.rotation().components().map(f64::to_bits),
            components.map(f64::to_bits)
        );
        assert_eq!(rebuilt.translation(), Vec3::new(1.5, -2.5, 3.5));
        assert_eq!(rebuilt.pivot(), Vec3::new(0.25, -0.25, 0.75));
    }
}
