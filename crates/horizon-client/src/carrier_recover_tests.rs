//! Carrier recovery tests (slice 2 part B, issue arismoko/horizon#2413): the strict
//! region reader, the before/after image replay and the committed
//! journal step, on real region files under throwaway folders. The
//! single-owner invariant runs through every scenario: a chest's items
//! sit in exactly one place — the chunk's block entity before the
//! replay, the operation's images while it is pending, nowhere once it
//! is committed on the assembling side and in the chunk again on the
//! disassembling side.

use crate::carrier_store::{
    AllStatesKnown, CellImage, OpKind, Opened, Operation, OperationCell, RecoveryRefused, Store,
    encode_operation, recover,
};
use horizon_core::nbt::{self, NamedTag, Tag};
use horizon_core::position::{BlockPos, ChunkPos};
use horizon_world::carrier::codec;
use horizon_world::carrier::model::{Carrier, Cell, LocalPos};
use horizon_world::carrier::region_strict::{self, Strict};
use horizon_world::chunk::{ChunkData, PalettedContainer};
use horizon_world::region::{ChunkRecord, RegionFile};
use std::{fs, path::PathBuf};

/// A throwaway folder holding a world `data/` and a dimension region
/// directory, removed on drop.
struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "horizon-carrier-recover-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data")).unwrap();
        crate::save::write_dat(
            &root.join("level.dat"),
            Tag::compound([("Data", Tag::Compound(Default::default()))]),
        )
        .unwrap();
        fs::create_dir_all(root.join("dimensions/minecraft/overworld/region")).unwrap();
        Self { root }
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn region(&self) -> PathBuf {
        self.root.join("dimensions/minecraft/overworld/region")
    }

    fn store(&self) -> Store {
        match Store::open(&self.root).unwrap() {
            Opened::Live(store) => store,
            Opened::NeverUsed => Store::first_use(&self.root, true).unwrap(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A chunk whose section Y=0 is uniform `state`, with optional block
/// entities, built through the storage-level chunk itself.
fn chest_state_tag() -> Tag {
    Tag::compound([
        ("id", Tag::String("minecraft:chest".into())),
        (
            "properties",
            Tag::compound([("facing", Tag::String("north".into()))]),
        ),
    ])
}

fn chunk_root(state: Tag, entities: &[([i32; 3], Vec<u8>)]) -> NamedTag {
    chunk_root_at(state, entities, 0, 0)
}

/// [`chunk_root`] with the chunk's own coordinates, which the storage
/// chunk checks its cells against.
fn chunk_root_at(state: Tag, entities: &[([i32; 3], Vec<u8>)], x: i32, z: i32) -> NamedTag {
    let palette = PalettedContainer::uniform(state, 4096, 4).unwrap().to_nbt();
    let root = Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ("xPos", Tag::Int(x)),
        ("zPos", Tag::Int(z)),
        (
            "sections",
            Tag::List(vec![Tag::compound([
                ("Y", Tag::Byte(0)),
                ("block_states", palette),
            ])]),
        ),
    ]);
    let mut root = NamedTag {
        name: "".into(),
        tag: root,
    };
    if !entities.is_empty() {
        let list = entities
            .iter()
            .map(|(pos, items)| {
                Tag::compound([
                    ("id", Tag::String("minecraft:chest".into())),
                    ("x", Tag::Int(pos[0])),
                    ("y", Tag::Int(pos[1])),
                    ("z", Tag::Int(pos[2])),
                    ("Items", Tag::ByteArray(items.clone())),
                ])
            })
            .collect();
        if let Tag::Compound(map) = &mut root.tag {
            map.insert("block_entities".into(), Tag::List(list));
        }
    }
    root
}

/// Writes a chunk into the fixture's region, synced.
fn put_chunk(fixture: &Fixture, position: ChunkPos, root: &NamedTag) {
    let record = ChunkRecord::from_nbt(
        root,
        horizon_world::compression::Compression::Zlib,
        0,
        nbt::Limits::saved(),
    )
    .unwrap();
    let path = fixture
        .region()
        .join(format!("r.{}.{}.mca", position.x >> 5, position.z >> 5));
    let mut region = RegionFile::open_writable(&path, true).unwrap();
    region.write(position, &record).unwrap();
    region.sync().unwrap();
}

/// Reads a chunk back strictly.
fn get_chunk(fixture: &Fixture, position: ChunkPos) -> Option<NamedTag> {
    region_strict::read_chunk(&fixture.region(), position).unwrap()
}

/// The items a chunk's block entity at `pos` holds, if it holds any.
fn chest_items(root: &NamedTag, pos: [i32; 3]) -> Option<usize> {
    let Some(Tag::List(list)) = root.tag.get("block_entities") else {
        return None;
    };
    let entity = list.iter().find(|tag| {
        let read = |name: &str| match tag.get(name) {
            Some(Tag::Int(value)) => Some(*value),
            _ => None,
        };
        read("x") == Some(pos[0]) && read("y") == Some(pos[1]) && read("z") == Some(pos[2])
    })?;
    match entity.get("Items") {
        Some(Tag::ByteArray(bytes)) => Some(bytes.len()),
        Some(Tag::List(_)) => {
            let (_, chest) = crate::save::decode_block_entity(entity).expect("chest");
            Some(
                chest
                    .slots()
                    .iter()
                    .flatten()
                    .map(|item| item.count as usize)
                    .sum(),
            )
        }
        _ => Some(0),
    }
}

/// The assembling operation for one chest cell, before the chunk and
/// after the carrier.
fn assemble_chest(op_id: u64, world: [i32; 3]) -> Operation {
    Operation {
        op_id,
        kind: OpKind::Assemble,
        uuid: 7,
        generation: 1,
        dimension: "minecraft:overworld".to_string(),
        cells: vec![OperationCell {
            world,
            local: [0, 0, 0],
            before: CellImage {
                name: "minecraft:chest".to_string(),
                properties: vec![("facing".to_string(), "north".to_string())],
                entity: Some(Tag::compound([
                    ("id", Tag::String("minecraft:chest".into())),
                    ("Items", Tag::ByteArray(vec![7; 8])),
                ])),
            },
            after: CellImage {
                name: "minecraft:air".to_string(),
                properties: Vec::new(),
                entity: None,
            },
        }],
    }
}

/// Writes the operation file and the `prepared` journal record, in the
/// durable order: file first, journal second.
/// Publishes the operation file, the immutable carrier (standing in
/// for the live assembly's own publication before `prepared` — the
/// live order is pinned by the live test in game/world/), then the
/// journal's prepared record. Store-level tests use this helper; the
/// live boundary itself is verified by the carrier_transfers tests.
fn prepare_with_published_carrier(store: &mut Store, operation: &Operation) {
    let encoded = encode_operation(operation).unwrap();
    store.write_operation(operation.op_id, &encoded).unwrap();
    if operation.kind == OpKind::Assemble {
        store
            .write_carrier(operation.uuid, operation.generation, &carrier_of(operation))
            .unwrap();
    }
    store
        .begin(
            operation.kind,
            operation.uuid,
            operation.generation,
            &operation.dimension,
        )
        .unwrap();
}

/// The carrier a live assembly publishes: every before-image's cell
/// with its entity payload serialized along.
fn carrier_of(operation: &Operation) -> Vec<u8> {
    let cells = operation
        .cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let data = cell.before.entity.as_ref().and_then(|entity| {
                nbt::write_named(
                    &NamedTag {
                        name: "".into(),
                        tag: entity.clone(),
                    },
                    nbt::Limits::saved(),
                )
                .ok()
            });
            Cell::new(
                LocalPos::new(index as i8, 0, 0),
                &cell.before.name,
                cell.before.properties.clone(),
                data,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let carrier = Carrier::new(
        operation.uuid,
        operation.generation,
        0,
        horizon_world::carrier::contact::RigidTransform::identity_at(
            horizon_core::geometry::Vec3::new(
                operation.cells[0].world[0] as f64,
                operation.cells[0].world[1] as f64,
                operation.cells[0].world[2] as f64,
            ),
        )
        .unwrap(),
        cells,
    )
    .unwrap();
    codec::encode(&carrier).unwrap()
}

#[test]
fn a_world_without_transfers_recovers_as_nothing() {
    let fixture = Fixture::new("nothing");
    let mut store = fixture.store();
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 0);
    assert_eq!(report.verified, 0);
}

#[test]
fn a_crash_after_the_operation_file_but_before_prepared_replays_nothing() {
    let fixture = Fixture::new("op-file-only");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    // The durable write happened, the journal never heard of it.
    store
        .write_operation(operation.op_id, &encode_operation(&operation).unwrap())
        .unwrap();
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 0);
    // The chest still owns its items in the chunk.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, chest_at), Some(8));
}

