//! arismoko/horizon#792 stage 3: the general record's live load and save, through the game
//! fixture. The record codec's own tests (`general.rs`) cover the format;
//! these cover what the game does with it: publish a tree through the same
//! join boundary a fresh spawn takes, remember its identities, save one
//! record per root (never for `noSave`), and refuse a whole tree — codec
//! fault, structural fault or a UUID a live entity already owns — with zero
//! published rows.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use std::collections::HashMap;

/// Declares the fixture's two types: `mymod:robot`, which saves, and
/// `mymod:ghost`, which never does.
struct Declare;

#[mod_api::__subscriptions]
impl mod_api::Hooks for Declare {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        for (path, save) in [("robot", true), ("ghost", false)] {
            r.declare_general_entity(
                path,
                mod_api::GeneralEntityCategory::Misc,
                0.8,
                1.9,
                if save {
                    mod_api::GeneralSavePolicy::Save
                } else {
                    mod_api::GeneralSavePolicy::NoSave
                },
                true,
                8,
                3,
                1,
                &[],
            )
            .unwrap();
        }
    }
}

/// A fixture game whose frozen registry holds the two declared types. The
/// registration phase is scoped around the whole fixture: the game harness
/// re-runs the installed mods' `register`, and a general type is only
/// declared while that phase is current. The runtime initializes first: a
/// scope entered before the platform installs its instrumentation writes a
/// fallback nobody reads, and the declaration then refuses as frozen.
fn with_types(test: impl FnOnce(&mut Game<'_>)) {
    mod_platform::__private::initialize().expect("test registration requires the mod API runtime");
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mymod", Box::new(Declare))].into();
    mod_platform::testing::with_mods(mods, || with_game(test));
}

/// One record's save tag, the format `to_tag` writes.
fn record_tag(type_key: &str, uuid: [i32; 4], payload: &[u8], riders: Vec<Tag>) -> Tag {
    let mut tag = GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0, 90.0],
        uuid,
        envelope: GeneralTypeEnvelope {
            type_key: type_key.to_owned(),
            payload_version: 3,
            payload_bytes: payload.to_vec(),
        },
        passengers: Vec::new(),
    }
    .to_tag();
    if let (Tag::Compound(map), false) = (&mut tag, riders.is_empty()) {
        map.insert(format::PASSENGERS.into(), Tag::List(riders));
    }
    tag
}

/// The saved rows of one chunk, through the same walk the save uses.
fn saved_chunk(game: &mut Game<'_>, key: ChunkKey) -> Vec<Tag> {
    let mut out: HashMap<ChunkKey, Vec<(u32, horizon_client::join_seq::JoinSeq, Tag)>> =
        HashMap::new();
    let mut keys = HashSet::new();
    keys.insert(key);
    game.save_every_entity(&keys, &mut out);
    out.remove(&key)
        .unwrap_or_default()
        .into_iter()
        .map(|(_, _, tag)| tag)
        .collect()
}

/// A loaded tree publishes its rows through the same join boundary a fresh
/// spawn takes: the root and each rider are rows of one list, the riders
/// mounted, and every identity remembered. Saving the chunk writes one
/// record per root — the riders inside it — with the payload bytes it
/// loaded.
#[test]
#[ignore = "daily"]
fn general_tree_loads_joins_and_saves_one_record() {
    with_types(|game| {
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"rider", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"payload", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_eq!(
            game.target_position(crate::game::commands::Target::General(rows[0].id)),
            Some([0.5, 64.0, 0.5])
        );
        assert_eq!(rows[0].payload_bytes, b"payload");
        assert_eq!(rows[0].payload_version, 3);
        assert!(rows[0].join.is_joined() && rows[1].join.is_joined());
        assert_eq!(game.persist.uuids.get(&rows[0].id), Some(&[1, 2, 3, 4]));
        assert_eq!(game.persist.uuids.get(&rows[1].id), Some(&[9, 9, 9, 9]));
        // One record per root, the rider inside it, the payload as loaded.
        let saved = saved_chunk(game, (0, 0));
        assert_eq!(saved.len(), 1);
        let Tag::List(riders) = saved[0].get(format::PASSENGERS).unwrap() else {
            panic!("the rider saves inside its root");
        };
        assert_eq!(riders.len(), 1);
        assert_eq!(
            saved[0].get(format::PAYLOAD),
            Some(&Tag::ByteArray(b"payload".to_vec()))
        );
        assert_eq!(
            saved[0].get(format::UUID),
            Some(&Tag::IntArray(vec![1, 2, 3, 4]))
        );
    });
}

