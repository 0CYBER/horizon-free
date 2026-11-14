//! Carrier transfers in the server's game: the fences that hold the
//! world's cells and the carrier still while an operation is between
//! prepared and committed, and the assembly that runs at a tick
//! boundary (issue #2413, slice 2 part C).
//!
//! A transfer is one synchronous call on the server's thread at a tick
//! boundary: no tick runs between `prepared` and `committed`, so routes
//! without their own fence chokepoint (the player's manual block
//! placement, guest container clicks, attachment writes) cannot
//! interleave today. If the commit ever becomes asynchronous, those
//! routes need chokepoints of their own.
//!
//! The fences are one set of locked world cells plus locked carriers on
//! the server's level. Each refusal route is checked at one chokepoint:
//! breaking (`Game::break_block`), every mod write through the level's
//! write gate (`native_level_write_allowed`), pistons
//! (`Game::allow_piston_move`) and block-entity container opening
//! (`refuse_locked_container`). With no operation in flight the check is
//! one load of an empty set.

use crate::carrier_store::{MAX_CARRIERS, Store};
use crate::game::Game;
use crate::save::block_entity_encode;
use horizon_core::position::BlockPos;
use horizon_world::carrier::codec;
use horizon_world::carrier::contact::RigidTransform;
use horizon_world::carrier::model::{Carrier, Cell, Invalid, LocalPos, MAX_CELLS};
use horizon_world::level::LevelAccess;
use horizon_world::level::data::state_flags::REPLACEABLE;

/// Why a transfer refused. Refusals leave no partial transfer.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug)]
pub(crate) enum TransferError {
    /// A cell's chunk is not loaded and ticking.
    CellNotLoaded([i32; 3]),
    /// More cells than the carrier model allows (4,096).
    TooManyCells(usize),
    /// A cell holds a block this slice does not move (not a plain block
    /// or a passive block entity).
    UnsupportedBlock([i32; 3]),
    /// Saved block-entity tags have no transfer payload adapter.
    SavedBlockEntity([i32; 3]),
    /// The registry's own states cannot fit the operation envelope.
    Envelope,
    /// The model refused the assembled carrier.
    Model(Invalid),
    /// The store or its files refused.
    Store(String),
    /// Revalidation after the lift found a cell that did not move.
    Revalidation([i32; 3]),
    /// The rotation is not one of the cube's symmetries.
    NotCubeSymmetry,
    /// The selection's span exceeds the local coordinate range, or its
    /// offsets overflowed the checked arithmetic.
    LocalRange,
    /// An empty selection was refused before any indexing.
    EmptySelection,
    /// A rotation other than identity needs a state-rotation adapter
    /// (slice 4's); without one facing properties would be rewritten
    /// silently.
    RotationNeedsAdapter,
    /// A disassembly destination is neither air nor a replaceable block
    /// without a block entity.
    DestinationOccupied([i32; 3]),
    /// The carrier file is missing or does not decode.
    CarrierUnreadable(String),
    /// Plain I/O.
    Io(String),
    /// Only the logical server transfers; a client's mirror of the
    /// world receives the result through synchronization.
    NotLogicalServer,
}
impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CellNotLoaded(pos) => write!(
                f,
                "cell ({},{},{}) is not in a loaded, ticking chunk",
                pos[0], pos[1], pos[2]
            ),
            Self::TooManyCells(count) => write!(f, "{count} cells exceeds {MAX_CELLS}"),
            Self::UnsupportedBlock(pos) => write!(
                f,
                "cell ({},{},{}) holds a block this slice does not transfer",
                pos[0], pos[1], pos[2]
            ),
            Self::SavedBlockEntity(pos) => {
                write!(f, "saved block entity at {pos:?} prevents transfer")
            }
            Self::Envelope => write!(f, "the registry's states exceed the operation envelope"),
            Self::Model(invalid) => write!(f, "{invalid}"),
            Self::Store(text) => write!(f, "{text}"),
            Self::Revalidation(pos) => write!(
                f,
                "revalidation: cell ({},{},{}) did not move",
                pos[0], pos[1], pos[2]
            ),
            Self::NotCubeSymmetry => write!(f, "the rotation is not a cube symmetry"),
            Self::EmptySelection => write!(f, "an empty selection refuses"),
            Self::LocalRange => {
                write!(f, "the selection's span exceeds the local coordinate range")
            }
            Self::RotationNeedsAdapter => {
                write!(f, "a rotated disassembly needs a state adapter")
            }
            Self::DestinationOccupied(pos) => write!(
                f,
                "destination ({},{},{}) is neither air nor replaceable",
                pos[0], pos[1], pos[2]
            ),
            Self::CarrierUnreadable(why) => write!(f, "carrier file: {why}"),
            Self::Io(text) => write!(f, "carrier transfer I/O: {text}"),
            Self::NotLogicalServer => write!(
                f,
                "only the logical server transfers; this game is a client's mirror"
            ),
        }
    }
}

/// Whether a block entity is passive cargo this slice carries: a
/// container or a sign, banner, skull or bell — pure data, no ticker.
/// Ticking machines (furnaces, brewing stands, spawners, sculk,
/// jukeboxes, beacons, campfires, daylight detectors, conduits,
/// gateways) and world-dependent ones refuse the assembly until
/// slice 4's adapters exist.
fn block_entity_is_passive(entity: &horizon_client::chunk::block_entity::BlockEntity) -> bool {
    use horizon_client::chunk::block_entity::BlockEntity;
    matches!(
        entity,
        BlockEntity::Chest(_)
            | BlockEntity::Sign(_)
            | BlockEntity::Banner(_)
            | BlockEntity::Skull(_)
            | BlockEntity::Bell
            | BlockEntity::Lectern(_)
    )
}

/// Whether a live block entity's serialized form carries exactly the
/// image's content: the positional `x`/`y`/`z` are placement, not
/// content, so the comparison strips them first.
fn entities_content_equal(live: &horizon_core::nbt::Tag, image: &horizon_core::nbt::Tag) -> bool {
    let strip = |tag: &horizon_core::nbt::Tag| -> horizon_core::nbt::Tag {
        let mut clone = tag.clone();
        if let horizon_core::nbt::Tag::Compound(map) = &mut clone {
            for key in ["x", "y", "z"] {
                map.remove(&horizon_core::nbt::JavaString::from(key));
            }
        }
        clone
    };
    horizon_core::nbt::compare_nbt(&strip(live), Some(&strip(image)))
}

