//! Transactional carrier ownership store: slice 2, part A (issue arismoko/horizon#2413).
//!
//! The store owns the world's carrier files flat in the existing `data/`
//! directory — no new folders are ever created:
//! - `horizon_carriers.journal.{0,1}`: the two-slot journal;
//! - `horizon_carrier_op_<id>.nbt`: one operation file per in-flight
//!   transfer;
//! - `horizon_carrier_<uuid>-<gen>.nbt`: one file per live carrier.
//!
//! Every file is published whole through
//! [`horizon_server_files::durable`]: a synced temporary, then a
//! write-through move on Windows or `rename` plus a directory `fsync` on
//! Unix. The journal holds `next_op_id`, the index of live carriers, at
//! most one in-progress operation and tombstones, each record numbered
//! and CRC-protected; a new record always goes to the slot that does not
//! hold the newest valid one, and the initialized journal is never
//! deleted, so at least one valid slot survives every single-step crash.
//!
//! Nothing exists before the first explicit transfer: `first_use` writes
//! the `horizon:carriers_in_use` flag into `level.dat` first (its
//! replacement gap rolls back to an untouched world) and slot 0 second,
//! and a world that never assembles a carrier gains no file and changes
//! no byte. Opening follows the agreed table: no slots and no flag means
//! never used; a flag, or leftover op/carrier files, without slots
//! refuses the load and names the missing store; slots whose newest
//! valid record is from a newer schema refuse too.
//!
//! This is the storage layer only: nothing here is wired into gameplay,
//! `Game` or any public API (that is part B and slice 9).

use crate::save::{read_dat, write_dat};
use horizon_core::nbt::{self, JavaString, Limits, NamedTag, Tag};
use horizon_server_files::durable;
use horizon_world::compression::Compression;
use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

/// The journal slot file names, in slot order.
pub const SLOTS: [&str; 2] = ["horizon_carriers.journal.0", "horizon_carriers.journal.1"];
/// The `level.dat` key marking a world whose carriers exist.
pub const IN_USE_FLAG: &str = "horizon:carriers_in_use";
/// A whole slot file, header included, is at most this big.
pub const MAX_SLOT_FILE: usize = 1024 * 1024;
/// At most this many live carriers are indexed.
pub const MAX_CARRIERS: usize = 4096;
/// At most this many tombstones are pending.
pub const MAX_TOMBSTONES: usize = 4096;
/// An operation file is at most this big compressed, on disk.
pub const MAX_OP_COMPRESSED: usize = 32 * 1024 * 1024;
/// An operation file is at most this big decoded.
pub const MAX_OP_DECODED: usize = 40 * 1024 * 1024;
/// An operation file's NBT nests at most this deep (vanilla's own bound).
pub const MAX_OP_DEPTH: usize = 512;
/// An operation touches at most this many cells (two images each).
pub const MAX_OP_CELLS: usize = 4096;
/// One cell's block-entity image is at most this big serialized.
pub const MAX_CELL_ENTITY: usize = 64 * 1024;
/// All block-entity images of one operation side total at most this much.
pub const MAX_OP_ENTITIES: usize = 16 * 1024 * 1024;
/// The slot header: magic, version, record number, body length, CRC.
const SLOT_HEADER: usize = 4 + 2 + 8 + 4 + 4;
/// The slot magic.
const SLOT_MAGIC: [u8; 4] = *b"HCJS";
/// The slot schema this build writes.
const SLOT_VERSION: u16 = 1;
/// The operation-file schema this build writes.
const OP_VERSION: i32 = 1;
/// The node budget for one operation file: the exact schema shape never
/// needs more than a handful of nodes per cell.
const MAX_OP_NODES: usize = 10 * MAX_OP_CELLS;

/// Why a world's carrier store refused to open. Every refusal names the
/// store and never lets load continue on a guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The flag, or leftover carrier files, say the world has carriers,
    /// but no valid journal slot survives. The world must be restored
    /// from a coherent whole-world backup.
    MissingStore { flag: bool },
    /// Slots exist and none of them holds a valid record.
    NoValidSlot,
    /// The newest valid slot was written by a newer schema.
    NewerSchema { found: u16 },
    /// Reading the store failed on I/O.
    Io(String),
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStore { flag } => write!(
                f,
                "the carrier store is missing (flag: {flag}): journal slots absent while the \
                 world holds carrier data; restore the world from a whole-world backup"
            ),
            Self::NoValidSlot => {
                write!(f, "no valid carrier journal slot survives")
            }
            Self::NewerSchema { found } => write!(
                f,
                "carrier journal schema {found} is newer than this build's {SLOT_VERSION}"
            ),
            Self::Io(error) => write!(f, "carrier store I/O: {error}"),
        }
    }
}

/// The transfer kind an operation file and journal record name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpKind {
    /// World cells become carrier cells.
    Assemble,
    /// Carrier cells return to the world.
    Disassemble,
}
impl OpKind {
    /// The wire byte.
    const fn as_byte(self) -> u8 {
        match self {
            Self::Assemble => 0,
            Self::Disassemble => 1,
        }
    }
    /// The wire byte, decoded.
    const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Assemble),
            1 => Some(Self::Disassemble),
            _ => None,
        }
    }
    /// The operation file's kind string.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Assemble => "assemble",
            Self::Disassemble => "disassemble",
        }
    }
}

/// The one operation a journal may hold in progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InProgress {
    /// The operation's id, from `next_op_id`.
    pub op_id: u64,
    /// Whether it assembles or disassembles.
    pub kind: OpKind,
    /// The carrier it moves.
    pub uuid: u128,
    /// The carrier's generation after this operation.
    pub generation: u32,
    /// The dimension the operation runs in ('"minecraft:overworld"'), so
    /// recovery enters that level and no other.
    pub dimension: String,
}

/// The journal body: what the newest valid slot holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Journal {
    /// The next operation id to hand out; only grows.
    pub next_op_id: u64,
    /// The one operation that may be in flight, if any.
    pub in_progress: Option<InProgress>,
    /// Carriers whose file must not come back to life after a deletion
    /// that may not be durable yet.
    pub tombstones: Vec<(u128, u32)>,
    /// The live carriers, `(uuid, generation)` per entry.
    pub carriers: Vec<(u128, u32)>,
}
impl Journal {
    /// A fresh, empty journal handing out operation ids from one.
    pub const fn empty() -> Self {
        Self {
            next_op_id: 1,
            in_progress: None,
            tombstones: Vec::new(),
            carriers: Vec::new(),
        }
    }
    /// Whether the index lists this carrier.
    pub fn lists(&self, uuid: u128, generation: u32) -> bool {
        self.carriers.contains(&(uuid, generation))
    }
    /// Whether this carrier is tombstoned.
    pub fn tombstoned(&self, uuid: u128, generation: u32) -> bool {
        self.tombstones.contains(&(uuid, generation))
    }
}

/// An opened store: `NeverUsed` when the world has no carriers, or the
/// live journal.
#[derive(Debug)]
pub enum Opened {
    /// No slots, no flag, no files: the world never used carriers.
    NeverUsed,
    /// A valid journal rules the store.
    Live(Store),
}

/// The live store. Every mutation writes the next slot durably before it
/// returns, so a crash always leaves the previous record intact.
#[derive(Debug)]
pub struct Store {
    data: PathBuf,
    /// The newest valid record's number, and the slot it lives in.
    entry_no: u64,
    last_slot: usize,
    journal: Journal,
    /// The index-capacity bound this store enforces; the default
    /// [`MAX_CARRIERS`], smaller in tests to exercise the overflow
    /// paths in seconds.
    capacity: usize,
    /// The tombstones whose carrier files were already absent when the
    /// store opened. Only those may clear: a file deleted during this
    /// session may not be durably deleted yet on Windows, where the
    /// directory entry cannot be synced, so its tombstone holds until a
    /// later open finds the file gone.
    tombstones_absent_at_open: Vec<(u128, u32)>,
}
impl Store {
    /// Eligibility captured at the actual world open, shared by all transfers.
    pub fn retention_snapshot(&self) -> Vec<(u128, u32)> {
        self.tombstones_absent_at_open.clone()
    }

    /// Replace the constructor's disk sample with the world-session snapshot.
    pub fn use_retention_snapshot(&mut self, snapshot: &[(u128, u32)]) {
        self.tombstones_absent_at_open = snapshot.to_vec();
    }

    /// Reads the world's store and applies the opening table: slot
    /// presence, the `level.dat` flag, leftover carrier files and slot
    /// validity decide between [`Opened::NeverUsed`], [`Opened::Live`]
    /// and a typed [`Refusal`]. Nothing is written or cleaned here.
    pub fn open(world_root: &Path) -> Result<Opened, Refusal> {
        Self::open_with_capacity(world_root, MAX_CARRIERS)
    }