/// `noSave`: instances load and live like any other, but produce zero
/// persisted records.
#[test]
#[ignore = "daily"]
fn no_save_type_writes_zero_records() {
    with_types(|game| {
        game.load_listed(
            "mymod:ghost",
            None,
            None,
            record_tag("mymod:ghost", [5, 6, 7, 8], b"", Vec::new()),
        )
        .unwrap();
        assert_eq!(game.resident_entities.general.rows.len(), 1);
        assert!(saved_chunk(game, (0, 0)).is_empty());
    });
}

/// A UUID a live entity of the level already owns refuses the whole tree:
/// zero rows publish, rider included, and the refusing record names it.
#[test]
#[ignore = "daily"]
fn duplicate_uuid_with_a_live_entity_refuses_the_whole_tree() {
    with_types(|game| {
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        assert_eq!(game.resident_entities.general.rows.len(), 1);
        // A different root whose rider carries the live row's UUID.
        let rider = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        let root = record_tag("mymod:robot", [8, 8, 8, 8], b"", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        assert_eq!(
            game.resident_entities.general.rows.len(),
            1,
            "the whole tree refuses, not only the duplicate rider"
        );
        assert_eq!(game.resident_entities.general.rows[0].uuid, [1, 2, 3, 4]);
    });
}

/// A codec refusal (an envelope field of the wrong shape) publishes zero
/// rows and leaves nothing behind: the next valid record loads cleanly.
#[test]
#[ignore = "daily"]
fn codec_refusal_publishes_nothing_and_poisons_nothing() {
    with_types(|game| {
        let mut broken = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        if let Tag::Compound(map) = &mut broken {
            map.insert(format::PAYLOAD.into(), Tag::Int(1));
        }
        game.load_listed("mymod:robot", None, None, broken).unwrap();
        assert!(game.resident_entities.general.rows.is_empty());
        // The next valid record through the same entry point stands.
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        assert_eq!(game.resident_entities.general.rows.len(), 1);
    });
}

/// A structural refusal (a NaN coordinate) publishes zero rows of the tree.
#[test]
#[ignore = "daily"]
fn structural_refusal_publishes_nothing() {
    with_types(|game| {
        let mut nan = record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new());
        if let Tag::Compound(map) = &mut nan {
            map.insert(
                format::POSITION.into(),
                Tag::List(vec![
                    Tag::Double(f64::NAN.to_bits()),
                    Tag::Double(64.0f64.to_bits()),
                    Tag::Double(0.5f64.to_bits()),
                ]),
            );
        }
        game.load_listed("mymod:robot", None, None, nan).unwrap();
        assert!(game.resident_entities.general.rows.is_empty());
    });
}

/// A structurally sound tree with a type this load cannot resolve hands the
/// compound back (`Err`), keeping it for the missing mod's path instead of
/// publishing or dropping it.
#[test]
#[ignore = "daily"]
fn unknown_rider_type_hands_the_tree_back() {
    with_types(|game| {
        let rider = record_tag("gone:drone", [9, 9, 9, 9], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider]);
        let handed = root.clone();
        assert_eq!(
            game.load_listed("mymod:robot", None, None, root),
            Err(handed.clone())
        );
        assert!(game.general_quarantine_admits(&handed, "mymod:robot", Vec3::new(0.0, 0.0, 0.0)));
        assert!(game.resident_entities.general.rows.is_empty());
    });
}