impl Game<'_> {
    /// Assembles `cells` into one carrier. One synchronous call on the
    /// server's thread at a tick boundary — no tick runs between
    /// `prepared` and `committed`, which is why routes without their own
    /// fence chokepoint (the player's manual block placement, guest
    /// container clicks, attachment writes) cannot interleave; if the
    /// commit ever becomes asynchronous, they will need chokepoints.
    ///
    /// Order: preflight; the operation encoded in memory; the affected
    /// chunks forced to disk with the writer's acknowledged barrier
    /// (before the #2488 obligation, under which saves refuse); a
    /// revalidation that every cell still holds exactly its preflight
    /// state and entity — a mismatch refuses with the world and the
    /// store untouched; then, under the obligation, the operation file
    /// and `prepared` durably, the lift (owned removal, no drops), the
    /// fences, and forward only: after `prepared` an I/O failure leaves
    /// the fences up, marks the transfer broken so every later transfer
    /// refuses until the world reopens, and hands the operation to
    /// recovery at open. The carrier file and the chunks go out through
    /// the flush barrier, the journal commits, the fences drop.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn carrier_assemble(
        &mut self,
        uuid: u128,
        cells: &[BlockPos],
    ) -> Result<(), TransferError> {
        use crate::carrier_store::{CellImage, OpKind, encode_operation, envelope_preflight};
        use crate::carrier_store::{Operation, OperationCell};
        use horizon_client::join_seq::visibility::PassLease;

        // A remote guest's mirror never transfers: the server's own
        // transfer reaches it through synchronization, so a guest's
        // call refuses with a reason rather than silently succeeding
        // (the disassembly's gate refuses the same way).
        if !self.is_logical_server() {
            return Err(TransferError::NotLogicalServer);
        }
        if self.world.carrier_fences.is_broken() {
            return Err(TransferError::Store(
                "a previous carrier transfer did not finish; reopen the world".into(),
            ));
        }
        if cells.is_empty() {
            // An empty assembly refuses before any indexing or lookup.
            return Err(TransferError::EmptySelection);
        }
        if cells.len() > MAX_CELLS {
            return Err(TransferError::TooManyCells(cells.len()));
        }
        let registry = self.pack.assets.registry;
        envelope_preflight(registry).map_err(|_| TransferError::Envelope)?;

        // Preflight: loaded chunks; plain blocks or passive block
        // entities (this slice moves containers; ticking adapters are
        // slice 4's).
        let air = registry
            .default_state("minecraft:air")
            .ok_or_else(|| TransferError::Io("no air in the registry".into()))?;
        for pos in cells {
            let (x, z) = (pos.x >> 4, pos.z >> 4);
            if !self.world.chunks.contains_key(&(x, z)) {
                return Err(TransferError::CellNotLoaded([pos.x, pos.y, pos.z]));
            }
            if self.world.block(pos.x, pos.y, pos.z) == air {
                return Err(TransferError::UnsupportedBlock([pos.x, pos.y, pos.z]));
            }
            // Saved-only entities have no carrier payload adapter yet.
            if super::spill::saved_entity(&self.world, [pos.x, pos.y, pos.z]).is_some() {
                return Err(TransferError::SavedBlockEntity([pos.x, pos.y, pos.z]));
            }
            // The passive-or-supported rule runs before anything is
            // lifted: a block entity this slice does not carry (ticking
            // machines, world-dependent ones) refuses in preflight, with
            // the world untouched.
            if let Some(entity) = self.world.block_entities.get(&(pos.x, pos.y, pos.z))
                && !block_entity_is_passive(entity)
            {
                return Err(TransferError::UnsupportedBlock([pos.x, pos.y, pos.z]));
            }
        }

        // The store exists only from the first actual transfer. The
        // capacity override is the tests' retention seam.
        #[cfg(test)]
        let capacity = self
            .world
            .carrier_fences
            .capacity_override
            .unwrap_or(MAX_CARRIERS);
        #[cfg(not(test))]
        let capacity = MAX_CARRIERS;
        let mut store = match Store::open_with_capacity(&self.world_root, capacity) {
            Ok(crate::carrier_store::Opened::Live(store)) => store,
            Ok(crate::carrier_store::Opened::NeverUsed) => {
                // The gate reads the flag captured at world open, not
                // the folder's existence now: a data folder created
                // this session does not pass for an old one.
                Store::first_use_with_capacity(
                    &self.world_root,
                    self.world.carrier_data_existed_at_open,
                    capacity,
                )
                .map_err(|e| TransferError::Store(e.to_string()))?
            }
            Err(e) => return Err(TransferError::Store(e.to_string())),
        };

        store.use_retention_snapshot(&self.carrier_retention_snapshot);

        // Before/after images from the live world, encoded in memory —
        // a transfer that cannot be stored never lifts a block.
        let mut operation_cells = Vec::with_capacity(cells.len());
        for pos in cells {
            let key = [pos.x, pos.y, pos.z];
            let state = self.world.block(pos.x, pos.y, pos.z);
            let descriptor = registry
                .state(state)
                .ok_or(TransferError::UnsupportedBlock(key))?;
            let entity = self
                .world
                .block_entities
                .get(&(key[0], key[1], key[2]))
                .map(|entity| {
                    block_entity_encode::encode((key[0], key[1], key[2]), entity, &descriptor.name)
                });
            operation_cells.push(OperationCell {
                world: key,
                local: [0, 0, 0],
                before: CellImage {
                    name: descriptor.name.clone(),
                    properties: descriptor
                        .properties
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                    entity,
                },
                after: CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            });
        }
        // Local coordinates derive from the selection's minimum corner
        // — the pivot — so the input order of `cells` no longer shapes
        // geometry: a cell's local position is its offset from that
        // corner, computed with checked arithmetic and checked against
        // the i8 range before it narrows. The sparse layout survives
        // (gaps stay gaps), and any selection whose span exceeds the
        // local range refuses before a block is lifted.
        let pivot = [
            cells.iter().map(|p| p.x).min().unwrap_or(0),
            cells.iter().map(|p| p.y).min().unwrap_or(0),
            cells.iter().map(|p| p.z).min().unwrap_or(0),
        ];
        for cell in &mut operation_cells {
            let local = [
                cell.world[0]
                    .checked_sub(pivot[0])
                    .ok_or(TransferError::LocalRange)?,
                cell.world[1]
                    .checked_sub(pivot[1])
                    .ok_or(TransferError::LocalRange)?,
                cell.world[2]
                    .checked_sub(pivot[2])
                    .ok_or(TransferError::LocalRange)?,
            ];
            if local
                .iter()
                .any(|v| !(i32::from(i8::MIN)..=i32::from(i8::MAX)).contains(v))
            {
                return Err(TransferError::LocalRange);
            }
            cell.local = [local[0] as i8, local[1] as i8, local[2] as i8];
        }
        let carrier_cells = operation_cells
            .iter()
            .map(|cell| -> Result<Cell, TransferError> {
                // The entity rides the carrier file whole: its NBT
                // serialized once, so a later disassembly replays it. A
                // serialization failure is a refusal — the entity is
                // never silently dropped from the carrier.
                let data = cell
                    .before
                    .entity
                    .as_ref()
                    .map(|entity| {
                        horizon_core::nbt::write_named(
                            &horizon_core::nbt::NamedTag {
                                name: "".into(),
                                tag: entity.clone(),
                            },
                            horizon_core::nbt::Limits::saved(),
                        )
                        .map_err(|e| TransferError::Io(e.to_string()))
                    })
                    .transpose()?;
                Cell::new(
                    LocalPos::new(cell.local[0], cell.local[1], cell.local[2]),
                    &cell.before.name,
                    cell.before.properties.clone(),
                    data,
                )
                .map_err(TransferError::Model)
            })
            .collect::<Result<Vec<_>, TransferError>>()?;
        // A re-assembly of a UUID the journal remembers takes the next
        // generation, bounded by the signed operation format. The durable
        // operation counter is a lower bound even for a fresh UUID.
        let generation = store
            .next_generation(uuid)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        let carrier = Carrier::new(
            uuid,
            generation,
            0,
            RigidTransform::identity_at(horizon_core::geometry::Vec3::new(
                pivot[0] as f64,
                pivot[1] as f64,
                pivot[2] as f64,
            ))
            .map_err(|_| TransferError::Io("the identity placement refused".into()))?,
            carrier_cells,
        )
        .map_err(TransferError::Model)?;

        let op_id = store.next_op_id();
        let operation = Operation {
            op_id,
            kind: OpKind::Assemble,
            uuid,
            generation,
            dimension: self.dim.id().to_string(),
            cells: operation_cells.clone(),
        };
        let encoded = encode_operation(&operation).map_err(|_| TransferError::Envelope)?;

        // The affected chunks reach the disk, acknowledged, before the
        // obligation: under #2488 saves refuse, so this must precede the
        // lease. Replay will find its pre-image here.
        store
            .check_admission(OpKind::Assemble, uuid, generation)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        self.save_affected(cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;

        // Revalidation, before any intent is published: every cell must
        // still hold exactly the state and the entity the images were
        // built from. A mismatch refuses with the world and the store
        // untouched.
        self.revalidate_cells(&operation_cells, registry)?;

        // From here the path runs forward to `committed`. The lift is
        // under #2488's obligation, fenced until the barrier confirms.
        let _lease = PassLease::enter();
        // The immutable carrier is published and verified by reading it
        // back BEFORE any intent exists: once `prepared` runs, the
        // cargo's only durable owner is this file, and recovery only
        // verifies it — nothing after the lift ever rewrites it.
        let carrier_bytes =
            codec::encode(&carrier).map_err(|e| TransferError::Store(e.to_string()))?;
        store
            .write_carrier(uuid, generation, &carrier_bytes)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        let read_back = crate::carrier_store::read_bounded(
            &store.carrier_path(uuid, generation),
            codec::MAX_INPUT_BYTES,
        )
        .map_err(|e| TransferError::Store(e.to_string()))?;
        if read_back != carrier_bytes {
            return Err(TransferError::Store(
                "the published carrier did not read back identically".into(),
            ));
        }
        store
            .write_operation(op_id, &encoded)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        store
            .begin(OpKind::Assemble, uuid, generation, self.dim.id())
            .map_err(|e| TransferError::Store(e.to_string()))?;
        for pos in cells {
            let old = self.world.block(pos.x, pos.y, pos.z);
            // Owned removal without drops or notifications: the carrier
            // takes the entity whole, and the cell becomes air through
            // the level's own write path.
            self.with_level(|_b, level, ticks| {
                level.remove_block_entity(*pos, old, false);
                _b.set_block(level, ticks, *pos, air, 3);
            });
            self.world.modified.insert((pos.x >> 4, pos.z >> 4));
        }
        // Fence the world cells and the carrier until the barrier lifts.
        for pos in cells {
            self.world.carrier_fences.lock_cell([pos.x, pos.y, pos.z]);
        }
        self.world.carrier_fences.lock_carrier(uuid);
        drop(_lease);

        // The application is this same synchronous call's own writes; a
        // cell that is not air here is a bug, not a refusal path.
        for pos in cells {
            debug_assert_eq!(self.world.block(pos.x, pos.y, pos.z), air);
        }

        // Forward only: an I/O failure past `prepared` keeps the fences,
        // marks the transfer broken and leaves the operation to
        // recovery at world open.
        if let Err(error) = self.finish_assemble(&mut store, uuid, generation, cells) {
            log::error!("carrier transfer did not finish: {error}");
            self.world.carrier_fences.mark_broken();
            carrier_stop_session(self, &error);
            return Err(error);
        }
        for pos in cells {
            self.world.carrier_fences.unlock_cell([pos.x, pos.y, pos.z]);
        }
        self.world.carrier_fences.unlock_carrier(uuid);
        Ok(())
    }

    /// Disassembles the carrier back into the world, the assembly's
    /// mirror with the same order rules. `translation` places the
    /// carrier's local origin in the world on the grid; `rotation` must
    /// be one of the cube's symmetries, and this slice — with no
    /// state-rotation adapter — accepts only the identity, refusing
    /// rather than silently rewriting facing properties. Destinations
    /// must be air or a replaceable block without a block entity; the
    /// entity returns from the carrier's own payload, then the ordinary
    /// neighbour updates run. One synchronous server call: no tick runs
    /// between `prepared` and `committed`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn carrier_disassemble(
        &mut self,
        uuid: u128,
        generation: u32,
        translation: [i32; 3],
        rotation: [f64; 4],
    ) -> Result<(), TransferError> {
        use crate::carrier_store::{
            CellImage, OpKind, Operation, OperationCell, cube_symmetry_rotation, envelope_preflight,
        };
        use horizon_client::join_seq::visibility::PassLease;
        use horizon_core::nbt::{self, Tag};

        // A remote guest's mirror never transfers: the server's own
        // transfer reaches it through synchronization, so a guest's
        // call refuses with a reason rather than silently succeeding
        // (the assembly's gate refuses the same way). This is the
        // logical-server guard the review's item 5 asked for.
        if !self.is_logical_server() {
            return Err(TransferError::NotLogicalServer);
        }
        if self.world.carrier_fences.is_broken() {
            return Err(TransferError::Store(
                "a previous carrier transfer did not finish; reopen the world".into(),
            ));
        }
        if !cube_symmetry_rotation(rotation) {
            return Err(TransferError::NotCubeSymmetry);
        }
        if rotation != [1.0, 0.0, 0.0, 0.0] {
            return Err(TransferError::RotationNeedsAdapter);
        }
        let registry = self.pack.assets.registry;
        envelope_preflight(registry).map_err(|_| TransferError::Envelope)?;

        #[cfg(test)]
        let capacity = self
            .world
            .carrier_fences
            .capacity_override
            .unwrap_or(MAX_CARRIERS);
        #[cfg(not(test))]
        let capacity = MAX_CARRIERS;
        let mut store = match Store::open_with_capacity(&self.world_root, capacity) {
            Ok(crate::carrier_store::Opened::Live(store)) => store,
            _ => {
                return Err(TransferError::Store(
                    "the carrier store does not open".into(),
                ));
            }
        };
        store.use_retention_snapshot(&self.carrier_retention_snapshot);
        if !store.journal().lists(uuid, generation) {
            return Err(TransferError::CarrierUnreadable(
                "the journal does not list this carrier".into(),
            ));
        }
        let carrier_bytes = crate::carrier_store::read_bounded(
            &store.carrier_path(uuid, generation),
            codec::MAX_INPUT_BYTES,
        )
        .map_err(|e| TransferError::CarrierUnreadable(e.to_string()))?;
        let carrier = codec::decode(&carrier_bytes)
            .map_err(|e| TransferError::CarrierUnreadable(e.to_string()))?;

        // Destinations: on loaded chunks, air or replaceable without a
        // block entity. The images are built from the carrier itself.
        let mut operation_cells = Vec::with_capacity(carrier.cells().len());
        for cell in carrier.cells() {
            // Destination arithmetic is checked: an overflowing
            // placement refuses rather than wrapping the world.
            let offset = [
                i32::from(cell.pos().x()),
                i32::from(cell.pos().y()),
                i32::from(cell.pos().z()),
            ];
            let world = [
                translation[0]
                    .checked_add(offset[0])
                    .ok_or(TransferError::LocalRange)?,
                translation[1]
                    .checked_add(offset[1])
                    .ok_or(TransferError::LocalRange)?,
                translation[2]
                    .checked_add(offset[2])
                    .ok_or(TransferError::LocalRange)?,
            ];
            if !self
                .world
                .chunks
                .contains_key(&(world[0] >> 4, world[2] >> 4))
            {
                return Err(TransferError::CellNotLoaded(world));
            }
            let state = self.world.block(world[0], world[1], world[2]);
            let occupied = self
                .world
                .block_entities
                .contains_key(&(world[0], world[1], world[2]));
            let replaceable =
                self.with_level(|b, _level, _ticks| b.facts(state).flags & REPLACEABLE != 0);
            if super::spill::saved_entity(&self.world, world).is_some() {
                return Err(TransferError::SavedBlockEntity(world));
            }
            if !replaceable || occupied {
                return Err(TransferError::DestinationOccupied(world));
            }
            // An entity payload that does not decode refuses the whole
            // disassembly — it is never silently dropped.
            let entity = cell
                .data()
                .map(|bytes| {
                    nbt::read_named(bytes, horizon_core::nbt::Limits::saved())
                        .map(|(named, _)| named.tag)
                        .map_err(|e| TransferError::CarrierUnreadable(e.to_string()))
                })
                .transpose()?;
            let properties = cell
                .properties()
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect();
            // The destination's before-image records what is really
            // there — the replaceable block's own state, not a guess
            // that it was air.
            let before_descriptor = registry.state(state);
            let before = if state == registry.default_state("minecraft:air").unwrap_or(state) {
                CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                }
            } else {
                CellImage {
                    name: before_descriptor
                        .map(|d| d.name.clone())
                        .unwrap_or_default(),
                    properties: before_descriptor
                        .map(|d| {
                            d.properties
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                    entity: None,
                }
            };
            operation_cells.push(OperationCell {
                world,
                local: [cell.pos().x(), cell.pos().y(), cell.pos().z()],
                before,
                after: CellImage {
                    name: cell.name().to_string(),
                    properties,
                    entity,
                },
            });
        }

        let cells: Vec<BlockPos> = operation_cells
            .iter()
            .map(|cell| BlockPos::new(cell.world[0], cell.world[1], cell.world[2]))
            .collect();
        let op_id = store.next_op_id();
        let operation = Operation {
            op_id,
            kind: OpKind::Disassemble,
            uuid,
            generation,
            dimension: self.dim.id().to_string(),
            cells: operation_cells.clone(),
        };
        let encoded = crate::carrier_store::encode_operation(&operation)
            .map_err(|_| TransferError::Envelope)?;

        store
            .check_admission(OpKind::Disassemble, uuid, generation)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        self.save_affected(&cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;
        self.revalidate_destinations(&operation_cells)?;

        let _lease = PassLease::enter();
        store
            .write_operation(op_id, &encoded)
            .map_err(|e| TransferError::Store(e.to_string()))?;
        store
            .begin(OpKind::Disassemble, uuid, generation, self.dim.id())
            .map_err(|e| TransferError::Store(e.to_string()))?;
        for cell in &operation_cells {
            let pos = BlockPos::new(cell.world[0], cell.world[1], cell.world[2]);
            // The state resolves from the saved properties, not the
            // block's default: an oak stair kept facing=north and
            // half=bottom through the round trip, never snapping to the
            // default facing.
            let state = {
                let descriptor = horizon_world::chunk::BlockState {
                    name: cell.after.name.clone(),
                    properties: cell.after.properties.iter().cloned().collect(),
                };
                self.pack
                    .assets
                    .registry
                    .resolve(&descriptor)
                    .map_err(|_| TransferError::UnsupportedBlock(cell.world))?
            };
            // The block through the level's own write path — neighbour
            // updates included — then the entity from the carrier's
            // payload.
            self.with_level(|_b, level, ticks| {
                _b.set_block(level, ticks, pos, state, 3);
            });
            self.world.modified.insert((pos.x >> 4, pos.z >> 4));
            if let Some(Tag::Compound(map)) = cell.after.entity.clone() {
                self.with_level(|_b, level, _ticks| {
                    level.load_block_entity(pos, map);
                });
            }
        }
        for cell in &operation_cells {
            self.world.carrier_fences.lock_cell(cell.world);
        }
        self.world.carrier_fences.lock_carrier(uuid);
        drop(_lease);

        if let Err(error) = self.finish_disassemble(&mut store, uuid, generation, &cells) {
            log::error!("carrier transfer did not finish: {error}");
            self.world.carrier_fences.mark_broken();
            carrier_stop_session(self, &error);
            return Err(error);
        }
        for cell in &operation_cells {
            self.world.carrier_fences.unlock_cell(cell.world);
        }
        self.world.carrier_fences.unlock_carrier(uuid);
        Ok(())
    }

    /// The disassembly's commit tail: the chunks and the barrier, then
    /// `committed` — whose tombstone holds until the carrier file's
    /// deletion proves durable at a later open.
    fn finish_disassemble(
        &mut self,
        store: &mut Store,
        uuid: u128,
        generation: u32,
        cells: &[BlockPos],
    ) -> Result<(), TransferError> {
        self.save_affected(cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;
        #[cfg(test)]
        if self.world.carrier_fences.fail_flush {
            return Err(TransferError::Io(
                "injected: the flush barrier failed after prepared".into(),
            ));
        }
        self.save
            .flush()
            .map_err(|e| TransferError::Io(e.to_string()))?;
        store
            .commit()
            .map_err(|e| TransferError::Store(e.to_string()))?;
        // The tombstone this commit leaves holds the carrier from coming
        // back until a later open finds the file durably gone.
        if let Err(error) = std::fs::remove_file(store.carrier_path(uuid, generation)) {
            log::warn!("carrier file removal deferred: {error}");
        }
        // Only tombstones eligible at the actual world open may clear.
        // New deletions retain their witnesses throughout this session;
        // reaching the finite tombstone bound requires reopening.
        store
            .finish_retention()
            .map_err(|e| TransferError::Store(e.to_string()))?;
        Ok(())
    }

    /// Whether every destination still holds what the images recorded:
    /// replaceable, with no block entity.
    fn revalidate_destinations(
        &mut self,
        cells: &[crate::carrier_store::OperationCell],
    ) -> Result<(), TransferError> {
        for cell in cells {
            let pos = cell.world;
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let replaceable =
                self.with_level(|b, _level, _ticks| b.facts(state).flags & REPLACEABLE != 0);
            let entity = self
                .world
                .block_entities
                .contains_key(&(pos[0], pos[1], pos[2]));
            if super::spill::saved_entity(&self.world, pos).is_some() {
                return Err(TransferError::SavedBlockEntity(pos));
            }
            if !replaceable || entity {
                return Err(TransferError::Revalidation(pos));
            }
        }
        Ok(())
    }

    /// The commit tail: the carrier file, the affected chunks and the
    /// flush barrier, then `committed`.
    fn finish_assemble(
        &mut self,
        store: &mut Store,
        uuid: u128,
        generation: u32,
        cells: &[BlockPos],
    ) -> Result<(), TransferError> {
        let _ = (uuid, generation);
        self.save_affected(cells)
            .map_err(|e| TransferError::Io(e.to_string()))?;
        #[cfg(test)]
        if self.world.carrier_fences.fail_flush {
            return Err(TransferError::Io(
                "injected: the flush barrier failed after prepared".into(),
            ));
        }
        self.save
            .flush()
            .map_err(|e| TransferError::Io(e.to_string()))?;
        store
            .commit()
            .map_err(|e| TransferError::Store(e.to_string()))?;
        Ok(())
    }

    /// Whether every cell still holds exactly its image's state and
    /// entity, read straight from the live world.
    fn revalidate_cells(
        &self,
        cells: &[crate::carrier_store::OperationCell],
        registry: &horizon_world::registry::BlockRegistry,
    ) -> Result<(), TransferError> {
        for cell in cells {
            let pos = cell.world;
            if super::spill::saved_entity(&self.world, pos).is_some() {
                return Err(TransferError::SavedBlockEntity(pos));
            }
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let descriptor = registry
                .state(state)
                .ok_or(TransferError::Revalidation(pos))?;
            let same_state =
                descriptor.name == cell.before.name
                    && descriptor.properties.len() == cell.before.properties.len()
                    && cell.before.properties.iter().all(|(key, value)| {
                        descriptor.properties.get(key).is_some_and(|v| v == value)
                    });
            if !same_state {
                return Err(TransferError::Revalidation(pos));
            }
            let found_entity = self.world.block_entities.get(&(pos[0], pos[1], pos[2]));
            match (found_entity, &cell.before.entity) {
                (None, None) => {}
                // Presence alone is not a witness: the live entity must
                // serialize to exactly the image's content.
                (Some(live), Some(image)) => {
                    let name = registry
                        .state(state)
                        .map(|descriptor| descriptor.name.as_str())
                        .unwrap_or("");
                    let encoded = block_entity_encode::encode((pos[0], pos[1], pos[2]), live, name);
                    if !entities_content_equal(&encoded, image) {
                        return Err(TransferError::Revalidation(pos));
                    }
                }
                _ => return Err(TransferError::Revalidation(pos)),
            }
        }
        Ok(())
    }
    /// The world's carrier state at open (`Game::load_world_state`):
    /// the store's first-use gate reads the data folder's existence
    /// now — a folder created later this session (by the first save)
    /// does not pass for an old one — and a transfer that never
    /// finished recovers before the first tick and any player. A
    /// refusal fails the open, and no save runs after it.
    pub(in crate::game) fn carrier_world_opened(&mut self) -> crate::Result<()> {
        self.world.carrier_data_existed_at_open = self.world_root.join("data").is_dir();
        self.carrier_retention_snapshot = match Store::open(&self.world_root) {
            Ok(crate::carrier_store::Opened::Live(store)) => store.retention_snapshot(),
            Ok(crate::carrier_store::Opened::NeverUsed) => Vec::new(),
            Err(error) => {
                self.world.carrier_fences.mark_broken();
                return Err(format!("world load refused: {error}").into());
            }
        };
        let result = self.carrier_recover_all_levels();
        if result.is_err() {
            self.world.carrier_fences.mark_broken();
        }
        result
    }

    /// `Game::break_block` behind the carrier fence: a cell held
    /// between prepared and committed refuses before the break runs.
    pub(in crate::game) fn carrier_break_block(&mut self, pos: [i32; 3]) -> crate::Result<()> {
        if self.world.carrier_fences.locks_cell(&pos) {
            return Err("a carrier transfer holds this cell".into());
        }
        self.player_break_block(pos).map(|_| ())
    }

    /// Recovers the world's carrier store at open, before any chunk
    /// ticks and before any player joins: the level in place first, then
    /// every parked level through the same enter/return route the
    /// autosave uses. A refusal fails the world's load with a clear
    /// message — and nothing saves after it: the open itself aborts,
    /// while the store stays exactly as recovery found it.
    pub(crate) fn carrier_recover_all_levels(&mut self) -> crate::Result<()> {
        // No pending operation: no level is touched at all.
        let dimension = match crate::carrier_store::Store::open(&self.world_root) {
            Ok(crate::carrier_store::Opened::Live(store)) => {
                match store.journal().in_progress.as_ref() {
                    Some(op) => op.dimension.clone(),
                    // No pending operation — but retention still runs: the
                    // production cleanup clears finished disassemblies'
                    // tombstones and orphaned files at every open.
                    None => {
                        self.carrier_cleanup();
                        return Ok(());
                    }
                }
            }
            Ok(crate::carrier_store::Opened::NeverUsed) => return Ok(()),
            Err(error) => return Err(format!("world load refused: {error}").into()),
        };
        // Recovery enters the operation's own dimension and no other: a
        // transfer in the Nether recovers in the Nether even when the
        // world opens in the Overworld with the Nether not parked -
        // enter_level opens an unheld level on demand. A dimension id
        // the world cannot hold at all refuses the load.
        let target = match dimension.as_str() {
            "minecraft:overworld" => crate::game::Dim::Overworld,
            "minecraft:the_nether" => crate::game::Dim::Nether,
            "minecraft:the_end" => crate::game::Dim::End,
            _ => match crate::game::Dim::extra(&dimension) {
                Some(dim) => dim,
                None => {
                    return Err(format!(
                        "world load refused: the pending carrier operation names \
                         {dimension}, which this world does not hold"
                    )
                    .into());
                }
            },
        };
        let back = self.dim;
        if target != back {
            self.enter_level(target).map_err(|e| {
                format!("world load refused: carrier recovery could not enter {dimension}: {e}")
            })?;
        }
        let recovered = self.carrier_recover_current();
        if target != back {
            self.enter_level(back).map_err(|e| {
                format!("world load refused: recovery could not return from {dimension}: {e}")
            })?;
        }
        recovered.map_err(|e| format!("world load refused: {e}"))?;
        // Production cleanup, as every open runs it: orphans, stale
        // operation files and tombstones of durably deleted carriers
        // clear here. New session deletions remain bounded by the journal's
        // capacity, not by an assumption that unlink is durable.
        self.carrier_cleanup();
        Ok(())
    }

    /// Cleans the store by the valid journal — the validated cleanup of
    /// the storage layer, run from world open. Errors are logged and
    /// swallowed: cleanup is retention work, never a load refusal.
    fn carrier_cleanup(&mut self) {
        let mut store = match crate::carrier_store::Store::open(&self.world_root) {
            Ok(crate::carrier_store::Opened::Live(store)) => store,
            _ => return,
        };
        if let Err(error) = store.cleanup() {
            log::warn!("carrier store cleanup: {error}");
        }
    }

    /// Recovers the level in place: no store means nothing to do; a live
    /// store's in-progress operation replays against this level's region
    /// directory, and only if the operation names this level.
    fn carrier_recover_current(&mut self) -> Result<(), String> {
        let mut store = match crate::carrier_store::Store::open(&self.world_root) {
            Ok(crate::carrier_store::Opened::Live(store)) => store,
            Ok(crate::carrier_store::Opened::NeverUsed) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        if store.journal().in_progress.is_none() {
            return Ok(());
        }
        // SaveDir::root is the dimension folder; the regions live in its
        // region/ child.
        let region = self.save.root.join("region");
        let states = crate::carrier_store::RegistryStates::new(self.pack.assets.registry);
        crate::carrier_store::recover(&mut store, &region, &states, self.dim.id())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Saves every affected chunk and waits for the writer's barrier, so
    /// the sectors are on disk with the regions synced.
    #[cfg_attr(not(test), allow(dead_code))]
    fn save_affected(&mut self, cells: &[BlockPos]) -> crate::Result<()> {
        let mut keys = Vec::new();
        for pos in cells {
            let key = (pos.x >> 4, pos.z >> 4);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let chunks = keys
            .iter()
            .filter_map(|key| self.world.chunks.get(key).cloned())
            .collect::<Vec<_>>();
        if chunks.is_empty() {
            return Ok(());
        }
        self.save
            .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        self.save.flush()
    }
}

/// Stops the session after a failed carrier commit, the same way any
/// fatal save error does: `Game::error` is set (the first error wins),
/// the event loop exits through it — a tick's error reaches
/// `Game::exit_on_error` from the event handler's result
/// (runtime/window_handler.rs), which calls `event_loop.exit()` — and
/// every world save refuses while the fences are broken
/// (`save_game_with`, world/autosave.rs), so the exit path's save and
/// any autosave write nothing over the affected regions. The prepared
/// operation stays on disk for recovery at the next open.
fn carrier_stop_session(game: &mut Game<'_>, error: &TransferError) {
    if game.error.is_none() {
        game.error = Some(format!("carrier transfer failed: {error}").into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use crate::game::spill;
    use horizon_core::geometry::Direction as CoreDirection;
    use horizon_core::position::BlockPos;
    use std::sync::Arc;

    /// One transfer's whole life: the chest moves to the carrier, its
    /// cell becomes air, the operation commits, and the block has
    /// exactly one owner — the carrier file, decoded back.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn assembling_moves_a_chest_to_one_owner_and_commits() {
        let root =
            std::env::temp_dir().join(format!("horizon-carrier-transfer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        crate::save::write_dat(
            &root.join("level.dat"),
            horizon_core::nbt::Tag::compound([(
                "Data",
                horizon_core::nbt::Tag::Compound(Default::default()),
            )]),
        )
        .unwrap();
        with_game(|game| {
            // The harness's world root is empty: give the transfer a
            // throwaway world of its own, as the save tests do.
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                .unwrap();

            // The cell is air and the fence is lifted.
            assert_eq!(game.world.block(4, 64, 4), air);
            assert!(!game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            assert!(!game.world.carrier_fences.locks_carrier(7));

            // One owner: the chest lives in the carrier file, and only
            // there — the op file is gone and the journal committed.
            let carrier_bytes = std::fs::read(
                root.join("data/horizon_carrier_00000000000000000000000000000007-1.nbt"),
            )
            .unwrap();
            let carrier = codec::decode(&carrier_bytes).unwrap();
            assert_eq!(carrier.uuid(), 7);
            assert_eq!(carrier.cells().len(), 1);
            assert_eq!(carrier.cells()[0].name(), "minecraft:chest");
            let mut leftovers = std::fs::read_dir(root.join("data")).unwrap();
            assert!(leftovers.all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("horizon_carrier_op_")
            }));
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// While a fence holds a cell, each refusal route refuses exactly at
    /// its one chokepoint, and the moment the fence clears they pass.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn fenced_cells_refuse_every_route_at_its_chokepoint() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let fenced = [4, 64, 4];
            game.world.carrier_fences.lock_cell(fenced);
            game.world.carrier_fences.lock_carrier(7);
            // The carrier is fenced too: its inventory and motion have
            // no route of their own until slice 4, so the fence read is
            // the seam that will refuse them.
            assert!(game.world.carrier_fences.locks_carrier(7));

            // Breaking.
            assert!(game.break_block(fenced).is_err());
            // Pistons: a move pushing the fenced cell refuses whole.
            game.with_level(|_b, level, _ticks| {
                assert!(!horizon_world::level::LevelAccess::allow_piston_move(
                    level,
                    BlockPos::new(4, 64, 3),
                    CoreDirection::North,
                    true,
                    &[BlockPos::new(4, 64, 4)],
                    &[]
                ));
            });
            // The block entity's container does not open.
            assert!(game.refuse_locked_container((4, 64, 4), "minecraft:chest"));
            // Every mod write through the level gate refuses.
            let level = game.level(game.dim.id()).unwrap();
            assert!(!game.native_level_write_allowed(level, fenced));

            // No operation, no refusals: the checks cost one set load.
            game.world.carrier_fences.unlock_cell(fenced);
            assert!(game.native_level_write_allowed(level, fenced));
            assert!(!game.refuse_locked_container((4, 64, 4), "minecraft:chest"));
        });
    }

    /// (item 7) An empty selection refuses before any indexing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_empty_assembly_refuses() {
        with_game(|game| match game.carrier_assemble(51, &[]) {
            Err(TransferError::EmptySelection) => {}
            other => panic!("an empty selection must refuse, got {other:?}"),
        });
    }

    /// (item 7) A destination arithmetic that overflows refuses instead
    /// of wrapping the world.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_overflowing_destination_refuses() {
        let root = transfer_root("overflow");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(52, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            // A translation that overflows i32 on add refuses.
            match game.carrier_disassemble(52, 1, [i32::MAX, 64, 4], [1.0, 0.0, 0.0, 0.0]) {
                // Overflow past the loaded area reports the unreachable
                // chunk as often as the range: either refusal is honest.
                Err(TransferError::LocalRange) | Err(TransferError::CellNotLoaded(_)) => {}
                other => panic!("an overflowing destination must refuse, got {other:?}"),
            }
            // The carrier stays whole.
            assert!(
                root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 52_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// (item 7) An unsupported block entity in the selection refuses in
    /// preflight — the world keeps the block and its entity.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_ticking_block_entity_refuses_before_the_lift() {
        let root = transfer_root("passive");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let furnace = registry.default_state("minecraft:furnace").unwrap();
            game.world.set(4, 64, 4, furnace).unwrap();
            game.world.block_entities.insert(
                (4, 64, 4),
                horizon_client::chunk::block_entity::BlockEntity::Furnace(
                    horizon_client::chunk::block_entity::Furnace {
                        kind: horizon_client::chunk::block_entity::FurnaceKind::Furnace,
                        items: [None, None, None],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 0,
                        speed: 1.0,
                        recipes_used: Default::default(),
                    },
                ),
            );
            match game.carrier_assemble(53, &[BlockPos::new(4, 64, 4)]) {
                Err(TransferError::UnsupportedBlock(pos)) => assert_eq!(pos, [4, 64, 4]),
                other => panic!("a furnace must refuse, got {other:?}"),
            }
            // The world is untouched: the block and its entity remain,
            // and no store was created for this transfer.
            assert_eq!(game.world.block(4, 64, 4), furnace);
            assert!(game.world.block_entities.contains_key(&(4, 64, 4)));
            assert!(
                !root
                    .join(format!("data/horizon_carrier_{:032x}-1.nbt", 53_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The live index capacity is separate from the retained tombstone
    /// capacity: ten sequential transfers fit an eight-carrier index,
    /// but all ten deletion witnesses survive this world session.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn repeated_transfers_beyond_the_index_capacity_stay_bounded() {
        let root = transfer_root("capacity");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            // Eight live index entries are allowed; tombstones have their
            // own independent section bound.
            game.world.carrier_fences.capacity_override = Some(8);
            for i in 0..10u32 {
                game.world.set(4, 64, 4, chest).unwrap();
                game.carrier_assemble(1000 + i as u128, &[BlockPos::new(4, 64, 4)])
                    .unwrap_or_else(|e| panic!("cycle {i}: {e}"));
                game.carrier_disassemble(
                    1000 + i as u128,
                    2 * i + 1,
                    [4, 64, 4],
                    [1.0, 0.0, 0.0, 0.0],
                )
                .unwrap_or_else(|e| panic!("cycle {i}: {e}"));
            }
            let crate::carrier_store::Opened::Live(store) = Store::open(&root).unwrap() else {
                panic!("must open");
            };
            assert_eq!(store.journal().tombstones.len(), 10);
            // Bounded: no operation files, no carrier files beyond the
            // journal's two slots.
            let mut files = 0;
            for entry in std::fs::read_dir(root.join("data")).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("horizon_carrier") {
                    files += 1;
                }
            }
            assert!(
                files <= 2,
                "the store must stay bounded at the two slots, found {files} files"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn capacity_refusal_leaves_world_untouched() {
        let root = transfer_root("admission");
        with_game(|game| {
            game.world_root = root.clone();
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.carrier_fences.capacity_override = Some(0);
            game.world.set(4, 64, 4, chest).unwrap();
            let error = game
                .carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                .unwrap_err();
            assert!(error.to_string().contains("CarrierCapacityReopenNeeded"));
            assert_eq!(game.world.block(4, 64, 4), chest);
            assert!(!game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            let crate::carrier_store::Opened::Live(store) =
                Store::open_with_capacity(&root, 0).unwrap()
            else {
                panic!("must open")
            };
            assert!(store.journal().in_progress.is_none());
            assert!(store.journal().carriers.is_empty());
            assert!(!root.join("dimensions/minecraft/overworld/region").exists());
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn saved_tag_refuses_assembly_and_disassembly_without_mutation() {
        let root = transfer_root("saved-tag");
        with_game(|game| {
            game.world_root = root.clone();
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            let entity = horizon_client::chunk::block_entity::BlockEntity::Chest(vec![
                None,
                Some(horizon_client::items::ItemStack {
                    item: "minecraft:stone".into(),
                    count: 3,
                    damage: 0,
                    extra: None,
                }),
            ]);
            let tag = horizon_client::save::block_entity_encode::encode(
                (4, 64, 4),
                &entity,
                "minecraft:chest",
            );
            spill::put_saved_entity(&mut game.world, [4, 64, 4], tag.clone());
            assert!(matches!(
                game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)]),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            assert_eq!(game.world.block(4, 64, 4), chest);
            assert!(
                std::fs::read_dir(root.join("data"))
                    .unwrap()
                    .next()
                    .is_none()
            );
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).saved = None;
            game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            spill::put_saved_entity(&mut game.world, [4, 64, 4], tag.clone());
            assert!(matches!(
                game.carrier_disassemble(7, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0]),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            let images = [crate::carrier_store::OperationCell {
                world: [4, 64, 4],
                local: [0, 0, 0],
                before: crate::carrier_store::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
                after: crate::carrier_store::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            }];
            assert!(matches!(
                game.revalidate_destinations(&images),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            assert_eq!(game.world.block(4, 64, 4), air);
            assert_eq!(spill::saved_entity(&game.world, [4, 64, 4]), Some(tag));
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn assembly_revalidation_refuses_saved_tag_without_mutation() {
        let root = transfer_root("assembly-revalidation-saved-tag");
        with_game(|game| {
            game.world_root = root.clone();
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            let entity = horizon_client::chunk::block_entity::BlockEntity::Chest(vec![
                None,
                Some(horizon_client::items::ItemStack {
                    item: "minecraft:stone".into(),
                    count: 3,
                    damage: 0,
                    extra: None,
                }),
            ]);
            let tag = horizon_client::save::block_entity_encode::encode(
                (4, 64, 4),
                &entity,
                "minecraft:chest",
            );
            spill::put_saved_entity(&mut game.world, [4, 64, 4], tag.clone());
            let images = [crate::carrier_store::OperationCell {
                world: [4, 64, 4],
                local: [0, 0, 0],
                before: crate::carrier_store::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
                after: crate::carrier_store::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            }];
            assert!(matches!(
                game.revalidate_cells(&images, registry),
                Err(TransferError::SavedBlockEntity([4, 64, 4]))
            ));
            assert_eq!(game.world.block(4, 64, 4), chest);
            assert_eq!(spill::saved_entity(&game.world, [4, 64, 4]), Some(tag));
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The store's default capacity is the contract's 4,096.
    #[test]
    fn the_default_index_capacity_is_4096() {
        use crate::carrier_store::MAX_CARRIERS;
        assert_eq!(MAX_CARRIERS, 4096);
        // The public constructors carry the default; a store built with
        // it enforces the same bound at encode, visible by refusing an
        // index past it.
        let root =
            std::env::temp_dir().join(format!("horizon-capacity-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        let mut journal = crate::carrier_store::Journal::default();
        for i in 0..=MAX_CARRIERS as u128 {
            journal.carriers.push((i, 1));
        }
        assert!(
            journal.encode_for_tests(MAX_CARRIERS).is_err(),
            "4097 entries must refuse at the default bound"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// (item 6) A crash after a finished disassembly: the tombstone
    /// outlives the session, and the next open's recovery-plus-cleanup
    /// resolves it cleanly — the carrier file is gone, the journal
    /// holds no operation, and the world reopens.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_crash_after_a_finished_disassembly_recovers_clean() {
        let root = transfer_root("post-disassembly");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(61, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            game.carrier_disassemble(61, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0])
                .unwrap();

            // The next open: recovery (nothing pending) plus the
            // production cleanup, which the open route runs.
            game.carrier_recover_all_levels().unwrap();

            // Clean: no tombstone, no carrier file, no operation.
            let store = match crate::carrier_store::Store::open(&root).unwrap() {
                crate::carrier_store::Opened::Live(store) => store,
                _ => panic!("the store must open"),
            };
            assert!(store.journal().in_progress.is_none());
            assert!(!store.journal().tombstoned(61, 1));
            assert!(
                !root
                    .join(format!("data/horizon_carrier_{:032x}-1.nbt", 61_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// (item 2) Sparse, non-collinear selections keep their geometry:
    /// the local grid is offset from the selection's minimum corner, so
    /// gaps stay gaps and the input order is irrelevant — the same
    /// selection shuffled produces the byte-identical carrier.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sparse_selection_geometry_survives_and_order_is_irrelevant() {
        let root = transfer_root("geometry");
        let sparse = [
            BlockPos::new(4, 64, 4),
            BlockPos::new(9, 64, 4),
            BlockPos::new(4, 68, 7),
            BlockPos::new(7, 71, 9),
        ];
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            for pos in sparse {
                game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
            }
            // Forward order.
            game.carrier_assemble(31, &sparse).unwrap();
            let forward =
                std::fs::read(root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 31_u128)))
                    .unwrap();
            let carrier = codec::decode(&forward).unwrap();
            // Local positions are offsets from the minimum corner (4,64,4):
            // the gaps survive as gaps.
            let mut locals: Vec<[i8; 3]> = carrier
                .cells()
                .iter()
                .map(|c| [c.pos().x(), c.pos().y(), c.pos().z()])
                .collect();
            locals.sort();
            assert_eq!(locals, [[0, 0, 0], [0, 4, 3], [3, 7, 5], [5, 0, 0]]);

            // The same selection in reverse order (a second UUID):
            // the cells decode to the identical geometry.
            for pos in sparse {
                game.world.set(pos.x, pos.y, pos.z, stone).unwrap();
            }
            let mut reversed = sparse.to_vec();
            reversed.reverse();
            game.carrier_assemble(32, &reversed).unwrap();
            let backward =
                std::fs::read(root.join(format!("data/horizon_carrier_{:032x}-2.nbt", 32_u128)))
                    .unwrap();
            let forward_carrier = codec::decode(&forward).unwrap();
            let backward_carrier = codec::decode(&backward).unwrap();
            assert_eq!(forward_carrier.transform(), backward_carrier.transform());
            assert_eq!(
                forward_carrier.cells(),
                backward_carrier.cells(),
                "the geometry is identical regardless of input order"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// (item 2) A non-default state survives assembly and disassembly
    /// exactly: oak stairs keep facing=north and half=bottom, a chest
    /// keeps facing — resolved from the saved properties at placement,
    /// never from the block's default state.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn non_default_states_survive_the_round_trip() {
        let root = transfer_root("states");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            // Oak stairs, facing north, half bottom — not the default
            // (the default stairs face north only by luck; resolve the
            // exact state and check it differs from the default).
            // Oak stairs, facing south and top-half — the opposite of
            // the default — so the test genuinely pins a non-default
            // state through the round trip.
            let descriptor = horizon_world::chunk::BlockState {
                name: "minecraft:oak_stairs".to_string(),
                properties: [
                    ("facing".to_string(), "south".to_string()),
                    ("half".to_string(), "top".to_string()),
                    ("shape".to_string(), "straight".to_string()),
                    ("waterlogged".to_string(), "false".to_string()),
                ]
                .into_iter()
                .collect(),
            };
            let stairs = registry.resolve(&descriptor).unwrap();
            let default_stairs = registry.default_state("minecraft:oak_stairs").unwrap();
            let stairs_props = &registry.state(stairs).unwrap().properties;
            let default_props = &registry.state(default_stairs).unwrap().properties;
            assert!(
                stairs_props
                    .iter()
                    .any(|(k, v)| { default_props.get(k).is_some_and(|dv| dv != v) }),
                "south/top must differ from the default stairs state"
            );
            game.world.set(4, 64, 4, stairs).unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.carrier_assemble(41, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            assert_eq!(game.world.block(4, 64, 4), air);
            game.carrier_disassemble(41, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0])
                .unwrap();
            // The exact state came back, not the default.
            assert_eq!(
                game.world.block(4, 64, 4),
                stairs,
                "the non-default state survives the round trip"
            );
            let restored = registry.state(game.world.block(4, 64, 4)).unwrap();
            assert_eq!(restored.name, "minecraft:oak_stairs");
            assert_eq!(
                restored.properties.get("facing").map(String::as_str),
                stairs_props.get("facing").map(String::as_str)
            );
            assert_eq!(
                restored.properties.get("half").map(String::as_str),
                stairs_props.get("half").map(String::as_str)
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// (a1) The live order, not the test helper: a real carrier_assemble
    /// whose flush fails after prepared, then recovery — the cargo (cells,
    /// states, block entities, items) must be in the published carrier
    /// file, and the source cell empty. If the live code ever publishes
    /// the carrier after prepared again, this test fails.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_live_assembly_publishes_the_cargo_before_prepared() {
        let root = transfer_root("live-cargo");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.world.set(6, 64, 4, chest).unwrap();
            game.world.block_entities.insert(
                (4, 64, 4),
                horizon_client::chunk::block_entity::BlockEntity::Chest(Vec::new()),
            );
            game.world.block_entities.insert(
                (6, 64, 4),
                horizon_client::chunk::block_entity::BlockEntity::Chest(vec![
                    None,
                    Some(horizon_client::items::ItemStack {
                        item: "minecraft:stone".into(),
                        count: 3,
                        damage: 0,
                        extra: None,
                    }),
                ]),
            );

            // Crash after prepared.
            game.world.carrier_fences.fail_flush = true;
            assert!(
                game.carrier_assemble(21, &[BlockPos::new(4, 64, 4), BlockPos::new(6, 64, 4)])
                    .is_err()
            );

            // Recovery, as the next open runs it.
            let mut store = match crate::carrier_store::Store::open(&root).unwrap() {
                crate::carrier_store::Opened::Live(store) => store,
                _ => panic!("the store must reopen"),
            };
            let region = root.join("dimensions/minecraft/overworld/region");
            let states = crate::carrier_store::RegistryStates::new(registry);
            let report =
                crate::carrier_store::recover(&mut store, &region, &states, "minecraft:overworld")
                    .unwrap();
            assert_eq!(report.applied + report.verified, 1);
            assert!(store.journal().in_progress.is_none());

            // The source cells are empty.
            for x in [4, 6] {
                assert_eq!(game.world.block(x, 64, 4), air);
            }

            // The cargo lives in the published carrier: both cells, the
            // chest states, and the entities' items.
            let bytes =
                std::fs::read(root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 21_u128)))
                    .unwrap();
            let carrier = codec::decode(&bytes).unwrap();
            assert_eq!(carrier.cells().len(), 2, "the whole cargo is carried");
            let names: Vec<&str> = carrier.cells().iter().map(|c| c.name()).collect();
            assert!(names.iter().all(|n| *n == "minecraft:chest"));
            // The entity payload rides each cell; one of them carries
            // an item stack.
            let carried: Vec<usize> = carrier
                .cells()
                .iter()
                .map(|c| c.data().map(|d| d.len()).unwrap_or(0))
                .collect();
            assert!(
                carried.iter().all(|len| *len > 0),
                "both entities are carried: {carried:?}"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A flush that fails after `prepared`: the fences hold, the next
    /// transfer refuses, and recovery at a reopen finishes the
    /// operation with the chest's items in exactly one place.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_flush_failure_after_prepared_recovers_on_reopen() {
        let root =
            std::env::temp_dir().join(format!("horizon-carrier-flush-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        write_level_dat(&root);
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            // The writer's barrier fails after prepared.
            game.world.carrier_fences.fail_flush = true;
            assert!(
                game.carrier_assemble(7, &[BlockPos::new(4, 64, 4)])
                    .is_err()
            );
            // The fences hold and the world refuses further transfers.
            assert!(game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            assert!(game.world.carrier_fences.is_broken());
            assert!(
                game.carrier_assemble(8, &[BlockPos::new(6, 64, 6)])
                    .is_err()
            );

            // Recovery, as the next open runs it: the operation replays,
            // the journal commits and the chest is air in the world.
            let mut store = match crate::carrier_store::Store::open(&root).unwrap() {
                crate::carrier_store::Opened::Live(store) => store,
                _ => panic!("the store must reopen"),
            };
            let region = root.join("dimensions/minecraft/overworld/region");
            let states = crate::carrier_store::RegistryStates::new(registry);
            let report =
                crate::carrier_store::recover(&mut store, &region, &states, "minecraft:overworld")
                    .unwrap();
            // The interrupted save left the post-state on disk, so the
            // reopen verifies it rather than replaying — either way the
            // operation commits and exactly one side owns the chest.
            assert_eq!(report.applied + report.verified, 1);
            assert!(
                store.journal().in_progress.is_none(),
                "the operation committed"
            );
            // One owner: the items live in the carrier file, and the
            // operation file is gone.
            assert!(
                root.join("data/horizon_carrier_00000000000000000000000000000007-1.nbt")
                    .is_file()
            );
            assert!(!root.join("data/horizon_carrier_op_1.nbt").is_file());
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review item 5's fail-closed: a commit that fails after
    /// `prepared` stops the whole session, not just further transfers.
    /// The game's fatal error is set (what the event loop's error path
    /// turns into its exit), a further edit of the fenced cell refuses,
    /// every save refuses so nothing is written over the region before
    /// the exit, and the next open's recovery still finishes the
    /// operation with exactly one owner.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_failed_commit_stops_the_session_until_recovery() {
        let root = transfer_root("fail-closed");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            // The writer's barrier fails after prepared.
            game.world.carrier_fences.fail_flush = true;
            assert!(
                game.carrier_assemble(9, &[BlockPos::new(4, 64, 4)])
                    .is_err()
            );

            // The session stopped like a fatal save error: `Game::error`
            // is set — the same flag the event loop's error path exits
            // the loop through (`exit_on_error`).
            assert!(game.error.is_some(), "the failed commit stops the session");

            // A further edit of the fenced cell refuses.
            assert!(game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            assert!(game.break_block([4, 64, 4]).is_err());

            // Every save refuses — the autosave's and the exit path's
            // chokepoint — so nothing is written over the region
            // between the failure and the exit.
            let region = root.join("dimensions/minecraft/overworld/region");
            let before = region_files(&region);
            assert!(
                game.save_game().is_err(),
                "a save refuses after the failure"
            );
            assert_eq!(before, region_files(&region), "the region is untouched");

            // Recovery, as the next open runs it: the operation replays
            // or verifies, commits, and the chest has exactly one owner.
            let mut store = match crate::carrier_store::Store::open(&root).unwrap() {
                crate::carrier_store::Opened::Live(store) => store,
                _ => panic!("the store must reopen"),
            };
            let states = crate::carrier_store::RegistryStates::new(registry);
            let report =
                crate::carrier_store::recover(&mut store, &region, &states, "minecraft:overworld")
                    .unwrap();
            assert_eq!(report.applied + report.verified, 1);
            assert!(store.journal().in_progress.is_none());
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Review item 5's logical-server guard: a client mirror's transfer
    /// refuses with a reason — the assembly's and the disassembly's
    /// gates alike — and the refusal leaves the world untouched: no
    /// store is created, nothing is written.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_client_mirror_transfer_refuses_without_touching_the_world() {
        let root = std::env::temp_dir().join(format!(
            "horizon-carrier-client-refusal-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.world_root = root.clone();

            let refused = game.carrier_disassemble(3, 1, [0, 64, 0], [1.0, 0.0, 0.0, 0.0]);
            assert!(matches!(refused, Err(TransferError::NotLogicalServer)));

            // The assembly's gate refuses the same way, not silently.
            let refused = game.carrier_assemble(3, &[BlockPos::new(0, 64, 0)]);
            assert!(matches!(refused, Err(TransferError::NotLogicalServer)));

            // The world is untouched: no store, no journal, no data.
            assert!(!root.join("data").exists());
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Assemble then disassemble: at every step the chest has exactly
    /// one owner — the world, the carrier's images or the carrier file.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn assemble_then_disassemble_keeps_one_owner_at_every_step() {
        let root = transfer_root("cycle");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            game.carrier_assemble(9, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            assert_eq!(game.world.block(4, 64, 4), air);
            let carrier_bytes =
                std::fs::read(root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 9_u128)))
                    .unwrap();
            assert_eq!(codec::decode(&carrier_bytes).unwrap().cells().len(), 1);

            game.carrier_disassemble(9, 1, [4, 64, 4], [1.0, 0.0, 0.0, 0.0])
                .unwrap();
            assert_eq!(game.world.block(4, 64, 4), chest);
            // The carrier file is gone: one owner, the world.
            assert!(
                !root
                    .join(format!("data/horizon_carrier_{:032x}-1.nbt", 9_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A destination that already holds a block entity refuses, with
    /// the carrier intact.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_destination_with_a_block_entity_refuses() {
        let root = transfer_root("dest");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(5, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            // A second chest occupies the destination cell.
            game.world.set(10, 64, 10, chest).unwrap();
            game.world.block_entities.insert(
                (10, 64, 10),
                horizon_client::chunk::block_entity::BlockEntity::Chest(Vec::new()),
            );
            match game.carrier_disassemble(5, 1, [10, 64, 10], [1.0, 0.0, 0.0, 0.0]) {
                Err(TransferError::DestinationOccupied(pos)) => assert_eq!(pos, [10, 64, 10]),
                other => panic!("expected an occupied destination, got {other:?}"),
            }
            assert!(
                root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 5_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A rotation that is not a cube symmetry refuses; a cube rotation
    /// without an adapter refuses too; the carrier survives both.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn rotations_refuse_without_symmetry_or_an_adapter() {
        let root = transfer_root("rot");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(6, &[BlockPos::new(4, 64, 4)])
                .unwrap();

            // A sixth of a turn about Y: not a cube symmetry.
            let half = 0.5_f64;
            match game.carrier_disassemble(6, 1, [8, 64, 8], [half, 0.0, half, 0.0]) {
                Err(TransferError::NotCubeSymmetry) => {}
                other => panic!("expected a symmetry refusal, got {other:?}"),
            }
            // A quarter turn is a symmetry, but no adapter exists yet.
            let quarter = std::f64::consts::FRAC_1_SQRT_2;
            match game.carrier_disassemble(6, 1, [8, 64, 8], [quarter, 0.0, quarter, 0.0]) {
                Err(TransferError::RotationNeedsAdapter) => {}
                other => panic!("expected an adapter refusal, got {other:?}"),
            }
            assert!(
                root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 6_u128))
                    .is_file()
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Revalidation itself: images built from the world, the world then
    /// changed underneath — the mismatch refuses and nothing moved.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn revalidation_refuses_a_changed_cell_without_touching_the_world() {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            let descriptor = registry.state(chest).unwrap();
            let properties = descriptor
                .properties
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let images = [crate::carrier_store::OperationCell {
                world: [4, 64, 4],
                local: [0, 0, 0],
                before: crate::carrier_store::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties,
                    entity: None,
                },
                after: crate::carrier_store::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            }];
            assert!(game.revalidate_cells(&images, registry).is_ok());
            game.world.set(4, 64, 4, stone).unwrap();
            match game.revalidate_cells(&images, registry) {
                Err(TransferError::Revalidation(pos)) => assert_eq!(pos, [4, 64, 4]),
                other => panic!("expected a revalidation refusal, got {other:?}"),
            }
            // The world keeps the changed cell; nothing was lifted.
            assert_eq!(game.world.block(4, 64, 4), stone);
        });
    }

    /// A Nether transfer that crashed after `prepared`, recovered by a
    /// world opening in the Overworld with the Nether not parked: the
    /// operation replays in the Nether, the Overworld is untouched and
    /// the player stays where the world opened.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_nether_transfer_recovers_while_the_world_opens_in_the_overworld() {
        let root = transfer_root("nether");
        // Opening a second level on demand reads the data-fix tables of
        // the prepared resources (HORIZON_RESOURCES is their root for
        // that lookup; the test harness points its sibling variable at
        // the same folder).
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();

            // The transfer happens in the Nether, opened on demand.
            game.enter_level(crate::game::Dim::Nether).unwrap();
            assert_eq!(game.dim, crate::game::Dim::Nether);
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/the_nether"),
                horizon_client::save::LevelShape::NETHER,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.world.set(4, 64, 4, chest).unwrap();
            game.world.carrier_fences.fail_flush = true;
            let failed = game.carrier_assemble(11, &[BlockPos::new(4, 64, 4)]);
            assert!(failed.is_err(), "the injected flush must fail: {failed:?}");
            let why = failed.unwrap_err().to_string();
            assert!(
                why.contains("injected"),
                "the failure must be the injected flush, not an earlier refusal: {why}"
            );

            // The world reopens in the Overworld: recovery enters the
            // Nether itself, replays there and returns.
            game.world.carrier_fences.fail_flush = false;
            game.enter_level(crate::game::Dim::Overworld).unwrap();
            assert_eq!(game.dim, crate::game::Dim::Overworld);
            // A fresh open holds no parked level: drop the Nether so
            // recovery has to open it from disk, as at world load.
            game.levels.parked.remove(&crate::game::Dim::Nether);
            assert!(!game.levels.parked.contains_key(&crate::game::Dim::Nether));
            game.carrier_recover_all_levels().unwrap();
            assert_eq!(
                game.dim,
                crate::game::Dim::Overworld,
                "the player stays in the level the world opened in"
            );

            // One owner: the items are in the carrier file, the Nether
            // cell is air, and the Overworld never wrote a thing.
            assert!(
                root.join(format!("data/horizon_carrier_{:032x}-1.nbt", 11_u128))
                    .is_file()
            );
            assert!(!root.join("data/horizon_carrier_op_1.nbt").is_file());
            let nether = root.join("dimensions/minecraft/the_nether");
            let chunk = horizon_world::carrier::region_strict::read_chunk(
                &nether.join("region"),
                horizon_core::position::ChunkPos::new(0, 0),
            )
            .unwrap()
            .expect("the nether chunk is on disk");
            let data = horizon_world::chunk::ChunkData::new(chunk).unwrap();
            assert_eq!(
                data.block(horizon_core::position::BlockPos::new(4, 64, 4))
                    .unwrap()
                    .unwrap()
                    .name,
                "minecraft:air"
            );
            let overworld = root.join("dimensions/minecraft/overworld/region");
            assert!(
                !overworld.join("r.0.0.mca").exists(),
                "the overworld's region is untouched"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A journal naming a dimension the world cannot hold refuses the
    /// load with that name, and no level is touched.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_unknown_dimension_refuses_the_load_without_touching_levels() {
        let root = transfer_root("nope");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            // A prepared operation whose journal names minecraft:nope.
            let mut store = crate::carrier_store::Store::first_use(&root, true).unwrap();
            store
                .begin(
                    crate::carrier_store::OpKind::Assemble,
                    12,
                    1,
                    "minecraft:nope",
                )
                .unwrap();
            drop(store);

            let back = game.dim;
            let refused = game.carrier_world_opened();
            match refused {
                Err(error) => {
                    let text = error.to_string();
                    assert!(
                        text.contains("minecraft:nope")
                            && text.contains("which this world does not hold"),
                        "{text}"
                    );
                }
                Ok(()) => panic!("an unknown dimension must refuse the load"),
            }
            assert!(game.admit_level_save().is_err());
            // No level was touched: the game still holds the level it
            // opened in, with nothing parked.
            assert_eq!(game.dim, back);
            assert!(!game.world.carrier_fences.locks_carrier(12));
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The region writer's per-chunk ordering: an older snapshot,
    /// submitted first and completing after the post-state's write,
    /// leaves the post-state on disk. The writer is one thread over an
    /// mpsc channel (batches run in submission order) and
    /// `IOWorker.pendingWrites` replaces a chunk's pending record with
    /// the newer submission's sequence, so its `newest` filter
    /// (save.rs: "a chunk queued again since is written once, by its
    /// newest batch") drops the older snapshot. No record number of
    /// our own is needed; this test pins the guarantee.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_older_snapshot_completing_late_cannot_overwrite_the_post_state() {
        let root = transfer_root("writer");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            let save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let dirt = registry.default_state("minecraft:dirt").unwrap();
            let mut pre_chunk = horizon_client::chunk::Chunk::empty(0, 0);
            pre_chunk.set(4, 64, 4, stone.0);
            let pre = Arc::new(pre_chunk);
            let mut post_chunk = horizon_client::chunk::Chunk::empty(0, 0);
            post_chunk.set(4, 64, 4, dirt.0);
            let post = Arc::new(post_chunk);

            // Both snapshots submitted back to back, one barrier: the
            // pre-image's write lands behind the post-state's.
            save.save_later(&[pre], registry, world_biomes!(game))
                .unwrap();
            save.save_later(&[post], registry, world_biomes!(game))
                .unwrap();
            save.flush().unwrap();

            let chunk = horizon_world::carrier::region_strict::read_chunk(
                &root.join("dimensions/minecraft/overworld/region"),
                horizon_core::position::ChunkPos::new(0, 0),
            )
            .unwrap()
            .expect("the chunk is on disk");
            let data = horizon_world::chunk::ChunkData::new(chunk).unwrap();
            assert_eq!(
                data.block(horizon_core::position::BlockPos::new(4, 64, 4))
                    .unwrap()
                    .unwrap()
                    .name,
                "minecraft:dirt",
                "the post-state must survive the older snapshot"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Every save route refuses while the #2488 obligation holds —
    /// autosave, save-all, requested parked saves and the shutdown
    /// save all enter through `admit_entity_save` — and admits again
    /// the moment the transfer commits. The transfer runs inside one
    /// synchronous call, so the steps cannot interleave with a save;
    /// this pins the gate each route passes.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn every_save_route_refuses_under_the_obligation_and_admits_after_commit() {
        let root = transfer_root("saves");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();

            // Under the obligation — the transfer's own state between
            // acquire and release — every route refuses.
            {
                let _lease = horizon_client::join_seq::visibility::PassLease::enter();
                assert!(game.save_game_with(true).is_err(), "autosave");
                assert!(game.save_level(true).is_err(), "save-all");
                game.levels.request_save(game.dim);
                game.run_requested_saves();
                // The requested save was refused while lifted and stays
                // queued for the next tick, as #2488 defines.
                assert!(game.levels.save_pending(game.dim));
            }
            // Committed: the routes admit again (the full save writes
            // the harness's throwaway world, which is allowed here).
            game.carrier_assemble(13, &[BlockPos::new(4, 64, 4)])
                .unwrap();
            assert!(game.save_game_with(true).is_ok(), "after the commit");
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The supported backup procedure: `save-all` flush, no pending
    /// operation, copy the world folder, load the copy — the carriers
    /// and chunks come out identical.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_supported_backup_restores_identical_carriers_and_chunks() {
        let root = transfer_root("backup");
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.save = std::sync::Arc::new(horizon_client::save::SaveDir::new(
                root.join("dimensions/minecraft/overworld"),
                horizon_client::save::LevelShape::OVERWORLD,
            ));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            game.world.set(4, 64, 4, chest).unwrap();
            game.carrier_assemble(14, &[BlockPos::new(4, 64, 4)])
                .unwrap();

            // save-all with its flush, and no pending operation.
            assert!(game.save_game_with(true).is_ok());
            let store = match crate::carrier_store::Store::open(&root).unwrap() {
                crate::carrier_store::Opened::Live(store) => store,
                _ => panic!("the store must open"),
            };
            assert!(
                store.journal().in_progress.is_none(),
                "no operation pending: the world is quiescent"
            );
            drop(store);

            // Copy the whole world folder.
            let copy = root.parent().unwrap().join(format!(
                "horizon-carrier-backup-copy-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&copy);
            copy_dir(&root, &copy).unwrap();

            // Load the copy: the carrier file and every region byte
            // are identical to the original's.
            let carrier = format!("data/horizon_carrier_{:032x}-1.nbt", 14_u128);
            assert_eq!(
                std::fs::read(root.join(&carrier)).unwrap(),
                std::fs::read(copy.join(&carrier)).unwrap(),
                "the carrier file copies identically"
            );
            let original =
                std::fs::read(root.join("dimensions/minecraft/overworld/region/r.0.0.mca"))
                    .unwrap();
            let restored =
                std::fs::read(copy.join("dimensions/minecraft/overworld/region/r.0.0.mca"))
                    .unwrap();
            assert_eq!(original, restored, "the chunk bytes copy identically");
            // And the copy's store opens with the carrier listed.
            match crate::carrier_store::Store::open(&copy).unwrap() {
                crate::carrier_store::Opened::Live(store) => {
                    assert!(store.journal().lists(14, 1));
                }
                _ => panic!("the copy's store must open"),
            }
            let _ = std::fs::remove_dir_all(&copy);
        });
        let _ = std::fs::remove_dir_all(&root);
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }

    /// A fresh throwaway world root with `data/` and a readable
    /// `level.dat`, named after the test.
    /// The region directory's files with their bytes, for before/after
    /// comparisons that no save wrote anything.
    fn region_files(region: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(region) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                files.push((name, std::fs::read(entry.path()).unwrap_or_default()));
            }
        }
        files.sort();
        files
    }

    fn transfer_root(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("horizon-carrier-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        write_level_dat(&root);
        root
    }

    fn write_level_dat(root: &std::path::Path) {
        // A current DataVersion inside Data keeps the world-gen reader
        // out of the data fixers, whose tables the prepared test
        // resources do not carry.
        let mut data = horizon_core::nbt::Compound::new();
        data.insert(
            "DataVersion".into(),
            horizon_core::nbt::Tag::Int(horizon_world::DATA_VERSION),
        );
        crate::save::write_dat(
            &root.join("level.dat"),
            horizon_core::nbt::Tag::compound([("Data", horizon_core::nbt::Tag::Compound(data))]),
        )
        .unwrap();
    }

    /// The #2488 obligation is what autosave and `save-all` refuse on:
    /// holding it refuses, releasing it admits again.
    #[test]
    fn the_save_obligation_gates_saves() {
        use horizon_client::join_seq::visibility::{PassLease, passes_settled};
        let _lease = PassLease::enter();
        assert!(!passes_settled());
        assert!(Game::admit_entity_save().is_err());
        drop(_lease);
        assert!(passes_settled());
        assert!(Game::admit_entity_save().is_ok());
    }

    /// A world without transfers creates nothing and fences nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_world_without_transfers_writes_no_store_files() {
        let root =
            std::env::temp_dir().join(format!("horizon-carrier-quiet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        with_game(|game| {
            game.world_root = root.clone();
            // The fixture world pre-exists with its data folder, as an
            // opened world would.
            game.world.carrier_data_existed_at_open = true;
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(4, 64, 4, stone).unwrap();
            assert!(!game.world.carrier_fences.locks_cell(&[4, 64, 4]));
            let names: Vec<String> = std::fs::read_dir(root.join("data"))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_str().unwrap().to_string())
                .collect();
            assert!(
                names.is_empty(),
                "a world without transfers wrote {names:?}"
            );
        });
        let _ = std::fs::remove_dir_all(&root);
    }
}
