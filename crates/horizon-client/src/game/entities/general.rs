//! General entity rows (arismoko/horizon#792 stage 2): the record codec for mod-declared
//! entity types. The record is the save format's unit: common fields every
//! kind shares, plus the type envelope (namespaced key, payload schema
//! version, payload bytes) and the passenger tree. Validation runs every
//! check before any of it may reach a row; a refusal names the first fault
//! and never panics, clamps or truncates into a valid-looking entity.

/// The design's bounds, re-exported from the one `limits` block the
/// registrar quotes (`mod_api::general_entity_limits`).
pub use mod_api::general_entity_limits as limits;

use super::Game;
use super::commands::Target;
use super::entity::Entity;
use super::entity_initialization::TransferredEntityState;
use super::persist::ChunkKey;
use horizon_client::entity_rules::EntityRules;
use horizon_client::impl_joined;
use horizon_client::join_seq::{JoinList, JoinSeq};
use horizon_core::fast_hash::{FxHashMap, FxHashSet};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_core::nbt::Tag;
use horizon_world::move_gate::MoveGate;
use std::borrow::Cow;
use std::collections::HashSet;

/// One general entity as a record: what save writes and load reads. The
/// engine's live row (`GeneralRow`, stage 3) is built from a validated
/// record; this type never holds runtime ids, slots or pointers.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralEntityRecord {
    /// The entity's position in blocks.
    pub position: [f64; 3],
    /// The entity's velocity in blocks per tick.
    pub velocity: [f64; 3],
    /// The entity's yaw and pitch in degrees.
    pub rotation: [f32; 2],
    /// The entity's UUID, the identity a reload continues.
    pub uuid: [i32; 4],
    /// The type envelope: which registered type owns the payload, at which
    /// schema version, with which bounded payload bytes.
    pub envelope: GeneralTypeEnvelope,
    /// The entities riding this one, stored inside its record.
    pub passengers: Vec<GeneralEntityRecord>,
}

/// The mod-owned half of a record: the registered type's namespaced key, the
/// payload's schema version and the payload bytes themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct GeneralTypeEnvelope {
    /// The type's namespaced id (`mymod:robot`), as declared.
    pub type_key: String,
    /// The payload's schema version, as declared by the type.
    pub payload_version: u16,
    /// The payload bytes, at most [`limits::PAYLOAD_LIMIT`] long.
    pub payload_bytes: Vec<u8>,
}

/// Why a general entity record was refused before any of it could reach a
/// row. Each fault names the record's uuid in its diagnostic string.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneralRecordError {
    /// A mod payload exceeds [`limits::PAYLOAD_LIMIT`]. The bound applies to
    /// mod payloads only, never to a record of a vanilla type.
    PayloadTooLarge { size: usize },
    /// The decoded data of a payload is nested deeper than
    /// [`limits::MAX_NESTING`]. The record's own NBT walk fires this when a
    /// record carries data nested past the bound; the passenger tree's depth
    /// has its own, stricter refusal.
    NestingTooDeep { depth: u32 },
    /// A passenger chain is deeper than [`limits::MAX_PASSENGER_DEPTH`].
    PassengerDepthTooDeep { depth: u32 },
    /// A passenger's UUID is one of its own ancestors: a cycle.
    PassengerCycle { uuid: [i32; 4] },
    /// The same UUID appears twice in one record tree.
    DuplicateUuid { uuid: [i32; 4] },
    /// A position, velocity or rotation component is not finite.
    NotFinite,
    /// The type envelope is not what the format writes: a field of the wrong
    /// tag kind or shape (`Payload` not a byte array, `Pos` not three
    /// doubles, a passenger list of things that are not compounds). The
    /// record never reaches a row and nothing of it is kept.
    InvalidEnvelope { field: &'static str },
}

impl GeneralRecordError {
    /// The refusal's diagnostic, for the load summary.
    pub fn message(&self) -> String {
        match self {
            GeneralRecordError::PayloadTooLarge { size } => format!(
                "general entity payload {size} exceeds {}",
                limits::PAYLOAD_LIMIT
            ),
            GeneralRecordError::NestingTooDeep { depth } => format!(
                "general entity nesting {depth} exceeds {}",
                limits::MAX_NESTING
            ),
            GeneralRecordError::PassengerDepthTooDeep { depth } => format!(
                "general entity passenger depth {depth} exceeds {}",
                limits::MAX_PASSENGER_DEPTH
            ),
            GeneralRecordError::PassengerCycle { uuid } => {
                format!("general entity passenger cycle at {uuid:?}")
            }
            GeneralRecordError::DuplicateUuid { uuid } => {
                format!("general entity duplicate uuid {uuid:?}")
            }
            GeneralRecordError::NotFinite => {
                "general entity has a non-finite coordinate".to_owned()
            }
            GeneralRecordError::InvalidEnvelope { field } => {
                format!("general entity envelope field {field:?} is malformed")
            }
        }
    }
}

/// The outcome of checking a record against every bound: ready to become a
/// row, or structurally sound but of a type this load cannot resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneralRecordCheck {
    /// Every check passed; the record may reach a row.
    Ready,
    /// Every structural check passed, but the type key's namespace belongs
    /// to neither vanilla nor a loaded mod. Stage 3 keeps such a record
    /// byte for byte as quarantined data; it never reaches a row here.
    UnknownType,
}

/// Check one record tree against every bound, before any of it may reach a
/// row. `is_known_type` answers whether a type key's namespace belongs to
/// vanilla or a loaded mod; the caller supplies the frozen registry's view.
///
/// Structural faults (payload size, passenger depth, cycles,
/// duplicate UUIDs, non-finite coordinates) refuse the tree whatever its
/// type; only a structurally sound record of an unresolvable type reports
/// [`GeneralRecordCheck::UnknownType`], so quarantine never preserves an
/// over-budget or malformed payload. The design's separate data-nesting
/// bound is the payload decode's to check (stage 3), not this walk's: a
/// passenger tree reaches the passenger-depth refusal first. Nothing is
/// written, allocated beyond the walk's own bookkeeping, or panicked on; a
/// refused tree leaves no partial state, and the next valid record checks
/// cleanly.
pub fn check_general_record(
    record: &GeneralEntityRecord,
    is_known_type: impl Fn(&str) -> bool,
) -> Result<GeneralRecordCheck, GeneralRecordError> {
    let mut seen = FxHashSet::default();
    check_record(record, &mut Vec::new(), &mut seen, 1, &is_known_type)
}

fn check_record(
    record: &GeneralEntityRecord,
    ancestors: &mut Vec<[i32; 4]>,
    seen: &mut FxHashSet<[i32; 4]>,
    depth: u32,
    is_known_type: &impl Fn(&str) -> bool,
) -> Result<GeneralRecordCheck, GeneralRecordError> {
    if !record
        .position
        .iter()
        .chain(record.velocity.iter())
        .all(|component| component.is_finite())
        || !record
            .rotation
            .iter()
            .all(|component| component.is_finite())
    {
        return Err(GeneralRecordError::NotFinite);
    }
    if record.envelope.payload_bytes.len() > limits::PAYLOAD_LIMIT {
        return Err(GeneralRecordError::PayloadTooLarge {
            size: record.envelope.payload_bytes.len(),
        });
    }
    if depth - 1 > limits::MAX_PASSENGER_DEPTH {
        return Err(GeneralRecordError::PassengerDepthTooDeep { depth: depth - 1 });
    }
    if ancestors.contains(&record.uuid) {
        return Err(GeneralRecordError::PassengerCycle { uuid: record.uuid });
    }
    if !seen.insert(record.uuid) {
        return Err(GeneralRecordError::DuplicateUuid { uuid: record.uuid });
    }
    ancestors.push(record.uuid);
    let mut unknown = false;
    for passenger in &record.passengers {
        match check_record(passenger, ancestors, seen, depth + 1, is_known_type)? {
            GeneralRecordCheck::Ready => {}
            GeneralRecordCheck::UnknownType => unknown = true,
        }
    }
    ancestors.pop();
    if unknown || !is_known_type(&record.envelope.type_key) {
        return Ok(GeneralRecordCheck::UnknownType);
    }
    Ok(GeneralRecordCheck::Ready)
}