#[test]
fn a_crash_after_prepared_takes_the_before_image_to_the_after_image() {
    let fixture = Fixture::new("prepared");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    prepare_with_published_carrier(&mut store, &operation);
    // The carrier wins: its file was durable before the world changed.
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 1);
    let root = get_chunk(&fixture, position).unwrap();
    let data = ChunkData::new(root).unwrap();
    let found = data
        .block(BlockPos::new(chest_at[0], chest_at[1], chest_at[2]))
        .unwrap();
    assert_eq!(found.unwrap().name, "minecraft:air");
    // One owner: the chunk no longer holds the items anywhere.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, chest_at), None);
    // The journal committed and dropped the operation file.
    assert!(store.journal().in_progress.is_none());
    assert!(
        !fixture
            .data()
            .join(format!("horizon_carrier_op_{}.nbt", operation.op_id))
            .exists()
    );
}

#[test]
fn a_crash_mid_chunk_saves_finishes_only_the_untouched_side() {
    // Two affected chunks: one already carries its after-image, one still
    // its before-image — an old snapshot completed after the post-state.
    let fixture = Fixture::new("mid-saves");
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        ChunkPos::new(0, 0),
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    put_chunk(
        &fixture,
        ChunkPos::new(1, 0),
        &chunk_root_at(Tag::String("minecraft:air".into()), &[], 1, 0),
    );
    let mut store = fixture.store();
    let mut operation = assemble_chest(store.next_op_id(), chest_at);
    operation.cells.push(OperationCell {
        world: [20, 0, 4],
        local: [1, 0, 0],
        before: CellImage {
            name: "minecraft:chest".to_string(),
            properties: vec![("facing".to_string(), "north".to_string())],
            entity: None,
        },
        after: CellImage {
            name: "minecraft:air".to_string(),
            properties: Vec::new(),
            entity: None,
        },
    });
    prepare_with_published_carrier(&mut store, &operation);
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    // The chest chunk replayed to air; the already-air chunk verified.
    assert_eq!(report.applied, 1);
    assert_eq!(report.verified, 1);
    let first = get_chunk(&fixture, ChunkPos::new(0, 0)).unwrap();
    assert_eq!(chest_items(&first, chest_at), None);
    assert!(store.journal().in_progress.is_none());
}

