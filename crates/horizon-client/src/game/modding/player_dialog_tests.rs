//! `player_show_dialog`, `player_show_dialog_id` and `player_clear_dialog` send a player's client
//! what `/dialog show` and `/dialog clear` send it.
use std::sync::{Arc, atomic::AtomicUsize, mpsc};

use horizon_client::lan::{Out, PlayerSender};
use horizon_core::nbt::{Compound, Tag};
use horizon_protocol::{
    buf::Reader,
    packets::play::{Clientbound as C, DialogHolder},
};

use super::native_guest_turn_tests::with_guests;

fn sent(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("unexpected disconnect: {reason:?}"),
        })
        .map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).unwrap()
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_dialog_is_shown_by_value_or_by_registry_id_and_cleared() {
    with_guests(|game| {
        let (tx, rx) = mpsc::channel();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |g| {
                g.sender = PlayerSender::new(tx, Arc::new(AtomicUsize::new(0)))
            })
            .unwrap();
        let (shopper, other) = (game.player(12).unwrap(), game.player(13).unwrap());
        // `Dialog.DIRECT_CODEC` needs a `title`.
        let notice = Compound::from_iter([
            ("type".into(), Tag::String("minecraft:notice".into())),
            ("title".into(), Tag::String("Welcome".into())),
        ]);
        assert!(game.player_show_dialog(shopper, &notice));
        let id = game.ui.registries.ids("minecraft:dialog")[0].to_string();
        assert!(game.player_show_dialog_id(shopper, &id));
        assert!(game.player_clear_dialog(shopper));
        // Nothing for an id the registry hasn't, and nothing reaches the other player.
        assert!(!game.player_show_dialog_id(shopper, "mymod:nothing"));
        assert!(game.player_show_dialog(other, &notice));
        // A dialog a client cannot read is refused unsent: no title, an unknown type, and a
        // tag nested deeper than the wire carries.
        let untitled =
            Compound::from_iter([("type".into(), Tag::String("minecraft:notice".into()))]);
        let unknown = Compound::from_iter([
            ("type".into(), Tag::String("minecraft:nothing".into())),
            ("title".into(), Tag::String("x".into())),
        ]);
        let mut deep = Tag::String("x".into());
        for _ in 0..600 {
            deep = Tag::Compound(Compound::from_iter([("k".into(), deep)]));
        }
        let too_deep = Compound::from_iter([
            ("type".into(), Tag::String("minecraft:notice".into())),
            ("title".into(), Tag::String("x".into())),
            ("extra".into(), deep),
        ]);
        for bad in [&untitled, &unknown, &too_deep] {
            assert!(!game.player_show_dialog(shopper, bad));
        }
        // A player whose body another player's turn has displaced is still sent to.
        game.as_guest(13, |game| {
            assert!(game.player_clear_dialog(shopper));
        })
        .unwrap();
        let packets = sent(&rx);
        let [
            C::ShowDialog(by_value),
            C::ShowDialog(by_id),
            C::ClearDialog(_),
            C::ClearDialog(_),
        ] = packets.as_slice()
        else {
            panic!("expected two ShowDialog and two ClearDialog, got {packets:?}");
        };
        assert_eq!(by_value.dialog, DialogHolder::Direct(Tag::Compound(notice)));
        assert_eq!(by_id.dialog, DialogHolder::Registry(0));
    });
}