/// The save format's common field names, vanilla's own (`Entity.saveWithoutId`)
/// for the fields every kind shares, and the envelope's names for the rest.
mod format {
    /// The type's namespaced id (`id`, as every entity compound carries it).
    pub const TYPE: &str = "id";
    /// `Pos`: three doubles.
    pub const POSITION: &str = "Pos";
    /// `Motion`: three doubles.
    pub const VELOCITY: &str = "Motion";
    /// `Rotation`: two floats (yaw, pitch).
    pub const ROTATION: &str = "Rotation";
    /// `UUID`: four ints.
    pub const UUID: &str = "UUID";
    /// The payload's schema version, a short.
    pub const PAYLOAD_VERSION: &str = "PayloadVersion";
    /// The payload bytes, a byte array.
    pub const PAYLOAD: &str = "Payload";
    /// `Passengers`: the riding records, stored inside their vehicle's.
    pub const PASSENGERS: &str = "Passengers";
}

fn doubles(values: [f64; 3]) -> Tag {
    Tag::List(values.map(|value| Tag::Double(value.to_bits())).to_vec())
}

/// Read one three-double field the format writes (`Pos`, `Motion`), with its
/// vanilla default when the compound omits it.
fn doubles3(tag: &Tag, field: &'static str) -> Result<[f64; 3], GeneralRecordError> {
    let Some(Tag::List(values)) = tag.get(field) else {
        return Ok([0.0; 3]);
    };
    if values.len() != 3 {
        return Err(GeneralRecordError::InvalidEnvelope { field });
    }
    let mut out = [0.0; 3];
    for (slot, value) in values.iter().enumerate() {
        let Tag::Double(bits) = value else {
            return Err(GeneralRecordError::InvalidEnvelope { field });
        };
        out[slot] = f64::from_bits(*bits);
    }
    Ok(out)
}

impl GeneralEntityRecord {
    /// The record as one save compound: the common fields under their vanilla
    /// names, the type envelope (`id`, `PayloadVersion`, `Payload`), and the
    /// passenger tree inside `Passengers`, exactly what save writes and load
    /// reads. Runtime slots, network ids and pointers never appear.
    pub fn to_tag(&self) -> Tag {
        let mut map = horizon_core::nbt::Compound::new();
        map.insert(
            format::TYPE.into(),
            Tag::String(self.envelope.type_key.as_str().into()),
        );
        map.insert(format::POSITION.into(), doubles(self.position));
        map.insert(format::VELOCITY.into(), doubles(self.velocity));
        map.insert(
            format::ROTATION.into(),
            Tag::List(
                self.rotation
                    .map(|value| Tag::Float(value.to_bits()))
                    .to_vec(),
            ),
        );
        map.insert(format::UUID.into(), Tag::IntArray(self.uuid.to_vec()));
        map.insert(
            format::PAYLOAD_VERSION.into(),
            Tag::Short(i16::from_ne_bytes(
                self.envelope.payload_version.to_ne_bytes(),
            )),
        );
        map.insert(
            format::PAYLOAD.into(),
            Tag::ByteArray(self.envelope.payload_bytes.clone()),
        );
        if !self.passengers.is_empty() {
            map.insert(
                format::PASSENGERS.into(),
                Tag::List(self.passengers.iter().map(Self::to_tag).collect()),
            );
        }
        Tag::Compound(map)
    }
}

/// The deepest nesting of one record compound's own data, the passenger
/// wrappers excluded: each riding record is a compound of the same shape as
/// its root, so its depth is checked where it is read, and the passenger
/// chain has its own bound ([`limits::MAX_PASSENGER_DEPTH`]).
fn data_nesting(tag: &Tag, passengers_key: &horizon_core::nbt::JavaString) -> u32 {
    match tag {
        Tag::Compound(map) => {
            let riders = map.get(passengers_key);
            1 + map
                .values()
                .filter(|value| !riders.is_some_and(|riding| std::ptr::eq(*value, riding)))
                .map(|value| data_nesting(value, passengers_key))
                .max()
                .unwrap_or(0)
        }
        Tag::List(values) => {
            1 + values
                .iter()
                .map(|value| data_nesting(value, passengers_key))
                .max()
                .unwrap_or(0)
        }
        _ => 1,
    }
}

/// The encoded NBT size of a tag, as its writer emits it (name payloads
/// included, strings counted as their UTF-8 bytes): the measure the
/// quarantine record bound applies.
pub(super) fn nbt_size(tag: &Tag) -> u64 {
    match tag {
        Tag::End => 1,
        Tag::Byte(_) => 1,
        Tag::Short(_) => 2,
        Tag::Int(_) => 4,
        Tag::Long(_) => 8,
        Tag::Float(_) => 4,
        Tag::Double(_) => 8,
        Tag::ByteArray(bytes) => 4 + bytes.len() as u64,
        Tag::String(text) => 2 + text.to_string_lossy().len() as u64,
        Tag::List(values) => 5 + values.iter().map(nbt_size).sum::<u64>(),
        Tag::Compound(map) => {
            1 + map
                .iter()
                .map(|(field, value)| {
                    1 + 2 + field.to_string_lossy().len() as u64 + nbt_size(value)
                })
                .sum::<u64>()
        }
        Tag::IntArray(ints) => 4 + ints.len() as u64 * 4,
        Tag::LongArray(longs) => 4 + longs.len() as u64 * 8,
    }
}

/// One record compound as the save format reads it: the common fields (their
/// vanilla defaults when absent), the type envelope and the passenger tree.
/// The record's own data must nest no deeper than [`limits::MAX_NESTING`];
/// shape faults refuse with [`GeneralRecordError::InvalidEnvelope`], and
/// nothing is read from a record that refused.
fn record_from_compound(tag: &Tag) -> Result<GeneralEntityRecord, GeneralRecordError> {
    let passengers_key = horizon_core::nbt::JavaString::from(format::PASSENGERS);
    let nesting = data_nesting(tag, &passengers_key);
    if nesting > limits::MAX_NESTING {
        return Err(GeneralRecordError::NestingTooDeep { depth: nesting });
    }
    let Some(Tag::String(type_key)) = tag.get(format::TYPE) else {
        return Err(GeneralRecordError::InvalidEnvelope {
            field: format::TYPE,
        });
    };
    let position = doubles3(tag, format::POSITION)?;
    let velocity = doubles3(tag, format::VELOCITY)?;
    let rotation = match tag.get(format::ROTATION) {
        None => [0.0; 2],
        Some(Tag::List(values)) if values.len() == 2 => {
            let mut out = [0.0f32; 2];
            for (slot, value) in values.iter().enumerate() {
                let Tag::Float(bits) = value else {
                    return Err(GeneralRecordError::InvalidEnvelope {
                        field: format::ROTATION,
                    });
                };
                out[slot] = f32::from_bits(*bits);
            }
            out
        }
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::ROTATION,
            });
        }
    };
    let uuid = match tag.get(format::UUID) {
        Some(Tag::IntArray(ints)) if ints.len() == 4 => {
            let mut out = [0i32; 4];
            out.copy_from_slice(ints);
            out
        }
        _ => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::UUID,
            });
        }
    };
    let payload_version = match tag.get(format::PAYLOAD_VERSION) {
        None => 0,
        Some(Tag::Short(version)) => u16::from_ne_bytes(version.to_ne_bytes()),
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::PAYLOAD_VERSION,
            });
        }
    };
    let payload_bytes = match tag.get(format::PAYLOAD) {
        None => Vec::new(),
        Some(Tag::ByteArray(bytes)) => bytes.clone(),
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::PAYLOAD,
            });
        }
    };
    let passengers = match tag.get(format::PASSENGERS) {
        None => Vec::new(),
        Some(Tag::List(values)) if values.iter().all(|value| matches!(value, Tag::Compound(_))) => {
            values
                .iter()
                .map(record_from_compound)
                .collect::<Result<Vec<_>, _>>()?
        }
        Some(_) => {
            return Err(GeneralRecordError::InvalidEnvelope {
                field: format::PASSENGERS,
            });
        }
    };
    Ok(GeneralEntityRecord {
        position,
        velocity,
        rotation,
        uuid,
        envelope: GeneralTypeEnvelope {
            type_key: type_key.to_string_lossy(),
            payload_version,
            payload_bytes,
        },
        passengers,
    })
}