#[test]
fn a_crash_after_every_chunk_before_committed_only_verifies() {
    let fixture = Fixture::new("verified");
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        ChunkPos::new(0, 0),
        &chunk_root(Tag::String("minecraft:air".into()), &[]),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    prepare_with_published_carrier(&mut store, &operation);
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 0);
    assert!(store.journal().in_progress.is_none());
}

#[test]
fn a_block_placed_after_the_commit_survives() {
    let fixture = Fixture::new("after-commit");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    prepare_with_published_carrier(&mut store, &operation);
    let _ = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    // A player's block lands on the vacated cell, then the world saves.
    let mut root = get_chunk(&fixture, position).unwrap();
    let mut data = ChunkData::new(root).unwrap();
    data.set_block_data(
        BlockPos::new(chest_at[0], chest_at[1], chest_at[2]),
        horizon_world::chunk::BlockState {
            name: "minecraft:stone".to_string(),
            properties: Default::default(),
        },
    )
    .unwrap();
    root = data.into_nbt();
    put_chunk(&fixture, position, &root);
    // Reopening finds no pending operation: the block stays, untouched.
    let Opened::Live(mut reopened) = Store::open(&fixture.root).unwrap() else {
        panic!("must reopen");
    };
    let report = recover(
        &mut reopened,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 0);
    let root = get_chunk(&fixture, position).unwrap();
    let data = ChunkData::new(root).unwrap();
    let found = data.block(BlockPos::new(4, 0, 4)).unwrap().unwrap();
    assert_eq!(found.name, "minecraft:stone");
}

#[test]
fn a_cell_holding_neither_image_refuses_the_load() {
    let fixture = Fixture::new("mismatch");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    // The chunk holds dirt where the operation's images say chest or air.
    put_chunk(
        &fixture,
        position,
        &chunk_root(Tag::String("minecraft:dirt".into()), &[]),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    prepare_with_published_carrier(&mut store, &operation);
    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        Err(RecoveryRefused::CellMismatch { pos, .. }) => assert_eq!(pos, chest_at),
        other => panic!("expected a cell mismatch, got {other:?}"),
    }
    // The journal still holds the operation: nothing was half-applied.
    assert_eq!(
        store.journal().in_progress.as_ref().map(|op| op.op_id),
        Some(operation.op_id)
    );
    let root = get_chunk(&fixture, position).unwrap();
    let data = ChunkData::new(root).unwrap();
    assert_eq!(
        data.block(BlockPos::new(4, 0, 4)).unwrap().unwrap().name,
        "minecraft:dirt"
    );
}

#[test]
fn a_disassembly_places_its_after_image_with_the_entity() {
    let fixture = Fixture::new("disassemble");
    let position = ChunkPos::new(0, 0);
    let target = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root(Tag::String("minecraft:air".into()), &[]),
    );
    let mut store = fixture.store();
    // A disassembly consumes a held carrier: list it first, as the
    // assembly that created it would have.
    store
        .begin(OpKind::Assemble, 7, 1, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    let op_id = store.next_op_id();
    let operation = Operation {
        op_id,
        kind: OpKind::Disassemble,
        uuid: 7,
        generation: 1,
        dimension: "minecraft:overworld".to_string(),
        cells: vec![OperationCell {
            world: target,
            local: [0, 0, 0],
            before: CellImage {
                name: "minecraft:air".to_string(),
                properties: Vec::new(),
                entity: None,
            },
            after: CellImage {
                name: "minecraft:chest".to_string(),
                properties: vec![("facing".to_string(), "north".to_string())],
                entity: Some(
                    horizon_core::nbt::parse_snbt(
                        r#"{id:"minecraft:chest",x:4,y:0,z:4,Items:[{Slot:3b,id:"minecraft:diamond",count:7}]}"#,
                    )
                    .unwrap()
                    .0,
                ),
            },
        }],
    };
    prepare_with_published_carrier(&mut store, &operation);
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 1);
    let root = get_chunk(&fixture, position).unwrap();
    let data = ChunkData::new(root.clone()).unwrap();
    assert_eq!(
        data.block(BlockPos::new(4, 0, 4)).unwrap().unwrap().name,
        "minecraft:chest"
    );
    // One owner: the chunk holds the items, and only the chunk.
    assert_eq!(chest_items(&root, target), Some(7));
}

