//! Real compiled guest contention, bounded by deterministic receiving turns and a fake clock.
use super::*;
use crate::{
    lan::{NetEvent, PlayerSender},
    net::ingress::Consumer,
};
use std::{
    sync::mpsc,
    thread::JoinHandle,
    time::{Duration, Instant},
};

const CHANNEL: &str = "wasm_fixture:busy";
struct Case {
    transport: Arc<Transport>,
    endpoint: Endpoint,
    main: MainQueue,
    _receiver: Consumer<NetEvent>,
    out: Box<dyn Fn() -> Vec<Vec<u8>>>,
}
impl Case {
    fn new() -> Self {
        let transport = Transport::capture(Side::Server).unwrap();
        transport
            .accept(transport.local.manifest.entries())
            .unwrap();
        transport.finish_configuration().unwrap();
        let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
        let endpoint = Endpoint::server(sender);
        let (events, receiver) = crate::lan::event_channel();
        let main = MainQueue::Server {
            events,
            endpoint: endpoint.clone(),
            conn: Some(1),
        };
        Self {
            transport,
            endpoint,
            main,
            _receiver: receiver,
            out: Box::new(out),
        }
    }
    fn incoming(&self, byte: u8, size: usize) -> Incoming {
        Incoming::new(
            &self.transport,
            negotiation::Phase::Play,
            horizon_protocol::packets::custom_payload::CustomPayload {
                channel: CHANNEL.into(),
                data: vec![byte; size],
            },
        )
        .unwrap()
    }
    fn dispatch(&self, incoming: Incoming) {
        let MainQueue::Server {
            events,
            endpoint,
            conn,
        } = &self.main
        else {
            unreachable!()
        };
        dispatch(
            incoming,
            self.endpoint.clone(),
            MainQueue::Server {
                events: events.clone(),
                endpoint: endpoint.clone(),
                conn: *conn,
            },
        );
    }
}
struct Holding {
    release: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}