/// A saved general entity compound as a record: it and every compound of
/// its riding tree read under the format's checks, so a refusal names the
/// first fault and leaves nothing read behind (the caller discards the
/// whole tree).
/// The frozen type registry: every declared general entity type, resolved
/// once into dense slots. The registry is data — built from the installed
/// declarations and never hand-kept — and freezes with them: duplicates
/// across mods refuse at freeze, naming the type.
pub struct GeneralTypeCatalog {
    entries: Vec<mod_api::__private::GeneralEntityTypeDeclaration>,
    by_key: FxHashMap<String, usize>,
}

impl GeneralTypeCatalog {
    /// Freeze the installed declarations into slots, sorted by declaring
    /// mod id then type id, so every side running the same mods builds the
    /// same slot numbers. A duplicate type key refuses, naming it.
    pub fn freeze(
        declarations: &[mod_api::__private::GeneralEntityTypeDeclaration],
    ) -> Result<Self, String> {
        let mut sorted = declarations.to_vec();
        sorted.sort_by(|left, right| (&left.mod_id, &left.id).cmp(&(&right.mod_id, &right.id)));
        let mut by_key = FxHashMap::with_capacity_and_hasher(sorted.len(), Default::default());
        for (slot, declaration) in sorted.iter().enumerate() {
            // Enumeration already yields the catalog index without narrowing.
            if by_key.insert(declaration.id.clone(), slot).is_some() {
                return Err(format!(
                    "duplicate general entity type {} (declared by {})",
                    declaration.id, declaration.mod_id
                ));
            }
        }
        Ok(Self {
            entries: sorted,
            by_key,
        })
    }

    /// The slot a type key resolved to, if the registry holds it.
    pub fn slot(&self, type_key: &str) -> Option<usize> {
        self.by_key.get(type_key).copied()
    }

    /// How many types the registry froze; every slot is below this.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The type key a slot belongs to.
    pub fn key(&self, slot: usize) -> &str {
        &self.entries[slot].id
    }

    /// The type's declared width and height, for the row's bounds.
    pub fn dimensions(&self, slot: usize) -> (f64, f64) {
        let entry = &self.entries[slot];
        (entry.width, entry.height)
    }

    /// The type's declared save policy, the writer of a record consults: a
    /// `noSave` type's rows produce zero persisted records.
    pub fn save_policy(&self, slot: usize) -> mod_api::GeneralSavePolicy {
        self.entries[slot].save_policy
    }
}

/// The game's frozen registry, built from the installed declarations: the
/// types were declared before any world or connection opened, and the
/// installed set never changes after. The cached catalog is keyed on the
/// declarations' own address, so a process sees one build and a test thread
/// that swaps its installed parts (`TEST_PARTS`) resolves the new set
/// instead of the first one it touched.
pub fn general_type_catalog() -> &'static GeneralTypeCatalog {
    thread_local! {
        static CACHED: std::cell::RefCell<Option<(usize, &'static GeneralTypeCatalog)>> =
            const { std::cell::RefCell::new(None) };
    }
    let declarations = mod_api::__private::general_entity_type_declarations();
    let at = declarations.as_ptr() as usize;
    CACHED.with(|cached| {
        if let Some((frozen_at, catalog)) = *cached.borrow()
            && frozen_at == at
        {
            return catalog;
        }
        let catalog = Box::leak(Box::new(
            GeneralTypeCatalog::freeze(declarations).unwrap_or_else(|duplicate| {
                // Unreachable: a duplicate key was already refused at
                // registration — every mod declares through the one
                // registrar `register_all` runs (`parts/shared.rs`), so the
                // second mod's `declare_general_entity` sees the first's
                // key and returns `Duplicate`, never reaching here.
                unreachable!("duplicate general entity type at freeze: {duplicate}")
            }),
        ));
        *cached.borrow_mut() = Some((at, catalog));
        catalog
    })
}

/// The engine's live general rows (arismoko/horizon#792): identity, join identity, type
/// slot, common state and the attachment holder. No runtime slot or pointer
/// persists; identity-to-row lookup stays with `EntityTable`
/// (`entity_lookup`, `server_entity_lookup`), never a second index here.
#[derive(Clone, Debug)]
pub struct GeneralRow {
    /// The row's runtime network id, from the game's one counter.
    pub id: u32,
    /// The row's UUID; a reload continues the identity, not old handles.
    pub uuid: [i32; 4],
    /// The row's join identity: its admission into the resident table.
    pub join: JoinSeq,
    /// The frozen registry's slot for the row's type.
    pub type_slot: usize,
    /// The position in blocks.
    pub position: Vec3,
    /// Where the row stood when the last tick began.
    pub previous: Vec3,
    /// The velocity in blocks per tick.
    pub velocity: Vec3,
    /// The yaw and pitch in degrees.
    pub rotation: [f32; 2],
    /// How far the row has fallen; stage 3's load and stage 4's tick own it.
    pub fall_distance: f64,
    /// The payload's schema version, as the record carried it and the writer
    /// returns it: no decoder owns it before stage 4.
    pub payload_version: u16,
    /// The record's payload bytes, held as loaded and written back unchanged.
    pub payload_bytes: Vec<u8>,
    /// The row whose vehicle this one rides (`None` for a root): a rider
    /// saves inside its root's record, never as its own.
    pub mount: Option<u32>,
    /// The entity-owned attachment registration lifetime (arismoko/horizon#796: the sole
    /// Part-data store; there is no parallel entity data map).
    pub attachment_lifetime: horizon_mod_runtime::attachments::AttachmentLifetime,
}

impl EntityRules for GeneralRow {}
impl_joined!(GeneralRow);
horizon_mod_runtime::impl_attachment_lifetime!(GeneralRow);

/// The general rows' list, one per level, bound to residency like the other
/// families' lists.
pub type GeneralRows = JoinList<GeneralRow, true>;

/// The game's general entity store: one join list per level, wired once
/// into `resident_owners!` and `entity_lists!` as `Kind::General`.
#[derive(Default)]
pub struct GeneralStore {
    /// The level's general rows, in join order.
    pub rows: GeneralRows,
    /// The rows' rider adjacency, reused across the level's entity passes
    /// (`tick.rs` refreshes it).
    pub riders: RiderIndex,
    /// How many times a live row's mount was rewritten (`entity_stop_riding`
    /// is the only path): each change retires the rider adjacency, which the
    /// rows' own write stamp does not see.
    pub mount_writes: u64,
}