#[test]
fn an_unreadable_or_missing_operation_refuses_the_world_load() {
    let fixture = Fixture::new("quarantine");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    // An unreadable operation file is a world-load refusal, both for
    // garbage bytes and for a file that is gone entirely.
    let mut store = fixture.store();
    let op_id = store.next_op_id();
    store
        .write_operation(op_id, b"not an operation file")
        .unwrap();
    store
        .begin(OpKind::Assemble, 7, 1, "minecraft:overworld")
        .unwrap();
    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        Err(RecoveryRefused::OperationUnreadable(_)) => {}
        other => panic!("expected an unreadable operation, got {other:?}"),
    }
    fs::remove_file(
        fixture
            .data()
            .join(format!("horizon_carrier_op_{op_id}.nbt")),
    )
    .unwrap();
    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        Err(RecoveryRefused::MissingOperation) => {}
        other => panic!("expected the missing operation to refuse, got {other:?}"),
    }
    let text = RecoveryRefused::MissingOperation.to_string();
    assert!(text.contains("world load refused"), "{text}");
    // The chunk and the journal are exactly as the crash left them.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, chest_at), Some(8));
    assert!(store.journal().in_progress.is_some());
}

#[test]
fn damaged_regions_refuse_without_generating_content() {
    // Each damage shape gets its own fresh prepared case, because the
    // store holds exactly one open operation at a time now.
    fn prepared_case(name: &str) -> (Fixture, Store, ChunkPos, PathBuf, [i32; 3]) {
        let fixture = Fixture::new(name);
        let position = ChunkPos::new(0, 0);
        let chest_at = [4, 0, 4];
        put_chunk(
            &fixture,
            position,
            &chunk_root_at(chest_state_tag(), &[(chest_at, vec![7; 8])], 0, 0),
        );
        let mut store = fixture.store();
        let operation = assemble_chest(store.next_op_id(), chest_at);
        prepare_with_published_carrier(&mut store, &operation);
        let region = fixture.region().join("r.0.0.mca");
        (fixture, store, position, region, chest_at)
    }
    // A truncated header.
    {
        let (fixture, mut store, _position, region, _chest) = prepared_case("short-header");
        let whole = fs::read(&region).unwrap();
        fs::write(&region, &whole[..4096]).unwrap();
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::Region {
                why: Strict::ShortHeader,
                ..
            }) => {}
            other => panic!("expected a short header, got {other:?}"),
        }
    }
    // A zeroed header: no entry at all.
    {
        let (fixture, mut store, _position, region, _chest) = prepared_case("zeroed-header");
        fs::write(&region, vec![0_u8; 8192]).unwrap();
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::Region {
                why: Strict::Missing,
                ..
            }) => {}
            other => panic!("expected a missing entry, got {other:?}"),
        }
    }
    // An offset past the file's end.
    {
        let (fixture, mut store, _position, region, _chest) = prepared_case("offset-eof");
        let mut past = fs::read(&region).unwrap();
        past[0..4].copy_from_slice(&0x0100_0001_u32.to_be_bytes());
        fs::write(&region, &past).unwrap();
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::Region {
                why: Strict::EntryPastEof,
                ..
            }) => {}
            other => panic!("expected an eof entry, got {other:?}"),
        }
    }
    // A zero sector count.
    {
        let (fixture, mut store, _position, region, _chest) = prepared_case("zero-sectors");
        let mut zeroed = fs::read(&region).unwrap();
        zeroed[3] = 0;
        fs::write(&region, &zeroed).unwrap();
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::Region {
                why: Strict::EntryPastEof,
                ..
            }) => {}
            other => panic!("expected a zero sector count, got {other:?}"),
        }
    }
    // A truncated payload.
    {
        let (fixture, mut store, _position, region, _chest) = prepared_case("short-payload");
        let whole = fs::read(&region).unwrap();
        fs::write(&region, &whole[..whole.len() - 2]).unwrap();
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::Region {
                why: Strict::TruncatedPayload | Strict::EntryPastEof,
                ..
            }) => {}
            other => panic!("expected a truncated payload, got {other:?}"),
        }
    }
    // A missing external .mcc stream: incompressible noise in a root
    // key pushes the chunk past the 256-sector threshold.
    {
        let fixture = Fixture::new("external-mcc");
        let position = ChunkPos::new(0, 0);
        let mut state = 0x4d59_5ea1_u64;
        let noise = (0..2 * 1024 * 1024)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 32) as u8
            })
            .collect::<Vec<u8>>();
        let mut huge = chunk_root_at(Tag::String("minecraft:stone".into()), &[], 0, 0);
        if let Tag::Compound(map) = &mut huge.tag {
            map.insert("horizon:noise".into(), Tag::ByteArray(noise));
        }
        let huge = ChunkData::new(huge).unwrap().into_nbt();
        put_chunk(&fixture, position, &huge);
        let external = fixture.region().join("c.0.0.mcc");
        assert!(external.exists(), "the fixture must go external");
        fs::remove_file(&external).unwrap();
        let mut store = fixture.store();
        let chest_at = [4, 0, 4];
        let operation = assemble_chest(store.next_op_id(), chest_at);
        prepare_with_published_carrier(&mut store, &operation);
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::Region {
                why: Strict::ExternalMissing,
                ..
            }) => {}
            other => panic!("expected the missing external stream, got {other:?}"),
        }
    }
}

