use super::*;
use horizon_protocol::payload_negotiation::{Flow, Manifest, RegistrationPhase};
use std::{sync::mpsc, time::Duration};

fn connection() -> Arc<Connection> {
    let mut manifest = Manifest::builder();
    manifest
        .register(
            RegistrationPhase::Play,
            "test:raw",
            Flow::ToServer,
            "1",
            true,
        )
        .unwrap();
    let local = Arc::new(Local::new(manifest.finish().unwrap(), None));
    let connection = Connection::new(local.clone(), Side::Client);
    connection.accept(local.manifest.entries()).unwrap();
    connection
}
#[test]
fn configuration_acknowledgement_waits_for_main_root_commit() {
    let connection = connection();
    let old = connection.epoch().unwrap();
    let pending = old.main_task();
    let (started, start) = mpsc::channel();
    let (done, result) = mpsc::channel();
    let owner = connection.clone();
    let thread = std::thread::spawn(move || {
        started.send(()).unwrap();
        done.send(owner.finish_configuration()).unwrap();
    });
    start.recv().unwrap();
    assert!(result.recv_timeout(Duration::from_millis(20)).is_err());
    drop(pending);
    result
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    assert!(!old.active());
    assert_eq!(connection.epoch().unwrap().phase, Phase::Play);
    connection.begin_configuration();
    assert!(connection.epoch().is_none());
}
#[test]
fn close_releases_a_waiting_configuration_without_running_queued_work() {
    let connection = connection();
    let old = connection.epoch().unwrap();
    let pending = old.main_task();
    let (done, result) = mpsc::channel();
    let owner = connection.clone();
    let thread = std::thread::spawn(move || {
        done.send(owner.finish_configuration()).unwrap();
    });
    connection.close();
    assert!(
        result
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_err()
    );
    assert!(!old.active());
    assert!(connection.epoch().is_none());
    drop(pending);
    thread.join().unwrap();
}

#[test]
fn ingress_leases_survive_phase_churn_and_refund_outside_queues() {
    let connection = connection();
    let cost = INGRESS_RECORD_OVERHEAD + byte_channel::message_overhead::<()>();
    let snapshot_charge = connection.ingress.stats().reserved_bytes;
    let mut leases = Vec::new();
    for _ in 0..(INGRESS_BUDGET - snapshot_charge) / cost {
        leases.push(connection.reserve_ingress(0).unwrap());
    }
    assert!(matches!(
        connection.reserve_ingress(0),
        Err(PayloadSendError::Backpressure)
    ));
    let old = connection.epoch().unwrap();
    connection.begin_configuration();
    assert!(!old.active());
    // An old retained epoch still owns its snapshot and input charges. Starting
    // another configuration cannot bypass pressure by creating a fresh table.
    assert!(
        connection
            .accept(connection.local.manifest.entries())
            .is_err()
    );
    assert!(matches!(
        connection.reserve_ingress(0),
        Err(PayloadSendError::Backpressure)
    ));
    let lease = leases.pop().unwrap();
    // Moving ownership out of staging does not release retained bytes.
    assert!(matches!(
        connection.reserve_ingress(0),
        Err(PayloadSendError::Backpressure)
    ));
    drop(lease);
    let replacement = connection.reserve_ingress(0).unwrap();
    drop(replacement);
    drop(leases);
    assert_eq!(connection.ingress.stats().reserved_bytes, snapshot_charge);
    drop(old);
    assert_eq!(connection.ingress.stats().reserved_bytes, 0);
    connection
        .accept(connection.local.manifest.entries())
        .unwrap();
    connection.finish_configuration().unwrap();
    let snapshot_charge = connection.ingress.stats().reserved_bytes;
    let all = connection
        .reserve_ingress(INGRESS_BUDGET - snapshot_charge - cost)
        .unwrap();
    assert!(matches!(
        connection.reserve_ingress(0),
        Err(PayloadSendError::Backpressure)
    ));
    connection.close();
    assert!(matches!(
        connection.reserve_ingress(0),
        Err(PayloadSendError::Disconnected)
    ));
    drop(all);
    assert_eq!(connection.ingress.stats().reserved_bytes, 0);
    for _ in 0..3 {
        connection.begin_configuration();
        assert!(
            connection
                .accept(connection.local.manifest.entries())
                .is_err()
        );
        assert!(connection.epoch().is_none());
    }
}

#[test]
fn publication_gate_orders_close_and_isolates_replaced_epochs() {
    use std::sync::{Barrier, atomic::AtomicUsize};
    let connection = connection();
    let old = connection.epoch().unwrap();
    assert_eq!(old.with_active(|| 17), Some(17));
    connection.begin_configuration();
    connection
        .accept(connection.local.manifest.entries())
        .unwrap();
    let epoch = connection.epoch().unwrap();
    assert_eq!(old.with_active(|| panic!("stale publication")), None::<()>);
    assert_eq!(epoch.with_active(|| 23), Some(23));
    let start = Arc::new(Barrier::new(2));
    let published = Arc::new(AtomicUsize::new(0));
    let worker_start = start.clone();
    let worker_published = published.clone();
    let worker_epoch = epoch.clone();
    let worker = std::thread::spawn(move || {
        worker_start.wait();
        worker_epoch.with_active(|| {
            assert!(worker_epoch.active());
            worker_published.fetch_add(1, Ordering::SeqCst);
        })
    });
    start.wait();
    connection.close();
    let at_close = published.load(Ordering::SeqCst);
    let result = worker.join().unwrap();
    assert_eq!(published.load(Ordering::SeqCst), at_close);
    assert_eq!(at_close, usize::from(result.is_some()));
    assert_eq!(
        epoch.with_active(|| panic!("closed publication")),
        None::<()>
    );
}

#[test]
fn empty_transport_has_no_mutable_connection_or_ingress_budget() {
    let transport = super::super::transport::Transport::empty();
    assert!(transport.connection.is_none());
    assert!(transport.player().is_none());
    assert!(matches!(
        transport.reserve_ingress(0),
        Err(PayloadSendError::Disconnected)
    ));
}

#[test]
fn game_bound_player_can_be_replaced_but_not_revived_after_close() {
    let connection = connection();
    let player = |id| {
        mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), id),
            id as u128,
        )
    };
    assert!(connection.player().is_none());
    connection.bind_player(player(1));
    assert_eq!(connection.player(), Some(player(1)));
    connection.bind_player(player(2));
    assert_eq!(connection.player(), Some(player(2)));
    connection.close();
    connection.bind_player(player(3));
    assert!(connection.player().is_none());
}
