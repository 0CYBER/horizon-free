//! Carrier ownership store tests (slice 2 part A, issue arismoko/horizon#2413): the
//! two-slot journal, first use, the opening table, cleanup, the operation
//! codec's envelope and crash injection at the file level. Every test
//! runs on a throwaway world folder, never through `Game`.

use crate::carrier_store::{
    AllStatesKnown, Cleanup, IN_USE_FLAG, Journal, MAX_CELL_ENTITY, MAX_OP_CELLS,
    MAX_OP_COMPRESSED, MAX_OP_DECODED, MAX_OP_ENTITIES, OpError, OpKind, Opened, Operation,
    OperationCell, Refusal, SLOTS, StateValidator, Store, decode_operation, encode_operation,
};
use horizon_core::nbt::{self, Tag};
use std::{fs, path::PathBuf};

/// A throwaway world root with a `data/` folder, removed on drop.
struct WorldDir {
    root: PathBuf,
}
impl WorldDir {
    /// A fresh world whose `data/` exists; `name` keeps parallel tests
    /// apart under the process's temp folder.
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "horizon-carrier-store-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data")).unwrap();
        let world = Self { root };
        world.write_level_dat(false);
        world
    }

    /// A world without `data/`, as a save this session would create it.
    fn without_data(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "horizon-carrier-store-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn level_dat(&self) -> PathBuf {
        self.root.join("level.dat")
    }

    /// A minimal valid `level.dat`, with the in-use flag when asked.
    fn write_level_dat(&self, flag: bool) {
        let mut data = horizon_core::nbt::Compound::new();
        if flag {
            data.insert(IN_USE_FLAG.into(), Tag::Byte(1));
        }
        crate::save::write_dat(
            &self.level_dat(),
            Tag::compound([("Data", Tag::Compound(data))]),
        )
        .unwrap();
    }

    /// Every `horizon*` and `.dat*` name under `data/`, sorted.
    fn store_files(&self) -> Vec<String> {
        let mut names = fs::read_dir(self.data())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn slot_path(&self, slot: usize) -> PathBuf {
        self.data().join(SLOTS[slot])
    }

    fn first_use(&self) -> Store {
        Store::first_use(&self.root, true).unwrap()
    }
}
impl Drop for WorldDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A state validator that knows nothing.
struct NoStatesKnown;
impl StateValidator for NoStatesKnown {
    fn known(&self, _name: &str, _properties: &[(String, String)]) -> bool {
        false
    }
}

/// An operation with `cells` cells whose before-images each carry a
/// `fill`-byte pseudo-random entity; after-images are air.
fn sample_operation(op_id: u64, cells: usize, fill: usize) -> Operation {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    let mut bytes = |len: usize| -> Vec<u8> {
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 33) as u8
            })
            .collect()
    };
    Operation {
        op_id,
        kind: OpKind::Assemble,
        uuid: 0x0011_2233_4455_6677_8899_aabb_ccdd_eeff,
        generation: 3,
        dimension: "minecraft:overworld".to_string(),
        cells: (0..cells)
            .map(|i| OperationCell {
                world: [i as i32, 64, -(i as i32)],
                local: [((i % 256) as i32 - 128) as i8, 0, 0],
                before: crate::carrier_store::CellImage {
                    name: "minecraft:chest".to_string(),
                    properties: vec![("facing".to_string(), "north".to_string())],
                    entity: Some(Tag::compound([("Items", Tag::ByteArray(bytes(fill)))])),
                },
                after: crate::carrier_store::CellImage {
                    name: "minecraft:air".to_string(),
                    properties: Vec::new(),
                    entity: None,
                },
            })
            .collect(),
    }
}

/// One chest item, wherever it lives, as a byte count for owner checks.
fn chest_items(tag: &Tag) -> Option<usize> {
    tag.get("Items").map(|items| match items {
        Tag::ByteArray(bytes) => bytes.len(),
        _ => 0,
    })
}

#[test]
fn flag_round_trips_through_level_dat() {
    let world = WorldDir::new("flag-round-trip");
    crate::carrier_store::set_in_use_flag(&world.level_dat()).unwrap();
    assert!(crate::carrier_store::in_use_flag(&world.root));
}