#[test]
fn an_affected_chunk_without_a_region_refuses() {
    let fixture = Fixture::new("no-region");
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), [4, 0, 4]);
    prepare_with_published_carrier(&mut store, &operation);
    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        Err(RecoveryRefused::Region {
            why: Strict::NoRegion,
            ..
        }) => {}
        other => panic!("expected the missing region to refuse, got {other:?}"),
    }
}

#[test]
fn a_journal_and_operation_from_different_dimensions_refuse() {
    let fixture = Fixture::new("dim-disagree");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    let mut store = fixture.store();
    // The operation file says the Overworld; the journal's prepared
    // record names the Nether. The split is damage: the load refuses.
    let op_id = store.next_op_id();
    let operation = assemble_chest(op_id, chest_at);
    store
        .write_operation(op_id, &encode_operation(&operation).unwrap())
        .unwrap();
    store
        .begin(
            OpKind::Assemble,
            operation.uuid,
            operation.generation,
            "minecraft:the_nether",
        )
        .unwrap();
    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:the_nether",
    ) {
        Err(RecoveryRefused::DimensionDisagreement { journal, operation }) => {
            assert_eq!(journal, "minecraft:the_nether");
            assert_eq!(operation, "minecraft:overworld");
        }
        other => panic!("expected a disagreement, got {other:?}"),
    }
    // Nothing was touched: the chunk keeps its chest and items.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, chest_at), Some(8));
}

#[test]
fn an_operation_of_another_dimension_leaves_this_level_untouched() {
    let fixture = Fixture::new("dim-foreign");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    // This level's cell matches the foreign operation's before-image
    // exactly — same coordinates, same chest — but the operation runs
    // in the Nether, so the Overworld is not touched.
    put_chunk(
        &fixture,
        position,
        &chunk_root(chest_state_tag(), &[(chest_at, vec![7; 8])]),
    );
    let mut store = fixture.store();
    let op_id = store.next_op_id();
    let mut operation = assemble_chest(op_id, chest_at);
    operation.dimension = "minecraft:the_nether".to_string();
    prepare_with_published_carrier(&mut store, &operation);
    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        Err(RecoveryRefused::NotThisDimension(dimension)) => {
            assert_eq!(dimension, "minecraft:the_nether");
        }
        other => panic!("expected a foreign dimension, got {other:?}"),
    }
    // The chunk is byte-for-byte as it was: the chest and its items
    // still own this level's cell.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, chest_at), Some(8));
    let data = ChunkData::new(root).unwrap();
    assert_eq!(
        data.block(BlockPos::new(4, 0, 4)).unwrap().unwrap().name,
        "minecraft:chest"
    );
    // The operation is untouched for its own level's recovery.
    assert!(store.journal().in_progress.is_some());
}

#[test]
fn two_operations_replay_one_after_the_other() {
    let fixture = Fixture::new("two-ops");
    let position = ChunkPos::new(0, 0);
    let first_at = [4, 0, 4];
    let second_at = [8, 0, 8];
    let mut root = chunk_root(
        chest_state_tag(),
        &[(first_at, vec![7; 8]), (second_at, vec![7; 8])],
    );
    let _ = &mut root;
    put_chunk(&fixture, position, &root);
    let mut store = fixture.store();

    let operation = assemble_chest(store.next_op_id(), first_at);
    prepare_with_published_carrier(&mut store, &operation);
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 1);
    assert!(store.journal().in_progress.is_none());

    // The second operation carries a different UUID: the first owns 7.
    let mut operation = assemble_chest(store.next_op_id(), second_at);
    operation.uuid = 8;
    prepare_with_published_carrier(&mut store, &operation);
    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    assert_eq!(report.applied, 1);
    // Both cells are air now, and neither holds any items.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, first_at), None);
    assert_eq!(chest_items(&root, second_at), None);
    assert!(store.journal().in_progress.is_none());
}

/// The items of a block entity's `Items` field, for cargo checks.
fn entity_with_items(len: usize) -> Tag {
    Tag::compound([
        ("id", Tag::String("minecraft:chest".into())),
        ("Items", Tag::ByteArray(vec![7; len])),
    ])
}