impl Holding {
    fn finish(&mut self) {
        self.release.take().unwrap().send(()).unwrap();
        self.thread.take().unwrap().join().unwrap();
    }
}
impl Drop for Holding {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn held_guest(run: impl FnOnce(&staging_probe::Watch, &mut Holding)) {
    horizon_mod_wasm::copy_probe::with_load_budget(|| {
        let watch = staging_probe::Watch::begin(CHANNEL);
        let (entered, release) = watch.pause_next();
        let call = mod_api::testing::inherit_mods(|_: &()| {
            let blocker = Case::new();
            blocker.dispatch(blocker.incoming(0xff, 1));
            assert!(!blocker.transport.closed());
        });
        let thread =
            horizon_guest_stack::spawn("payload busy holder".into(), move || call(&())).unwrap();
        let mut holding = Holding {
            release: Some(release),
            thread: Some(thread),
        };
        entered
            .recv_timeout(Duration::from_secs(10))
            .expect("actual guest did not enter its host import");
        run(&watch, &mut holding);
    });
}
fn expected(byte: u8, size: usize) -> Vec<u8> {
    let mut packet = vec![
        horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD as u8,
        CHANNEL.len() as u8,
    ];
    packet.extend_from_slice(CHANNEL.as_bytes());
    packet.resize(packet.len() + size, byte);
    packet
}

/// Called within the existing compiled fixture's isolated installation, before its trap case.
pub(super) fn exercise() {
    horizon_mod_wasm::test_log::capture_logs();
    held_guest(|watch, holding| {
        let case = Case::new();
        let first = case.incoming(0x31, 3 << 20);
        let first_credit = Arc::downgrade(&first.lease);
        let second = case.incoming(0x72, 3 << 20);
        let second_credit = Arc::downgrade(&second.lease);
        case.dispatch(first);
        case.dispatch(second);
        assert!(!case.transport.closed());
        assert!(
            (case.out)().is_empty(),
            "Busy must publish neither reply nor disconnect"
        );
        assert!(first_credit.upgrade().is_some() && second_credit.upgrade().is_some());
        assert!(
            case.transport.reserve_ingress(12 << 20).is_err(),
            "both retained payloads consume the physical 16 MiB budget"
        );
        let refused = Incoming::classify(
            &case.transport,
            negotiation::Phase::Play,
            horizon_protocol::packets::custom_payload::CustomPayload {
                channel: CHANNEL.into(),
                data: vec![0; 11 << 20],
            },
        )
        .err()
        .expect("the retained queue must refuse excess ingress");
        assert!(matches!(
            refused.error,
            mod_api::PayloadSendError::Backpressure
        ));
        let reason = refused.to_string();
        for field in [
            "handler failed at the inbound queue limit",
            "wasm_fixture",
            "IPayloadHandler::handle",
            "payload busy holder",
            "held for",
        ] {
            assert!(reason.contains(field), "missing {field}: {reason}");
        }
        holding.finish();
        dispatch_pending(&case.endpoint, &case.main);
        let packets = (case.out)();
        assert_eq!(packets.len(), 2, "{:?}", watch.failures());
        for (packet, byte) in packets.iter().zip([0x31, 0x72]) {
            assert!(
                *packet == expected(byte, 3 << 20),
                "ordered echo bytes differ for {byte:#x}"
            );
        }
        assert!(!case.transport.closed());
        assert!(first_credit.upgrade().is_none() && second_credit.upgrade().is_none());
        assert!(case.transport.reserve_ingress(12 << 20).is_ok());
        assert!(watch.failures().is_empty());
    });
    held_guest(|_watch, holding| {
        horizon_mod_wasm::test_log::take_logs();
        let case = Case::new();
        let incoming = case.incoming(1, 1);
        let now = Instant::now();
        let credit = Arc::downgrade(&incoming.lease);
        case.dispatch(incoming);
        case.dispatch(case.incoming(2, 1));
        // Expiry still wins if the group frees before the next receiving turn.
        holding.finish();
        let expired = now + Duration::from_secs(61);
        super::executor::dispatch_pending_at(&case.endpoint, &case.main, expired);
        super::executor::dispatch_pending_at(&case.endpoint, &case.main, expired);
        assert!(case.transport.closed());
        assert!(credit.upgrade().is_none());
        let packets = (case.out)();
        assert_eq!(
            packets.len(),
            1,
            "one final failure, not one failure per retry/payload"
        );
        assert_eq!(
            horizon_protocol::buf::Reader::new(&packets[0])
                .varint()
                .unwrap(),
            horizon_protocol::ids::play::clientbound::DISCONNECT
        );
        let failures: Vec<_> = horizon_mod_wasm::test_log::take_logs()
            .into_iter()
            .filter(|message| message.contains(CHANNEL))
            .collect();
        assert_eq!(failures.len(), 1);
        let diagnostic = &failures[0];
        assert!(
            diagnostic.contains("deadline expired after 60 s"),
            "{diagnostic}"
        );
        for field in [
            "wasm_fixture",
            "IPayloadHandler::handle",
            "payload busy holder",
            "held for",
            " ms",
        ] {
            assert!(diagnostic.contains(field), "missing {field}: {diagnostic}");
        }
    });
    held_guest(|watch, holding| {
        let case = Case::new();
        let incoming = case.incoming(3, 1);
        let credit = Arc::downgrade(&incoming.lease);
        case.dispatch(incoming);
        assert!(credit.upgrade().is_some());
        case.transport.begin_configuration();
        assert!(
            credit.upgrade().is_none(),
            "phase cancellation releases retained ingress"
        );
        holding.finish();
        dispatch_pending(&case.endpoint, &case.main);
        assert!(
            (case.out)().is_empty(),
            "cancelled epoch must never deliver old bytes"
        );
        assert!(watch.failures().is_empty());
    });
}