    /// [`Self::open`] with an explicit index capacity (tests).
    pub fn open_with_capacity(world_root: &Path, capacity: usize) -> Result<Opened, Refusal> {
        let data = world_root.join("data");
        let flag = in_use_flag(world_root);
        let Some(records) = read_slots(&data)? else {
            if residual_files(&data)? {
                // Leftover carrier files without a journal are a missing
                // store whatever the flag says.
                return Err(Refusal::MissingStore { flag });
            }
            if flag {
                // The flag without slots: the store was used and is gone.
                return Err(Refusal::MissingStore { flag: true });
            }
            return Ok(Opened::NeverUsed);
        };
        // A newer schema is reported even when the other slot is valid:
        // the newer record is what the world actually was.
        let mut newest: Option<(u64, &SlotRecord)> = None;
        let mut newer = None;
        for record in &records {
            if let SlotError::NewerSchema { found } = record.error() {
                newer = Some(found);
            }
            if record.valid() && newest.is_none_or(|(no, _)| record.entry_no > no) {
                newest = Some((record.entry_no, record));
            }
        }
        if let Some(found) = newer {
            return Err(Refusal::NewerSchema { found });
        }
        let Some((entry_no, record)) = newest else {
            return Err(Refusal::NoValidSlot);
        };
        let slot = record.slot;
        let journal = record.body.clone();
        let tombstones_absent_at_open = tombstones_without_files(&data, &journal)?;
        Ok(Opened::Live(Self {
            data,
            entry_no,
            last_slot: slot,
            journal,
            capacity,
            tombstones_absent_at_open,
        }))
    }

    /// The store's first-ever use, on the explicit call of the first
    /// transfer's preflight (part B). Allowed only when `data/` existed
    /// when the world was opened, so its directory entry was published by
    /// an earlier session. The flag goes into `level.dat` first — a crash
    /// in its replacement gap leaves an untouched world that opens as
    /// never used — then slot 0 holds an empty initialized journal,
    /// published durably and read back before anything else may happen.
    pub fn first_use(world_root: &Path, data_existed_at_open: bool) -> Result<Self, FirstUseError> {
        Self::first_use_with_capacity(world_root, data_existed_at_open, MAX_CARRIERS)
    }

    /// [`Self::first_use`] with an explicit index capacity (tests).
    pub fn first_use_with_capacity(
        world_root: &Path,
        data_existed_at_open: bool,
        capacity: usize,
    ) -> Result<Self, FirstUseError> {
        if !data_existed_at_open {
            return Err(FirstUseError::DataCreatedThisSession);
        }
        match Self::open(world_root)? {
            Opened::Live(_) => return Err(FirstUseError::AlreadyInUse),
            Opened::NeverUsed => {}
        }
        let level_dat = world_root.join("level.dat");
        set_in_use_flag(&level_dat).map_err(FirstUseError::Flag)?;
        if !in_use_flag(world_root) {
            return Err(FirstUseError::Flag("flag did not read back".into()));
        }
        let data = world_root.join("data");
        let body = Journal::empty();
        let bytes = encode_slot(1, &body, capacity).map_err(FirstUseError::Slot)?;
        durable::write(&data.join(SLOTS[0]), &bytes)
            .map_err(|e| FirstUseError::Io(e.to_string()))?;
        // Read the slot back: only a store that opens is a store.
        match Self::open_with_capacity(world_root, capacity)? {
            Opened::Live(store) => Ok(store),
            Opened::NeverUsed => Err(FirstUseError::Slot(
                "slot 0 did not read back as valid".into(),
            )),
        }
    }

    /// The journal as the newest valid record holds it.
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Hands out the next operation id. Callers record it with
    /// [`Self::begin`] before writing any operation file.
    pub fn next_op_id(&self) -> u64 {
        self.journal.next_op_id
    }

    /// The generation a re-assembly of `uuid` takes: one past the
    /// highest generation the journal remembers for it — index entry or
    /// tombstone — computed with checked arithmetic; a carrier whose
    /// remembered generation is `u32::MAX` refuses rather than
    /// wrapping. The durable operation counter is also a lower bound,
    /// so deleting tombstones cannot revive an old generation.
    pub fn next_generation(&self, uuid: u128) -> Result<u32, StoreIoError> {
        let remembered = self
            .journal
            .carriers
            .iter()
            .chain(self.journal.tombstones.iter())
            .filter(|(u, _)| *u == uuid)
            .map(|(_, generation)| *generation)
            .max();
        let generation = self
            .journal
            .next_op_id
            .max(remembered.map_or(1, |last| u64::from(last) + 1));
        if generation > i32::MAX as u64 {
            return Err(StoreIoError("carrier generation limit reached".into()));
        }
        Ok(generation as u32)
    }

    /// Records the operation as in progress and publishes the next slot.
    /// Called after the operation file is durable, before the world
    /// changes.
    /// Refuse before publishing files or saving chunks. A full journal with a
    /// pending operation is reachable only from the unfixed, unshipped version;
    /// recovery of that state is outside this admission contract.
    pub fn check_admission(
        &self,
        kind: OpKind,
        uuid: u128,
        generation: u32,
    ) -> Result<(), StoreIoError> {
        let full = match kind {
            OpKind::Assemble => self.journal.carriers.len() >= self.capacity,
            OpKind::Disassemble => {
                self.journal.tombstones.len() >= MAX_TOMBSTONES
                    && !self.journal.tombstones.contains(&(uuid, generation))
            }
        };
        if full {
            return Err(StoreIoError(
                "CarrierCapacityReopenNeeded: carrier store capacity reached; reopen needed".into(),
            ));
        }
        Ok(())
    }

    pub fn begin(
        &mut self,
        kind: OpKind,
        uuid: u128,
        generation: u32,
        dimension: &str,
    ) -> Result<u64, StoreIoError> {
        self.check_admission(kind, uuid, generation)?;
        // One operation at a time: a nested begin leaves the open one
        // untouched and refuses.
        if self.journal.in_progress.is_some() {
            return Err(StoreIoError("an operation is already in progress".into()));
        }
        // Identity ownership, by kind:
        // - an assembly creates a carrier, so the UUID must be free —
        //   a second one would overwrite the first carrier's payload;
        // - a disassembly consumes the carrier its handle names, so the
        //   UUID and generation must be exactly the index's — a stale
        //   handle refuses.
        let listed = match kind {
            OpKind::Assemble => self.journal.carriers.iter().any(|(held, _)| *held == uuid),
            OpKind::Disassemble => self.journal.lists(uuid, generation),
        };
        match (kind, listed) {
            (OpKind::Assemble, true) => {
                return Err(StoreIoError(format!(
                    "carrier {uuid:032x} already exists at generation {generation}"
                )));
            }
            (OpKind::Disassemble, false) => {
                return Err(StoreIoError(format!(
                    "carrier {uuid:032x} at generation {generation} is not held: a stale handle refuses"
                )));
            }
            _ => {}
        }
        let op_id = self.journal.next_op_id;
        if generation > i32::MAX as u32 {
            return Err(StoreIoError("carrier generation limit reached".into()));
        }
        if op_id > i64::MAX as u64 {
            return Err(StoreIoError("operation id exhausted".into()));
        }
        let next_op_id = op_id
            .checked_add(1)
            .ok_or_else(|| StoreIoError("operation id exhausted".into()))?;
        self.entry_no
            .checked_add(1)
            .ok_or_else(|| StoreIoError("journal entry counter exhausted".into()))?;
        let previous = self.journal.clone();
        self.journal.in_progress = Some(InProgress {
            op_id,
            kind,
            uuid,
            generation,
            dimension: dimension.to_string(),
        });
        self.journal.next_op_id = next_op_id;
        if let Err(error) = self.write_slot() {
            self.journal = previous;
            return Err(error);
        }
        Ok(op_id)
    }

    /// Finishes the in-progress operation: the carrier index and
    /// tombstones take the operation's effect, the in-progress record
    /// clears, the next slot publishes, and the operation file is durably
    /// deleted before the next transfer may start.
    pub fn commit(&mut self) -> Result<(), StoreIoError> {
        let Some(op) = self.journal.in_progress.clone() else {
            return Err(StoreIoError("no operation in progress".into()));
        };
        let previous = self.journal.clone();
        match op.kind {
            OpKind::Assemble => {
                if !self.journal.lists(op.uuid, op.generation) {
                    self.journal.carriers.push((op.uuid, op.generation));
                }
            }
            OpKind::Disassemble => {
                self.journal
                    .carriers
                    .retain(|&c| c != (op.uuid, op.generation));
                if !self.journal.tombstoned(op.uuid, op.generation) {
                    self.journal.tombstones.push((op.uuid, op.generation));
                }
            }
        }
        self.journal.in_progress = None;
        if let Err(error) = self.write_slot() {
            self.journal = previous;
            return Err(error);
        }
        durable::remove(&self.operation_path(op.op_id)).map_err(|e| StoreIoError(e.to_string()))?;
        Ok(())
    }

    /// The operation file's path for `op_id`.
    pub fn operation_path(&self, op_id: u64) -> PathBuf {
        self.data.join(format!("horizon_carrier_op_{op_id}.nbt"))
    }

    /// Publishes an encoded operation file durably.
    pub fn write_operation(&self, op_id: u64, encoded: &[u8]) -> Result<(), StoreIoError> {
        durable::write(&self.operation_path(op_id), encoded)
            .map_err(|e| StoreIoError(e.to_string()))
    }