#[test]
fn only_cube_symmetry_rotations_pass() {
    let quarter_y = [
        std::f64::consts::FRAC_1_SQRT_2,
        0.0,
        std::f64::consts::FRAC_1_SQRT_2,
        0.0,
    ];
    assert!(crate::carrier_store::cube_symmetry_rotation([
        1.0, 0.0, 0.0, 0.0
    ]));
    assert!(crate::carrier_store::cube_symmetry_rotation(quarter_y));
    assert!(crate::carrier_store::cube_symmetry_rotation([
        -1.0, 0.0, 0.0, 0.0
    ]));
    // A 180-degree turn about Y, a 90-degree turn about X.
    assert!(crate::carrier_store::cube_symmetry_rotation([
        0.0, 0.0, 1.0, 0.0
    ]));
    assert!(crate::carrier_store::cube_symmetry_rotation([
        std::f64::consts::FRAC_1_SQRT_2,
        std::f64::consts::FRAC_1_SQRT_2,
        0.0,
        0.0
    ]));
    // Anything not a cube symmetry refuses: a sixth of a turn about Y,
    // a diagonal axis, a zero quaternion, a non-finite value.
    assert!(!crate::carrier_store::cube_symmetry_rotation([
        0.5_f64.sqrt(),
        0.0,
        0.5_f64.sqrt() / 6.0,
        0.0
    ]));
    assert!(!crate::carrier_store::cube_symmetry_rotation([
        0.5, 0.5, 0.5, 0.5
    ]));
    assert!(!crate::carrier_store::cube_symmetry_rotation([0.0; 4]));
    assert!(!crate::carrier_store::cube_symmetry_rotation([
        f64::NAN,
        0.0,
        0.0,
        0.0
    ]));
}

#[test]
fn crc32_matches_the_ieee_vector() {
    assert_eq!(crc32fast::hash(b"123456789"), 0xcbf4_3926);
}

#[test]
fn a_world_without_carriers_creates_nothing() {
    let world = WorldDir::new("unused");
    assert!(matches!(Store::open(&world.root), Ok(Opened::NeverUsed)));
    // Reading the flag and opening again still creates nothing.
    assert!(!crate::carrier_store::in_use_flag(&world.root));
    assert!(matches!(Store::open(&world.root), Ok(Opened::NeverUsed)));
    assert!(world.store_files().is_empty());
    assert!(fs::read_dir(world.data()).unwrap().next().is_none());
}

#[test]
fn first_use_passes_each_step_and_survives_each_crash() {
    // Crash before anything: never used.
    let world = WorldDir::new("first-use-clean");
    assert!(matches!(Store::open(&world.root), Ok(Opened::NeverUsed)));

    // Crash inside the level.dat replacement gap: the old file is still
    // there (level.dat_old holds the pre-replacement copy), so the world
    // opens as never used with no store files.
    let world = WorldDir::new("first-use-flag-gap");
    fs::copy(world.level_dat(), world.root.join("level.dat_old")).unwrap();
    fs::remove_file(world.level_dat()).unwrap();
    assert!(matches!(Store::open(&world.root), Ok(Opened::NeverUsed)));
    assert!(world.store_files().is_empty());

    // Crash after the flag, before slot 0: the flag without slots refuses.
    let world = WorldDir::new("first-use-flag-only");
    crate::carrier_store::set_in_use_flag(&world.level_dat()).unwrap();
    assert_eq!(
        Store::open(&world.root).unwrap_err(),
        Refusal::MissingStore { flag: true }
    );

    // The finished first use opens live with an empty journal.
    let world = WorldDir::new("first-use-done");
    let store = world.first_use();
    assert_eq!(*store.journal(), Journal::empty());
    drop(store);
    match Store::open(&world.root) {
        Ok(Opened::Live(store)) => {
            assert_eq!(*store.journal(), Journal::empty());
        }
        other => panic!("expected a live store, got {other:?}"),
    }

    // Crash of the whole data/ entry: the store is missing, by flag.
    let world = WorldDir::new("first-use-data-lost");
    world.first_use();
    fs::remove_dir_all(world.data()).unwrap();
    assert_eq!(
        Store::open(&world.root).unwrap_err(),
        Refusal::MissingStore { flag: true }
    );
}

#[test]
fn first_use_refuses_a_data_folder_created_this_session() {
    let world = WorldDir::without_data("first-use-new-data");
    fs::create_dir_all(world.data()).unwrap();
    world.write_level_dat(false);
    match Store::first_use(&world.root, false) {
        Err(error) => {
            let text = error.to_string();
            assert!(text.contains("save and reopen"), "{text}");
        }
        Ok(_) => panic!("a session-created data folder must refuse"),
    }
    assert!(world.store_files().is_empty());
}

