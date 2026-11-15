//! Real network executor/provider tests. No World or substitute payload context.
use super::*;
use crate::{
    lan::{NetEvent, PlayerSender},
    net::ingress::{Consumer, Producer},
};
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadContextError, PayloadWork, PayloadWorkStatus, Registrar,
};
use std::sync::{
    Mutex, Weak,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;

const CHANNEL: &str = "executor_probe:message";
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Success,
    Pressure,
    Error,
    Panic,
    Cancel,
    PublicationClosed,
}
struct Probe {
    mode: Mode,
    events: Producer<NetEvent>,
    receiver: Mutex<Option<Consumer<NetEvent>>>,
    transport: Mutex<Weak<Transport>>,
    drops: Arc<AtomicUsize>,
    calls: AtomicUsize,
}
struct ReentrantWork {
    epoch: Arc<connection::Epoch>,
    drops: Arc<AtomicUsize>,
}
impl PayloadWork for ReentrantWork {
    fn run(&self, _: &mut dyn IPayloadContext) -> Result<(), String> {
        panic!("network dispatch must not execute queued main work inline")
    }
}
impl Drop for ReentrantWork {
    fn drop(&mut self) {
        // A publication-gate Drop mutant deadlocks here. The bounded worker
        // completion assertion below fails without joining the stuck thread.
        let _ = self.epoch.with_active(|| ());
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl IPayloadHandler for Probe {
    fn handle(
        &self,
        _: PayloadBytes,
        ctx: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        assert!(matches!(
            ctx.server_host(),
            Err(PayloadContextError::WrongThread)
        ));
        let transport = self.transport.lock().unwrap().upgrade().unwrap();
        for _ in 0..2 {
            let before = self.events.stats().reserved_bytes;
            let result = ctx.enqueue_work(Arc::new(ReentrantWork {
                epoch: transport.epoch().unwrap(),
                drops: self.drops.clone(),
            }));
            if self.mode == Mode::Pressure {
                assert_eq!(result, Err(PayloadContextError::Backpressure));
                assert_eq!(self.events.stats().reserved_bytes, before);
            } else {
                assert_eq!(result, Ok(PayloadWorkStatus::Queued));
                assert!(
                    self.events.stats().reserved_bytes > before,
                    "Queued must already own main-provider credit"
                );
            }
            assert_eq!(
                self.events.stats().queued_items,
                0,
                "no publication during callback"
            );
        }
        if self.mode != Mode::Pressure {
            ctx.reply(&PayloadId(CHANNEL.into()), PayloadBytes(vec![1]))
                .unwrap();
        }
        // Also prove that no assertion was caught as a handler panic.
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Mode::Error => {
                return Err(mod_api::PayloadHandleError::Failed(
                    "deliberate handler error".into(),
                ));
            }
            Mode::Panic => panic!("deliberate handler panic after staging"),
            Mode::Cancel => transport.close(),
            Mode::PublicationClosed => drop(self.receiver.lock().unwrap().take()),
            _ => {}
        }
        Ok(())
    }
}
struct Registration(Arc<Probe>);
#[mod_api::__subscriptions]
impl Hooks for Registration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(HandlerThread::Network)
            .play_bidirectional(
                PayloadId(CHANNEL.into()),
                self.0.clone(),
                Some(self.0.clone()),
            );
    }
}
fn with_case(
    mode: Mode,
    bytes: usize,
    run: impl FnOnce(Arc<Probe>, Incoming, Endpoint, Box<dyn Fn() -> Vec<Vec<u8>>>),
) {
    let (events, receiver) = if mode == Mode::Pressure {
        crate::lan::event_channel_with_capacity(
            horizon_mod_runtime::byte_channel::message_overhead::<NetEvent>(),
        )
    } else {
        crate::lan::event_channel()
    };
    let probe = Arc::new(Probe {
        mode,
        events,
        receiver: Mutex::new(Some(receiver)),
        transport: Mutex::new(Weak::new()),
        drops: Arc::default(),
        calls: AtomicUsize::new(0),
    });
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "executor_probe",
            Box::new(Registration(probe.clone())),
        )]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let transport = Transport::capture(Side::Server).unwrap();
            assert!(transport.local.owns(CHANNEL));
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            *probe.transport.lock().unwrap() = Arc::downgrade(&transport);
            let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
            let endpoint = Endpoint::server(sender);
            let incoming = Incoming::new(
                &transport,
                negotiation::Phase::Play,
                horizon_protocol::packets::custom_payload::CustomPayload {
                    channel: CHANNEL.into(),
                    data: vec![0; bytes],
                },
            )
            .unwrap();
            run(probe, incoming, endpoint, Box::new(out));
        },
    );
}
fn queue(probe: &Probe, endpoint: &Endpoint) -> MainQueue {
    MainQueue::Server {
        events: probe.events.clone(),
        endpoint: endpoint.clone(),
        conn: Some(1),
    }
}