/// Root-to-rider adjacency of the general rows, as a first-child and
/// next-sibling index over the rows' places: rebuilt in place, and only
/// when the rows' storage changed or a mount was rewritten, so a stable
/// world's pass allocates nothing and visits each edge once. The sibling
/// chains keep the storage's own order, which is the join order — a
/// vehicle's riders ride in the order their tree loaded, as
/// `addPassenger`'s list does.
#[derive(Default)]
pub struct RiderIndex {
    stamp: Option<horizon_client::join_seq::ListStamp>,
    length: usize,
    mounts: u64,
    /// Each row's first rider's place; `None` carries none.
    pub(super) first_child: Vec<Option<usize>>,
    /// Each rider's next sibling's place on the same vehicle.
    pub(super) next_sibling: Vec<Option<usize>>,
    /// Each vehicle's latest rider of the ongoing rebuild.
    last_child: Vec<Option<usize>>,
    /// Each row's place by id, for the rebuild's edge pass.
    places: FxHashMap<u32, usize>,
}

impl RiderIndex {
    /// Re-derive the adjacency unless nothing changed since the last pass:
    /// one pass over the rows, each edge written once, no allocation while
    /// the row count and the mounts hold still.
    pub fn refresh(&mut self, rows: &GeneralRows, mounts: u64) {
        let stamp = rows.stamp();
        if self.stamp == Some(stamp) && self.length == rows.len() && self.mounts == mounts {
            return;
        }
        self.stamp = Some(stamp);
        self.length = rows.len();
        self.mounts = mounts;
        // The buffers are reused — clear first and resize back, keeping the
        // capacity: a plain resize keeps the surviving slots' old links too,
        // so a row list that shrank leaves links to places that no longer
        // exist, and the next pass would walk off the end.
        self.first_child.clear();
        self.first_child.resize(rows.len(), None);
        self.next_sibling.clear();
        self.next_sibling.resize(rows.len(), None);
        self.last_child.clear();
        self.last_child.resize(rows.len(), None);
        self.places.clear();
        for (place, row) in rows.iter().enumerate() {
            self.places.insert(row.id, place);
        }
        for (place, row) in rows.iter().enumerate() {
            let Some(vehicle) = row.mount else {
                continue;
            };
            let Some(&vehicle_place) = self.places.get(&vehicle) else {
                continue;
            };
            match self.last_child[vehicle_place] {
                // The chain's tail takes the new rider; the head opens it.
                Some(tail) => self.next_sibling[tail] = Some(place),
                None => self.first_child[vehicle_place] = Some(place),
            }
            self.last_child[vehicle_place] = Some(place);
        }
    }
}