#[test]
fn a_torn_newest_slot_falls_back_at_every_offset() {
    let world = WorldDir::new("torn-slot");
    let mut store = world.first_use();
    store
        .begin(OpKind::Assemble, 7, 1, "minecraft:overworld")
        .unwrap();
    let newest = fs::read(world.slot_path(1)).unwrap();
    // The newest record lives in slot 1; the first-use record in slot 0.
    assert!(fs::read(world.slot_path(0)).unwrap().len() < newest.len());
    for cut in 0..newest.len() {
        let torn: Vec<u8> = newest[..cut].to_vec();
        fs::write(world.slot_path(1), &torn).unwrap();
        match Store::open(&world.root) {
            Ok(Opened::Live(store)) => {
                // The older valid record rules: the begin was not adopted.
                assert!(store.journal().in_progress.is_none(), "cut {cut}");
            }
            other => panic!("cut {cut} must fall back, got {other:?}"),
        }
    }
    // A flipped byte anywhere — magic, version, record number, length,
    // CRC or body — makes the record invalid: the CRC covers every header
    // field after the magic, so a flip can never forge a newer record or
    // a newer schema.
    for at in 0..newest.len() {
        let mut flipped = newest.clone();
        flipped[at] ^= 0x40;
        fs::write(world.slot_path(1), &flipped).unwrap();
        match Store::open(&world.root) {
            Ok(Opened::Live(store)) => assert!(store.journal().in_progress.is_none()),
            other => panic!("flip {at} must fall back, got {other:?}"),
        }
    }
    // Both slots torn: refuse.
    fs::write(world.slot_path(0), b"garbage").unwrap();
    fs::write(world.slot_path(1), &newest[..10]).unwrap();
    assert_eq!(Store::open(&world.root).unwrap_err(), Refusal::NoValidSlot);
}

#[test]
fn many_replacements_in_a_row_keep_the_newest_record() {
    let world = WorldDir::new("many-replacements");
    let mut store = world.first_use();
    for expected in 1..=100 {
        // Each record leaves the previous operation open, so the next
        // begin must reopen first — exactly the one-operation rule.
        if expected > 1 {
            let Opened::Live(reopened) = Store::open(&world.root).unwrap() else {
                panic!("must reopen before record {expected}");
            };
            store = reopened;
            // Close the open record by committing it, as recovery would.
            store.commit().unwrap();
        }
        store
            .begin(OpKind::Assemble, expected as u128, 1, "minecraft:overworld")
            .unwrap();
        let Opened::Live(reopened) = Store::open(&world.root).unwrap() else {
            panic!("must reopen after record {expected}");
        };
        assert_eq!(
            reopened.journal().in_progress.as_ref().unwrap().op_id,
            expected
        );
        store = reopened;
    }
    let Opened::Live(store) = Store::open(&world.root).unwrap() else {
        unreachable!();
    };
    assert_eq!(store.journal().next_op_id, 101);
}

#[test]
fn leftover_files_without_slots_refuse_whatever_the_flag() {
    for flag in [false, true] {
        let world = WorldDir::new(&format!("leftover-{flag}"));
        let store = world.first_use();
        let op = store.operation_path(9);
        let carrier = store.carrier_path(0xabcd, 2);
        drop(store);
        fs::write(op, b"leftover").unwrap();
        fs::write(carrier, b"leftover").unwrap();
        fs::remove_file(world.slot_path(0)).unwrap();
        let _ = fs::remove_file(world.slot_path(1));
        world.write_level_dat(flag);
        assert_eq!(
            Store::open(&world.root).unwrap_err(),
            Refusal::MissingStore { flag },
            "flag {flag}"
        );
        let text = Store::open(&world.root).unwrap_err().to_string();
        assert!(text.contains("backup"), "{text}");
    }
}

#[test]
fn a_newer_slot_schema_refuses() {
    let world = WorldDir::new("newer-schema");
    world.first_use();
    // A corrupted version byte is an invalid record, not a schema: the
    // CRC covers the version, so the broken slot just loses.
    let mut newer = fs::read(world.slot_path(0)).unwrap();
    newer[4..6].copy_from_slice(&99_u16.to_le_bytes());
    fs::write(world.slot_path(0), &newer).unwrap();
    assert_eq!(Store::open(&world.root).unwrap_err(), Refusal::NoValidSlot);
}

