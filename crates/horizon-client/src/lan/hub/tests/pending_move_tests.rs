//! `player_move`: a changed report is held for the mods, ruled on, and the inbox waits behind it.
use super::*;
pub(super) use crate::lan::guest::InboxItem;

pub(super) fn with_input_context<T>(run: impl FnOnce(&mut TickCtx<'_>) -> T) -> T {
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    run(&mut TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    })
}

pub(super) fn at(x: f64) -> S {
    S::MovePlayerPos(play::MovePlayerPos {
        x,
        y: 64.0,
        z: 0.0,
        flags: 1,
    })
}

/// A report that moves and claims no ground.
pub(super) fn walking(x: f64) -> S {
    S::MovePlayerPos(play::MovePlayerPos {
        x,
        y: 64.0,
        z: 0.0,
        flags: 0,
    })
}

pub(super) fn turned(yaw: f32) -> S {
    S::MovePlayerRot(play::MovePlayerRot {
        yaw,
        pitch: 0.0,
        flags: 1,
    })
}

/// A hub at tick 10 whose guest 0 stands at `(0, 64, 0)`, entered its level at tick 0 and listens (or not).
pub(super) fn standing(listened: bool, inbox: Vec<S>) -> Hub {
    let (mut hub, _rx) = hub_with_a_guest();
    hub.moves_listened = listened;
    // Long after the guest entered its level (tick 0): its first tick there is long past.
    hub.ticks = 10;
    hub.guests.update(0, |g| {
        g.load_timeout = 0;
        g.position = [0.0, 64.0, 0.0];
        g.first_good = g.position;
        g.yaw = 0.0;
        g.pitch = 0.0;
        g.last_reported = (g.position, [0.0, 0.0]);
        g.ticks = 1;
        g.inbox.extend(inbox.into_iter().map(InboxItem::uncredited));
    });
    hub
}

pub(super) fn drain(hub: &mut Hub) -> Vec<HostEvent> {
    with_input_context(|ctx| hub.drain_packets(ctx))
}

pub(super) fn moves(events: &[HostEvent]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            HostEvent::PlayerMove(conn) => Some(*conn),
            _ => None,
        })
        .collect()
}

pub(super) fn tick_end() -> S {
    S::ClientTickEnd(horizon_protocol::packets::Empty)
}

/// The guest's accepted look as `[yaw, pitch]`.
fn look(hub: &Hub) -> [f32; 2] {
    [hub.guests[0].yaw, hub.guests[0].pitch]
}

#[test]
fn loading_blocks_movement_and_attacks_until_the_load_timeout_expires() {
    for (timeout, accepted) in [(2, false), (0, true)] {
        let mut hub = standing(false, vec![S::Attack(play::Attack { id: 7 }), walking(1.0)]);
        hub.guests.update(0, |g| {
            g.loaded = false;
            g.load_timeout = timeout;
        });
        let mut events = drain(&mut hub);
        // Delivering an attack yields before the following movement packet.
        events.extend(drain(&mut hub));
        assert_eq!(
            events.iter().any(|event| matches!(event,
            HostEvent::Packet(_, packet) if matches!(&**packet, S::Attack(_)))),
            accepted
        );
        assert_eq!(
            hub.guests[0].position,
            [if accepted { 1.0 } else { 0.0 }, 64.0, 0.0]
        );
        assert!(!hub.guests[0].loaded);
    }
}

#[test]
fn two_reports_in_one_tick_are_heard_in_order_each_from_the_one_before_when_both_pass() {
    let mut hub = standing(true, vec![turned(10.0), turned(20.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn], "the first is held");
    assert_eq!(hub.guests[0].inbox.len(), 1, "the second keeps its place");
    assert_eq!(
        look(&hub),
        [0.0, 0.0],
        "nothing is applied before the ruling"
    );
    let first = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (first.from_rotation, first.to_rotation),
        ([0.0, 0.0], [10.0, 0.0])
    );
    hub.resolve_move(conn, true);
    assert_eq!(look(&hub), [10.0, 0.0]);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let second = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (second.from_rotation, second.to_rotation),
        ([10.0, 0.0], [20.0, 0.0])
    );
    hub.resolve_move(conn, true);
    assert_eq!(look(&hub), [20.0, 0.0]);
    assert!(hub.guests[0].inbox.is_empty());
}

