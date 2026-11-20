//! `player_pong`: only a client's answer to a ping the server sent is told to the game, once,
//! with its id, in order.
use super::pending_move_tests::{drain, standing};
use super::*;
use horizon_protocol::packets::{configuration::Ping, play::Serverbound as S};

fn pongs(events: &[HostEvent]) -> Vec<(u32, i32)> {
    events
        .iter()
        .filter_map(|e| match e {
            HostEvent::Pong { conn, id } => Some((*conn, *id)),
            _ => None,
        })
        .collect()
}

/// The server pings guest 0 with `id`.
fn ping(hub: &Hub, id: i32) {
    hub.guests[0].send(&horizon_protocol::packets::play::Clientbound::Ping(Ping {
        id,
    }));
}

fn hub_with(inbox: Vec<S>, pings: &[i32]) -> Hub {
    let mut hub = standing(false, inbox);
    hub.pong_listened = true;
    for id in pings {
        ping(&hub, *id);
    }
    hub
}

#[test]
fn a_pong_answering_a_ping_is_told_to_the_game_in_order_with_its_id() {
    let mut hub = hub_with(
        vec![S::Pong(Ping { id: 7 }), S::Pong(Ping { id: -3 })],
        &[7, -3],
    );
    let conn = hub.guests[0].conn;
    assert_eq!(pongs(&drain(&mut hub)), [(conn, 7), (conn, -3)]);
}

#[test]
fn an_unsolicited_pong_queues_nothing() {
    let mut hub = hub_with(vec![S::Pong(Ping { id: 7 })], &[8]);
    assert!(pongs(&drain(&mut hub)).is_empty());
}

#[test]
fn a_repeated_pong_for_the_same_ping_queues_once() {
    let mut hub = hub_with(vec![S::Pong(Ping { id: 7 }); 3], &[7]);
    let conn = hub.guests[0].conn;
    assert_eq!(pongs(&drain(&mut hub)), [(conn, 7)]);
}

#[test]
fn the_pings_waiting_for_an_answer_stay_bounded() {
    let hub = hub_with(vec![], &[]);
    for id in 0..10_000 {
        ping(&hub, id);
    }
    assert_eq!(
        hub.guests[0].pings.lock().unwrap().len(),
        crate::lan::ping_ledger::MAX_PENDING
    );
}

#[test]
fn with_no_listener_a_pong_queues_nothing() {
    let mut hub = hub_with(vec![S::Pong(Ping { id: 7 })], &[7]);
    hub.pong_listened = false;
    assert!(pongs(&drain(&mut hub)).is_empty());
}
