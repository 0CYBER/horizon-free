use super::*;
use crate::game::native_guest_turn_tests::with_guests;
use horizon_core::nbt::{Compound, Tag};
use horizon_mod_runtime::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
use mod_api::{AttachmentAccess, ServerHost};

const VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "activation",
    name: "value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok(Compound::from([("n".into(), Tag::Int(*n))])),
        decode: |data| match data.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: true,
    sync: false,
};

fn snapshot(
    game: &Game<'_>,
    dim: Dim,
) -> Arc<horizon_mod_runtime::attachments::index::AttachmentSnapshot<AttachmentHolder>> {
    game.attachment_indexes[&dim]
        .lock()
        .unwrap()
        .snapshot(&("activation".into(), "value".into()))
}

#[test]
#[ignore = "daily"]
fn saved_placement_respawn_removal_and_reconnect_publish_player_rows() {
    with_guests(|game| {
        game.world_root = game.save.root.clone();
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        VALUE
            .set(guests.attachments_mut(12).unwrap(), Holder::Level, 31)
            .unwrap();
        horizon_client::lan::playerdata::save(&guests[0], &game.world_root);
        guests.update_all(|g| g.placed = false);
        game.lan_guest_joined(12, &[], [0.5, 64.0, 0.5]);
        game.lan_guest_joined(13, &[], [0.5, 64.0, 0.5]);
        let before = snapshot(game, Dim::Overworld);
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].entry.read(&VALUE), Ok(&31));
        assert!(
            !game.lan.as_ref().unwrap().hub.guests[1]
                .attachments
                .has_index_binding()
        );
        let first = game.lan.as_ref().unwrap().hub.guests[0]
            .attachments
            .registration()
            .unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.dead = true);
        game.guest_respawn(12);
        let current = game.lan.as_ref().unwrap().hub.guests[0]
            .attachments
            .registration()
            .unwrap();
        assert!(!first.same_registration(&current));
        assert!(!first.unregister());
        assert_eq!(
            snapshot(game, Dim::Overworld)[0].entry.read(&VALUE),
            Ok(&31)
        );
        let mut removed = game.lan.as_mut().unwrap().hub.guests.remove(0);
        assert!(snapshot(game, Dim::Overworld).is_empty());
        // Reuse the UUID, with a new connection and runtime identity, while the old record survives.
        let mut replacement = game.lan.as_mut().unwrap().hub.guests.remove(0);
        replacement.uuid = removed.uuid;
        replacement.conn = 99;
        replacement.entity_id = 199;
        replacement.placed = false;
        game.lan.as_mut().unwrap().hub.guests.push(replacement);
        game.lan_guest_joined(99, &[], [0.5, 64.0, 0.5]);
        removed.attachments.retire();
        assert!(!current.unregister());
        let fresh = snapshot(game, Dim::Overworld);
        assert_eq!(fresh.len(), 1);
        assert_ne!(fresh[0].holder, before[0].holder);
        let AttachmentHolder::Player(player) = fresh[0].holder else {
            panic!("not a player")
        };
        assert_eq!(player, game.player(12).unwrap());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&31));
    });
}

#[test]
#[ignore = "daily"]
fn first_stores_bind_once_and_guest_turns_preserve_the_registration() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let host: &mut dyn ServerHost = game;
        VALUE.set(host, Holder::Player(12), 7).unwrap();
        let first = game.lan.as_ref().unwrap().hub.guests[0]
            .attachments
            .registration()
            .unwrap();
        let before = snapshot(game, Dim::Overworld);
        game.as_guest(12, |game| {
            let host: &mut dyn ServerHost = game;
            VALUE.set(host, Holder::Player(12), 8).unwrap();
            assert!(
                game.player_state
                    .attachments
                    .registration()
                    .unwrap()
                    .same_registration(&first)
            );
        })
        .unwrap();
        game.as_guest(13, |game| {
            let host: &mut dyn ServerHost = game;
            VALUE.set(host, Holder::Player(13), 9).unwrap();
        })
        .unwrap();
        let current = game.lan.as_ref().unwrap().hub.guests[0]
            .attachments
            .registration()
            .unwrap();
        assert!(first.same_registration(&current));
        assert_eq!(snapshot(game, Dim::Overworld).len(), 2);
        assert_eq!(before[0].entry.read(&VALUE), Ok(&7));
    });
}

#[test]
#[ignore = "daily"]
fn travel_moves_rows_before_guest_callbacks_and_keeps_captured_values() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let host: &mut dyn ServerHost = game;
        VALUE.set(host, Holder::Player(12), 17).unwrap();
        let before = snapshot(game, Dim::Overworld);
        let first = game.lan.as_ref().unwrap().hub.guests[0]
            .attachments
            .registration()
            .unwrap();
        game.start_server_trip(12, Dim::Nether, None, false);
        assert!(snapshot(game, Dim::Overworld).is_empty());
        let nether = snapshot(game, Dim::Nether);
        assert_eq!(nether.len(), 1);
        assert_eq!(nether[0].entry.read(&VALUE), Ok(&17));
        assert_eq!(before[0].entry.read(&VALUE), Ok(&17));
        assert!(!first.unregister());
        let AttachmentHolder::Player(player) = nether[0].holder else {
            panic!("not a player")
        };
        assert_eq!(player, game.player(12).unwrap());
        game.enter_level(Dim::Overworld).unwrap();
        let host: &mut dyn ServerHost = game;
        VALUE.set(host, Holder::Player(12), 18).unwrap();
        assert!(snapshot(game, Dim::Overworld).is_empty());
        assert_eq!(snapshot(game, Dim::Nether)[0].entry.read(&VALUE), Ok(&18));
        assert_eq!(nether[0].entry.read(&VALUE), Ok(&17));
        game.lan.as_mut().unwrap().hub.guests.clear();
        assert!(snapshot(game, Dim::Nether).is_empty());
        assert!(
            game.attachment_slot(Holder::Player(12), AttachmentAccess::Store)
                .is_err()
        );
    });
}

#[test]
#[ignore = "daily"]
fn queued_level_teleport_moves_the_player_row_and_later_writes_stay_in_the_destination() {
    with_guests(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let host: &mut dyn ServerHost = game;
        VALUE.set(host, Holder::Player(12), 21).unwrap();
        let before = snapshot(game, Dim::Overworld);
        game.level_teleports
            .push(crate::game::server_travel::LevelTeleport {
                conn: 12,
                dim: Dim::Nether,
                at: [8.5, 70.0, 8.5],
                rotation: None,
                vehicle: None,
            });
        game.run_level_teleports();
        assert!(snapshot(game, Dim::Overworld).is_empty());
        let nether = snapshot(game, Dim::Nether);
        assert_eq!(nether.len(), 1);
        assert_eq!(nether[0].entry.read(&VALUE), Ok(&21));
        let AttachmentHolder::Player(player) = nether[0].holder else {
            panic!("not a player")
        };
        assert_eq!(player, game.player(12).unwrap());
        let host: &mut dyn ServerHost = game;
        VALUE.set(host, Holder::Player(12), 22).unwrap();
        assert!(snapshot(game, Dim::Overworld).is_empty());
        assert_eq!(snapshot(game, Dim::Nether)[0].entry.read(&VALUE), Ok(&22));
        assert_eq!(before[0].entry.read(&VALUE), Ok(&21));
    });
}