impl Game<'_> {
    /// One general row as a record: its own state, and its riders' records
    /// collected from their mount links — a rider is a row like its root and
    /// saves inside the root's record, never on its own. `visited` keeps a
    /// degenerate mount graph (not a tree) from recursing into itself.
    fn general_row_record(
        &self,
        id: u32,
        visited: &mut HashSet<u32>,
    ) -> Option<GeneralEntityRecord> {
        let rows = &self.resident_entities.general.rows;
        let row = rows.iter().find(|row| row.id == id)?;
        if general_type_catalog().save_policy(row.type_slot) == mod_api::GeneralSavePolicy::NoSave
            || !visited.insert(row.id)
        {
            return None;
        }
        let mut record = GeneralEntityRecord {
            position: [row.position.x, row.position.y, row.position.z],
            velocity: [row.velocity.x, row.velocity.y, row.velocity.z],
            rotation: row.rotation,
            uuid: row.uuid,
            envelope: GeneralTypeEnvelope {
                type_key: general_type_catalog().key(row.type_slot).to_owned(),
                payload_version: row.payload_version,
                payload_bytes: row.payload_bytes.clone(),
            },
            passengers: Vec::new(),
        };
        for rider in rows.iter().filter(|row| row.mount == Some(id)) {
            if let Some(rider_record) = self.general_row_record(rider.id, visited) {
                record.passengers.push(rider_record);
            }
        }
        Some(record)
    }

    /// Write each riding row's attachments into its own record inside the
    /// saved tree (`save_lists!` writes the root's): a rider's identity
    /// travels with its record, matched by UUID inside `Passengers`.
    fn write_general_rider_attachments(&self, tag: &mut Tag, root: u32) {
        let rows = &self.resident_entities.general.rows;
        let Some(Tag::List(riders)) = tag.get_mut(format::PASSENGERS) else {
            return;
        };
        for rider in riders.iter_mut() {
            let Some(Tag::IntArray(words)) = rider.get(format::UUID) else {
                continue;
            };
            if words.len() != 4 {
                continue;
            }
            let uuid = [words[0], words[1], words[2], words[3]];
            let Some(row) = rows
                .iter()
                .find(|row| row.uuid == uuid && row.mount == Some(root))
            else {
                continue;
            };
            crate::game::persist::write_attachments(rider, row.id, &self.entity_attachments);
            self.write_general_rider_attachments(rider, row.id);
        }
    }

    /// The unknown-kind gate of the load pass (`spawn_loaded_entity`): may
    /// this record of a kind no listed family claims go to `load_other`,
    /// the existing preserve-byte-for-byte path? An unknown `minecraft:`
    /// kind and a bare vanilla name keep that existing path unchanged. A
    /// namespaced kind of a loaded mod that never declared the type is
    /// skipped with a warning (its own mod would own the record, and a
    /// table that is not its own must not keep it). A kind of a namespace
    /// no loaded mod owns — a missing mod's record — is preserved, within
    /// the quarantine bounds: a record over
    /// [`limits::QUARANTINE_RECORD_LIMIT`], or a chunk already holding
    /// [`limits::QUARANTINE_CHUNK_LIMIT`] of them, is skipped with a
    /// warning instead. What passes is preserved exactly as loaded: the
    /// unknown-kind path holds the compound untouched (`persist.raw`),
    /// never ticks it (an `OtherEntity` has no tick) and writes it back
    /// from that store.
    pub(in crate::game) fn general_quarantine_admits(
        &self,
        tag: &Tag,
        kind: &str,
        at: Vec3,
    ) -> bool {
        let Some((namespace, _)) = kind.split_once(':') else {
            return true;
        };
        if namespace == "minecraft" {
            return true;
        }
        // A declared root can have been handed back because a rider's mod
        // is absent. Apply namespace ownership to the unresolved tree types,
        // not to the already-resolved root.
        fn missing_types_owned(record: &GeneralEntityRecord) -> bool {
            let key = &record.envelope.type_key;
            (general_type_catalog().slot(key).is_some()
                || key
                    .split_once(':')
                    .is_some_and(|(ns, _)| ns != "minecraft" && !mod_namespace_loaded(ns)))
                && record.passengers.iter().all(missing_types_owned)
        }
        let missing_tree = general_type_catalog().slot(kind).is_some()
            && record_from_compound(tag).is_ok_and(|record| missing_types_owned(&record));
        if mod_namespace_loaded(namespace) && !missing_tree {
            log::warn!(
                "Horizon: {kind} names no type of loaded mod {namespace}; skipping its record"
            );
            return false;
        }
        let size = nbt_size(tag);
        if size > limits::QUARANTINE_RECORD_LIMIT {
            log::warn!(
                "Horizon: quarantined general entity record of {size} bytes exceeds {}; skipping it",
                limits::QUARANTINE_RECORD_LIMIT
            );
            return false;
        }
        let chunk = chunk_of(at);
        let quarantined_here = self
            .others
            .iter()
            .filter(|other| quarantined_kind(&other.kind) && chunk_of(other.position) == chunk)
            .count();
        if quarantined_here >= limits::QUARANTINE_CHUNK_LIMIT {
            log::warn!(
                "Horizon: chunk {:?} already holds {} quarantined general entity records; skipping the next",
                chunk,
                limits::QUARANTINE_CHUNK_LIMIT
            );
            return false;
        }
        true
    }

    /// Carry a general entity tree into another level
    /// (`Entity.teleportCrossDimension` for `Target::General`): the whole
    /// tree is validated before anything of it moves — the destination may
    /// not already hold a general row of one of its UUIDs — then each row
    /// leaves the source list and joins the destination's, its identity
    /// (id, UUID, attachments) unchanged but its join identity new, so no
    /// handle of the source level reaches it again. A failure after the
    /// rows left — the destination level cannot open — puts every row back
    /// in the source, at its place and with the join identity it had; a
    /// failure before leaves the tree untouched. The root lands at `at`;
    /// a rider keeps its own position until the tree's own tick moves it.
    pub(in crate::game) fn teleport_general_tree_across(
        &mut self,
        root_id: u32,
        destination: crate::game::Dim,
        at: [f64; 3],
    ) -> Result<bool, String> {
        let source = self.dim;
        if source == destination {
            return Ok(false);
        }
        // The whole tree, root first, each row exactly once: a mount link
        // this walk cannot reach from the root is not the tree's.
        let rows = &self.resident_entities.general.rows;
        let Some(root) = rows
            .iter()
            .position(|row| row.id == root_id && row.mount.is_none())
        else {
            return Ok(false);
        };
        let mut tree = vec![root];
        let mut cursor = 0;
        while cursor < tree.len() {
            let vehicle = rows[tree[cursor]].id;
            for (index, row) in rows.iter().enumerate() {
                if row.mount == Some(vehicle) && !tree.contains(&index) {
                    tree.push(index);
                }
            }
            cursor += 1;
        }
        // Validate the destination before anything moves: no general row of
        // the tree's UUIDs may already live there.
        let uuids: HashSet<[i32; 4]> = tree.iter().map(|&index| rows[index].uuid).collect();
        if self
            .levels
            .parked_general_holds_any_uuid(destination, &uuids)
        {
            return Err(format!(
                "destination {} already holds a general entity of the tree's uuids",
                destination.id()
            ));
        }
        // Extract: highest index first, each row with the place it left.
        tree.sort_unstable();
        let mut extracted: Vec<(usize, GeneralRow)> = Vec::new();
        for &index in tree.iter().rev() {
            extracted.push((index, self.resident_entities.general.rows.remove(index)));
        }
        extracted.reverse();
        // The removals are leaves of the source level
        // (`PersistentEntitySectionManager.stopTracking`): flush them there.
        self.native_leaves_flush();
        // The only failing step: if the destination cannot open, every row
        // returns to the source, at its place and with its join identity.
        if let Err(error) = self.enter_level(destination) {
            for (index, row) in extracted {
                self.resident_entities.general.rows.insert(index, row);
            }
            return Err(format!("{error}"));
        }
        // The tree joins the destination: identities unchanged, join
        // identity new (a fresh admission — no source handle reaches it).
        for (_, mut row) in extracted {
            row.join = JoinSeq::UNJOINED;
            if row.id == root_id {
                row.position = Vec3::new(at[0], at[1], at[2]);
                row.previous = row.position;
            }
            self.resident_entities.general.rows.push(row);
        }
        if let Err(error) = self.enter_level(source) {
            log::error!("could not reopen {}: {error}", source.id());
        }
        Ok(true)
    }

    /// Load one saved general entity tree whose root kind the frozen
    /// registry holds: tag → record → every check, then the whole tree
    /// publishes through the same join boundary a fresh spawn takes. A
    /// structurally sound tree of types this load cannot resolve hands the
    /// tag back (`Err`), so the missing mod's record keeps byte for byte by
    /// the existing unknown-kind path; every other refusal — a malformed
    /// envelope, a structural fault, a UUID a live level entity already
    /// owns — drops the whole tree with a warning and publishes nothing.
    pub(in crate::game) fn load_general_tree(
        &mut self,
        tag: Tag,
        transferred: Option<TransferredEntityState>,
    ) -> Result<(), Tag> {
        let record = match record_from_compound(&tag) {
            Ok(record) => record,
            Err(error) => {
                log::warn!(
                    "Horizon: refusing general entity record: {}",
                    error.message()
                );
                return Ok(());
            }
        };
        let catalog = general_type_catalog();
        match check_general_record(&record, |key| catalog.slot(key).is_some()) {
            Ok(GeneralRecordCheck::Ready) => {}
            Ok(GeneralRecordCheck::UnknownType) => return Err(tag),
            Err(error) => {
                log::warn!(
                    "Horizon: refusing general entity record: {}",
                    error.message()
                );
                return Ok(());
            }
        }
        // A UUID a live entity of the level already owns refuses the whole
        // tree (`uuid_admission_refused`, the boundary every loaded kind
        // shares), before any row of it exists.
        if general_tree_uuid_admission_refused(self, &record) {
            log::warn!(
                "Horizon: refusing general entity record: a live entity owns its uuid {:?}",
                record.uuid
            );
            return Ok(());
        }
        self.insert_general_record(record, &tag, None, transferred);
        Ok(())
    }

    /// Insert one record as a row (and, recursively, its riders as rows of
    /// their vehicle): the same join list a fresh spawn pushes into, the
    /// same remember (`persist.uuids`, the saved attachments) every loaded
    /// kind takes, and each row's next runtime id from the game's counter.
    fn insert_general_record(
        &mut self,
        record: GeneralEntityRecord,
        tag: &Tag,
        mount: Option<u32>,
        transferred: Option<TransferredEntityState>,
    ) {
        let slot = general_type_catalog()
            .slot(&record.envelope.type_key)
            .expect("check_general_record resolved every type of the tree");
        self.next_entity += 1;
        let id = self.next_entity;
        // Install before `remember_uuid` (the same ordering a transferred
        // mob's state takes): the destination is the same logical entity.
        if let Some(state) = transferred {
            self.adopt_transferred_entity_state(id, state);
        }
        self.remember_uuid(id, Some(record.uuid), tag);
        let position = Vec3::new(record.position[0], record.position[1], record.position[2]);
        let mut row = GeneralRow {
            id,
            uuid: record.uuid,
            join: JoinSeq::UNJOINED,
            type_slot: slot,
            previous: position,
            position,
            velocity: Vec3::new(record.velocity[0], record.velocity[1], record.velocity[2]),
            rotation: record.rotation,
            fall_distance: 0.0,
            payload_version: record.envelope.payload_version,
            payload_bytes: record.envelope.payload_bytes,
            mount,
            attachment_lifetime: Default::default(),
        };
        self.bind_saved_entity_attachments(id, &mut row.attachment_lifetime);
        self.resident_entities.general.rows.push(row);
        // The riders' tags sit inside `Passengers` in the order the record
        // tree was built from them, so each rider's own tag (its saved
        // attachments) is the one at its index.
        let riders: Vec<(&GeneralEntityRecord, &Tag)> = match tag.get(format::PASSENGERS) {
            Some(Tag::List(list)) => record.passengers.iter().zip(list.iter()).collect(),
            _ => Vec::new(),
        };
        for (rider, rider_tag) in riders {
            self.insert_general_record(rider.clone(), rider_tag, Some(id), None);
        }
    }
}

