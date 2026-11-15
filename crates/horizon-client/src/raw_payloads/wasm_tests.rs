//! Actual compiled guest against the real bounded server provider and Outbox.
use super::*;
use mod_api::{Hooks, PayloadBytes, Registrar};
struct Receipt;
impl mod_api::IPayloadHandler for Receipt {
    fn handle(
        &self,
        _: PayloadBytes,
        _: &mut dyn mod_api::IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        panic!("receipt must remain in the physical Outbox fixture")
    }
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar.payload_handlers().registrar("1").play_to_client(
            PayloadId("raw_probe:receipt".into()),
            Some(Arc::new(Receipt)),
        );
    }
}
use crate::lan::{NetEvent, PlayerSender};

#[test]
#[ignore = "requires compiled HORIZON_WASM_FIXTURE and HORIZON_WASM_STD"]
fn compiled_guest_observes_physical_provider_pressure() {
    // Enters the guest: libtest's thread is smaller than the guest stack policy requires.
    horizon_guest_stack::run_on_guest_stack(
        compiled_guest_observes_physical_provider_pressure_on_guest_stack,
    );
}

fn compiled_guest_observes_physical_provider_pressure_on_guest_stack() {
    // Only this test initializes points in this process. Run explicitly by exact name.
    use horizon_mod_package::manifest::Package;
    let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
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
        &std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest,
            module: Package::read_wasm(path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[info]).unwrap();
    let mut mods = loaded.mods;
    mods.push(mod_api::ModEntry::new("raw_probe", Box::new(Native)));
    mod_platform::testing::with_mods(mods.into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
            .unwrap();
        super::busy_wasm_tests::exercise();
        let transport = Transport::capture(Side::Server).unwrap();
        transport
            .accept(transport.local.manifest.entries())
            .unwrap();
        transport.finish_configuration().unwrap();
        let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let (events, _receiver) = crate::lan::event_channel_with_capacity(
            horizon_mod_runtime::byte_channel::message_overhead::<NetEvent>(),
        );
        let incoming = Incoming::new(
            &transport,
            negotiation::Phase::Play,
            horizon_protocol::packets::custom_payload::CustomPayload {
                channel: "wasm_fixture:pressure".into(),
                data: vec![1],
            },
        )
        .unwrap();
        let credit = Arc::downgrade(&incoming.lease);
        dispatch(
            incoming,
            endpoint.clone(),
            MainQueue::Server {
                events: events.clone(),
                endpoint,
                conn: Some(1),
            },
        );
        assert!(
            !transport.closed(),
            "guest must return success only after observing actual Backpressure"
        );
        assert!(out().is_empty(), "no disconnect or reply may be published");
        assert_eq!(events.stats().queued_items, 0);
        assert_eq!(events.stats().reserved_bytes, 0);
        assert!(
            credit.upgrade().is_none(),
            "actual ingress credit is released after refusal"
        );
        let (events, _receiver) = crate::lan::event_channel();
        let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let body: Vec<u8> = (0..1 << 20).map(|i| (i % 251) as u8).collect();
        let expected = body.clone();
        // Measures copies and allocations, not time: the probes slow the guest call past a hook's
        // 100 ms budget on a loaded runner, so the call gets the load budget.
        let ((_, copies), counts) = horizon_mod_wasm::copy_probe::with_load_budget(|| {
            horizon_threads::allocation_probe::measure(|| {
                horizon_mod_wasm::copy_probe::measure(|| {
                    let incoming = Incoming::new(
                        &transport,
                        negotiation::Phase::Play,
                        horizon_protocol::packets::custom_payload::CustomPayload {
                            channel: "wasm_fixture:measure".into(),
                            data: body,
                        },
                    )
                    .unwrap();
                    dispatch(
                        incoming,
                        endpoint.clone(),
                        MainQueue::Server {
                            events,
                            endpoint,
                            conn: Some(1),
                        },
                    );
                })
            })
        });
        // Independent vanilla packet reference: never call the encoder under test.
        // Both this packet id and this fixed channel length fit one-byte VarInts.
        let mut expected_packet = vec![
            horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD as u8,
            b"wasm_fixture:measure".len() as u8,
        ];
        expected_packet.extend_from_slice(b"wasm_fixture:measure");
        expected_packet.extend_from_slice(&expected);
        assert_eq!(out(), [expected_packet]);
        assert!(!transport.closed());
        eprintln!(
            "1 MiB actual guest ingress -> NETWORK callback -> physical Outbox allocation requests: {counts:?}; includes realloc requests, excludes input construction and output snapshot"
        );
        eprintln!(
            "Actual shared-memory runtime copy bytes: {copies:?}; excludes guest-internal encode/decode and physical packet encoding copies"
        );
        assert!(copies.to_guest_bytes >= 1 << 20 && copies.from_guest_bytes >= 1 << 20);
        assert!(
            counts.large_calls > 0,
            "measurement must observe the actual wire body allocations"
        );
        // Run the poisoning case last. Keep the real provider receiver alive and
        // undrained so epoch cancellation cannot hide incorrectly published work.
        let (events, _receiver) = crate::lan::event_channel();
        let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let staging = staging_probe::Watch::begin("wasm_fixture:network_trap");
        let incoming = Incoming::new(
            &transport,
            negotiation::Phase::Play,
            horizon_protocol::packets::custom_payload::CustomPayload {
                channel: "wasm_fixture:network_trap".into(),
                data: vec![1],
            },
        )
        .unwrap();
        let credit = Arc::downgrade(&incoming.lease);
        dispatch(
            incoming,
            endpoint.clone(),
            MainQueue::Server {
                events: events.clone(),
                endpoint,
                conn: Some(1),
            },
        );
        assert_eq!(
            staging.snapshot(),
            staging_probe::Counts {
                work: 1,
                replies: 1
            },
            "both physical preparations must succeed before the intentional trap"
        );
        assert!(transport.closed(), "the actual guest handler must trap");
        assert_eq!(
            events.stats().queued_items,
            0,
            "no staged work may publish, even if its epoch is later cancelled"
        );
        assert_eq!(events.stats().reserved_bytes, 0);
        assert!(credit.upgrade().is_none());
        let leaked = encode(
            negotiation::Phase::Play,
            Side::Server,
            &PayloadId("raw_probe:receipt".into()),
            PayloadBytes(vec![0xde, 0xad]),
        );
        assert!(
            !out().iter().any(|packet| packet == &leaked),
            "physical Outbox must contain no application reply before disconnect"
        );
    });
}