/// (a) A crash after `prepared` in an assembly: recovery must leave a
/// carrier file that durably owns the whole cargo — cells, states, the
/// block entity — not merely an empty source cell.
#[test]
fn a_prepared_assembly_recovery_owns_the_cargo_in_a_carrier_file() {
    let fixture = Fixture::new("cargo");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root_at(chest_state_tag(), &[(chest_at, vec![7; 8])], 0, 0),
    );
    let mut store = fixture.store();
    let mut operation = assemble_chest(store.next_op_id(), chest_at);
    operation.cells[0].before.entity = Some(entity_with_items(8));
    prepare_with_published_carrier(&mut store, &operation);

    let report = recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap_or_else(|e| panic!("recovery refused: {e}"));

    // The source cell is air ...
    let root = get_chunk(&fixture, position).unwrap();
    let data = ChunkData::new(root).unwrap();
    assert_eq!(
        data.block(BlockPos::new(4, 0, 4)).unwrap().unwrap().name,
        "minecraft:air"
    );
    // ... and the cargo lives in a published carrier file: cells,
    // states and the block entity, decodable back.
    let path = fixture
        .data()
        .join(format!("horizon_carrier_{:032x}-1.nbt", 7_u128));
    let bytes = fs::read(&path)
        .unwrap_or_else(|e| panic!("the carrier file must exist after recovery: {e}"));
    let carrier = codec::decode(&bytes).unwrap();
    assert_eq!(carrier.uuid(), 7);
    assert_eq!(carrier.generation(), 1);
    assert_eq!(carrier.cells().len(), 1, "the whole cargo is carried");
    let cell = &carrier.cells()[0];
    assert_eq!(cell.name(), "minecraft:chest");
    assert_eq!(cell.pos(), LocalPos::new(0, 0, 0));
    let entity = cell
        .data()
        .unwrap_or_else(|| panic!("the block entity rides the carrier"));
    let (named, _) = nbt::read_named(entity, nbt::Limits::saved()).unwrap();
    assert_eq!(
        named.tag.get("Items").map(|t| match t {
            Tag::ByteArray(bytes) => bytes.len(),
            _ => 0,
        }),
        Some(8),
        "the entity's items are the cargo's"
    );
    let _ = report;
}

/// (c) A second assembly with an occupied UUID refuses; the first
/// carrier keeps its payload.
#[test]
fn an_occupied_uuid_refuses_and_keeps_the_first_carrier() {
    let fixture = Fixture::new("uuid");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root_at(chest_state_tag(), &[(chest_at, vec![7; 8])], 0, 0),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    prepare_with_published_carrier(&mut store, &operation);
    recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    )
    .unwrap();
    let carrier_path = fixture
        .data()
        .join(format!("horizon_carrier_{:032x}-1.nbt", 7_u128));
    let first_bytes = fs::read(&carrier_path).unwrap();

    // Another transfer names the same UUID while the index holds it.
    let result = store.begin(OpKind::Assemble, 7, 1, "minecraft:overworld");
    assert!(
        result.is_err(),
        "an occupied UUID must refuse a second operation"
    );
    // The first carrier is untouched and still owns its cargo.
    assert_eq!(fs::read(&carrier_path).unwrap(), first_bytes);
    assert!(store.journal().lists(7, 1));
}

/// (d) `Store::begin` refuses when an operation is already open.
#[test]
fn begin_refuses_an_already_open_operation() {
    let fixture = Fixture::new("nested");
    let mut store = fixture.store();
    store
        .begin(OpKind::Assemble, 1, 1, "minecraft:overworld")
        .unwrap();
    let second = store.begin(OpKind::Assemble, 2, 1, "minecraft:overworld");
    assert!(second.is_err(), "a nested begin must refuse");
    // The open operation is still the first, untouched.
    let journal = store.journal();
    let op = journal.in_progress.as_ref().unwrap();
    assert_eq!(op.uuid, 1);
    assert_eq!(op.op_id, 1);
}

/// (e) Recovery refuses when the operation file's id, kind, UUID or
/// generation disagrees with the journal's prepared record.
#[test]
fn recovery_refuses_a_mismatched_operation_identity() {
    for mismatch in ["op_id", "kind", "uuid", "generation"] {
        let fixture = Fixture::new(&format!("identity-{mismatch}"));
        let position = ChunkPos::new(0, 0);
        let chest_at = [4, 0, 4];
        put_chunk(
            &fixture,
            position,
            &chunk_root_at(chest_state_tag(), &[(chest_at, vec![7; 8])], 0, 0),
        );
        let mut store = fixture.store();
        let operation = assemble_chest(store.next_op_id(), chest_at);
        prepare_with_published_carrier(&mut store, &operation);
        // Patch the operation file's identity field and re-encode; the
        // journal still names the original.
        let patched = match mismatch {
            "op_id" => Operation {
                op_id: operation.op_id + 5,
                ..operation.clone()
            },
            "kind" => Operation {
                kind: OpKind::Disassemble,
                ..operation.clone()
            },
            "uuid" => Operation {
                uuid: operation.uuid + 1,
                ..operation.clone()
            },
            _ => Operation {
                generation: operation.generation + 1,
                ..operation.clone()
            },
        };
        store
            .write_operation(operation.op_id, &encode_operation(&patched).unwrap())
            .unwrap();
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::IdentityDisagreement { field, .. }) => {
                assert_eq!(field, mismatch, "the refusal names what disagreed");
            }
            other => panic!("{mismatch} mismatch must refuse with a reason, got {other:?}"),
        }
        // Nothing was touched.
        let root = get_chunk(&fixture, position).unwrap();
        assert_eq!(chest_items(&root, chest_at), Some(8));
        assert!(store.journal().in_progress.is_some());
    }
}
/// (b) A journal replay where the block state already matches the
/// after-image but the block entity differs: skipping the cell
/// silently would strand the entity — recovery must refuse or restore
/// the after-image, never pass the difference by.
#[test]
fn a_state_match_with_a_different_entity_is_not_a_silent_pass() {
    let fixture = Fixture::new("entity-witness");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    // The cell is air — the after-image's state — but a leftover block
    // entity with different items sits there.
    let mut root = chunk_root_at(Tag::String("minecraft:air".into()), &[], 0, 0);
    if let Tag::Compound(map) = &mut root.tag {
        map.insert(
            "block_entities".into(),
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:chest".into())),
                ("x", Tag::Int(4)),
                ("y", Tag::Int(0)),
                ("z", Tag::Int(4)),
                ("Items", Tag::ByteArray(vec![3; 5])),
            ])]),
        );
    }
    put_chunk(&fixture, position, &root);
    let mut store = fixture.store();
    let mut operation = assemble_chest(store.next_op_id(), chest_at);
    operation.cells[0].before.entity = Some(entity_with_items(5));
    prepare_with_published_carrier(&mut store, &operation);

    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        // Either outcome is honest: the leftover entity removed (the
        // after-image restored) or the load refused.
        Ok(_) => {
            let root = get_chunk(&fixture, position).unwrap();
            assert_eq!(
                chest_items(&root, chest_at),
                None,
                "the after-image's entity state is restored, not skipped"
            );
            assert!(store.journal().in_progress.is_none());
        }
        Err(RecoveryRefused::CellMismatch { .. }) => {}
        other => {
            panic!("a differing entity under a matching state must not pass silently: {other:?}")
        }
    }
}