/// Whether a namespace belongs to a loaded package: its own id or one of
/// its manifest's namespaces (`owns_namespace`, read from the engine side).
fn mod_namespace_loaded(namespace: &str) -> bool {
    mod_api::__private::loaded_mods().iter().any(|package| {
        package.id() == namespace || package.namespaces().any(|owned| owned == namespace)
    })
}

/// Whether a kind is a namespaced non-`minecraft` one — the shape a
/// quarantined (missing-mod) record's kind has.
pub(in crate::game) fn quarantined_kind(kind: &str) -> bool {
    kind.split_once(':')
        .is_some_and(|(namespace, _)| namespace != "minecraft")
}

/// Whether any UUID of the record tree names a live entity of the level
/// (`uuid_admission_refused` over the whole tree, in preorder).
fn general_tree_uuid_admission_refused(game: &Game<'_>, record: &GeneralEntityRecord) -> bool {
    fn refused(game: &Game<'_>, record: &GeneralEntityRecord) -> bool {
        game.uuid_admission_refused(Some(record.uuid))
            || record.passengers.iter().any(|rider| refused(game, rider))
    }
    refused(game, record)
}

impl Entity for GeneralRow {
    fn id(&self) -> u32 {
        self.id
    }
    fn position(&self) -> Vec3 {
        self.position
    }
    fn position_mut(&mut self) -> &mut Vec3 {
        &mut self.position
    }
    fn previous_position(&self) -> Vec3 {
        self.previous
    }
    fn velocity(&self) -> Vec3 {
        self.velocity
    }
    fn velocity_mut(&mut self) -> &mut Vec3 {
        &mut self.velocity
    }
    fn reset_fall_distance(&mut self) {
        self.fall_distance = 0.0;
    }
    /// A stage 2 row never joins a world, so only a push reaches it, and it
    /// carries no physics of its own: the move is a plain displacement.
    fn move_entity(&mut self, gate: &mut MoveGate, movement: Vec3) {
        if gate.skipped() {
            return;
        }
        let position = self.position_mut();
        *position = position.add_vector(movement);
    }
    fn rotation(&self) -> [f32; 2] {
        self.rotation
    }
    fn set_rotation(&mut self, rotation: [f32; 2]) {
        self.rotation = rotation;
    }
    fn bounds(&self) -> Aabb {
        let (width, height) = general_type_catalog().dimensions(self.type_slot);
        let half = width / 2.0;
        let at = self.position;
        Aabb::new(
            Vec3::new(at.x - half, at.y, at.z - half),
            Vec3::new(at.x + half, at.y + height, at.z + half),
        )
    }
    fn selector_target(&self) -> Option<Target> {
        // Stage 2 rows never join a world; a selector cannot name one yet.
        None
    }
    fn native_type(&self) -> Cow<'static, str> {
        Cow::Owned(general_type_catalog().key(self.type_slot).to_owned())
    }
    fn save_nbt(
        game: &mut Game<'_>,
        index: usize,
        keys: &HashSet<ChunkKey>,
    ) -> Option<(ChunkKey, Tag)> {
        let row = &game.resident_entities.general.rows[index];
        let catalog = general_type_catalog();
        // `noSave`: instances produce zero persisted records.
        if catalog.save_policy(row.type_slot) == mod_api::GeneralSavePolicy::NoSave {
            return None;
        }
        // A rider saves inside its root's record, never as its own.
        if row.mount.is_some() {
            return None;
        }
        let key = chunk_of(row.position);
        if !keys.contains(&key) {
            return None;
        }
        let record = game.general_row_record(row.id, &mut HashSet::new())?;
        let mut tag = record.to_tag();
        game.write_general_rider_attachments(&mut tag, row.id);
        Some((key, tag))
    }
    fn load_nbt(
        game: &mut Game<'_>,
        kind: &str,
        _uuid: Option<[i32; 4]>,
        _player_uuid: Option<[i32; 4]>,
        tag: Tag,
        transferred: &mut Option<TransferredEntityState>,
    ) -> Result<(), Tag> {
        // Only a type the frozen registry holds loads here; anything else
        // hands the compound back for the unknown-kind paths.
        if general_type_catalog().slot(kind).is_none() {
            return Err(tag);
        }
        game.load_general_tree(tag, transferred.take())
    }
}