    /// The carrier file's path.
    pub fn carrier_path(&self, uuid: u128, generation: u32) -> PathBuf {
        self.data
            .join(format!("horizon_carrier_{uuid:032x}-{generation}.nbt"))
    }

    /// Publishes a carrier file durably.
    pub fn write_carrier(
        &self,
        uuid: u128,
        generation: u32,
        encoded: &[u8],
    ) -> Result<(), StoreIoError> {
        durable::write(&self.carrier_path(uuid, generation), encoded)
            .map_err(|e| StoreIoError(e.to_string()))
    }

    /// Retention for a just-finished operation, called from the live
    /// transfer path right after `commit`: the disassembly's tombstone
    /// clears only if its carrier file was already absent at open.
    /// File absence in this session is not proof of durable deletion.
    /// This is the per-operation counterpart of [`Self::cleanup`], not a
    /// replacement: world open still runs the full cleanup for files
    /// left by crashed sessions.
    pub fn finish_retention(&mut self) -> Result<(), StoreIoError> {
        let tombstones = self.journal.tombstones.clone();
        let mut changed = false;
        for (uuid, generation) in tombstones {
            if self.tombstones_absent_at_open.contains(&(uuid, generation))
                && !self
                    .data
                    .join(format!("horizon_carrier_{uuid:032x}-{generation}.nbt"))
                    .exists()
            {
                self.journal.tombstones.retain(|t| *t != (uuid, generation));
                changed = true;
            }
        }
        if changed {
            self.write_slot()?;
        }
        Ok(())
    }

    /// Cleans the store by the valid journal: carrier files outside the
    /// index are orphans and go; an operation file that is not the
    /// in-progress one goes; a tombstone holds until its carrier file's
    /// deletion is durable, then clears with a new slot. Nothing is
    /// silently adopted: what the journal does not know is deleted or
    /// refused, never inferred into ownership.
    pub fn cleanup(&mut self) -> Result<Cleanup, StoreIoError> {
        let mut report = Cleanup::default();
        let names = fs::read_dir(&self.data)
            .map_err(|e| StoreIoError(e.to_string()))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        let mut journal_changed = false;
        for name in &names {
            if let Some(op_id) = parse_operation_name(name) {
                let current = self.journal.in_progress.as_ref().map(|op| op.op_id);
                if current != Some(op_id) {
                    durable::remove(&self.data.join(name))
                        .map_err(|e| StoreIoError(e.to_string()))?;
                    report.stale_operations += 1;
                }
            } else if let Some((uuid, generation)) = parse_carrier_name(name) {
                if self.journal.lists(uuid, generation) {
                    continue;
                }
                durable::remove(&self.data.join(name)).map_err(|e| StoreIoError(e.to_string()))?;
                if self.journal.tombstoned(uuid, generation) {
                    // The deletion happened this session: on Windows it is
                    // not provably durable yet, so the tombstone holds
                    // until a later open finds the file gone.
                } else {
                    report.orphan_carriers += 1;
                }
            }
        }
        for &(uuid, generation) in &self.tombstones_absent_at_open {
            if self.journal.tombstoned(uuid, generation) {
                self.journal.tombstones.retain(|t| *t != (uuid, generation));
                journal_changed = true;
                report.resolved_tombstones += 1;
            }
        }
        if journal_changed {
            self.write_slot()?;
        }
        Ok(report)
    }

    /// Writes the next record into the slot that does not hold the newest
    /// valid one, durably, and only then adopts it.
    fn write_slot(&mut self) -> Result<(), StoreIoError> {
        let next_entry = self
            .entry_no
            .checked_add(1)
            .ok_or_else(|| StoreIoError("journal entry counter exhausted".into()))?;
        let bytes = encode_slot(next_entry, &self.journal, self.capacity)
            .map_err(|e| StoreIoError(e.to_string()))?;
        let slot = 1 - self.last_slot;
        durable::write(&self.data.join(SLOTS[slot]), &bytes)
            .map_err(|e| StoreIoError(e.to_string()))?;
        self.entry_no = next_entry;
        self.last_slot = slot;
        Ok(())
    }
}

/// What one cleanup pass removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// Carrier files deleted that no index entry owned.
    pub orphan_carriers: usize,
    /// Operation files deleted that were not in progress.
    pub stale_operations: usize,
    /// Tombstones cleared after their carrier file's durable deletion.
    pub resolved_tombstones: usize,
}

/// Why a first use did not happen.
#[derive(Debug)]
pub enum FirstUseError {
    /// `data/` was created during this session; its directory entry is
    /// not durable yet. Save and reopen the world first.
    DataCreatedThisSession,
    /// The store already exists (or refuses to open).
    AlreadyInUse,
    /// Writing or reading back the `level.dat` flag failed.
    Flag(String),
    /// The first slot could not be written or read back.
    Slot(String),
    /// Plain I/O.
    Io(String),
}
impl From<Refusal> for FirstUseError {
    fn from(refusal: Refusal) -> Self {
        Self::Io(refusal.to_string())
    }
}
impl fmt::Display for FirstUseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataCreatedThisSession => write!(
                f,
                "the world's data folder was created this session; save and reopen before \
                 the first carrier transfer"
            ),
            Self::AlreadyInUse => write!(f, "the carrier store is already in use"),
            Self::Flag(error) => write!(f, "carriers_in_use flag: {error}"),
            Self::Slot(error) => write!(f, "first journal slot: {error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

/// The registry-backed state validator: a state image is known exactly
/// when the block registry resolves it — the name exists and every
/// property with this value is declared on that block. This is part B's
/// replacement of part A's all-accepted seam.
pub struct RegistryStates<'a> {
    registry: &'a horizon_world::registry::BlockRegistry,
}
impl<'a> RegistryStates<'a> {
    /// Wraps the live block registry.
    pub fn new(registry: &'a horizon_world::registry::BlockRegistry) -> Self {
        Self { registry }
    }
}
impl StateValidator for RegistryStates<'_> {
    fn known(&self, name: &str, properties: &[(String, String)]) -> bool {
        let descriptor = horizon_world::chunk::BlockState {
            name: name.to_string(),
            properties: properties.iter().cloned().collect(),
        };
        self.registry.resolve(&descriptor).is_ok()
    }
}

/// The longest state the registry declares, serialized the way an
/// operation file spells it (name plus every `key=value`). Computed once
/// at load; a registry whose states cannot fit the operation envelope
/// refuses preflight, never a half-stored transfer.
pub fn longest_state_bytes(registry: &horizon_world::registry::BlockRegistry) -> usize {
    let mut longest = 0_usize;
    for id in 0..registry.len() {
        let Some(state) = registry.state(horizon_world::registry::BlockStateId(id as u32)) else {
            continue;
        };
        let mut size = state.name.len();
        for (key, value) in &state.properties {
            size += key.len() + value.len() + 4;
        }
        longest = longest.max(size);
    }
    longest
}

/// Whether a whole operation's worst shape fits the envelope: two images
/// per cell at the registry's longest state, the full 16 MiB of
/// block-entity data, and framing. Refused at preflight, before any
/// block is lifted, when the registry's own declarations outgrow the
/// envelope.
pub fn envelope_preflight(
    registry: &horizon_world::registry::BlockRegistry,
) -> Result<(), OpError> {
    let worst = 2 * 4096 * longest_state_bytes(registry) + MAX_OP_ENTITIES + 1024 * 1024;
    if worst > MAX_OP_DECODED {
        return Err(OpError::Malformed(
            "the block registry's longest state exceeds the operation envelope",
        ));
    }
    Ok(())
}

/// Whether a rotation is one of the cube's symmetries — a multiple of
/// ninety degrees about a single coordinate axis. Disassembly without a
/// state-rotation adapter (slice 4's `StructureCellPart`) refuses
/// anything else rather than silently rewriting facing properties: at
/// most two components are nonzero, the angle cosine is 0 or ±√½ or ±1,
/// and the sine rides exactly one axis.
pub fn cube_symmetry_rotation(components: [f64; 4]) -> bool {
    let [w, x, y, z] = components;
    if w == 0.0 && x == 0.0 && y == 0.0 && z == 0.0 {
        return false;
    }
    for value in [w, x, y, z] {
        if !value.is_finite() {
            return false;
        }
        let magnitude = value.abs();
        let on_axis = (magnitude - 1.0).abs() < 1.0e-12 || magnitude < 1.0e-12;
        let on_half = (magnitude - std::f64::consts::FRAC_1_SQRT_2).abs() < 1.0e-12;
        if !on_axis && !on_half {
            return false;
        }
    }
    // At most one of the vector components may be nonzero alongside the
    // angle cosine: a quarter turn about exactly one axis.
    [x, y, z].iter().filter(|v| v.abs() >= 1.0e-12).count() <= 1
}

/// What a recovery pass found and did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recovered {
    /// Chunks whose cells still held their before-images and were taken
    /// to the after-images.
    pub applied: usize,
    /// Chunks that already held their after-images untouched.
    pub verified: usize,
}

