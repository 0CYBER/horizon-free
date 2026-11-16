use super::*;
use horizon_client::{datapack_net, lan::Out};
use horizon_protocol::{buf::Reader, packets::play::Clientbound};

struct Synced;
#[mod_api::__subscriptions]
impl Hooks for Synced {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.declare_datapack_registry(
            "spell",
            DatapackRegistrySync::Required,
            vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
        )
        .unwrap();
    }
}
fn apply_packets(rx: &std::sync::mpsc::Receiver<Out>) -> usize {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("unexpected refusal: {reason:?}"),
        })
        .filter_map(|body| {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            match Clientbound::decode(id, r.rest()).unwrap() {
                Clientbound::CustomPayload(p) if p.channel == datapack_net::CHANNEL => {
                    datapack_net::accept_chunk(&p.data).unwrap();
                    Some(())
                }
                _ => None,
            }
        })
        .count()
}

#[test]
#[ignore = "daily"]
fn datapack_game_reload_delivers_only_successful_generations_and_disconnects_refused_guests() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let pack = root.join("datapacks/base");
    write(
        &pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"fixture","min_format":94,"max_format":94}}"#,
    );
    let entry = pack.join("data/registry_test/registry_test/spell/fire.json");
    write(&entry, r#"{"power":7}"#);
    write(
        &root.join("datapacks.json"),
        r#"{"enabled":["vanilla","file/base"],"disabled":[]}"#,
    );
    mod_platform::testing::with_mods(
        vec![ModEntry::new("registry_test", Box::new(Synced))].into(),
        || {
            with_game(|game| {
                game.load_world(request(root)).unwrap();
                let offer = datapack_net::offer().unwrap();
                datapack_net::clear_received();
                let datapack_net::Step::Send(bytes) = datapack_net::server_step(true, Some(&offer))
                else {
                    panic!("initial snapshot required");
                };
                datapack_net::accept(&bytes).unwrap();
                let first = datapack_net::received().unwrap();
                let handle = first
                    .handle("registry_test:spell", "registry_test:fire")
                    .unwrap();
                let rx = crate::game::lan::world_spawn_sync_tests::host_for(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.brand = "horizon".into();
                    g.sender.datapack_offer = Some(offer.into());
                });
                write(&entry, r#"{"power":9}"#);
                assert!(game.run_command("reload"));
                assert!(
                    apply_packets(&rx) > 0,
                    "the coordinator must send its committed generation"
                );
                let received = datapack_net::received().unwrap();
                assert_eq!(received.generation(), 2);
                assert_eq!(
                    received.get(&handle),
                    Err(mod_api::DatapackReadError::StaleGeneration)
                );
                let next = received
                    .handle("registry_test:spell", "registry_test:fire")
                    .unwrap();
                assert_eq!(
                    received.get(&next).unwrap().get("power"),
                    Some(&DatapackValue::Int(9))
                );
                write(&entry, "{broken");
                assert!(!game.run_command("reload"));
                assert_eq!(
                    apply_packets(&rx),
                    0,
                    "failed preparation must send no replacement"
                );
                assert_eq!(datapack_net::received().unwrap().generation(), 2);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.sender.datapack_offer = Some(Arc::from([255_u8])));
                write(&entry, r#"{"power":11}"#);
                assert!(game.run_command("reload"));
                assert!(rx.try_iter().any(|out| matches!(out, Out::Disconnect(horizon_core::nbt::Tag::String(reason)) if String::from_utf16_lossy(&reason.0).contains("registries are unreadable"))));
                datapack_net::clear_received();
            });
        },
    );
}