/// The chunk a position lies in, the save walk's own arithmetic.
fn chunk_of(at: Vec3) -> ChunkKey {
    ((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4)
}

#[path = "general/tick.rs"]
mod tick;

#[cfg(test)]
#[path = "general/save_load_tests.rs"]
mod save_load_tests;

#[cfg(test)]
#[path = "general/tick_tests.rs"]
mod tick_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn record(uuid: [i32; 4], type_key: &str) -> GeneralEntityRecord {
        GeneralEntityRecord {
            position: [0.0, 64.0, 0.0],
            velocity: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0],
            uuid,
            envelope: GeneralTypeEnvelope {
                type_key: type_key.to_owned(),
                payload_version: 1,
                payload_bytes: Vec::new(),
            },
            passengers: Vec::new(),
        }
    }

    fn known(type_key: &str) -> impl Fn(&str) -> bool {
        let owned = type_key.to_owned();
        move |key| key == owned
    }

    /// A minimal valid tree checks ready, whatever its passengers' order.
    #[test]
    fn valid_record_with_passengers_checks_ready() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut rider = record([5, 6, 7, 8], "mymod:robot");
        rider
            .passengers
            .push(record([9, 10, 11, 12], "mymod:robot"));
        root.passengers.push(rider);
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
    }

    /// A mod payload one byte over the bound refuses with its size; at the
    /// bound it stands. Vanilla records never pass through this codec, so
    /// the bound cannot clip them.
    #[test]
    fn payload_bound_refuses_one_byte_over() {
        let mut over = record([1, 2, 3, 4], "mymod:robot");
        over.envelope.payload_bytes = vec![0; 65_537];
        assert_eq!(
            check_general_record(&over, known("mymod:robot")),
            Err(GeneralRecordError::PayloadTooLarge { size: 65_537 })
        );
        let mut at = record([5, 6, 7, 8], "mymod:robot");
        at.envelope.payload_bytes = vec![0; 65_536];
        assert_eq!(
            check_general_record(&at, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
    }

    /// A passenger chain one ride deeper than the passenger bound refuses,
    /// the root's own level excluded from the count. The design's separate
    /// data-nesting bound is stage 3's payload decode; this walk never
    /// reaches it, because this refusal fires first.
    #[test]
    fn passenger_depth_bound_refuses_past_the_limit() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut node = &mut root;
        for index in 0..limits::MAX_PASSENGER_DEPTH + 1 {
            node.passengers
                .push(record([index as i32 + 1, 0, 0, 0], "mymod:robot"));
            node = node.passengers.last_mut().unwrap();
        }
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Err(GeneralRecordError::PassengerDepthTooDeep {
                depth: limits::MAX_PASSENGER_DEPTH + 1
            })
        );
    }

    #[test]
    fn literal_depth_boundary_and_hostile_chain() {
        for depth in [16, 4096] {
            let mut root = record([1, 2, 3, 4], "mymod:robot");
            let mut node = &mut root;
            for index in 0..depth {
                node.passengers
                    .push(record([index + 1, 0, 0, 0], "mymod:robot"));
                node = node.passengers.last_mut().unwrap();
            }
            let expected = if depth == 16 {
                Ok(GeneralRecordCheck::Ready)
            } else {
                Err(GeneralRecordError::PassengerDepthTooDeep { depth: 17 })
            };
            assert_eq!(check_general_record(&root, known("mymod:robot")), expected);
            // Tear down iteratively too: recursive Vec drop is not the validator.
            while let Some(child) = root.passengers.pop() {
                root = child;
            }
        }
    }

    #[test]
    fn non_finite_rotation_refuses() {
        for bad in [f32::NAN, f32::INFINITY] {
            let mut record = record([1, 2, 3, 4], "mymod:robot");
            record.rotation[0] = bad;
            assert_eq!(
                check_general_record(&record, known("mymod:robot")),
                Err(GeneralRecordError::NotFinite)
            );
        }
    }

    /// A passenger carrying its own ancestor's UUID is a cycle, refused
    /// wherever in the tree the repeat hides.
    #[test]
    fn passenger_cycle_refuses() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut rider = record([5, 6, 7, 8], "mymod:robot");
        // The rider's passenger is the root again.
        rider.passengers.push(record([1, 2, 3, 4], "mymod:robot"));
        root.passengers.push(rider);
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Err(GeneralRecordError::PassengerCycle { uuid: [1, 2, 3, 4] })
        );
    }

    /// Two records of one UUID in one tree refuse, siblings included.
    #[test]
    fn duplicate_uuid_refuses() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        root.passengers.push(record([5, 6, 7, 8], "mymod:robot"));
        root.passengers.push(record([5, 6, 7, 8], "mymod:robot"));
        assert_eq!(
            check_general_record(&root, known("mymod:robot")),
            Err(GeneralRecordError::DuplicateUuid { uuid: [5, 6, 7, 8] })
        );
    }

    /// A NaN or infinite position, velocity or rotation component refuses
    /// before anything else is consulted.
    #[test]
    fn non_finite_coordinates_refuse() {
        for (slot, bad) in [(0, f64::NAN), (1, f64::NEG_INFINITY), (2, f64::NAN)] {
            let mut record = record([1, 2, 3, 4], "mymod:robot");
            record.position[slot] = bad;
            assert_eq!(
                check_general_record(&record, known("mymod:robot")),
                Err(GeneralRecordError::NotFinite)
            );
        }
        let mut velocity = record([1, 2, 3, 4], "mymod:robot");
        velocity.velocity[1] = f64::INFINITY;
        assert_eq!(
            check_general_record(&velocity, known("mymod:robot")),
            Err(GeneralRecordError::NotFinite)
        );
    }

    /// A structurally sound record of a type whose namespace belongs to
    /// neither vanilla nor a loaded mod reports `UnknownType`, the outcome
    /// stage 3 keeps as quarantined data; a structural fault in the same
    /// tree still refuses, so quarantine never preserves a bad payload.
    #[test]
    fn unknown_type_is_its_own_outcome() {
        let unknown = record([1, 2, 3, 4], "gone:robot");
        assert_eq!(
            check_general_record(&unknown, known("mymod:robot")),
            Ok(GeneralRecordCheck::UnknownType)
        );
        // A passenger of an unknown type poisons the root's readiness too:
        // the tree keeps together.
        let mut mixed = record([5, 6, 7, 8], "mymod:robot");
        mixed.passengers.push(record([9, 10, 11, 12], "gone:robot"));
        assert_eq!(
            check_general_record(&mixed, known("mymod:robot")),
            Ok(GeneralRecordCheck::UnknownType)
        );
        // But a structural fault refuses even for the unknown type.
        let mut over = record([13, 14, 15, 16], "gone:robot");
        over.envelope.payload_bytes = vec![0; 65_537];
        assert_eq!(
            check_general_record(&over, known("mymod:robot")),
            Err(GeneralRecordError::PayloadTooLarge { size: 65_537 })
        );
    }

    fn declaration(mod_id: &str, id: &str) -> mod_api::__private::GeneralEntityTypeDeclaration {
        mod_api::__private::GeneralEntityTypeDeclaration {
            mod_id: mod_id.to_owned(),
            id: id.to_owned(),
            category: mod_api::GeneralEntityCategory::Misc,
            width: 0.8,
            height: 1.9,
            save_policy: mod_api::GeneralSavePolicy::Save,
            summonable: true,
            tracking_chunks: 8,
            update_interval: 3,
            payload_version: 1,
            required_features: Vec::new(),
        }
    }

    /// The frozen registry resolves slots in one stable order — declaring
    /// mod id, then type id — and a duplicate type key refuses at freeze,
    /// naming it, so two mods cannot claim one slot number.
    #[test]
    fn catalog_freezes_slots_in_declaration_order_and_refuses_duplicates() {
        let catalog = GeneralTypeCatalog::freeze(&[
            declaration("zmod", "zmod:zeal"),
            declaration("amod", "amod:alpha"),
            declaration("amod", "amod:beta"),
        ])
        .unwrap();
        assert_eq!(catalog.key(0), "amod:alpha");
        assert_eq!(catalog.key(1), "amod:beta");
        assert_eq!(catalog.key(2), "zmod:zeal");
        assert_eq!(catalog.slot("amod:beta"), Some(1));
        assert_eq!(catalog.dimensions(2), (0.8, 1.9));
        match GeneralTypeCatalog::freeze(&[
            declaration("amod", "amod:same"),
            declaration("bmod", "amod:same"),
        ]) {
            Err(refusal) => assert!(refusal.contains("duplicate general entity type amod:same")),
            Ok(_) => panic!("a duplicate type key froze"),
        }
    }

    #[test]
    fn catalog_slots_do_not_wrap_at_u16_boundary() {
        let declarations: Vec<_> = (0..65_537)
            .map(|index| declaration("amod", &format!("amod:type_{index:05}")))
            .collect();
        let catalog = GeneralTypeCatalog::freeze(&declarations).unwrap();
        assert_eq!(catalog.slot("amod:type_65535"), Some(65_535));
        assert_eq!(catalog.slot("amod:type_65536"), Some(65_536));
        assert_eq!(catalog.key(65_536), "amod:type_65536");
        assert_eq!(
            catalog.save_policy(65_536),
            mod_api::GeneralSavePolicy::Save
        );
    }

    /// A general row pushed to its list binds residency the way
    /// `resident_owners!` wires `Kind::General` (the default arm's
    /// `bind_residency` with the row's id and join identity), and the entity
    /// table then resolves the row's id to that owner: the one lookup path
    /// every family shares, with no second index.
    #[test]
    fn general_row_binds_and_resolves_through_the_entity_table() {
        use horizon_client::join_seq::residency::table::{EntityLookup, EntityTable, SourceEntity};
        use horizon_client::join_seq::residency::{Journal, Kind, Owner};
        use mod_api::DimensionKey;
        let journal = Journal::default();
        let mut rows: GeneralRows = Default::default();
        let owner = Owner {
            dimension: DimensionKey::OVERWORLD,
            kind: Kind::General,
        };
        rows.bind_residency(&journal, owner, |row| (row.id, row.join));
        rows.push(GeneralRow {
            id: 42,
            uuid: [1, 2, 3, 4],
            join: horizon_client::join_seq::JoinSeq::UNJOINED,
            type_slot: 0,
            position: Vec3::new(0.5, 64.0, 0.5),
            previous: Vec3::new(0.5, 64.0, 0.5),
            velocity: Vec3::ZERO,
            rotation: [0.0, 0.0],
            fall_distance: 0.0,
            payload_version: 1,
            payload_bytes: Vec::new(),
            mount: None,
            attachment_lifetime: Default::default(),
        });
        let mut table = EntityTable::new(journal);
        let EntityLookup::Found { location, .. } =
            table.entity_lookup(&DimensionKey::OVERWORLD, 42, |at| {
                rows.locate(at.join).and_then(|index| {
                    rows.get(index).map(|row| SourceEntity {
                        id: row.id,
                        value: index,
                    })
                })
            })
        else {
            panic!("the general row never became resident");
        };
        assert_eq!(location.owner, owner);
        // The list keeps join order: removal is not a swap, and the row's
        // slot moves with it.
        let index = rows.len() - 1;
        assert_eq!(rows[index].id, 42);
        assert_ne!(
            rows[index].join,
            horizon_client::join_seq::JoinSeq::UNJOINED
        );
    }

    /// A refusal leaves no partial state: the very next valid record checks
    /// cleanly through the same entry point.
    #[test]
    fn refused_record_does_not_poison_the_next_one() {
        let mut over = record([1, 2, 3, 4], "mymod:robot");
        over.envelope.payload_bytes = vec![0; 65_537];
        assert!(check_general_record(&over, known("mymod:robot")).is_err());
        let good = record([5, 6, 7, 8], "mymod:robot");
        assert_eq!(
            check_general_record(&good, known("mymod:robot")),
            Ok(GeneralRecordCheck::Ready)
        );
    }

    /// The record codec round-trips a tree through the save format: what
    /// `to_tag` writes is what `record_from_tag` reads back, passengers and
    /// payload bytes included.
    #[test]
    fn record_round_trips_through_the_save_tag() {
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        root.position = [0.5, 64.0, -12.25];
        root.velocity = [0.0, -0.08, 0.25];
        root.rotation = [90.0, -45.0];
        root.envelope.payload_version = 7;
        root.envelope.payload_bytes = vec![9, 8, 7, 6];
        let mut rider = record([5, 6, 7, 8], "mymod:drone");
        rider.rotation = [-180.0, 30.0];
        rider.envelope.payload_bytes = vec![1];
        root.passengers.push(rider);
        let tag = root.to_tag();
        assert_eq!(record_from_compound(&tag), Ok(root));
    }

    /// The writer names the fields the format fixes: the common fields under
    /// their vanilla names, the envelope under its own.
    #[test]
    fn record_to_tag_uses_the_formats_field_names() {
        let tag = record([1, 2, 3, 4], "mymod:robot").to_tag();
        let Tag::Compound(map) = &tag else {
            panic!("a record writes a compound");
        };
        for field in [
            format::TYPE,
            format::POSITION,
            format::VELOCITY,
            format::ROTATION,
            format::UUID,
            format::PAYLOAD_VERSION,
            format::PAYLOAD,
        ] {
            let key = horizon_core::nbt::JavaString::from(field);
            assert!(map.contains_key(&key), "{field} missing");
        }
        // A lone record writes no passenger list, as vanilla's save does not.
        assert!(!map.contains_key(&horizon_core::nbt::JavaString::from(format::PASSENGERS)));
        assert_eq!(
            tag.get(format::TYPE),
            Some(&Tag::String("mymod:robot".into()))
        );
    }

    /// A record whose own data nests one level past the bound refuses with
    /// the nesting fault, and one level inside it stands; the passenger
    /// wrappers themselves never count toward this bound, so a tree at the
    /// full passenger depth still parses (its depth has its own refusal).
    #[test]
    fn record_refuses_data_nested_past_the_bound() {
        let deeply = |levels: u32| {
            let mut tag = record([1, 2, 3, 4], "mymod:robot").to_tag();
            let mut nested = Tag::Compound(Default::default());
            for _ in 1..levels {
                let mut next = Tag::Compound(Default::default());
                if let Tag::Compound(map) = &mut next {
                    map.insert("inner".into(), nested);
                }
                nested = next;
            }
            if let Tag::Compound(map) = &mut tag {
                map.insert("Extra".into(), nested);
            }
            tag
        };
        // The root compound is the first level: an `Extra` field nested to
        // the bound makes the record's data one level past it.
        assert_eq!(
            record_from_compound(&deeply(limits::MAX_NESTING)),
            Err(GeneralRecordError::NestingTooDeep {
                depth: limits::MAX_NESTING + 1
            })
        );
        assert!(record_from_compound(&deeply(limits::MAX_NESTING - 1)).is_ok());
        // The deepest legal passenger chain (16 riders) is not data nesting.
        let mut root = record([1, 2, 3, 4], "mymod:robot");
        let mut node = &mut root;
        for index in 0..limits::MAX_PASSENGER_DEPTH {
            node.passengers
                .push(record([index as i32 + 5, 0, 0, 0], "mymod:drone"));
            node = node.passengers.last_mut().unwrap();
        }
        assert_eq!(
            record_from_compound(&root.to_tag())
                .unwrap()
                .passengers
                .len(),
            1
        );
    }

    /// A field of the wrong shape refuses the whole record as a malformed
    /// envelope, naming the field: no defaulting, no partial read.
    #[test]
    fn record_refuses_malformed_envelope_fields() {
        let base = || record([1, 2, 3, 4], "mymod:robot").to_tag();
        let malformed = |field: &'static str, value: Tag| {
            let mut tag = base();
            if let Tag::Compound(map) = &mut tag {
                map.insert(field.into(), value);
            }
            tag
        };
        for (field, value, expected) in [
            (
                format::PAYLOAD,
                Tag::Int(1),
                GeneralRecordError::InvalidEnvelope {
                    field: format::PAYLOAD,
                },
            ),
            (
                format::PAYLOAD_VERSION,
                Tag::Int(1),
                GeneralRecordError::InvalidEnvelope {
                    field: format::PAYLOAD_VERSION,
                },
            ),
            (
                format::UUID,
                Tag::IntArray(vec![1, 2, 3]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::UUID,
                },
            ),
            (
                format::ROTATION,
                Tag::List(vec![Tag::Float(0.0f32.to_bits())]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::ROTATION,
                },
            ),
            (
                format::POSITION,
                Tag::List(vec![
                    Tag::Double(0.0f64.to_bits()),
                    Tag::Float(64.0f32.to_bits()),
                    Tag::Double(0.0f64.to_bits()),
                ]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::POSITION,
                },
            ),
            (
                format::PASSENGERS,
                Tag::List(vec![Tag::String("robot".into())]),
                GeneralRecordError::InvalidEnvelope {
                    field: format::PASSENGERS,
                },
            ),
            (
                format::TYPE,
                Tag::Int(1),
                GeneralRecordError::InvalidEnvelope {
                    field: format::TYPE,
                },
            ),
        ] {
            assert_eq!(
                record_from_compound(&malformed(field, value)),
                Err(expected),
                "{field}"
            );
        }
    }

    /// The encoded size the quarantine bound reads: a payload's bytes count
    /// into it one for one, so the gate can tell a record at the bound from
    /// one byte past it.
    #[test]
    fn nbt_size_counts_payload_bytes_one_for_one() {
        let at = GeneralEntityRecord {
            envelope: GeneralTypeEnvelope {
                payload_bytes: vec![0; 1024],
                ..record([1, 2, 3, 4], "mymod:robot").envelope
            },
            ..record([1, 2, 3, 4], "mymod:robot")
        };
        let over = GeneralEntityRecord {
            envelope: GeneralTypeEnvelope {
                payload_bytes: vec![0; 1025],
                ..at.envelope.clone()
            },
            ..at.clone()
        };
        assert_eq!(
            nbt_size(&over.to_tag()) - nbt_size(&at.to_tag()),
            1,
            "one payload byte must add one encoded byte"
        );
    }
}