/// The unload path: `unload_entities` (game.rs) runs `entity_chunks` with
/// `remove`, whose sweep (persist/unload_sweep.rs) first saves every row
/// through `save_lists!` and then removes them and forgets their ids. A
/// root and its rider leave together as one record; the resident table no
/// longer knows the old id (what an `EntityRef` resolves through), and the
/// record that comes back loads as exactly one tree of the same UUIDs,
/// under fresh ids from the never-decreasing counter.
#[test]
#[ignore = "daily"]
fn general_row_unloads_with_its_chunk_and_reloads_once() {
    use crate::game::entity_residency::SlotAccess;
    use mod_api::DimensionKey;
    use std::num::NonZeroU32;
    with_types(|game| {
        game.reset_entity_residency();
        let grandchild = record_tag("mymod:robot", [8, 8, 8, 8], b"grandchild", Vec::new());
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"rider", vec![grandchild]);
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"root", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (old_root, old_rider) = (rows[0].id, rows[1].id);
        let id = |value: u32| NonZeroU32::new(value).unwrap();
        for known in [old_root, old_rider] {
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(known), |_| true),
                SlotAccess::Done(true)
            ));
        }
        // The chunk unloads: one record leaves (the rider inside its root),
        // and both rows are gone.
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert_eq!(chunks.len(), 1);
        let (_, mut records) = chunks.remove(0);
        assert_eq!(records.len(), 1);
        assert!(game.resident_entities.general.rows.is_empty());
        // The old ids no longer resolve: a stale handle cannot reach either
        // row again, and the counter never hands their ids back out.
        for gone in [old_root, old_rider] {
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(gone), |_| panic!(
                    "a stale id resolved"
                )),
                SlotAccess::Missing
            ));
        }
        // The same record loads back as one tree of the same UUIDs.
        game.load_listed("mymod:robot", None, None, records.remove(0))
            .unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].uuid, [8, 8, 8, 8]);
        assert_eq!(rows[2].mount, Some(rows[1].id));
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].uuid, [9, 9, 9, 9]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_ne!(rows[0].id, old_root);
        assert_ne!(rows[1].id, old_rider);
    });
}

/// The missing-mod quarantine: a record of a namespace no loaded mod owns
/// keeps the existing unknown-kind path — held as loaded (`persist.raw`),
/// never ticked, written back from that store — byte for byte.
#[test]
#[ignore = "daily"]
fn quarantine_preserves_a_missing_mods_record_byte_for_byte() {
    with_types(|game| {
        game.others.clear();
        let mut tag = record_tag("gone:robot", [7, 7, 7, 7], b"opaque", Vec::new());
        if let Tag::Compound(fields) = &mut tag {
            // Vanilla fallback loading strips these fields; an opaque mod record must not.
            fields.insert("Team".into(), Tag::String("missing-mod-team".into()));
            fields.insert("leash".into(), Tag::Compound(Default::default()));
        }
        let loaded = tag.clone();
        game.spawn_loaded_entity((0, 0), tag, None);
        assert_eq!(game.others.len(), 1);
        assert_eq!(game.others[0].kind, "gone:robot");
        // What leaves the chunk is the very compound that arrived.
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        let (_, mut records) = chunks.remove(0);
        assert_eq!(records.len(), 1);
        assert_eq!(records.remove(0), loaded);
    });
}

/// A loaded mod's namespace with an unknown type skips with a warning:
/// nothing is published, quarantined or kept for it.
#[test]
#[ignore = "daily"]
fn loaded_mods_unknown_type_skips() {
    // The package inventory is what owns a namespace: "mymod" is a loaded
    // package here, while no type of it was ever declared.
    mod_platform::__private::with_test_packages(&["mymod"], || {
        // No fixture declaration: the type is unknown precisely because no
        // loaded mod declared it.
        with_game(|game| {
            game.others.clear();
            game.spawn_loaded_entity(
                (0, 0),
                record_tag("mymod:mystery", [7, 7, 7, 7], b"", Vec::new()),
                None,
            );
            assert!(game.others.is_empty());
            let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
            assert!(chunks.iter().all(|(_, records)| records.is_empty()));
        });
    });
}

/// An unknown `minecraft:` kind keeps its existing path unchanged —
/// preserved whatever its size, outside the quarantine bounds.
#[test]
#[ignore = "daily"]
fn unknown_minecraft_kind_keeps_its_existing_path() {
    with_types(|game| {
        game.others.clear();
        let mut oversized = record_tag("minecraft:mystery", [7, 7, 7, 7], b"", Vec::new());
        if let Tag::Compound(map) = &mut oversized {
            map.insert(
                "Extra".into(),
                Tag::ByteArray(vec![0; limits::QUARANTINE_RECORD_LIMIT as usize + 1]),
            );
        }
        game.spawn_loaded_entity((0, 0), oversized, None);
        assert_eq!(game.others.len(), 1);
        assert_eq!(game.others[0].kind, "minecraft:mystery");
    });
}