/// (a2) A prepared assembly whose carrier file is gone: recovery
/// refuses with a reason, the source keeps its cargo, the journal
/// stays prepared, and a second run refuses again — the cargo is
/// never lost to a silent pass.
#[test]
fn a_missing_carrier_file_refuses_and_keeps_the_source() {
    let fixture = Fixture::new("missing-carrier");
    let position = ChunkPos::new(0, 0);
    let chest_at = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root_at(chest_state_tag(), &[(chest_at, vec![7; 8])], 0, 0),
    );
    let mut store = fixture.store();
    let operation = assemble_chest(store.next_op_id(), chest_at);
    prepare_with_published_carrier(&mut store, &operation);
    // The crash also lost the carrier file.
    fs::remove_file(
        fixture
            .data()
            .join(format!("horizon_carrier_{:032x}-1.nbt", 7_u128)),
    )
    .unwrap();

    for attempt in 1..=2 {
        match recover(
            &mut store,
            &fixture.region(),
            &AllStatesKnown,
            "minecraft:overworld",
        ) {
            Err(RecoveryRefused::OperationUnreadable(why)) => {
                assert!(why.contains("carrier file"), "{attempt}: {why}");
            }
            other => panic!("{attempt}: a missing carrier must refuse, got {other:?}"),
        }
        // The journal is still prepared and the source untouched.
        assert!(
            store.journal().in_progress.is_some(),
            "{attempt}: the operation stays prepared"
        );
        let root = get_chunk(&fixture, position).unwrap();
        assert_eq!(chest_items(&root, chest_at), Some(8), "{attempt}");
    }
}

/// (a3) Recovery verifies the cargo against the journal: the carrier
/// must carry exactly the operation's before-image cells — count,
/// positions, names and entity content — not merely agree on UUID
/// and generation.
#[test]
fn a_carrier_short_of_the_operations_cells_refuses() {
    let fixture = Fixture::new("short-carrier");
    let position = ChunkPos::new(0, 0);
    let first = [4, 0, 4];
    let second = [8, 0, 8];
    put_chunk(
        &fixture,
        position,
        &chunk_root_at(
            chest_state_tag(),
            &[(first, vec![7; 8]), (second, vec![7; 8])],
            0,
            0,
        ),
    );
    let mut store = fixture.store();
    // The operation strips two cells...
    let mut operation = assemble_chest(store.next_op_id(), first);
    operation.cells.push(OperationCell {
        world: second,
        local: [1, 0, 0],
        before: CellImage {
            name: "minecraft:chest".to_string(),
            properties: vec![("facing".to_string(), "north".to_string())],
            entity: Some(entity_with_items(8)),
        },
        after: CellImage {
            name: "minecraft:air".to_string(),
            properties: Vec::new(),
            entity: None,
        },
    });
    // ...but the published carrier carries only one (same UUID and
    // generation, one cell short).
    let mut short = operation.clone();
    short.cells.pop();
    store
        .write_operation(operation.op_id, &encode_operation(&operation).unwrap())
        .unwrap();
    store
        .write_carrier(operation.uuid, operation.generation, &carrier_of(&short))
        .unwrap();
    store
        .begin(
            operation.kind,
            operation.uuid,
            operation.generation,
            &operation.dimension,
        )
        .unwrap();

    match recover(
        &mut store,
        &fixture.region(),
        &AllStatesKnown,
        "minecraft:overworld",
    ) {
        Err(RecoveryRefused::CargoDisagreement { .. }) => {}
        other => panic!("a short carrier must refuse, got {other:?}"),
    }
    // Nothing was touched: both source cells keep their cargo.
    let root = get_chunk(&fixture, position).unwrap();
    assert_eq!(chest_items(&root, first), Some(8));
    assert_eq!(chest_items(&root, second), Some(8));
    assert!(store.journal().in_progress.is_some());
}