/// Why a recovery refused. Every variant but `NothingToDo` stops the
/// world's load: recovery never guesses, and never generates content.
#[derive(Debug)]
pub enum RecoveryRefused {
    /// The store itself refused to open.
    Store(Refusal),
    /// An in-progress operation's file is gone. By the approved contract
    /// that is a world-load refusal — the file was durable before
    /// `prepared`, so its absence is damage — never a quarantine.
    /// Quarantine belongs to a committed carrier whose content is
    /// unknown, which recovery here does not touch.
    MissingOperation,
    /// An in-progress operation's file does not decode. As with
    /// [`RecoveryRefused::MissingOperation`], the world's load refuses.
    OperationUnreadable(String),
    /// An affected chunk did not strictly read — damage, never absence.
    Region {
        /// The chunk's position.
        chunk: (i32, i32),
        /// Why it refused.
        why: horizon_world::carrier::region_strict::Strict,
    },
    /// A cell holds neither its before-image nor its after-image.
    CellMismatch {
        /// The cell's world position.
        pos: [i32; 3],
        /// What the cell actually holds, serialized.
        found: String,
    },
    /// The chunk NBT did not parse after a strict read.
    Chunk(String),
    /// The operation belongs to another dimension than the level being
    /// recovered; nothing was touched.
    NotThisDimension(String),
    /// The journal and the operation file name different dimensions:
    /// damage, so the world's load refuses.
    DimensionDisagreement {
        /// What the journal says.
        journal: String,
        /// What the operation file says.
        operation: String,
    },
    /// The operation file's identity (id, kind, UUID or generation)
    /// disagrees with the journal's prepared record: damage, so the
    /// world's load refuses.
    IdentityDisagreement {
        /// Which field disagreed.
        field: &'static str,
    },
    /// The prepared carrier's cargo disagrees with the operation's
    /// before-image: cell count, positions, names or entity content.
    CargoDisagreement {
        /// What disagreed.
        what: &'static str,
    },
    /// Plain I/O.
    Io(String),
}
impl fmt::Display for RecoveryRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(refusal) => write!(f, "{refusal}"),
            Self::MissingOperation => write!(
                f,
                "world load refused: the in-progress operation's file is gone"
            ),
            Self::OperationUnreadable(why) => {
                write!(
                    f,
                    "world load refused: the operation file does not decode: {why}"
                )
            }
            Self::Region { chunk, why } => {
                write!(f, "chunk ({},{}) refuses: {why}", chunk.0, chunk.1)
            }
            Self::CellMismatch { pos, found } => write!(
                f,
                "cell ({},{},{}) holds {found}, neither its before nor its after image",
                pos[0], pos[1], pos[2]
            ),
            Self::Chunk(why) => write!(f, "chunk data: {why}"),
            Self::NotThisDimension(dimension) => write!(
                f,
                "world load refused: the operation belongs to {dimension}, not this level"
            ),
            Self::DimensionDisagreement { journal, operation } => write!(
                f,
                "world load refused: the journal names {journal} but the operation file names {operation}"
            ),
            Self::IdentityDisagreement { field } => write!(
                f,
                "world load refused: the operation file's {field} disagrees with the journal"
            ),
            Self::CargoDisagreement { what } => write!(
                f,
                "world load refused: the prepared carrier's cargo disagrees: {what}"
            ),
            Self::Io(why) => write!(f, "recovery I/O: {why}"),
        }
    }
}

/// Replays the in-progress operation at world open, before chunks tick
/// and before players join: every affected chunk is strictly read, each
/// cell must hold either its before-image (apply the after-image) or its
/// after-image (leave it), and only when every write-back is synced does
/// the journal record `committed` and drop the operation file.
/// Roll-forward only — nothing is ever written back into a cell.
pub fn recover(
    store: &mut Store,
    region_dir: &Path,
    states: &dyn StateValidator,
    dimension: &str,
) -> Result<Recovered, RecoveryRefused> {
    let Some(op) = &store.journal().in_progress else {
        return Ok(Recovered::default());
    };
    let op_path = store.operation_path(op.op_id);
    let bytes = read_bounded(&op_path, MAX_OP_COMPRESSED).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => RecoveryRefused::MissingOperation,
        _ => RecoveryRefused::OperationUnreadable(error.to_string()),
    })?;
    let operation = decode_operation(&bytes, states)
        .map_err(|e| RecoveryRefused::OperationUnreadable(e.to_string()))?;
    // The operation file's identity must be exactly the journal's
    // prepared record: id, kind, carrier and generation, not only the
    // dimension.
    if operation.op_id != op.op_id {
        return Err(RecoveryRefused::IdentityDisagreement { field: "op_id" });
    }
    if operation.kind != op.kind {
        return Err(RecoveryRefused::IdentityDisagreement { field: "kind" });
    }
    if operation.uuid != op.uuid {
        return Err(RecoveryRefused::IdentityDisagreement { field: "uuid" });
    }
    if operation.generation != op.generation {
        return Err(RecoveryRefused::IdentityDisagreement {
            field: "generation",
        });
    }
    // The journal and the operation file must name one dimension, and
    // it must be the level recovery is running in: anything else is a
    // refusal that touches nothing.
    if operation.dimension != op.dimension {
        return Err(RecoveryRefused::DimensionDisagreement {
            journal: op.dimension.clone(),
            operation: operation.dimension.clone(),
        });
    }
    if operation.dimension != dimension {
        return Err(RecoveryRefused::NotThisDimension(
            operation.dimension.clone(),
        ));
    }
    // An assembling operation publishes its immutable carrier before
    // `prepared`: recovery verifies that file is in place with the
    // journal's UUID and generation — the cargo has exactly one
    // durable owner before the source is stripped — and refuses on
    // anything else. Nothing else here rewrites the carrier; the live
    // writer did, before prepared.
    if operation.kind == OpKind::Assemble {
        let path = store.carrier_path(operation.uuid, operation.generation);
        let bytes = read_bounded(&path, horizon_world::carrier::codec::MAX_INPUT_BYTES).map_err(
            |error| {
                RecoveryRefused::OperationUnreadable(format!(
                    "the prepared assembly's carrier file does not read: {error}"
                ))
            },
        )?;
        let carrier = horizon_world::carrier::codec::decode(&bytes).map_err(|e| {
            RecoveryRefused::OperationUnreadable(format!(
                "the prepared assembly's carrier does not decode: {e}"
            ))
        })?;
        if carrier.uuid() != operation.uuid || carrier.generation() != operation.generation {
            return Err(RecoveryRefused::IdentityDisagreement { field: "carrier" });
        }
        // The cargo itself must match the operation's before-image —
        // cell count, names and entity content — so a carrier that is
        // short of cells, or carries different ones, never strips a
        // source it cannot put back.
        if carrier.cells().len() != operation.cells.len() {
            return Err(RecoveryRefused::CargoDisagreement { what: "cell count" });
        }
        for cell in &operation.cells {
            let local = [cell.local[0], cell.local[1], cell.local[2]];
            let Some(carried) = carrier.cells().iter().find(|c| {
                [c.pos().x(), c.pos().y(), c.pos().z()] == local
                    && c.name() == cell.before.name
                    && c.properties().len() == cell.before.properties.len()
                    && c.properties().iter().all(|(key, value)| {
                        cell.before
                            .properties
                            .iter()
                            .any(|(k, v)| key.as_ref() == k && value.as_ref() == v)
                    })
            }) else {
                return Err(RecoveryRefused::CargoDisagreement { what: "cell state" });
            };
            let carried_entity = carried.data().map(|data| {
                horizon_core::nbt::read_named(data, horizon_core::nbt::Limits::saved())
                    .map(|(named, _)| named.tag)
            });
            let same_entity = match (&carried_entity, &cell.before.entity) {
                (None, None) => true,
                (Some(Ok(found)), Some(expected)) => entities_equal(found, expected),
                _ => false,
            };
            if !same_entity {
                return Err(RecoveryRefused::CargoDisagreement {
                    what: "entity content",
                });
            }
        }
    }
    let mut chunks: Vec<[i32; 2]> = Vec::new();
    for cell in &operation.cells {
        let key = [cell.world[0] >> 4, cell.world[2] >> 4];
        if !chunks.contains(&key) {
            chunks.push(key);
        }
    }
    let mut recovered = Recovered::default();
    for chunk in chunks {
        let root = horizon_world::carrier::region_strict::read_chunk(
            region_dir,
            horizon_core::position::ChunkPos::new(chunk[0], chunk[1]),
        )
        .map_err(|why| RecoveryRefused::Region {
            chunk: (chunk[0], chunk[1]),
            why,
        })?
        .ok_or(RecoveryRefused::Region {
            chunk: (chunk[0], chunk[1]),
            why: horizon_world::carrier::region_strict::Strict::Missing,
        })?;
        if replay_chunk(&operation, root, chunk, region_dir)? {
            recovered.applied += 1;
        } else {
            recovered.verified += 1;
        }
    }
    store
        .commit()
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    Ok(recovered)
}

