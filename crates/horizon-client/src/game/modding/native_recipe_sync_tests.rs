use crate::game::native_guest_turn_tests::with_guests;
use horizon_client::lan::recipes::ContentPart;
use std::sync::{Arc, Mutex};

struct MachineTypes;
#[mod_api::__subscriptions]
impl mod_api::Hooks for MachineTypes {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_recipe_type("recipe_sync_data:grinding");
        r.register_raw_recipe_serializer("recipe_sync_data:grinding", "recipe_sync_data:grinding");
    }
}

struct Selection(Arc<Mutex<Vec<u128>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Selection {
    fn datapack_sync(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DatapackSyncEvent<'_>,
    ) {
        let uuid = event.player.uuid();
        self.0.lock().unwrap().push(uuid);
        if uuid == 12 {
            event.recipe_types.push("recipe_sync_data:grinding".into());
        }
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn datapack_sync_selects_once_per_join_and_reload_for_each_recipient() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("selection", Box::new(Selection(calls.clone()))),
            mod_api::ModEntry::new("recipe_sync_data", Box::new(MachineTypes)),
        ]
        .into(),
        || {
            with_guests(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let mut receivers = Vec::new();
                for conn in [12, 13] {
                    let (tx, rx) = std::sync::mpsc::channel();
                    game.lan.as_mut().unwrap().hub.update_guest(conn, |g| {
                        g.sender = horizon_client::lan::PlayerSender::new(tx, Arc::default());
                    });
                    receivers.push(rx);
                }
                let host = game.lan.as_mut().unwrap();
                host.hub.guests.update_all(|g| g.brand = "horizon".into());
                game.prepare_recipe_join(12);
                game.prepare_recipe_join(13);
                assert_eq!(*calls.lock().unwrap(), [12, 13]);
                let host = game.lan.as_ref().unwrap();
                let a = &host.hub.guest(12).unwrap().recipe_sync;
                assert_eq!(a.types, ["recipe_sync_data:grinding"]);
                let last = ContentPart::decode(a.payloads.last().unwrap()).unwrap();
                assert_eq!(last.last, Some(vec!["recipe_sync_data:grinding".into()]));
                assert!(host.hub.guest(13).unwrap().recipe_sync.payloads.is_empty());
                for conn in [12, 13] {
                    horizon_client::lan::recipes::send_content(
                        game.lan.as_ref().unwrap().hub.guest(conn).unwrap(),
                    );
                }
                assert_eq!(
                    received(&receivers[0]),
                    [vec!["recipe_sync_data:grinding".to_owned()]]
                );
                assert!(received(&receivers[1]).is_empty());
                game.sync_recipe_content();
                assert_eq!(*calls.lock().unwrap(), [12, 13, 12, 13]);
                for conn in [12, 13] {
                    horizon_client::lan::recipes::send_content(
                        game.lan.as_ref().unwrap().hub.guest(conn).unwrap(),
                    );
                }
                assert_eq!(
                    received(&receivers[0]),
                    [vec!["recipe_sync_data:grinding".to_owned()]]
                );
                assert!(received(&receivers[1]).is_empty());
                let host = game.lan.as_ref().unwrap();
                assert_eq!(
                    host.hub.guest(12).unwrap().recipe_sync.types,
                    ["recipe_sync_data:grinding"]
                );
                assert!(host.hub.guest(13).unwrap().recipe_sync.payloads.is_empty());
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |g| g.brand = "vanilla".into());
                game.prepare_recipe_join(12);
                assert!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .recipe_sync
                        .payloads
                        .is_empty()
                );
                game.sync_recipe_content();
                assert_eq!(*calls.lock().unwrap(), [12, 13, 12, 13, 12, 12, 13]);
                assert!(
                    game.lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guest(12)
                        .unwrap()
                        .recipe_sync
                        .payloads
                        .is_empty()
                );
            });
        },
    );
}