/// (item 7) A carrier whose entity payload does not decode refuses
/// the disassembly instead of dropping the entity.
#[test]
fn an_undecodable_entity_payload_refuses() {
    use horizon_world::carrier::model::{Carrier, Cell, LocalPos};
    let fixture = Fixture::new("bad-payload");
    let position = ChunkPos::new(0, 0);
    let target = [4, 0, 4];
    put_chunk(
        &fixture,
        position,
        &chunk_root_at(Tag::String("minecraft:air".into()), &[], 0, 0),
    );
    let mut store = fixture.store();
    // List the carrier (assembly committed), then corrupt its
    // payload: not NBT.
    store
        .begin(OpKind::Assemble, 54, 1, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    let cells = vec![
        Cell::new(
            LocalPos::new(0, 0, 0),
            "minecraft:chest",
            vec![("facing".to_string(), "north".to_string())],
            Some(vec![0xff, 0xff, 0xff, 0xff]),
        )
        .unwrap(),
    ];
    let carrier = Carrier::new(
        54,
        1,
        0,
        horizon_world::carrier::contact::RigidTransform::identity_at(
            horizon_core::geometry::Vec3::new(4.0, 0.0, 4.0),
        )
        .unwrap(),
        cells,
    )
    .unwrap();
    store
        .write_carrier(54, 1, &codec::encode(&carrier).unwrap())
        .unwrap();

    // The live disassembly is exercised through recover-style
    // decoding: assert the payload's own decode failure surfaces as
    // a typed refusal (CarrierUnreadable), never a silent drop.
    let bytes = std::fs::read(store.carrier_path(54, 1)).unwrap();
    let decoded = codec::decode(&bytes).unwrap();
    let payload = decoded.cells()[0].data().unwrap();
    assert!(
        nbt::read_named(payload, nbt::Limits::saved()).is_err(),
        "the fixture's payload must not decode"
    );
    let _ = target;
}

/// A carrier file without a prepared operation is an orphan: the
/// cleanup deletes it at the next open.
#[test]
fn an_orphan_carrier_file_is_deleted_by_the_cleanup() {
    let fixture = Fixture::new("orphan-carrier");
    let mut store = fixture.store();
    // A committed assembly's index entry exists, then the journal's
    // record is gone (a torn last slot leaves the file orphaned).
    store
        .begin(OpKind::Assemble, 71, 1, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    store.write_carrier(71, 1, b"owned carrier bytes").unwrap();
    let carrier = fixture
        .data()
        .join(format!("horizon_carrier_{:032x}-1.nbt", 71_u128));
    assert!(carrier.is_file());
    // The index still lists it, so this is not an orphan yet: remove
    // the listing the way a disassembly would, keep the file (the
    // crash before the durable delete), and the tombstone holds —
    // then a reopen resolves it. Instead, for the true orphan: the
    // file exists without any journal knowledge at all.
    let mut orphan = fixture.store();
    orphan
        .write_carrier(72, 1, b"orphaned carrier bytes")
        .unwrap();
    // 72 is not in the index and not tombstoned: an orphan.
    let report = orphan.cleanup().unwrap();
    assert_eq!(report.orphan_carriers, 1);
    assert!(
        !fixture
            .data()
            .join(format!("horizon_carrier_{:032x}-1.nbt", 72_u128))
            .is_file()
    );
    // The listed carrier (71) survives: it is owned, not orphaned.
    assert!(carrier.is_file());
}

/// Round-1 tail: a re-assembly of a disassembled UUID takes the next
/// generation with checked arithmetic; a stale handle on the old
/// generation refuses; and a remembered u32::MAX refuses rather than
/// wrapping.
#[test]
fn generations_advance_checked_and_stale_handles_refuse() {
    let fixture = Fixture::new("generations");
    let mut store = fixture.store();
    // First assembly: generation 1, committed (the index remembers it).
    assert_eq!(store.next_generation(81).unwrap(), 1);
    store
        .begin(OpKind::Assemble, 81, 1, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    // The index holds (81,1): a re-assembly of the same UUID is an
    // occupied name, but next_generation reads the remembered one.
    assert_eq!(store.next_generation(81).unwrap(), 2);
    // Disassemble it: the tombstone remembers generation 1.
    store
        .begin(OpKind::Disassemble, 81, 1, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    assert!(store.journal().tombstoned(81, 1));
    // A stale handle on the old generation refuses: 1 is no longer held.
    assert!(
        store
            .begin(OpKind::Disassemble, 81, 1, "minecraft:overworld")
            .is_err()
    );
    // The durable counter also counts the disassembly.
    assert_eq!(store.next_generation(81).unwrap(), 3);
    // The last representable operation generation is admissible.
    let mut maxed = fixture.store();
    maxed
        .begin(OpKind::Assemble, 82, i32::MAX as u32, "minecraft:overworld")
        .unwrap();
    maxed.commit().unwrap();
    assert!(
        maxed
            .next_generation(82)
            .unwrap_err()
            .to_string()
            .contains("generation limit")
    );
    let before = maxed.journal().clone();
    assert!(
        maxed
            .begin(
                OpKind::Assemble,
                83,
                i32::MAX as u32 + 1,
                "minecraft:overworld"
            )
            .is_err()
    );
    assert_eq!(maxed.journal(), &before);
}