fn reply_count(snapshot: &dyn Fn() -> Vec<Vec<u8>>) -> usize {
    let reply = encode(
        negotiation::Phase::Play,
        Side::Server,
        &PayloadId(CHANNEL.into()),
        PayloadBytes(vec![1]),
    );
    snapshot().iter().filter(|body| *body == &reply).count()
}

#[test]
fn queued_work_reserves_before_acceptance_and_keeps_original_credit_until_last_drop() {
    with_case(Mode::Success, 3 << 20, |probe, incoming, endpoint, out| {
        let transport = endpoint.transport().clone();
        let original = Arc::downgrade(&incoming.lease);
        dispatch(incoming, endpoint.clone(), queue(&probe, &endpoint));
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.events.stats().queued_items, 2);
        let receiver = probe.receiver.lock().unwrap().take().unwrap();
        let mut work: Vec<_> = receiver
            .take_turn()
            .into_iter()
            .map(|receipt| receipt.into_parts())
            .collect();
        assert_eq!(work.len(), 2);
        assert!(work.iter().all(|(event, _credit)| matches!(event, NetEvent::RawPayload(delivery) if matches!(&delivery.incoming.body, Body::Work(_)))));
        assert!(original.upgrade().is_some());
        assert!(transport.reserve_ingress(14 << 20).is_err());
        drop(work.pop());
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
        assert!(original.upgrade().is_some());
        assert!(transport.reserve_ingress(14 << 20).is_err());
        drop(work);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
        assert!(original.upgrade().is_none());
        assert_eq!(probe.events.stats().reserved_bytes, 0);
        drop(
            transport
                .reserve_ingress(14 << 20)
                .expect("last work reference refunds physical credit"),
        );
        assert_eq!(reply_count(&*out), 1);
    });
}

#[test]
fn provider_pressure_refuses_work_before_acceptance() {
    with_case(Mode::Pressure, 1, |probe, incoming, endpoint, out| {
        let original = Arc::downgrade(&incoming.lease);
        dispatch(incoming, endpoint.clone(), queue(&probe, &endpoint));
        assert!(original.upgrade().is_none());
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
        assert_eq!(probe.events.stats().reserved_bytes, 0);
        assert_eq!(probe.events.stats().queued_items, 0);
        assert_eq!(reply_count(&*out), 0);
    });
}

#[test]
fn handler_error_or_epoch_cancellation_publishes_no_staged_work_or_replies() {
    for mode in [Mode::Error, Mode::Panic, Mode::Cancel] {
        with_case(mode, 1, |probe, incoming, endpoint, out| {
            let original = Arc::downgrade(&incoming.lease);
            dispatch(incoming, endpoint.clone(), queue(&probe, &endpoint));
            assert!(original.upgrade().is_none());
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
            assert_eq!(probe.events.stats().reserved_bytes, 0);
            assert_eq!(probe.events.stats().queued_items, 0);
            assert_eq!(reply_count(&*out), 0);
            if matches!(mode, Mode::Error | Mode::Panic) {
                // The player is told which mod's handler failed, never its reason.
                let sent = out();
                let has = |text: &[u8]| {
                    sent.iter()
                        .any(|packet| packet.windows(text.len()).any(|w| w == text))
                };
                assert!(has(b"a mod's network handler failed (executor_probe)"));
                assert!(!has(b"deliberate handler error"));
            }
        });
    }
}

#[test]
fn failed_publication_and_remaining_work_drop_outside_epoch_gate() {
    with_case(
        Mode::PublicationClosed,
        1,
        |probe, incoming, endpoint, out| {
            let original = Arc::downgrade(&incoming.lease);
            let main = queue(&probe, &endpoint);
            let (done, completion) = mpsc::channel();
            let worker = mod_api::testing::inherit_mods(move |_: &()| {
                dispatch(incoming, endpoint, main);
                done.send(()).unwrap();
            });
            let thread = std::thread::spawn(move || worker(&()));
            // Never join a stuck mutant: the timeout must fail this test rather
            // than hang the harness on opaque Work::drop reentering the gate.
            completion
                .recv_timeout(Duration::from_secs(5))
                .expect("dispatch deadlocked dropping opaque work under publication gate");
            thread.join().unwrap();
            assert!(original.upgrade().is_none());
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                probe.drops.load(Ordering::SeqCst),
                2,
                "both failed publication and unvisited staged work must be dropped"
            );
            assert_eq!(probe.events.stats().reserved_bytes, 0);
            assert_eq!(probe.events.stats().queued_items, 0);
            assert_eq!(reply_count(&*out), 0);
        },
    );
}

#[test]
fn disabled_owner_refuses_before_entering_its_native_handler() {
    with_case(Mode::Success, 1, |probe, incoming, endpoint, out| {
        let original = Arc::downgrade(&incoming.lease);
        assert!(mod_api::__private::set_mod_enabled("executor_probe", false));
        dispatch(incoming, endpoint.clone(), queue(&probe, &endpoint));
        assert!(endpoint.transport().closed());
        assert!(original.upgrade().is_none());
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
        assert_eq!(probe.events.stats().reserved_bytes, 0);
        assert_eq!(reply_count(&*out), 0);
    });
}