fn received(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> Vec<Vec<String>> {
    let mut types = Vec::new();
    for packet in captured_packets(rx) {
        if let horizon_protocol::packets::play::Clientbound::CustomPayload(payload) = packet {
            assert_eq!(payload.channel, "horizon:recipe_content");
            if let Some(last) = ContentPart::decode(&payload.data).unwrap().last {
                types.push(last);
            }
        }
    }
    types
}

struct MarkedSelection {
    senders: Arc<Mutex<std::collections::HashMap<u128, horizon_client::lan::PlayerSender>>>,
    decided: Arc<Mutex<Vec<u128>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for MarkedSelection {
    fn datapack_sync(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::DatapackSyncEvent<'_>,
    ) {
        let uuid = event.player.uuid();
        self.decided.lock().unwrap().push(uuid);
        if uuid == 12 {
            event.recipe_types.push("recipe_sync_data:grinding".into());
        }
        // A marker in the recipient's actual packet stream records the decision.
        let marker = horizon_protocol::packets::play::Clientbound::CustomPayload(
            horizon_protocol::packets::configuration::CustomPayload {
                channel: "testmod:selection_decided".into(),
                data: vec![],
            },
        );
        self.senders.lock().unwrap()[&uuid].send(marker.try_encode().unwrap());
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn datapack_sync_decision_precedes_join_recipe_packets() {
    let senders = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let decided = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new(
                "order",
                Box::new(MarkedSelection {
                    senders: senders.clone(),
                    decided: decided.clone(),
                }),
            ),
            mod_api::ModEntry::new("recipe_sync_data", Box::new(MachineTypes)),
        ]
        .into(),
        || {
            with_guests(|game| {
                let scratch = tempfile::tempdir().unwrap();
                game.world_root = scratch.path().to_path_buf();
                game.native_lifecycle.world_open_ready().unwrap();
                let mut receivers = Vec::new();
                for conn in [12, 13] {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let sender = horizon_client::lan::PlayerSender::new(tx, Arc::default());
                    senders
                        .lock()
                        .unwrap()
                        .insert(u128::from(conn), sender.clone());
                    game.lan.as_mut().unwrap().hub.update_guest(conn, |g| {
                        g.sender = sender;
                        g.brand = "horizon".into();
                        g.placed = false;
                    });
                    receivers.push(rx);
                }
                // Drive the production join path through Hub::place_new_player,
                // rather than calling the selection and send helpers separately.
                for conn in [12, 13] {
                    game.lan_guest_joined(conn, &[], [3.0, 64.0, 5.0]);
                }
                assert_eq!(*decided.lock().unwrap(), [12, 13]);
                for (index, rx) in receivers.iter().enumerate() {
                    let mut order = Vec::new();
                    for packet in captured_packets(rx) {
                        use horizon_protocol::packets::play::Clientbound as C;
                        match packet {
                            C::CustomPayload(p) if p.channel == "testmod:selection_decided" => {
                                order.push("decision")
                            }
                            C::UpdateRecipes(_) => order.push("recipes"),
                            C::CustomPayload(p) if p.channel == "horizon:recipe_content" => {
                                let content = ContentPart::decode(&p.data).unwrap();
                                assert_eq!(
                                    content.last,
                                    Some(vec!["recipe_sync_data:grinding".into()])
                                );
                                order.push("selected recipes");
                            }
                            _ => {}
                        }
                    }
                    if index == 0 {
                        assert_eq!(order, ["decision", "recipes", "selected recipes"]);
                    } else {
                        assert_eq!(order, ["decision", "recipes"]);
                    }
                }
            })
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn datapack_sync_disconnect_before_ask_sends_nothing_and_empty_reload_asks_nobody() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("disconnect", Box::new(Selection(calls.clone()))),
            mod_api::ModEntry::new("recipe_sync_data", Box::new(MachineTypes)),
        ]
        .into(),
        || {
            with_guests(|game| {
                game.native_lifecycle.world_open_ready().unwrap();
                let (tx, rx) = std::sync::mpsc::channel();
                game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                    g.sender = horizon_client::lan::PlayerSender::new(tx, Arc::default());
                    g.brand = "horizon".into();
                    g.placed = false;
                });
                // Deterministically force the gap: remove the admitted guest just
                // before the join helper resolves the recipient for ask_for.
                let guests = &mut game.lan.as_mut().unwrap().hub.guests;
                let index = guests.iter().position(|g| g.conn == 12).unwrap();
                drop(guests.remove(index));
                game.prepare_recipe_join(12);
                assert!(calls.lock().unwrap().is_empty());
                assert!(rx.try_iter().next().is_none());
                game.sync_recipe_content();
                assert_eq!(*calls.lock().unwrap(), [13]);
                drop(game.lan.as_mut().unwrap().hub.guests.remove(0));
                game.sync_recipe_content();
                assert_eq!(*calls.lock().unwrap(), [13], "empty reload never asks");
                assert!(rx.try_iter().next().is_none());
            })
        },
    );
}

fn captured_packets(
    rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>,
) -> Vec<horizon_protocol::packets::play::Clientbound> {
    use horizon_client::lan::Out;
    use horizon_protocol::{buf::Reader, packets::play::Clientbound};
    let mut packets = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(p) => vec![p],
            Out::Packets(p) => p,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            packets.push(Clientbound::decode(id, r.rest()).unwrap());
        }
    }
    packets
}