/// Applies one chunk's cells: strict compare against the images, the
/// after-image written through the storage-level chunk (no block
/// behavior, exactly what a replay may do), then a synced write-back.
fn replay_chunk(
    operation: &Operation,
    root: horizon_core::nbt::NamedTag,
    chunk: [i32; 2],
    region_dir: &Path,
) -> Result<bool, RecoveryRefused> {
    let entities = root_entities(&root);
    let mut data = horizon_world::chunk::ChunkData::new(root).map_err(|e| {
        RecoveryRefused::Chunk(format!(
            "chunk ({},{}) does not parse: {e}",
            chunk[0], chunk[1]
        ))
    })?;
    let mut touched = false;
    for cell in &operation.cells {
        if [cell.world[0] >> 4, cell.world[2] >> 4] != chunk {
            continue;
        }
        let pos =
            horizon_core::position::BlockPos::new(cell.world[0], cell.world[1], cell.world[2]);
        let found = data.block(pos).map_err(|e| {
            RecoveryRefused::Chunk(format!(
                "cell ({},{},{}) does not read: {e}",
                pos.x, pos.y, pos.z
            ))
        })?;
        let entity = entities
            .iter()
            .find(|tag| entity_at(tag, [pos.x, pos.y, pos.z]))
            .cloned();
        // Exact witnesses: the state AND the block entity must equal
        // the image's. A matching state over a different entity is a
        // mismatch, never a silent pass.
        let state_is = |image: &CellImage| match &found {
            None => image.name == "minecraft:air" && image.properties.is_empty(),
            Some(state) => {
                state.name == image.name
                    && state.properties.len() == image.properties.len()
                    && image
                        .properties
                        .iter()
                        .all(|(key, value)| state.properties.get(key).is_some_and(|v| v == value))
            }
        };
        let entity_is = |image: &CellImage| match (&entity, &image.entity) {
            (None, None) => true,
            (Some(found), Some(expected)) => entities_equal(found, expected),
            _ => false,
        };
        if state_is(&cell.after) && entity_is(&cell.after) {
            continue;
        }
        if !(state_is(&cell.before) && entity_is(&cell.before)) {
            let found = found
                .map(|state| {
                    let mut text = state.name.clone();
                    for (key, value) in &state.properties {
                        text.push_str(&format!(",{key}={value}"));
                    }
                    if entity.is_some() {
                        text.push_str(",+entity");
                    }
                    text
                })
                .unwrap_or_else(|| "minecraft:air".to_string());
            return Err(RecoveryRefused::CellMismatch {
                pos: [pos.x, pos.y, pos.z],
                found,
            });
        }
        let after = horizon_world::chunk::BlockState {
            name: cell.after.name.clone(),
            properties: cell.after.properties.iter().cloned().collect(),
        };
        data.set_block_data(pos, after).map_err(|e| {
            RecoveryRefused::Chunk(format!(
                "cell ({},{},{}) does not write: {e}",
                pos.x, pos.y, pos.z
            ))
        })?;
        touched = true;
    }
    if !touched {
        return Ok(false);
    }
    // Block entities ride the root's own list: the after-image's entity
    // is installed, the before position's cleared, by plain NBT edits
    // so nothing else in the chunk moves.
    let root = data.into_nbt();
    let root = edit_block_entities(root, operation, chunk)?;
    write_chunk_back(region_dir, root, chunk)?;
    Ok(true)
}

/// The chunk root's `block_entities` list, by value.
fn root_entities(root: &horizon_core::nbt::NamedTag) -> Vec<horizon_core::nbt::Tag> {
    match root.tag.get("block_entities") {
        Some(horizon_core::nbt::Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    }
}

/// Whether an entity tag sits at `pos`.
fn entity_at(tag: &horizon_core::nbt::Tag, pos: [i32; 3]) -> bool {
    let read = |name: &str| match tag.get(name) {
        Some(horizon_core::nbt::Tag::Int(value)) => Some(*value),
        _ => None,
    };
    read("x") == Some(pos[0]) && read("y") == Some(pos[1]) && read("z") == Some(pos[2])
}

/// Whether two serialized block entities carry the same content: the
/// exact replay witness. The positional `x`/`y`/`z` ints are placement,
/// not content — the stored image keeps its own coordinates — so the
/// comparison strips them and then compares everything else.
fn entities_equal(a: &horizon_core::nbt::Tag, b: &horizon_core::nbt::Tag) -> bool {
    let strip = |tag: &horizon_core::nbt::Tag| -> horizon_core::nbt::Tag {
        let mut clone = tag.clone();
        if let horizon_core::nbt::Tag::Compound(map) = &mut clone {
            for key in ["x", "y", "z"] {
                map.remove(&horizon_core::nbt::JavaString::from(key));
            }
        }
        clone
    };
    horizon_core::nbt::compare_nbt(&strip(a), Some(&strip(b)))
}

/// Installs the after-images' block entities and clears the before
/// positions' in the chunk root, by NBT edits.
fn edit_block_entities(
    mut root: horizon_core::nbt::NamedTag,
    operation: &Operation,
    chunk: [i32; 2],
) -> Result<horizon_core::nbt::NamedTag, RecoveryRefused> {
    use horizon_core::nbt::Tag;
    let Tag::Compound(map) = &mut root.tag else {
        return Err(RecoveryRefused::Chunk(
            "chunk root is not a compound".into(),
        ));
    };
    let mut entities = match map.get(&"block_entities".into()) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    for cell in &operation.cells {
        if [cell.world[0] >> 4, cell.world[2] >> 4] != chunk {
            continue;
        }
        let (x, y, z) = (cell.world[0], cell.world[1], cell.world[2]);
        let at = |tag: &Tag| -> bool {
            let read = |name: &str| match tag.get(name) {
                Some(Tag::Int(v)) => Some(*v),
                _ => None,
            };
            read("x") == Some(x) && read("y") == Some(y) && read("z") == Some(z)
        };
        entities.retain(|tag| !at(tag));
        if let Some(entity) = &cell.after.entity {
            let Tag::Compound(mut entity_map) = entity.clone() else {
                return Err(RecoveryRefused::Chunk(
                    "after-image entity is not a compound".into(),
                ));
            };
            for (key, value) in [("x", x), ("y", y), ("z", z)] {
                entity_map.insert(key.into(), Tag::Int(value));
            }
            entities.push(Tag::Compound(entity_map));
        }
    }
    if entities.is_empty() {
        map.remove(&"block_entities".into());
    } else {
        map.insert("block_entities".into(), Tag::List(entities));
    }
    Ok(root)
}

/// Writes the replayed chunk back through the region file, synced: the
/// write must be on disk before the journal says `committed`.
fn write_chunk_back(
    region_dir: &Path,
    root: horizon_core::nbt::NamedTag,
    chunk: [i32; 2],
) -> Result<(), RecoveryRefused> {
    use horizon_world::region::ChunkRecord;
    let position = horizon_core::position::ChunkPos::new(chunk[0], chunk[1]);
    let record = ChunkRecord::from_nbt(
        &root,
        Compression::Zlib,
        0,
        horizon_core::nbt::Limits::saved(),
    )
    .map_err(|e| RecoveryRefused::Chunk(e.to_string()))?;
    let path = region_dir.join(format!("r.{}.{}.mca", position.x >> 5, position.z >> 5));
    // The folders the region goes in are made, and synced at the sync below.
    let mut region = horizon_world::region::RegionFile::open_writable(&path, true)
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    region
        .write(position, &record)
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    region
        .sync()
        .map_err(|e| RecoveryRefused::Io(e.to_string()))?;
    Ok(())
}
/// A store-level I/O failure, with the operation's text.
#[derive(Debug)]
pub struct StoreIoError(pub String);
impl fmt::Display for StoreIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "carrier store: {}", self.0)
    }
}

/// Whether the `level.dat` flag is set.
pub fn in_use_flag(world_root: &Path) -> bool {
    read_dat(&world_root.join("level.dat"))
        .as_ref()
        .and_then(|root| root.get("Data"))
        .is_some_and(|data| data.get(IN_USE_FLAG).is_some())
}

/// Writes the flag into `level.dat` through the ordinary dat replacement
/// (temporary, rename, `.dat_old`): a crash in the replacement gap rolls
/// back to the flagless file, so no store state may exist yet.
pub fn set_in_use_flag(level_dat: &Path) -> Result<(), String> {
    let mut root = read_dat(level_dat).ok_or("level.dat does not read")?;
    let Tag::Compound(map) = &mut root else {
        return Err("level.dat root is not a compound".into());
    };
    let data = map
        .get_mut(&JavaString::from("Data"))
        .and_then(|tag| match tag {
            Tag::Compound(data) => Some(data),
            _ => None,
        });
    let Some(data) = data else {
        return Err("level.dat has no Data compound".into());
    };
    data.insert(IN_USE_FLAG.into(), Tag::Byte(1));
    write_dat(level_dat, root).map_err(|e| e.to_string())
}

/// A slot's record number plus body, with the slot it was read from.
#[derive(Clone, Debug)]
struct SlotRecord {
    slot: usize,
    entry_no: u64,
    body: Journal,
    error: Option<SlotError>,
}
impl SlotRecord {
    const fn valid(&self) -> bool {
        self.error.is_none()
    }
    fn error(&self) -> SlotError {
        self.error.unwrap_or(SlotError::Invalid)
    }
}

