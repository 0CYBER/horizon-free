use super::super::native_guest_turn_tests::with_guests;
use horizon_core::nbt::{Compound, Tag};
use horizon_mod_runtime::attachments::{
    AttachmentCodec, AttachmentError, AttachmentType, Holder, index::AttachmentIndex,
};
use mod_api::{AttachmentAccess, AttachmentHolder, ServerHost};
use std::sync::{Arc, Mutex};

const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "player_test",
    name: "value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |value| Ok(Compound::from([("n".into(), Tag::Int(*value))])),
        decode: |data| match data.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: true,
    sync: false,
};
const OTHER: AttachmentType<i32> = AttachmentType {
    name: "other",
    ..VALUE
};

fn rows(index: &Arc<Mutex<AttachmentIndex<AttachmentHolder>>>, name: &str) -> Vec<(u128, i32)> {
    let snapshot = index
        .lock()
        .unwrap()
        .snapshot(&("player_test".into(), name.into()));
    let mut values: Vec<_> = snapshot
        .iter()
        .map(|row| {
            let AttachmentHolder::Player(player) = row.holder else {
                panic!("not a player")
            };
            let ty = if name == "value" { &VALUE } else { &OTHER };
            (player.uuid(), *row.entry.read(ty).unwrap())
        })
        .collect();
    values.sort_unstable();
    values
}

#[test]
#[ignore = "daily"]
fn nested_player_turns_preserve_binding_and_rollback_whole_membership() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| g.uuid += 100);
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut bindings = Vec::new();
        for (conn, uuid, value) in [(12, 112, 7), (13, 113, 8)] {
            let player = game.player(uuid).unwrap();
            let map = game
                .attachment_map_quiet(Holder::Player(uuid), AttachmentAccess::Existing)
                .unwrap();
            VALUE.set(map, Holder::Level, value).unwrap();
            if uuid == 112 {
                OTHER.set(map, Holder::Level, 4).unwrap();
            }
            let storage = game
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .attachments_mut(conn)
                .unwrap();
            bindings.push(storage.bind_index(&index, AttachmentHolder::Player(player)));
        }
        let mut saved = game
            .attachment_slot(Holder::Player(112), AttachmentAccess::Existing)
            .unwrap()
            .clone();
        let saved_nbt = saved.to_nbt();
        let before = index
            .lock()
            .unwrap()
            .snapshot(&("player_test".into(), "value".into()));
        game.open_storage_transaction();
        game.as_guest(12, |game| {
            let host: &mut dyn ServerHost = game;
            VALUE.set(host, Holder::Player(112), 9).unwrap();
            OTHER.remove(host, Holder::Player(112)).unwrap();
            game.as_guest(13, |game| {
                assert!(
                    game.attachment_slot(Holder::Player(112), AttachmentAccess::Existing)
                        .is_err()
                );
                game.open_storage_transaction();
                let host: &mut dyn ServerHost = game;
                VALUE.set(host, Holder::Player(113), 88).unwrap();
                OTHER.set(host, Holder::Player(113), 6).unwrap();
                game.close_storage_transaction(false);
                assert_eq!(rows(&index, "value"), [(112, 9), (113, 8)]);
                assert!(rows(&index, "other").is_empty());
            })
            .unwrap();
            assert_eq!(rows(&index, "value"), [(112, 9), (113, 8)]);
        })
        .unwrap();
        game.close_storage_transaction(false);
        assert_eq!(rows(&index, "value"), [(112, 7), (113, 8)]);
        assert_eq!(rows(&index, "other"), [(112, 4)]);
        assert_eq!(VALUE.get_existing(&mut saved, Holder::Level), Ok(Some(7)));
        assert_eq!(saved.to_nbt(), saved_nbt);
        assert_eq!(before[0].entry.read(&VALUE), Ok(&7));
        for binding in bindings {
            assert!(
                binding.unregister(),
                "guest turns and rollback never rebind"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn retained_player_records_and_save_snapshots_do_not_keep_residency() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_all(|g| g.uuid += 100);
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        for (conn, uuid) in [(12, 112), (13, 113)] {
            let player = game.player(uuid).unwrap();
            let map = game
                .attachment_slot(Holder::Player(uuid), AttachmentAccess::Existing)
                .unwrap();
            VALUE.set(map, Holder::Level, uuid as i32).unwrap();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .attachments_mut(conn)
                .unwrap()
                .bind_index(&index, AttachmentHolder::Player(player));
        }
        let before = index
            .lock()
            .unwrap()
            .snapshot(&("player_test".into(), "value".into()));
        let mut saved = game
            .attachment_slot(Holder::Player(112), AttachmentAccess::Existing)
            .unwrap()
            .clone();
        let mut removed = game.lan.as_mut().unwrap().hub.guests.remove(0);
        assert_eq!(rows(&index, "value"), [(113, 113)]);
        VALUE
            .set(&mut removed.attachments, Holder::Level, 99)
            .unwrap();
        assert_eq!(rows(&index, "value"), [(113, 113)]);
        assert_eq!(VALUE.get_existing(&mut saved, Holder::Level), Ok(Some(112)));
        game.lan.as_mut().unwrap().hub.guests.clear();
        assert!(rows(&index, "value").is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&112));
        assert_eq!(before[1].entry.read(&VALUE), Ok(&113));
    });
}