/// The quarantine record bound: a missing-mod record one byte over
/// [`limits::QUARANTINE_RECORD_LIMIT`] skips; one inside it is kept.
#[test]
#[ignore = "daily"]
fn quarantine_record_limit_skips_one_byte_over() {
    with_types(|game| {
        game.others.clear();
        let sized = |extra: u64| {
            let mut tag = record_tag("gone:robot", [7, 7, 7, 7], b"", Vec::new());
            if let Tag::Compound(map) = &mut tag {
                map.insert("Extra".into(), Tag::ByteArray(vec![0; extra as usize]));
            }
            tag
        };
        // Size the record to exactly the bound, then one byte past it.
        let base = nbt_size(&sized(0));
        let fill = limits::QUARANTINE_RECORD_LIMIT - base;
        assert!(fill > 1, "the base record must sit below the bound");
        game.spawn_loaded_entity((0, 0), sized(fill), None);
        assert_eq!(game.others.len(), 1, "a record at the bound is kept");
        game.spawn_loaded_entity((0, 0), sized(fill + 1), None);
        assert_eq!(game.others.len(), 1, "one byte over skips");
    });
}

/// The quarantine chunk bound: the
/// [`limits::QUARANTINE_CHUNK_LIMIT`]'th record of one chunk is kept, the
/// next skips.
#[test]
#[ignore = "daily"]
fn quarantine_chunk_limit_skips_past_256() {
    with_types(|game| {
        game.others.clear();
        let at = |slot: usize| [slot as i32, 0, 0, 0];
        let tag = |slot: usize| record_tag("gone:robot", at(slot), b"", Vec::new());
        // Fill the chunk to one below the bound through the gate itself.
        for slot in 0..limits::QUARANTINE_CHUNK_LIMIT - 1 {
            game.spawn_loaded_entity((0, 0), tag(slot), None);
        }
        assert_eq!(game.others.len(), limits::QUARANTINE_CHUNK_LIMIT - 1);
        game.spawn_loaded_entity((0, 0), tag(250), None);
        assert_eq!(
            game.others.len(),
            limits::QUARANTINE_CHUNK_LIMIT,
            "the bound's own record is kept"
        );
        game.spawn_loaded_entity((0, 0), tag(251), None);
        assert_eq!(
            game.others.len(),
            limits::QUARANTINE_CHUNK_LIMIT,
            "the next record skips"
        );
        // A record of another chunk still quarantines.
        let mut elsewhere = tag(252);
        if let Tag::Compound(map) = &mut elsewhere {
            map.insert(
                format::POSITION.into(),
                Tag::List(
                    [1024.5f64, 64.0, 0.5]
                        .map(|v| Tag::Double(v.to_bits()))
                        .to_vec(),
                ),
            );
        }
        game.spawn_loaded_entity((64, 0), elsewhere, None);
        assert_eq!(game.others.len(), limits::QUARANTINE_CHUNK_LIMIT + 1);
    });
}

/// The transfer load path (`load_listed_transferred`) accepts a general
/// tree: what arrives with a transfer token is installed as the same
/// logical entity — its runtime state (attachments map) carried over to
/// the new id — instead of the old "refusing unsupported transferred
/// entity kind".
#[test]
#[ignore = "daily"]
fn transferred_general_tree_loads_as_the_same_logical_entity() {
    with_types(|game| {
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        let row = game.resident_entities.general.rows.remove(0);
        let state = game.take_transferred_entity_state(row.id);
        game.detach_entity(row.id);
        // The transferred arrival: a fresh id, the same UUID, its runtime
        // state (the attachments map, even empty) adopted, not refused.
        game.spawn_loaded_entity(
            (0, 0),
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
            Some(state),
        );
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_ne!(rows[0].id, row.id);
        assert!(game.entity_attachments.contains_key(&rows[0].id));
    });
}