#[test]
fn a_valid_newer_schema_slot_refuses() {
    let world = WorldDir::new("newer-valid-crc");
    world.first_use();
    // Patch the version to 99 and repair the CRC, so the record itself is
    // intact and only its schema is newer.
    let mut newer = fs::read(world.slot_path(0)).unwrap();
    newer[4..6].copy_from_slice(&99_u16.to_le_bytes());
    let body_len = u32::from_le_bytes(newer[14..18].try_into().unwrap()) as usize;
    let mut crc_input = Vec::with_capacity(14 + body_len);
    crc_input.extend_from_slice(&newer[4..18]);
    crc_input.extend_from_slice(&newer[22..22 + body_len]);
    newer[18..22].copy_from_slice(&crc32fast::hash(&crc_input).to_le_bytes());
    fs::write(world.slot_path(0), &newer).unwrap();
    assert_eq!(
        Store::open(&world.root).unwrap_err(),
        Refusal::NewerSchema { found: 99 }
    );
}

#[test]
fn hostile_slots_refuse_without_reading_their_bodies() {
    let world = WorldDir::new("hostile-slots");
    world.first_use();
    // A declared body length past the slot cap, and past the file.
    let mut hostile = fs::read(world.slot_path(0)).unwrap();
    hostile[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(world.slot_path(1), &hostile).unwrap();
    match Store::open(&world.root) {
        Ok(Opened::Live(store)) => assert_eq!(*store.journal(), Journal::empty()),
        other => panic!("the valid slot 0 must rule, got {other:?}"),
    }
    let mut over = fs::read(world.slot_path(0)).unwrap();
    // A slot file past the cap is invalid without its bytes being read.
    fs::write(world.slot_path(0), vec![0_u8; 1024 * 1024 + 1]).unwrap();
    fs::write(world.slot_path(1), vec![0_u8; 1024 * 1024 + 1]).unwrap();
    assert_eq!(Store::open(&world.root).unwrap_err(), Refusal::NoValidSlot);
    // Counts over their caps make a CRC-correct body invalid too.
    over[31..35].copy_from_slice(&u32::MAX.to_le_bytes());
    let body_len = u32::from_le_bytes(over[14..18].try_into().unwrap()) as usize;
    let mut crc_input = Vec::with_capacity(14 + body_len);
    crc_input.extend_from_slice(&over[4..18]);
    crc_input.extend_from_slice(&over[22..22 + body_len]);
    over[18..22].copy_from_slice(&crc32fast::hash(&crc_input).to_le_bytes());
    fs::write(world.slot_path(1), &over).unwrap();
    fs::remove_file(world.slot_path(0)).unwrap();
    assert_eq!(Store::open(&world.root).unwrap_err(), Refusal::NoValidSlot);
}

#[test]
fn cleanup_resolves_tombstones_and_deletes_orphans_and_stale_ops() {
    let world = WorldDir::new("cleanup");
    let mut store = world.first_use();
    let (uuid, generation) = (0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10_u128, 4);
    store.write_carrier(uuid, generation, b"carrier").unwrap();
    store
        .begin(OpKind::Assemble, uuid, generation, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    assert!(store.journal().lists(uuid, generation));
    // A stale operation file from an interrupted run.
    store.write_operation(1, b"stale").unwrap();
    assert_eq!(
        store.cleanup().unwrap(),
        Cleanup {
            orphan_carriers: 0,
            stale_operations: 1,
            resolved_tombstones: 0,
        }
    );
    // Disassembly tombstones the carrier; its file outlives the commit.
    store
        .begin(OpKind::Disassemble, uuid, generation, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    assert!(store.journal().tombstoned(uuid, generation));
    // The cleanup interrupted by that crash deletes the file, but this
    // session cannot prove the deletion durable (Windows has no directory
    // fsync), so the tombstone holds.
    assert_eq!(
        store.cleanup().unwrap(),
        Cleanup {
            orphan_carriers: 0,
            stale_operations: 0,
            resolved_tombstones: 0,
        }
    );
    assert!(store.journal().tombstoned(uuid, generation));
    assert!(!world.carrier_path(uuid, generation).exists());
    // A later open finds the file gone; only now the tombstone clears.
    drop(store);
    let Opened::Live(mut reopened) = Store::open(&world.root).unwrap() else {
        panic!("must reopen");
    };
    assert_eq!(
        reopened.cleanup().unwrap(),
        Cleanup {
            orphan_carriers: 0,
            stale_operations: 0,
            resolved_tombstones: 1,
        }
    );
    assert!(!reopened.journal().tombstoned(uuid, generation));
    // A second pass has nothing to do.
    assert_eq!(reopened.cleanup().unwrap(), Cleanup::default());

    // An orphan carrier file, known to no index and no tombstone.
    let orphan = 0xdead_beef_u128;
    fs::write(world.carrier_path(orphan, 9), b"orphan").unwrap();
    assert_eq!(
        reopened.cleanup().unwrap(),
        Cleanup {
            orphan_carriers: 1,
            stale_operations: 0,
            resolved_tombstones: 0,
        }
    );
    assert!(!world.carrier_path(orphan, 9).exists());
    // The in-progress operation file survives every cleanup.
    reopened
        .begin(OpKind::Assemble, 0x24, 1, "minecraft:overworld")
        .unwrap();
    let live = reopened.journal().in_progress.as_ref().unwrap().op_id;
    reopened.write_operation(live, b"live").unwrap();
    assert_eq!(reopened.cleanup().unwrap(), Cleanup::default());
}

/// `WorldDir` helpers used above.
impl WorldDir {
    fn carrier_path(&self, uuid: u128, generation: u32) -> PathBuf {
        self.data()
            .join(format!("horizon_carrier_{uuid:032x}-{generation}.nbt"))
    }
}

#[test]
fn operation_files_round_trip_exactly() {
    let operation = sample_operation(5, 3, 64);
    let encoded = encode_operation(&operation).unwrap();
    let decoded = decode_operation(&encoded, &AllStatesKnown).unwrap();
    assert_eq!(decoded, operation);
    // An unknown state is refused through the validator, not rewritten.
    assert_eq!(
        decode_operation(&encoded, &NoStatesKnown),
        Err(OpError::UnknownState)
    );
}

#[test]
fn hostile_operation_files_refuse_without_panic() {
    let world = WorldDir::new("hostile-ops");
    let store = world.first_use();
    let good = encode_operation(&sample_operation(1, 1, 8)).unwrap();

    // Bytes after the root.
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(matches!(
        decode_operation(&trailing, &AllStatesKnown),
        Err(OpError::Malformed(_))
    ));
    // An unknown version.
    let (root, _) = nbt::read_named(
        &horizon_world::compression::Compression::Gzip
            .decode(&good, 1 << 24)
            .unwrap(),
        nbt::Limits::saved(),
    )
    .unwrap();
    let Tag::Compound(mut map) = root.tag else {
        unreachable!();
    };
    map.insert("version".into(), Tag::Int(99));
    let patched = crate::save::write_dat(&world.data().join("patched.dat"), Tag::Compound(map));
    patched.unwrap();
    let patched = fs::read(world.data().join("patched.dat")).unwrap();
    assert_eq!(
        decode_operation(&patched, &AllStatesKnown),
        Err(OpError::Malformed("unsupported operation version"))
    );
    fs::remove_file(world.data().join("patched.dat")).unwrap();

    // A declared cell list past the cap: the reader bounds it by the
    // bytes actually present, and the count check refuses the rest.
    let mut bomb = Vec::new();
    bomb.extend_from_slice(&[0x0a, 0x00, 0x00]); // compound root, empty name
    bomb.extend_from_slice(&[0x09, 0x00, 0x05]); // list "cells"
    bomb.extend_from_slice(b"cells");
    bomb.extend_from_slice(&[0x0a]); // compound elements
    bomb.extend_from_slice(&0x7fff_ffff_u32.to_be_bytes());
    let _ = store; // the store is only the path holder here
    assert!(matches!(
        decode_operation(&bomb, &AllStatesKnown),
        Err(OpError::Malformed(_))
    ));

    // A compressed file past the envelope refuses before decompression.
    let huge = vec![0x1f, 0x8b]; // gzip header junk, but the size gate hits first
    let huge = [huge.as_slice(), &[0_u8; MAX_OP_COMPRESSED][..]].concat();
    assert_eq!(
        decode_operation(&huge, &AllStatesKnown),
        Err(OpError::CompressedTooBig { size: huge.len() })
    );
}

#[test]
#[ignore = "daily"]
fn entity_and_operation_envelopes_hold_exactly_at_their_limits() {
    // The largest entity image that fits passes; one more data byte
    // crosses the cell's bound and the encode refuses before any lift.
    let entity_size = |fill: usize| {
        nbt::write_named(
            &nbt::NamedTag {
                name: "".into(),
                tag: sample_operation(0, 1, fill).cells[0]
                    .before
                    .entity
                    .clone()
                    .unwrap(),
            },
            nbt::Limits::saved(),
        )
        .unwrap()
        .len()
    };
    let mut fitting = 0_usize;
    while entity_size(fitting + 1) <= MAX_CELL_ENTITY {
        fitting += 1;
    }
    assert!(entity_size(fitting) <= MAX_CELL_ENTITY);
    assert!(entity_size(fitting + 1) > MAX_CELL_ENTITY);
    decode_operation(
        &encode_operation(&sample_operation(1, 1, fitting)).unwrap(),
        &AllStatesKnown,
    )
    .unwrap();
    assert!(matches!(
        encode_operation(&sample_operation(1, 1, fitting + 1)),
        Err(OpError::EntityTooBig { .. })
    ));

    // 16 MiB of before-images in total passes; one byte more refuses.
    let cells = MAX_OP_ENTITIES / entity_size(fitting);
    let at_total = sample_operation(2, cells, fitting);
    let over_total = sample_operation(2, cells + 1, fitting);
    decode_operation(&encode_operation(&at_total).unwrap(), &AllStatesKnown).unwrap();
    assert!(matches!(
        encode_operation(&over_total),
        Err(OpError::EntitiesTooBig { .. })
    ));

    // The cell count bound.
    let over_cells = sample_operation(3, MAX_OP_CELLS + 1, 1);
    assert_eq!(
        encode_operation(&over_cells),
        Err(OpError::CellCount {
            count: MAX_OP_CELLS + 1
        })
    );
}

#[test]
#[ignore = "daily"]
fn a_decoded_operation_never_passes_the_decoded_envelope() {
    // A gzip payload that would decode past 40 MiB is refused by the
    // decompression budget, never materialized.
    // Compressible bytes, so the compressed-size gate passes and the
    // decompression budget is what refuses.
    let raw = vec![0_u8; MAX_OP_DECODED + 1];
    let compressed = horizon_world::compression::Compression::Gzip
        .encode(&raw)
        .unwrap();
    assert!(compressed.len() < MAX_OP_COMPRESSED);
    assert!(matches!(
        decode_operation(&compressed, &AllStatesKnown),
        Err(OpError::Malformed(_))
    ));
}

#[test]
#[ignore = "daily"]
fn a_thousand_operations_do_not_grow_the_store() {
    let world = WorldDir::new("thousand-ops");
    let mut store = world.first_use();
    let uuid = 0x7765_7265_u128;
    let mut files = 0_usize;
    let mut slot_size = 0_usize;
    for step in 0..1000 {
        // One carrier, assembled and disassembled back: the index and
        // tombstone list return to empty, as they must for a repeated
        // transfer, and cleanup runs like every world open.
        let op_id = store
            .begin(OpKind::Assemble, uuid, 1, "minecraft:overworld")
            .unwrap();
        let encoded = encode_operation(&sample_operation(op_id, 2, 32)).unwrap();
        store.write_operation(op_id, &encoded).unwrap();
        store.commit().unwrap();
        let op_id = store
            .begin(OpKind::Disassemble, uuid, 1, "minecraft:overworld")
            .unwrap();
        store
            .write_operation(
                op_id,
                &encode_operation(&sample_operation(op_id, 1, 8)).unwrap(),
            )
            .unwrap();
        store.commit().unwrap();
        // The tombstone outlives the session that deleted the file; the
        // next open, like the next server session, is what clears it.
        drop(store);
        let Opened::Live(mut reopened) = Store::open(&world.root).unwrap() else {
            panic!("must reopen after step {step}");
        };
        reopened.cleanup().unwrap();
        store = reopened;
        if step == 1 {
            // By the second round both slots have held a settled, empty
            // journal; that fixed size is the no-growth baseline.
            files = world.store_files().len();
            slot_size = fs::read(world.slot_path(0)).unwrap().len();
        }
    }
    // Two slots, no operation or carrier files, no growth.
    assert_eq!(world.store_files().len(), files);
    // The newest record keeps the settled size; the other slot may hold
    // one older record with a since-cleared tombstone, never more.
    let sizes = [
        fs::metadata(world.slot_path(0)).unwrap().len() as usize,
        fs::metadata(world.slot_path(1)).unwrap().len() as usize,
    ];
    assert_eq!(*sizes.iter().min().unwrap(), slot_size);
    assert!(*sizes.iter().max().unwrap() <= slot_size + 20);
    match Store::open(&world.root) {
        Ok(Opened::Live(reopened)) => assert_eq!(reopened.journal().next_op_id, 2001),
        other => panic!("expected a live store, got {other:?}"),
    }
}

#[test]
fn chest_ownership_survives_the_store_layers() {
    // The single-owner check the crash tests rely on, in miniature: an
    // operation's before-image carries the chest, its after-image is
    // air, and both layers answer for exactly one of them.
    let operation = sample_operation(1, 1, 128);
    let encoded = encode_operation(&operation).unwrap();
    let decoded = decode_operation(&encoded, &AllStatesKnown).unwrap();
    let cell = &decoded.cells[0];
    let before = cell.before.entity.as_ref().and_then(chest_items);
    let after = cell.after.entity.as_ref().and_then(chest_items);
    assert_eq!(before, Some(128));
    assert_eq!(after, None);
}

#[test]
fn first_use_leaves_the_flag_readable_after_vanilla_saves() {
    // A vanilla-style rebuild of level.dat drops unknown keys; the store
    // itself is the witness, so the world stays in use.
    let world = WorldDir::new("vanilla-save");
    world.first_use();
    world.write_level_dat(false);
    match Store::open(&world.root) {
        Ok(Opened::Live(_)) => {}
        other => panic!("slots must rule over a lost flag, got {other:?}"),
    }
}

#[test]
fn operation_signed_ranges_refuse_before_publication() {
    let mut operation = sample_operation(1, 1, 0);
    operation.generation = i32::MAX as u32;
    operation.op_id = i64::MAX as u64;
    assert!(encode_operation(&operation).is_ok());
    operation.generation += 1;
    assert!(encode_operation(&operation).is_err());
    operation.generation = 1;
    operation.op_id += 1;
    assert!(encode_operation(&operation).is_err());
}

#[test]
fn transfer_opens_do_not_widen_retention_eligibility() {
    let world = WorldDir::new("session-retention");
    let mut store = world.first_use();
    let (uuid, generation) = (0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10_u128, 4);
    store.write_carrier(uuid, generation, b"carrier").unwrap();
    store
        .begin(OpKind::Assemble, uuid, generation, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    assert!(store.journal().lists(uuid, generation));
    // A stale operation file from an interrupted run.
    store.write_operation(1, b"stale").unwrap();
    assert_eq!(
        store.cleanup().unwrap(),
        Cleanup {
            orphan_carriers: 0,
            stale_operations: 1,
            resolved_tombstones: 0,
        }
    );
    // Disassembly tombstones the carrier; its file outlives the commit.
    store
        .begin(OpKind::Disassemble, uuid, generation, "minecraft:overworld")
        .unwrap();
    store.commit().unwrap();
    assert!(store.journal().tombstoned(uuid, generation));
    // The cleanup interrupted by that crash deletes the file, but this
    // session cannot prove the deletion durable (Windows has no directory
    // fsync), so the tombstone holds.
    assert_eq!(
        store.cleanup().unwrap(),
        Cleanup {
            orphan_carriers: 0,
            stale_operations: 0,
            resolved_tombstones: 0,
        }
    );
    assert!(store.journal().tombstoned(uuid, generation));
    assert!(!world.carrier_path(uuid, generation).exists());
    let Opened::Live(mut transfer) = Store::open(&world.root).unwrap() else {
        panic!("must reopen");
    };
    transfer.use_retention_snapshot(&store.retention_snapshot());
    transfer.finish_retention().unwrap();
    assert!(transfer.journal().tombstoned(uuid, generation));
    let Opened::Live(mut next_session) = Store::open(&world.root).unwrap() else {
        panic!("must reopen");
    };
    next_session.finish_retention().unwrap();
    assert!(!next_session.journal().tombstoned(uuid, generation));
}
