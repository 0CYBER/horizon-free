//! The compiled wasm fixture's typed payload on a published server: the guest decodes the body,
//! refuses forged and retyped tokens, and replies through its own token; a guest trap after a
//! staged reply sends nothing, closes the session and stops the guest.
use super::typed_tests::{await_disconnect, await_reply, send};
use std::time::{Duration, Instant};

const NAME: &str =
    "game::payload_transport::typed_wasm_tests::compiled_guest_typed_payload_replies_and_traps";
const CHANNEL: &str = "wasm_fixture:typed";

#[test]
#[ignore = "daily"]
fn compiled_guest_typed_payload_replies_and_traps() {
    horizon_guest_stack::run_on_guest_stack(on_guest_stack);
}

fn on_guest_stack() {
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_TYPED_WASM_CHILD",
        &["reply", "trap"],
    ) {
        return;
    }
    use horizon_mod_package::manifest::Package;
    let case = std::env::var("HORIZON_TYPED_WASM_CHILD").unwrap();
    let path = std::env::var("HORIZON_WASM_FIXTURE")
        .expect("missing HORIZON_WASM_FIXTURE: build the matching compiled guest package");
    let path = std::path::Path::new(&path);
    let manifest = Package::read(path).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        vec![],
    );
    let loaded = horizon_mod_wasm::load(
        &std::fs::read(
            std::env::var("HORIZON_WASM_STD")
                .expect("missing HORIZON_WASM_STD: build the matching shared standard library"),
        )
        .unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest,
            module: Package::read_wasm(path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[info]).unwrap();
    horizon_mod_wasm::copy_probe::with_load_budget(|| {
        mod_platform::testing::with_mods(loaded.mods.into(), || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                game.settings.render_distance = 2;
                game.settings.simulation_distance = 2;
                let options = horizon_client::title::WorldOptions {
                    world_type: horizon_client::save::WorldPreset::Flat,
                    ..Default::default()
                };
                game.open_integrated(
                    "wasm_typed",
                    Some("Wasm typed"),
                    Some(1),
                    true,
                    Some(&options),
                )
                .unwrap();
                let deadline = Instant::now() + Duration::from_secs(60);
                while !game
                    .remote
                    .as_ref()
                    .is_some_and(|r| r.logged_in && r.joined)
                {
                    game.poll_remote().unwrap();
                    assert!(Instant::now() < deadline, "integrated play not ready");
                    std::thread::yield_now();
                }
                let port = game
                    .integrated
                    .as_ref()
                    .unwrap()
                    .call(|server| {
                        server.publish(super::super::lan::LanSettings {
                            port: 0,
                            guest_commands: false,
                            force_game_mode: false,
                        })
                    })
                    .expect("integrated server answered publication request")
                    .unwrap();
                let mut peer = super::sender_tests::connect_peer(port, "TypedGuest");
                match case.as_str() {
                    "reply" => {
                        // counter=7 as one VarInt byte; the guest answers counter=8.
                        send(&mut peer, CHANNEL, vec![7]);
                        assert_eq!(await_reply(&mut peer, CHANNEL), [8]);
                        send(&mut peer, CHANNEL, vec![7, 0]);
                        let reason = await_disconnect(&mut peer, CHANNEL);
                        assert!(reason.contains("wasm_fixture"), "{reason}");
                    }
                    "trap" => {
                        send(&mut peer, CHANNEL, vec![13]);
                        let reason = await_disconnect(&mut peer, CHANNEL);
                        assert!(reason.contains("wasm_fixture"), "{reason}");
                        // The trapped guest's callback no longer runs for a new session.
                        let mut next = super::sender_tests::connect_peer(port, "TypedNext");
                        send(&mut next, CHANNEL, vec![7]);
                        let reason = await_disconnect(&mut next, CHANNEL);
                        assert!(reason.contains("wasm_fixture"), "{reason}");
                    }
                    other => panic!("unknown case {other}"),
                }
                game.leave_server(None).unwrap();
            });
        });
    });
}