/// Why a slot does not hold a valid record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotError {
    /// The bytes are not a complete, CRC-correct record of this schema
    /// family (a torn or corrupt write, or junk).
    Invalid,
    /// The bytes decode as a record from a newer schema.
    NewerSchema { found: u16 },
}

/// Reads both slots. `Ok(None)` when neither file exists.
fn read_slots(data: &Path) -> Result<Option<[SlotRecord; 2]>, Refusal> {
    let bytes = [read_slot_file(data, 0)?, read_slot_file(data, 1)?];
    match (&bytes[0], &bytes[1]) {
        (None, None) => Ok(None),
        (a, b) => Ok(Some([
            read_slot(0, a.as_deref().unwrap_or(&[])),
            read_slot(1, b.as_deref().unwrap_or(&[])),
        ])),
    }
}

/// One slot file's bytes, `None` when the file does not exist. A file
/// past the slot cap is read as empty — an invalid record — without
/// reading its bytes.
fn read_slot_file(data: &Path, slot: usize) -> Result<Option<Vec<u8>>, Refusal> {
    let path = data.join(SLOTS[slot]);
    if fs::metadata(&path)
        .map(|meta| meta.len() as usize > MAX_SLOT_FILE)
        .unwrap_or(false)
    {
        return Ok(Some(Vec::new()));
    }
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Refusal::Io(error.to_string())),
    }
}

/// One slot read: header, lengths, CRC and body, each checked before the
/// memory behind it is touched.
fn read_slot(slot: usize, bytes: &[u8]) -> SlotRecord {
    let invalid = |entry_no| SlotRecord {
        slot,
        entry_no,
        body: Journal::empty(),
        error: Some(SlotError::Invalid),
    };
    let newer = |found| SlotRecord {
        slot,
        entry_no: 0,
        body: Journal::empty(),
        error: Some(SlotError::NewerSchema { found }),
    };
    if bytes.len() < SLOT_HEADER || bytes[..4] != SLOT_MAGIC {
        return invalid(0);
    }
    let entry_no = u64::from_le_bytes(bytes[6..14].try_into().unwrap());
    let body_len = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(bytes[18..22].try_into().unwrap());
    // The declared body is checked against the slot cap and the bytes
    // actually present before anything is allocated for it.
    if body_len > MAX_SLOT_FILE - SLOT_HEADER || bytes.len() - SLOT_HEADER != body_len {
        return invalid(entry_no);
    }
    // The CRC covers everything after the magic: the version, the record
    // number, the body length and the body. A single flipped bit anywhere
    // makes the record invalid — an older record can never flip into
    // looking newer, and a corrupted version byte is not a schema.
    if slot_crc(bytes) != crc {
        return invalid(entry_no);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version > SLOT_VERSION {
        return newer(version);
    }
    let body = &bytes[SLOT_HEADER..];
    match decode_journal(body) {
        Some(journal) => SlotRecord {
            slot,
            entry_no,
            body: journal,
            error: None,
        },
        None => invalid(entry_no),
    }
}

/// Encodes one slot: header, then the CRC-protected body. The body is
/// checked against the slot cap before the file is written.
fn encode_slot(entry_no: u64, journal: &Journal, capacity: usize) -> Result<Vec<u8>, String> {
    let body = encode_journal(journal, capacity)?;
    let len = SLOT_HEADER + body.len();
    if len > MAX_SLOT_FILE {
        return Err(format!(
            "journal record of {len} bytes exceeds {MAX_SLOT_FILE}"
        ));
    }
    let mut bytes = Vec::with_capacity(len);
    bytes.extend_from_slice(&SLOT_MAGIC);
    bytes.extend_from_slice(&SLOT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&entry_no.to_le_bytes());
    bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
    bytes.extend_from_slice(
        &slot_crc_input(SLOT_VERSION, entry_no, body.len(), &body).to_le_bytes(),
    );
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

/// The slot CRC's input: everything after the magic — version, record
/// number, body length — followed by the body itself.
fn slot_crc_input(version: u16, entry_no: u64, body_len: usize, body: &[u8]) -> u32 {
    let mut input = Vec::with_capacity(14 + body.len());
    input.extend_from_slice(&version.to_le_bytes());
    input.extend_from_slice(&entry_no.to_le_bytes());
    input.extend_from_slice(&(body_len as u32).to_le_bytes());
    input.extend_from_slice(body);
    crc32fast::hash(&input)
}

/// The CRC of an encoded slot record, [`slot_crc_input`] over its own
/// bytes.
fn slot_crc(bytes: &[u8]) -> u32 {
    let body_len = u32::from_le_bytes(bytes[14..18].try_into().unwrap()) as usize;
    slot_crc_input(
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u64::from_le_bytes(bytes[6..14].try_into().unwrap()),
        body_len,
        &bytes[SLOT_HEADER..SLOT_HEADER + body_len],
    )
}

/// The body serializer: `next_op_id`, the in-progress operation, the
/// tombstone and carrier counts, then their fixed-size entries.
impl Journal {
    /// Store-level bound check for tests: whether the journal fits the
    /// capacity at encode.
    pub fn encode_for_tests(&self, capacity: usize) -> Result<Vec<u8>, String> {
        encode_journal(self, capacity)
    }
}

fn encode_journal(journal: &Journal, capacity: usize) -> Result<Vec<u8>, String> {
    if journal.carriers.len() > capacity {
        return Err(format!("carrier index exceeds {capacity}"));
    }
    if journal.tombstones.len() > MAX_TOMBSTONES {
        return Err(format!("tombstones exceed {MAX_TOMBSTONES}"));
    }
    let mut body =
        Vec::with_capacity(8 + 30 + 8 + (journal.tombstones.len() + journal.carriers.len()) * 20);
    body.extend_from_slice(&journal.next_op_id.to_le_bytes());
    match &journal.in_progress {
        None => body.push(0),
        Some(op) => {
            body.push(1);
            body.extend_from_slice(&op.op_id.to_le_bytes());
            body.push(op.kind.as_byte());
            body.extend_from_slice(&op.uuid.to_le_bytes());
            body.extend_from_slice(&op.generation.to_le_bytes());
            body.extend_from_slice(&(op.dimension.len() as u16).to_le_bytes());
            body.extend_from_slice(op.dimension.as_bytes());
        }
    }
    body.extend_from_slice(&(journal.tombstones.len() as u32).to_le_bytes());
    for &(uuid, generation) in &journal.tombstones {
        body.extend_from_slice(&uuid.to_le_bytes());
        body.extend_from_slice(&generation.to_le_bytes());
    }
    body.extend_from_slice(&(journal.carriers.len() as u32).to_le_bytes());
    for &(uuid, generation) in &journal.carriers {
        body.extend_from_slice(&uuid.to_le_bytes());
        body.extend_from_slice(&generation.to_le_bytes());
    }
    Ok(body)
}

/// The body parser: fixed 20-byte entries, counts bounded and checked
/// against the bytes present before the vectors grow, and every byte of
/// the body must be consumed.
fn decode_journal(body: &[u8]) -> Option<Journal> {
    // next_op_id, the in-progress flag, then, per section, a u32 count and
    // 20-byte entries.
    let mut at = 0_usize;
    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
        let head = body.get(*at..*at + n)?;
        *at += n;
        Some(head)
    };
    let next_op_id = u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap());
    let in_progress = match take(&mut at, 1)?[0] {
        0 => None,
        1 => {
            let op_id = u64::from_le_bytes(take(&mut at, 8)?.try_into().unwrap());
            let kind = OpKind::from_byte(take(&mut at, 1)?[0])?;
            let uuid = u128::from_le_bytes(take(&mut at, 16)?.try_into().unwrap());
            let generation = u32::from_le_bytes(take(&mut at, 4)?.try_into().unwrap());
            let dimension_len = u16::from_le_bytes(take(&mut at, 2)?.try_into().unwrap()) as usize;
            // Bounded before allocation: the length fits what remains.
            if body.len() < at + dimension_len {
                return None;
            }
            let dimension = String::from_utf8(take(&mut at, dimension_len)?.to_vec()).ok()?;
            Some(InProgress {
                op_id,
                kind,
                uuid,
                generation,
                dimension,
            })
        }
        _ => return None,
    };
    let section = |at: &mut usize, cap: usize| -> Option<Vec<(u128, u32)>> {
        let count = u32::from_le_bytes(take(at, 4)?.try_into().unwrap()) as usize;
        // Bounded before allocation, with every entry's bytes present;
        // the caller's exact-consumption check catches any slack.
        if count > cap || body.len() < *at + count * 20 {
            return None;
        }
        let entries = (0..count)
            .map(|_| {
                let uuid = u128::from_le_bytes(take(at, 16)?.try_into().unwrap());
                let generation = u32::from_le_bytes(take(at, 4)?.try_into().unwrap());
                Some((uuid, generation))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(entries)
    };
    let tombstones = section(&mut at, MAX_TOMBSTONES)?;
    let carriers = section(&mut at, MAX_CARRIERS)?;
    if at != body.len() {
        return None;
    }
    Some(Journal {
        next_op_id,
        in_progress,
        tombstones,
        carriers,
    })
}

/// The tombstones whose carrier files are not in `data/` right now.
/// They are the ones a later cleanup may clear: their deletions predate
/// this open.
fn tombstones_without_files(data: &Path, journal: &Journal) -> Result<Vec<(u128, u32)>, Refusal> {
    if journal.tombstones.is_empty() {
        return Ok(Vec::new());
    }
    let present = fs::read_dir(data)
        .map_err(|e| Refusal::Io(e.to_string()))?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| parse_carrier_name(&name))
        .collect::<Vec<_>>();
    Ok(journal
        .tombstones
        .iter()
        .filter(|tombstone| !present.contains(tombstone))
        .copied()
        .collect())
}

/// Whether `data/` holds any carrier file (operation or carrier), which
/// without a journal is a missing store whatever the flag says.
fn residual_files(data: &Path) -> Result<bool, Refusal> {
    let entries = match fs::read_dir(data) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Refusal::Io(error.to_string())),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if parse_operation_name(name).is_some() || parse_carrier_name(name).is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `horizon_carrier_op_<id>.nbt` → the id.
fn parse_operation_name(name: &str) -> Option<u64> {
    name.strip_prefix("horizon_carrier_op_")?
        .strip_suffix(".nbt")?
        .parse()
        .ok()
}

/// `horizon_carrier_<uuid>-<gen>.nbt` → the carrier.
fn parse_carrier_name(name: &str) -> Option<(u128, u32)> {
    let stem = name
        .strip_prefix("horizon_carrier_")?
        .strip_suffix(".nbt")?;
    let (uuid, generation) = stem.split_once('-')?;
    if uuid.len() != 32 || !uuid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((
        u128::from_str_radix(uuid, 16).ok()?,
        generation.parse().ok()?,
    ))
}

/// One side of a cell's image: the block state as the registry names it,
/// plus the cell's full block-entity NBT when it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct CellImage {
    /// The state's namespaced block name.
    pub name: String,
    /// The state's properties, any order; part B validates them against
    /// the live registry through [`StateValidator`].
    pub properties: Vec<(String, String)>,
    /// The block entity's full NBT, if the cell owns one.
    pub entity: Option<Tag>,
}