#[test]
fn a_veto_of_the_first_of_two_reports_in_one_tick_corrects_and_the_second_is_not_heard() {
    let mut hub = standing(true, vec![turned(10.0), turned(20.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    hub.resolve_move(conn, false);
    assert_eq!(look(&hub), [0.0, 0.0], "the vetoed report is not applied");
    assert!(hub.guests[0].awaiting_teleport.is_some());
    // The second waited behind it, and meets the unconfirmed correction: only the server's own
    // handling of a report that arrives then, no hearing.
    assert!(moves(&drain(&mut hub)).is_empty());
    assert!(hub.guests[0].pending_move.is_none());
    assert!(hub.guests[0].inbox.is_empty());
    assert_eq!(look(&hub), [0.0, 0.0]);
}

#[test]
fn two_reports_in_one_tick_with_no_listener_are_committed_at_once_in_order() {
    let mut hub = standing(false, vec![turned(10.0), turned(20.0)]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert_eq!(look(&hub), [20.0, 0.0]);
    assert!(hub.guests[0].pending_move.is_none());
    assert!(hub.guests[0].inbox.is_empty());
}

#[test]
fn the_end_of_the_client_tick_and_the_next_report_keep_their_order_behind_a_held_report() {
    let mut hub = standing(true, vec![at(0.5), tick_end(), at(0.75)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    assert_eq!(hub.guests[0].inbox.len(), 2, "tick end and report wait");
    assert!(
        hub.guests[0].position_this_tick,
        "the tick end has not been read"
    );
    hub.resolve_move(conn, true);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let g = &hub.guests[0];
    assert!(
        !g.disconnect_pending,
        "the tick end was read before the report, so it is not a second position in one client tick"
    );
    assert_eq!(g.position, [0.5, 64.0, 0.0]);
    let second = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (second.from, second.to),
        ([0.5, 64.0, 0.0], [0.75, 64.0, 0.0])
    );
}

#[test]
fn a_veto_drops_the_report_and_corrects_the_client() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    let counter = hub.guests[0].teleport_counter;
    hub.resolve_move(conn, false);
    let g = &hub.guests[0];
    assert_eq!(g.position, [0.0, 64.0, 0.0], "the report is not applied");
    assert_eq!(
        g.teleport_counter,
        counter + 1,
        "the client got the teleport correction"
    );
    assert_eq!(
        g.awaiting_teleport.map(|(_, at)| at),
        Some([0.0, 64.0, 0.0])
    );
    assert!(g.pending_move.is_none());
    assert!(g.moves.is_empty(), "its bookkeeping was skipped");
}

#[test]
fn a_report_that_changes_nothing_is_not_held() {
    let mut hub = standing(true, vec![at(0.0)]);
    let events = drain(&mut hub);
    assert!(moves(&events).is_empty(), "the same position is not heard");
    assert!(hub.guests[0].pending_move.is_none());
}

#[test]
fn a_turn_alone_is_held() {
    let mut hub = standing(true, vec![turned(30.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(report.from, report.to, "only the look changed");
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([0.0, 0.0], [30.0, 0.0])
    );
}

#[test]
fn with_no_listener_the_report_is_committed_in_place_and_nothing_is_held() {
    let mut hub = standing(false, vec![at(0.5)]);
    let events = drain(&mut hub);
    assert!(moves(&events).is_empty());
    let g = &hub.guests[0];
    assert_eq!(g.position, [0.5, 64.0, 0.0]);
    assert!(g.pending_move.is_none());
    assert!(g.inbox.is_empty());
}

/// What a vetoed report does to the counters: the server's own checks and the anticheat's record
/// of the client's ticks saw it before the hook, and the packets behind it are read after the ruling, in order.
#[test]
fn a_vetoed_report_is_counted_before_the_hook_and_adds_no_violation() {
    let mut hub = standing(true, vec![walking(0.5), tick_end()]);
    let config = horizon_anticheat::Config::default();
    hub.set_anticheat(config.clone());
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let g = &hub.guests[0];
    assert_eq!(g.received_moves, 1, "the speed check's window counted it");
    assert_eq!(
        g.record.as_ref().map(|r| r.ended()),
        Some(0),
        "its tick end waits"
    );
    assert_eq!(
        g.anticheat.timer.ms, 0.0,
        "the timer has not read the tick end"
    );
    hub.resolve_move(conn, false);
    assert!(
        hub.guests[0].moves.is_empty(),
        "its bookkeeping was skipped"
    );
    assert!(moves(&drain(&mut hub)).is_empty());
    let g = &hub.guests[0];
    let record = g.record.as_ref().unwrap();
    assert_eq!(record.ended(), 1, "the tick end was read after the ruling");
    assert_eq!(
        record.ticks().last().and_then(|t| t.position),
        Some([0.5, 64.0, 0.0]),
        "the record holds what the client sent, vetoed or not"
    );
    assert_eq!(g.anticheat.timer.ms, -50.0, "one tick end of the timer");
    for check in horizon_anticheat::Check::ALL {
        assert_eq!(g.anticheat.vl(&config, *check, hub.ticks), 0.0, "{check:?}");
    }
}
