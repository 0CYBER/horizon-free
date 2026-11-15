//! A compiled CONFIGURATION handler entered inline by the real LAN connection thread.
use horizon_protocol::packets::configuration as cfg;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TEST: &str = "game::payload_transport::wasm_config_tests::compiled_guest_runs_on_configuration_connection_thread";
const CHANNEL: &str = "wasm_fixture:configuration";

#[test]
#[ignore = "requires prepared resources and compiled HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn compiled_guest_runs_on_configuration_connection_thread() {
    horizon_guest_stack::run_on_guest_stack(run);
}

fn run() {
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        TEST,
        "HORIZON_WASM_CONFIG_CHILD",
        &["tcp"],
    ) {
        return;
    }
    use horizon_mod_package::manifest::Package;
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
                    "wasm_config_transport",
                    Some("Wasm configuration transport"),
                    Some(1),
                    true,
                    Some(&options),
                )
                .unwrap();
                let deadline = Instant::now() + Duration::from_secs(60);
                while !game
                    .remote
                    .as_ref()
                    .is_some_and(|remote| remote.logged_in && remote.joined)
                {
                    game.poll_remote().unwrap();
                    assert!(Instant::now() < deadline, "integrated play not ready");
                    std::thread::yield_now();
                }
                let server = game.integrated.as_ref().unwrap();
                let port = server
                    .call(|server| {
                        server.publish(super::super::lan::LanSettings {
                            port: 0,
                            guest_commands: false,
                            force_game_mode: false,
                        })
                    })
                    .expect("integrated server answered publication request")
                    .unwrap();
                let (mut connection, local, _known) = super::config_tests::configuration_peer(port);

                // A reply while the Game thread is held proves Network dispatch did not
                // silently move this handler to main. Known packs also keeps it out of Play
                // (and hence out of io_pool): ConfigIo dispatches on horizon-lan-conn.
                let (release, held) = mpsc::channel();
                let release = Release(release);
                let (entered, ready) = mpsc::channel();
                assert!(server.post(move |_| {
                    entered.send(()).unwrap();
                    held.recv_timeout(Duration::from_secs(30)).unwrap();
                }));
                ready.recv_timeout(Duration::from_secs(30)).unwrap();
                for expected in [[1, 39, 24, 15, 9], [2, 39, 24, 15, 9]] {
                    connection
                        .send(
                            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                                channel: CHANNEL.into(),
                                data: vec![3, 5, 8, 13],
                            })
                            .try_encode_with_payload_codecs(&local.codecs)
                            .unwrap(),
                        )
                        .unwrap();
                    // The guest transforms the request and increments guest-owned state.
                    // Runtime guest entry calls check_current() on THIS receiving thread:
                    // replacing the LAN spawn with a default-stack thread refuses the call,
                    // so no valid reply can satisfy this assertion (even in release builds).
                    let (id, body) = connection.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::CustomPayload(payload) => {
                            assert_eq!(payload.channel, CHANNEL);
                            assert_eq!(payload.data, expected);
                        }
                        packet => panic!("expected inline guest configuration reply: {packet:?}"),
                    }
                }
                drop(release);
                // End this peer while still in CONFIGURATION; no Play executor can
                // satisfy the proof or consume a configuration reply afterward.
                drop(connection);
                game.leave_server(None).unwrap();
            });
        });
    });
}

// Release the real main thread before Game shutdown, including assertion unwinding.
struct Release(mpsc::Sender<()>);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