/// A tree travels between dimensions with its riders: the source ends with
/// zero rows and stale ids, the destination holds the same tree — same
/// UUIDs, same mount links — with fresh join identities, the root placed.
#[test]
#[ignore = "daily"]
fn general_tree_travels_to_another_dimension_with_its_riders() {
    use crate::game::entity_residency::SlotAccess;
    use mod_api::DimensionKey;
    use std::num::NonZeroU32;
    with_types(|game| {
        game.reset_entity_residency();
        let grandchild = record_tag("mymod:robot", [8, 8, 8, 8], b"", Vec::new());
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"", vec![grandchild]);
        let sibling = record_tag("mymod:robot", [7, 7, 7, 7], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider, sibling]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        let (old_root, old_rider, old_join) = (rows[0].id, rows[1].id, rows[0].join);
        assert!(
            game.teleport_general_tree_across(
                old_root,
                crate::game::Dim::Nether,
                [8.5, 100.0, 8.5]
            )
            .unwrap()
        );
        // The source keeps nothing; its stale ids no longer resolve.
        assert!(game.resident_entities.general.rows.is_empty());
        let id = |value: u32| NonZeroU32::new(value).unwrap();
        for gone in [old_root, old_rider] {
            assert!(matches!(
                game.with_resident_slot(&DimensionKey::OVERWORLD, id(gone), |_| panic!(
                    "a stale id resolved"
                )),
                SlotAccess::Missing
            ));
        }
        // The destination holds the same tree, freshly admitted.
        game.enter_level(crate::game::Dim::Nether).unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[2].uuid, [8, 8, 8, 8]);
        assert_eq!(rows[2].mount, Some(rows[1].id));
        assert_eq!(rows[3].uuid, [7, 7, 7, 7]);
        assert_eq!(rows[3].mount, Some(rows[0].id));
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].uuid, [9, 9, 9, 9]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_eq!(rows[0].position, Vec3::new(8.5, 100.0, 8.5));
        assert!(rows[0].join.is_joined() && rows[0].join != old_join);
        assert!(matches!(
            game.with_resident_slot(&DimensionKey::NETHER, id(rows[0].id), |_| true),
            SlotAccess::Done(true)
        ));
    });
}

/// A refused transfer returns the whole tree: a destination row of one of
/// the tree's UUIDs refuses before anything moves, and the source keeps
/// every row at its place with its join identity.
#[test]
#[ignore = "daily"]
fn refused_dimension_transfer_returns_the_whole_tree() {
    with_types(|game| {
        // Seed the destination with a row of the travelling UUID.
        game.enter_level(crate::game::Dim::Nether).unwrap();
        game.load_listed(
            "mymod:robot",
            None,
            None,
            record_tag("mymod:robot", [1, 2, 3, 4], b"", Vec::new()),
        )
        .unwrap();
        game.enter_level(crate::game::Dim::Overworld).unwrap();
        let grandchild = record_tag("mymod:robot", [8, 8, 8, 8], b"", Vec::new());
        let rider = record_tag("mymod:robot", [9, 9, 9, 9], b"", vec![grandchild]);
        let sibling = record_tag("mymod:robot", [7, 7, 7, 7], b"", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"", vec![rider, sibling]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let before: Vec<_> = game
            .resident_entities
            .general
            .rows
            .iter()
            .map(|row| (row.id, row.uuid, row.join, row.position))
            .collect();
        let root_id = game.resident_entities.general.rows[0].id;
        assert!(
            game.teleport_general_tree_across(root_id, crate::game::Dim::Nether, [8.5, 100.0, 8.5])
                .is_err()
        );
        let after: Vec<_> = game
            .resident_entities
            .general
            .rows
            .iter()
            .map(|row| (row.id, row.uuid, row.join, row.position))
            .collect();
        assert_eq!(before, after, "the tree returns exactly as it stood");
    });
}

#[test]
#[ignore = "daily"]
fn no_save_rider_is_omitted_from_saved_tree() {
    with_types(|game| {
        let rider = record_tag("mymod:ghost", [9, 9, 9, 9], b"rider", Vec::new());
        let root = record_tag("mymod:robot", [1, 2, 3, 4], b"payload", vec![rider]);
        game.load_listed("mymod:robot", None, None, root).unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(rows[1].mount, Some(rows[0].id));
        assert_eq!(rows[0].payload_bytes, b"payload");
        assert_eq!(rows[0].payload_version, 3);
        assert!(rows[0].join.is_joined() && rows[1].join.is_joined());
        assert_eq!(game.persist.uuids.get(&rows[0].id), Some(&[1, 2, 3, 4]));
        assert_eq!(game.persist.uuids.get(&rows[1].id), Some(&[9, 9, 9, 9]));
        // One record per root, the rider inside it, the payload as loaded.
        let saved = saved_chunk(game, (0, 0));
        assert_eq!(saved.len(), 1);
        assert!(saved[0].get(format::PASSENGERS).is_none());
        assert_eq!(
            saved[0].get(format::PAYLOAD),
            Some(&Tag::ByteArray(b"payload".to_vec()))
        );
        assert_eq!(
            saved[0].get(format::UUID),
            Some(&Tag::IntArray(vec![1, 2, 3, 4]))
        );
    });
}
