//! `player.push_resource_pack` and `player.pop_resource_pack` against real LAN guests, with a
//! literal test for each refusal.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use horizon_protocol::packets::play::Clientbound as C;
use mod_api::{
    EntityRef, PlayerPackRefusal as Refusal, PlayerRef, ResourcePackOffer, Text, WorldEpoch,
};

/// Routes a guest's packets to a channel the test reads.
fn listen(
    game: &mut super::Game<'_>,
    conn: u32,
) -> std::sync::mpsc::Receiver<horizon_client::lan::Out> {
    let (tx, rx) = std::sync::mpsc::channel();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(conn, |g| {
            g.sender = horizon_client::lan::PlayerSender::new(tx, Default::default());
        });
    rx
}

fn packets(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> Vec<C> {
    let mut out = Vec::new();
    while let Ok(horizon_client::lan::Out::Packet(body)) = rx.try_recv() {
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        out.extend(C::decode(id, r.rest()));
    }
    out
}

fn offer() -> ResourcePackOffer {
    let mut pack = ResourcePackOffer::new(7, "http://example.test/pack.zip");
    pack.sha1 = "0123456789abcdef0123456789abcdef01234567".into();
    pack.required = true;
    pack.prompt = Some(Text::literal("Use it"));
    pack
}

/// A pack goes to that player's client alone, whole, and a pop names the pack or all of them.
#[test]
#[ignore = "daily"]
fn a_pack_is_pushed_to_one_player_and_popped() {
    with_guests(|game| {
        let (a, b) = (game.player(12).unwrap(), game.player(13).unwrap());
        let (rx_a, rx_b) = (listen(game, 12), listen(game, 13));
        assert_eq!(b.push_resource_pack(game, offer()), Ok(()));
        assert_eq!(b.pop_resource_pack(game, Some(7)), Ok(()));
        assert_eq!(b.pop_resource_pack(game, None), Ok(()));
        assert!(packets(&rx_a).is_empty(), "the other player hears nothing");
        let sent = packets(&rx_b);
        let [
            C::ResourcePackPush(push),
            C::ResourcePackPop(one),
            C::ResourcePackPop(all),
        ] = &sent[..]
        else {
            panic!("{sent:?}");
        };
        assert_eq!(
            (
                push.id,
                push.url.as_str(),
                push.hash.as_str(),
                push.required
            ),
            (
                7,
                "http://example.test/pack.zip",
                "0123456789abcdef0123456789abcdef01234567",
                true
            )
        );
        assert!(push.prompt.is_some());
        assert_eq!((one.id, all.id), (Some(7), None));
        let _ = a;
    });
}

/// Each refusal is its own name and sends nothing; the limits themselves are taken.
#[test]
#[ignore = "daily"]
fn each_pack_refusal_has_its_own_name() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let rx = listen(game, 13);
        for url in ["u".repeat(32768), "🙂".repeat(16384)] {
            let mut long_url = offer();
            long_url.url = url;
            assert_eq!(
                b.push_resource_pack(game, long_url),
                Err(Refusal::UrlTooLong)
            );
            assert!(matches!(
                rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
        }
        for sha1 in [
            "a".repeat(41),
            "🙂".repeat(21),
            format!("{}a", "🙂".repeat(20)),
        ] {
            let mut long_hash = offer();
            long_hash.sha1 = sha1;
            assert_eq!(
                b.push_resource_pack(game, long_hash),
                Err(Refusal::HashTooLong)
            );
            assert!(matches!(
                rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
        }
        let entity = b.entity();
        let stale = mod_api::__private::game::player_ref(
            EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
            b.uuid(),
        );
        assert_eq!(stale.push_resource_pack(game, offer()), Err(Refusal::Stale));
        assert_eq!(stale.pop_resource_pack(game, None), Err(Refusal::Stale));
        game.next_entity = 100;
        let level = game.level(game.dim.id()).unwrap();
        let machine = game.fake_player(level, 1234, "machine").unwrap();
        assert_eq!(
            machine.push_resource_pack(game, offer()),
            Err(Refusal::NoConnection)
        );
        assert!(packets(&rx).is_empty());
        for (url, sha1) in [
            ("u".repeat(32767), "a".repeat(40)),
            (format!("{}u", "🙂".repeat(16383)), "🙂".repeat(20)),
        ] {
            let mut edge = offer();
            edge.url = url.clone();
            edge.sha1 = sha1.clone();
            assert_eq!(b.push_resource_pack(game, edge), Ok(()));
            let sent = packets(&rx);
            let [C::ResourcePackPush(push)] = &sent[..] else {
                panic!("{sent:?}");
            };
            assert_eq!((&push.url, &push.hash), (&url, &sha1));
        }
    });
}

/// On a client both answer `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_push_or_pop_a_pack() {
    with_client_game(|game| {
        let ghost: PlayerRef =
            mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        assert_eq!(
            ghost.push_resource_pack(game, offer()),
            Err(Refusal::Client)
        );
        assert_eq!(ghost.pop_resource_pack(game, None), Err(Refusal::Client));
    });
}

/// `resource_pack_status_v2` hears every answer with the reporting player's context, a dead
/// player's too; `resource_pack_status` keeps its three arguments and hears the living only.
#[test]
#[ignore = "daily"]
fn dead_guests_resource_pack_answers_reach_only_the_v2_hook() {
    use horizon_protocol::packets::{configuration::ResourcePackResponse, play::Serverbound as S};
    use mod_api::{
        Hooks, ModEntry, ResourcePackStatusEvent, ResourcePackStatusV2Event, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Debug, PartialEq)]
    struct Answer {
        version: u8,
        player: u128,
        name: String,
        status: i32,
        pack: Option<u128>,
        acting_player: Option<u128>,
    }
    struct Observer(Arc<Mutex<Vec<Answer>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn resource_pack_status(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ResourcePackStatusEvent<'_>,
        ) {
            self.0.lock().unwrap().push(Answer {
                version: 1,
                player: event.player,
                name: event.name.to_owned(),
                status: event.status,
                pack: None,
                acting_player: host.players().acting_player().map(|player| player.uuid()),
            });
        }
        fn resource_pack_status_v2(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ResourcePackStatusV2Event<'_>,
        ) {
            self.0.lock().unwrap().push(Answer {
                version: 2,
                player: event.player,
                name: event.name.to_owned(),
                status: event.status,
                pack: Some(event.pack),
                acting_player: host.players().acting_player().map(|player| player.uuid()),
            });
        }
    }
    let answers = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "observer",
            Box::new(Observer(answers.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(13, |guest| guest.dead = true)
                    .unwrap();
                let outer = game.lan_acting;
                // (connection, pack id, Action number); guest 13 is dead.
                let cases = [(12, 7, 3), (13, 8, 0), (13, 9, 1), (13, 10, 7)];
                for (conn, id, action) in cases {
                    game.lan_guest_packet(
                        conn,
                        S::ResourcePack(ResourcePackResponse { id, action }),
                    );
                    assert_eq!(game.lan_acting, outer);
                }
                let answer = |version, conn: u32, pack, status| Answer {
                    version,
                    player: u128::from(conn),
                    name: format!("guest{conn}"),
                    status,
                    pack: (version == 2).then_some(pack),
                    acting_player: Some(u128::from(conn)),
                };
                // v2 first, then v1 for the living guest 12; the dead guest 13 only reaches v2.
                let expected = vec![
                    answer(2, 12, 7, 3),
                    answer(1, 12, 7, 3),
                    answer(2, 13, 8, 0),
                    answer(2, 13, 9, 1),
                    answer(2, 13, 10, 7),
                ];
                assert_eq!(*answers.lock().unwrap(), expected);
            });
        },
    );
}
