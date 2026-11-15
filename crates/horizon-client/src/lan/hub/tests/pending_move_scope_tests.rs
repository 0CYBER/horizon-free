//! `player_move`: what is not heard (the first tick in a level, a teleport's confirmation, the veto's
//! correction) and what is (the next tick, the first report after a teleport).
use super::pending_move_tests::*;
use super::*;

fn confirm(hub: &Hub, yaw: f32) -> S {
    let (id, at) = hub.guests[0].awaiting_teleport.expect("a teleport waits");
    S::AcceptTeleportation(play::AcceptTeleportation {
        id,
        x: at[0],
        y: at[1],
        z: at[2],
        yaw,
        pitch: 0.0,
    })
}

fn push(hub: &mut Hub, packets: Vec<S>) {
    hub.guests.update(0, |g| {
        g.inbox
            .extend(packets.into_iter().map(InboxItem::uncredited));
    });
}

fn look(hub: &Hub) -> [f32; 2] {
    [hub.guests[0].yaw, hub.guests[0].pitch]
}

#[test]
fn the_first_tick_after_a_join_is_not_heard_and_the_second_is() {
    let mut hub = standing(true, vec![]);
    hub.guests.update(0, |g| g.placed = false);
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.place_new_player(conn, ctx));
    assert_eq!(
        hub.guests[0].level_entered, 10,
        "it entered its level at tick 10"
    );
    hub.guests.update(0, |g| g.load_timeout = 0);
    hub.ticks = 11;
    // The join's teleport is confirmed in the first tick, and the first report comes with it.
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert!(moves(&drain(&mut hub)).is_empty(), "tick 11 is the first");
    assert_eq!(look(&hub), [10.0, 0.0], "committed in place");
    hub.ticks = 12;
    push(&mut hub, vec![turned(20.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn], "tick 12 is the second");
    assert_eq!(look(&hub), [10.0, 0.0]);
}

#[test]
fn a_report_later_in_the_level_after_a_join_is_heard_like_any_other() {
    // The first report after joining may come long after the first tick: it is heard then.
    let mut hub = standing(true, vec![]);
    hub.guests.update(0, |g| {
        g.ticks = 0;
        g.level_entered = 3;
    });
    hub.ticks = 30;
    push(&mut hub, vec![turned(10.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
}

#[test]
fn the_first_tick_after_a_level_change_is_not_heard_and_the_second_is() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.change_level(0, [0.0, 64.0, 0.0], 0.0, None, false, 0, ctx));
    assert_eq!(hub.guests[0].level_entered, 10);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "the confirmation and the first tick are not heard"
    );
    assert_eq!(look(&hub), [10.0, 0.0]);
    assert!(hub.guests[0].pending_move.is_none());
    hub.ticks = 12;
    push(&mut hub, vec![turned(20.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn], "the second tick is");
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([10.0, 0.0], [20.0, 0.0])
    );
}

#[test]
fn a_teleport_in_the_level_is_not_a_first_tick_and_the_first_report_after_it_is_heard() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    assert_eq!(hub.guests[0].level_entered, 0, "the same level");
    hub.ticks = 11;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert_eq!(
        moves(&drain(&mut hub)),
        [conn],
        "the report, not the confirmation"
    );
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        report.from,
        [10.5, 70.0, 10.5],
        "from the teleport's destination"
    );
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([0.0, 0.0], [10.0, 0.0])
    );
}

#[test]
fn a_respawn_is_a_teleport_not_a_first_tick_and_the_first_report_after_it_is_heard() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.respawn(0, [0.5, 64.0, 0.5], Some([0.0; 2]), ctx));
    assert_eq!(hub.guests[0].level_entered, 0, "the same level");
    hub.guests.update(0, |g| g.load_timeout = 0);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(10.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        report.from,
        [0.5, 64.0, 0.5],
        "from the respawn's destination"
    );
    assert_eq!(report.to_rotation, [10.0, 0.0]);
}