/// Decides whether a state image is a state the live registry declares.
/// Part A ships the shape only; part B supplies the registry-backed one,
/// so a state can never be silently accepted or rewritten.
pub trait StateValidator {
    /// Whether the registry declares this name with these properties.
    fn known(&self, name: &str, properties: &[(String, String)]) -> bool;
}

/// Every state is known: part A's stand-in while the registry wiring is
/// part B's.
pub struct AllStatesKnown;
impl StateValidator for AllStatesKnown {
    fn known(&self, _name: &str, _properties: &[(String, String)]) -> bool {
        true
    }
}

/// One cell of an operation: its world and local positions, and its
/// before and after images.
#[derive(Clone, Debug, PartialEq)]
pub struct OperationCell {
    /// The world cell the transfer touches.
    pub world: [i32; 3],
    /// The carrier-local cell it maps to.
    pub local: [i8; 3],
    /// The cell before the transfer.
    pub before: CellImage,
    /// The cell after the transfer.
    pub after: CellImage,
}

/// One transfer's file: what assembly or disassembly does to every cell
/// it touches, so replay can finish or strip exactly one side.
#[derive(Clone, Debug, PartialEq)]
pub struct Operation {
    /// The operation's id, from the journal's `next_op_id`.
    pub op_id: u64,
    /// Whether it assembles or disassembles.
    pub kind: OpKind,
    /// The carrier it moves.
    pub uuid: u128,
    /// The carrier's generation after this operation.
    pub generation: u32,
    /// The dimension the operation runs in; the journal's record must
    /// name the same one or the world's load refuses.
    pub dimension: String,
    /// The touched cells, at most [`MAX_OP_CELLS`].
    pub cells: Vec<OperationCell>,
}

/// Why an operation file could not be encoded or decoded.
#[derive(Clone, Debug, PartialEq)]
pub enum OpError {
    /// The decoded NBT would pass [`MAX_OP_DECODED`].
    DecodedTooBig {
        /// The offending size.
        size: usize,
    },
    /// The compressed file would pass [`MAX_OP_COMPRESSED`].
    CompressedTooBig {
        /// The offending size.
        size: usize,
    },
    /// The file or its NBT does not match the schema.
    Malformed(&'static str),
    /// More cells than [`MAX_OP_CELLS`].
    CellCount {
        /// The offending count.
        count: usize,
    },
    /// A block-entity image past [`MAX_CELL_ENTITY`].
    EntityTooBig {
        /// The offending size.
        size: usize,
    },
    /// The block-entity images past [`MAX_OP_ENTITIES`] in total.
    EntitiesTooBig {
        /// The offending total.
        total: usize,
    },
    /// A state image the validator does not know.
    UnknownState,
    /// A state string (block id, property name or value) past the NBT
    /// string limit, which the disk writer would truncate to empty.
    StringTooLong {
        /// The offending length.
        len: usize,
    },
}
impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DecodedTooBig { size } => {
                write!(f, "operation decodes to {size} bytes over {MAX_OP_DECODED}")
            }
            Self::CompressedTooBig { size } => {
                write!(
                    f,
                    "operation compresses to {size} bytes over {MAX_OP_COMPRESSED}"
                )
            }
            Self::Malformed(what) => write!(f, "operation file malformed: {what}"),
            Self::CellCount { count } => write!(f, "{count} cells exceeds {MAX_OP_CELLS}"),
            Self::EntityTooBig { size } => {
                write!(
                    f,
                    "block entity image of {size} bytes exceeds {MAX_CELL_ENTITY}"
                )
            }
            Self::EntitiesTooBig { total } => {
                write!(
                    f,
                    "block entity images total {total} bytes over {MAX_OP_ENTITIES}"
                )
            }
            Self::UnknownState => write!(f, "unknown block state image"),
            Self::StringTooLong { len } => write!(
                f,
                "state string of {len} bytes exceeds the NBT string limit"
            ),
        }
    }
}

/// The NBT read/write budgets for one operation file.
fn op_limits() -> Limits {
    Limits {
        max_bytes: MAX_OP_DECODED,
        max_nodes: MAX_OP_NODES,
        max_depth: MAX_OP_DEPTH,
    }
}

/// Encodes the whole operation into memory and refuses it there, before
/// any block is lifted: the decoded NBT must fit [`MAX_OP_DECODED`] and
/// the compressed file [`MAX_OP_COMPRESSED`], so a transfer that cannot
/// be stored never starts.
pub fn encode_operation(operation: &Operation) -> Result<Vec<u8>, OpError> {
    if operation.generation > i32::MAX as u32 {
        return Err(OpError::Malformed("generation exhausted"));
    }
    if operation.op_id > i64::MAX as u64 {
        return Err(OpError::Malformed("operation id exhausted"));
    }
    if operation.cells.len() > MAX_OP_CELLS {
        return Err(OpError::CellCount {
            count: operation.cells.len(),
        });
    }
    // One NBT string each: a string past the wire limit would be written
    // empty, so the refusal happens here, before any block is lifted.
    // The limit is the model's own constant, the same one Cell::new
    // enforces for the carrier codec.
    use horizon_world::carrier::model::MAX_NBT_STRING_BYTES as NBT_STRING_LIMIT;
    let string = |text: &str| -> Result<(), OpError> {
        if text.len() > NBT_STRING_LIMIT {
            return Err(OpError::StringTooLong { len: text.len() });
        }
        Ok(())
    };
    for cell in &operation.cells {
        for image in [&cell.before, &cell.after] {
            string(&image.name)?;
            for (key, value) in &image.properties {
                string(key)?;
                string(value)?;
            }
        }
    }
    let mut entity_bytes = 0_usize;
    for cell in &operation.cells {
        for image in [&cell.before, &cell.after] {
            if let Some(entity) = &image.entity {
                let size = nbt::write_named(
                    &NamedTag {
                        name: "".into(),
                        tag: entity.clone(),
                    },
                    op_limits(),
                )
                .map_err(|_| OpError::Malformed("block entity does not encode"))?
                .len();
                if size > MAX_CELL_ENTITY {
                    return Err(OpError::EntityTooBig { size });
                }
                entity_bytes += size;
            }
        }
    }
    if entity_bytes > MAX_OP_ENTITIES {
        return Err(OpError::EntitiesTooBig {
            total: entity_bytes,
        });
    }
    let raw = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: operation_tag(operation),
        },
        op_limits(),
    )
    .map_err(|e| OpError::Malformed(leak(e.reason)))?;
    if raw.len() > MAX_OP_DECODED {
        return Err(OpError::DecodedTooBig { size: raw.len() });
    }
    let compressed = Compression::Gzip
        .encode(&raw)
        .map_err(|_| OpError::Malformed("operation does not compress"))?;
    if compressed.len() > MAX_OP_COMPRESSED {
        return Err(OpError::CompressedTooBig {
            size: compressed.len(),
        });
    }
    Ok(compressed)
}

/// Decodes an operation file: the compressed size is checked before
/// decompression, every length against its budget before allocation, and
/// state images go through the validator.
pub fn decode_operation(bytes: &[u8], states: &dyn StateValidator) -> Result<Operation, OpError> {
    if bytes.len() > MAX_OP_COMPRESSED {
        return Err(OpError::CompressedTooBig { size: bytes.len() });
    }
    let raw = Compression::Gzip
        .decode(bytes, MAX_OP_DECODED)
        .map_err(|_| OpError::Malformed("operation does not decompress"))?;
    let (root, used) =
        nbt::read_named(&raw, op_limits()).map_err(|e| OpError::Malformed(leak(e.reason)))?;
    if used != raw.len() {
        return Err(OpError::Malformed("bytes follow the operation root"));
    }
    let root = root.tag;
    if !matches!(root, Tag::Compound(_)) {
        return Err(OpError::Malformed("root is not a compound"));
    }
    let int = |key: &'static str| -> Result<i32, OpError> {
        match root.get(key) {
            Some(Tag::Int(value)) => Ok(*value),
            _ => Err(OpError::Malformed(leak_key(key))),
        }
    };
    let long = |key: &'static str| -> Result<i64, OpError> {
        match root.get(key) {
            Some(Tag::Long(value)) => Ok(*value),
            _ => Err(OpError::Malformed(leak_key(key))),
        }
    };
    if int("version")? != OP_VERSION {
        return Err(OpError::Malformed("unsupported operation version"));
    }
    let kind = match root.get("kind") {
        Some(Tag::String(value)) => match value.to_string().as_deref() {
            Ok("assemble") => OpKind::Assemble,
            Ok("disassemble") => OpKind::Disassemble,
            _ => return Err(OpError::Malformed("unknown operation kind")),
        },
        _ => return Err(OpError::Malformed(leak_key("kind"))),
    };
    let op_id = u64::try_from(long("op_id")?).map_err(|_| OpError::Malformed(leak_key("op_id")))?;
    let uuid = (((long("carrier_uuid_msb")? as u64) as u128) << 64)
        | (long("carrier_uuid_lsb")? as u64) as u128;
    let generation = u32::try_from(int("carrier_generation")?)
        .map_err(|_| OpError::Malformed(leak_key("carrier_generation")))?;
    let dimension = match root.get("dimension") {
        Some(Tag::String(value)) => value
            .to_string()
            .map_err(|_| OpError::Malformed("dimension is not valid UTF-16"))?,
        _ => return Err(OpError::Malformed(leak_key("dimension"))),
    };
    let cells_tag = match root.get("cells") {
        Some(Tag::List(cells)) => cells,
        _ => return Err(OpError::Malformed(leak_key("cells"))),
    };
    // The cell count is checked before the vector is allocated.
    if cells_tag.len() > MAX_OP_CELLS {
        return Err(OpError::CellCount {
            count: cells_tag.len(),
        });
    }
    let mut cells = Vec::with_capacity(cells_tag.len());
    let mut entity_bytes = 0_usize;
    for cell_tag in cells_tag {
        let Tag::Compound(cell_map) = cell_tag else {
            return Err(OpError::Malformed("cell is not a compound"));
        };
        let world = match cell_map.get(&"world".into()) {
            Some(Tag::IntArray(values)) if values.len() == 3 => [values[0], values[1], values[2]],
            _ => return Err(OpError::Malformed(leak_key("world"))),
        };
        let local = match cell_map.get(&"local".into()) {
            Some(Tag::ByteArray(values)) if values.len() == 3 => {
                [values[0] as i8, values[1] as i8, values[2] as i8]
            }
            _ => return Err(OpError::Malformed(leak_key("local"))),
        };
        let before = decode_image(cell_map.get(&"before".into()), states, &mut entity_bytes)?;
        let after = decode_image(cell_map.get(&"after".into()), states, &mut entity_bytes)?;
        cells.push(OperationCell {
            world,
            local,
            before,
            after,
        });
    }
    if entity_bytes > MAX_OP_ENTITIES {
        return Err(OpError::EntitiesTooBig {
            total: entity_bytes,
        });
    }
    Ok(Operation {
        op_id,
        kind,
        uuid,
        generation,
        dimension,
        cells,
    })
}

/// One cell image read from a cell compound.
fn decode_image(
    tag: Option<&Tag>,
    states: &dyn StateValidator,
    entity_bytes: &mut usize,
) -> Result<CellImage, OpError> {
    let Tag::Compound(map) = tag.ok_or(OpError::Malformed(leak_key("image")))? else {
        return Err(OpError::Malformed("image is not a compound"));
    };
    let name = match map.get(&"state".into()) {
        Some(Tag::String(value)) => value
            .to_string()
            .map_err(|_| OpError::Malformed("state name is not valid UTF-16"))?,
        _ => return Err(OpError::Malformed(leak_key("state"))),
    };
    let mut properties = Vec::new();
    if let Some(properties_map) = map.get(&"properties".into()) {
        let Tag::Compound(properties_map) = properties_map else {
            return Err(OpError::Malformed(leak_key("properties")));
        };
        // Bounded by the already-parsed compound's own node budget.
        properties = properties_map
            .iter()
            .map(|(key, value)| match value {
                Tag::String(value) => Ok((
                    key.to_string()
                        .map_err(|_| OpError::Malformed("property name is not valid UTF-16"))?,
                    value
                        .to_string()
                        .map_err(|_| OpError::Malformed("property value is not valid UTF-16"))?,
                )),
                _ => Err(OpError::Malformed("property is not a string")),
            })
            .collect::<Result<Vec<_>, _>>()?;
    }
    if !states.known(&name, &properties) {
        return Err(OpError::UnknownState);
    }
    let entity = match map.get(&"entity".into()) {
        Some(entity) => {
            let size = nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag: entity.clone(),
                },
                op_limits(),
            )
            .map_err(|e| OpError::Malformed(leak(e.reason)))?
            .len();
            if size > MAX_CELL_ENTITY {
                return Err(OpError::EntityTooBig { size });
            }
            *entity_bytes += size;
            Some(entity.clone())
        }
        None => None,
    };
    Ok(CellImage {
        name,
        properties,
        entity,
    })
}

/// The operation's NBT tree.
fn operation_tag(operation: &Operation) -> Tag {
    let cells = operation
        .cells
        .iter()
        .map(|cell| {
            let image = |image: &CellImage| {
                Tag::compound(
                    [("state", Tag::String(image.name.as_str().into()))]
                        .into_iter()
                        .chain((!image.properties.is_empty()).then(|| {
                            (
                                "properties",
                                Tag::compound(
                                    image
                                        .properties
                                        .iter()
                                        .map(|(key, value)| {
                                            (key.as_str(), Tag::String(value.as_str().into()))
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                            )
                        }))
                        .chain(image.entity.clone().map(|entity| ("entity", entity))),
                )
            };
            Tag::compound([
                (
                    "world",
                    Tag::IntArray(vec![cell.world[0], cell.world[1], cell.world[2]]),
                ),
                (
                    "local",
                    Tag::ByteArray(vec![
                        cell.local[0] as u8,
                        cell.local[1] as u8,
                        cell.local[2] as u8,
                    ]),
                ),
                ("before", image(&cell.before)),
                ("after", image(&cell.after)),
            ])
        })
        .collect::<Vec<_>>();
    Tag::compound([
        ("version", Tag::Int(OP_VERSION)),
        ("op_id", Tag::Long(operation.op_id as i64)),
        ("kind", Tag::String(operation.kind.as_str().into())),
        (
            "carrier_uuid_msb",
            Tag::Long((operation.uuid >> 64) as u64 as i64),
        ),
        ("carrier_uuid_lsb", Tag::Long(operation.uuid as u64 as i64)),
        ("carrier_generation", Tag::Int(operation.generation as i32)),
        (
            "dimension",
            Tag::String(operation.dimension.as_str().into()),
        ),
        ("cells", Tag::List(cells)),
    ])
}

/// A static reason from a shared NBT error string.
fn leak(reason: &'static str) -> &'static str {
    reason
}

/// A static reason naming `key`.
fn leak_key(key: &str) -> &'static str {
    match key {
        "version" => "version is not an int",
        "op_id" => "op_id is not a long",
        "kind" => "kind is not a string",
        "carrier_uuid_msb" => "carrier_uuid_msb is not a long",
        "carrier_uuid_lsb" => "carrier_uuid_lsb is not a long",
        "carrier_generation" => "carrier_generation is not an int",
        "dimension" => "dimension is not a string",
        "cells" => "cells is not a list",
        "world" => "world is not three ints",
        "local" => "local is not three bytes",
        "state" => "state is not a string",
        "properties" => "properties is not a compound",
        "entity" => "entity is not an NBT tag",
        _ => "unexpected field",
    }
}

/// Read a disk boundary with one sentinel byte, never allocating the whole file.
pub fn read_bounded(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "carrier input exceeds its byte limit",
        ));
    }
    Ok(bytes)
}