#[test]
fn a_confirmation_with_a_changed_rotation_is_validated_and_committed_but_not_heard() {
    let mut hub = standing(true, vec![]);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 90.0);
    push(&mut hub, vec![confirmation]);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "a changed yaw on a confirmation"
    );
    let g = &hub.guests[0];
    assert!(g.awaiting_teleport.is_none(), "it confirmed");
    assert!(g.pending_move.is_none());
    assert_eq!(
        (g.position, g.yaw),
        ([10.5, 70.0, 10.5], 90.0),
        "and committed in place"
    );
}

#[test]
fn a_confirmation_with_a_changed_position_is_not_heard() {
    let mut hub = standing(true, vec![]);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    let (id, _) = hub.guests[0].awaiting_teleport.unwrap();
    // The server's own handling: the accepted position is the teleport's, the packet's own values go
    // through the same validation as any report.
    let moved = S::AcceptTeleportation(play::AcceptTeleportation {
        id,
        x: 10.75,
        y: 70.0,
        z: 10.5,
        yaw: 0.0,
        pitch: 0.0,
    });
    push(&mut hub, vec![moved]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert!(hub.guests[0].pending_move.is_none());
    assert_eq!(
        hub.guests[0].position,
        [10.5, 70.0, 10.5],
        "the teleport's target, not the packet's position"
    );
}

#[test]
fn a_confirmation_never_commits_the_displacement_a_veto_refused() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    hub.resolve_move(conn, false);
    let (id, _) = hub.guests[0].awaiting_teleport.unwrap();
    // The client confirms the correction while claiming the position the veto refused.
    let refused = S::AcceptTeleportation(play::AcceptTeleportation {
        id,
        x: 0.5,
        y: 64.0,
        z: 0.0,
        yaw: 0.0,
        pitch: 0.0,
    });
    push(&mut hub, vec![refused]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert!(hub.guests[0].awaiting_teleport.is_none(), "it confirmed");
    assert_eq!(
        hub.guests[0].position,
        [0.0, 64.0, 0.0],
        "the correction's target"
    );
}

#[test]
fn the_correction_a_veto_causes_is_not_heard_when_the_client_confirms_it() {
    let mut hub = standing(true, vec![turned(10.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    hub.resolve_move(conn, false);
    hub.ticks = 11;
    // The client confirms with the look it has now, which differs from the accepted one.
    let confirmation = confirm(&hub, 10.0);
    push(&mut hub, vec![confirmation]);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "the correction is not heard"
    );
    assert!(hub.guests[0].pending_move.is_none());
    assert_eq!(
        look(&hub),
        [10.0, 0.0],
        "the confirmation was validated and committed"
    );
}

fn baseline(hub: &Hub) -> ([f64; 3], [f32; 2]) {
    hub.guests[0].last_reported
}

#[test]
fn the_baseline_is_the_destination_of_every_server_side_teleport() {
    let mut hub = standing(true, vec![]);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    assert_eq!(
        baseline(&hub),
        ([10.5, 70.0, 10.5], [0.0, 0.0]),
        "a teleport"
    );
    hub.resend_teleport(0, Some([11.5, 71.0, 11.5]));
    assert_eq!(baseline(&hub), ([11.5, 71.0, 11.5], [0.0, 0.0]), "a resend");
    with_input_context(|ctx| hub.respawn(0, [0.5, 64.0, 0.5], Some([30.0, 5.0]), ctx));
    assert_eq!(baseline(&hub), ([0.5, 64.0, 0.5], [30.0, 5.0]), "a respawn");
    with_input_context(|ctx| hub.change_level(0, [100.5, 49.0, 0.5], 90.0, None, false, 0, ctx));
    assert_eq!(
        baseline(&hub),
        ([100.5, 49.0, 0.5], [90.0, 5.0]),
        "a level change"
    );
}

#[test]
fn the_baseline_is_where_the_player_joined() {
    let mut hub = standing(true, vec![]);
    hub.guests.update(0, |g| {
        g.placed = false;
        g.last_reported = ([9.0; 3], [9.0; 2]);
    });
    let conn = hub.guests[0].conn;
    with_input_context(|ctx| hub.place_new_player(conn, ctx));
    let g = &hub.guests[0];
    assert_eq!(g.last_reported, (g.position, [g.yaw, g.pitch]));
}

#[test]
fn a_report_is_the_baseline_before_the_listener_runs_and_a_veto_puts_it_back() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    assert_eq!(
        baseline(&hub),
        ([0.5, 64.0, 0.0], [0.0, 0.0]),
        "the position given to the listener"
    );
    assert_eq!(
        hub.guests[0].position,
        [0.0, 64.0, 0.0],
        "the accepted one has not moved"
    );
    hub.resolve_move(conn, false);
    assert_eq!(
        baseline(&hub),
        ([0.0, 64.0, 0.0], [0.0, 0.0]),
        "the correction's destination"
    );
}

#[test]
fn a_report_equal_to_the_baseline_fires_nothing_though_the_accepted_look_differs() {
    // A confirmation with yaw 90 is committed, and the baseline stays the teleport's destination.
    let mut hub = standing(true, vec![]);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 90.0);
    push(&mut hub, vec![confirmation]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert_eq!(look(&hub), [90.0, 0.0], "accepted");
    assert_eq!(baseline(&hub).1, [0.0, 0.0], "baseline");
    // Back to the destination's look: equal to the baseline, so nothing fires, and it is committed.
    hub.ticks = 12;
    push(&mut hub, vec![turned(0.0)]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert_eq!(look(&hub), [0.0, 0.0]);
}

#[test]
fn a_report_that_differs_from_the_baseline_is_heard_from_the_baseline_not_the_accepted_look() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    let confirmation = confirm(&hub, 90.0);
    push(&mut hub, vec![confirmation]);
    drain(&mut hub);
    hub.ticks = 12;
    push(&mut hub, vec![turned(90.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (report.from_rotation, report.to_rotation),
        ([0.0, 0.0], [90.0, 0.0]),
        "the accepted yaw is 90 already, the baseline's is 0"
    );
}

#[test]
fn a_report_while_a_teleport_is_unconfirmed_is_not_heard_and_the_baseline_stays() {
    let mut hub = standing(true, vec![]);
    let conn = hub.guests[0].conn;
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    hub.ticks = 11;
    push(&mut hub, vec![at(20.0), turned(45.0)]);
    assert!(moves(&drain(&mut hub)).is_empty());
    assert_eq!(baseline(&hub), ([10.5, 70.0, 10.5], [0.0, 0.0]));
    assert_eq!(
        hub.guests[0].position,
        [10.5, 70.0, 10.5],
        "nothing applied"
    );
    // Confirmed: the first report after it is heard from the destination.
    hub.ticks = 12;
    let confirmation = confirm(&hub, 0.0);
    push(&mut hub, vec![confirmation, turned(45.0)]);
    assert_eq!(moves(&drain(&mut hub)), [conn]);
    let report = hub.pending_move_report(conn).unwrap();
    assert_eq!(
        (report.from, report.from_rotation),
        ([10.5, 70.0, 10.5], [0.0, 0.0])
    );
}

#[test]
fn an_unheard_report_in_place_moves_the_baseline_with_it() {
    let mut hub = standing(false, vec![at(0.5)]);
    drain(&mut hub);
    assert_eq!(
        baseline(&hub),
        ([0.5, 64.0, 0.0], [0.0, 0.0]),
        "no listener"
    );
    let mut hub = standing(true, vec![turned(10.0)]);
    hub.guests.update(0, |g| g.level_entered = 9);
    assert!(
        moves(&drain(&mut hub)).is_empty(),
        "the first tick in the level"
    );
    assert_eq!(
        baseline(&hub),
        ([0.0, 64.0, 0.0], [10.0, 0.0]),
        "first tick"
    );
}
