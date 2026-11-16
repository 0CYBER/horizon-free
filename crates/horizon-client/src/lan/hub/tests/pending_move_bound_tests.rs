//! `player_move`: how long a report may stay pending, and what ends one early.
use super::pending_move_tests::*;

#[test]
fn the_20th_tick_is_the_last_that_can_be_ruled_on() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    // Held at tick 10 as the first: tick 29 is the 20th.
    hub.ticks = 29;
    hub.resolve_move(conn, true);
    assert_eq!(hub.guests[0].position, [0.5, 64.0, 0.0], "ruled on in time");
    assert!(hub.guests[0].awaiting_teleport.is_none());
}

#[test]
fn the_21st_tick_finds_a_ruling_too_late_and_drops_the_report_with_the_correction() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    hub.ticks = 30;
    hub.resolve_move(conn, true);
    let g = &hub.guests[0];
    assert_eq!(
        g.position,
        [0.0, 64.0, 0.0],
        "never committed, though the mods said yes"
    );
    assert!(g.awaiting_teleport.is_some(), "the client was corrected");
    assert!(g.pending_move.is_none());
}

#[test]
fn a_report_nobody_rules_on_holds_its_inbox_to_the_20th_tick_and_is_dropped_at_the_21st() {
    let mut hub = standing(true, vec![at(0.5), tick_end()]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    hub.ticks = 29;
    drain(&mut hub);
    assert!(
        hub.guests[0].pending_move.is_some(),
        "still held at the 20th tick"
    );
    assert_eq!(hub.guests[0].inbox.len(), 1);
    hub.ticks = 30;
    drain(&mut hub);
    let g = &hub.guests[0];
    assert!(g.pending_move.is_none());
    assert_eq!(g.position, [0.0, 64.0, 0.0], "never committed");
    assert!(g.awaiting_teleport.is_some(), "the client was corrected");
    assert!(g.inbox.is_empty(), "the inbox was released");
    hub.resolve_move(conn, true);
    assert_eq!(
        hub.guests[0].position,
        [0.0, 64.0, 0.0],
        "a late ruling finds nothing"
    );
}

#[test]
fn a_teleport_or_a_level_change_drops_a_pending_report_without_a_correction() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    hub.teleport_command(0, [10.5, 70.0, 10.5], [false; 3]);
    assert!(hub.guests[0].pending_move.is_none());
    let counter = hub.guests[0].teleport_counter;
    hub.resolve_move(conn, true);
    assert_eq!(
        hub.guests[0].position,
        [10.5, 70.0, 10.5],
        "the teleport stands"
    );
    assert_eq!(
        hub.guests[0].teleport_counter, counter,
        "no second correction"
    );

    let mut hub = standing(true, vec![at(0.5)]);
    drain(&mut hub);
    hub.guests
        .update(0, |g| g.dimension = "minecraft:the_nether".into());
    let counter = hub.guests[0].teleport_counter;
    hub.resolve_move(conn, true);
    let g = &hub.guests[0];
    assert_eq!(
        g.position,
        [0.0, 64.0, 0.0],
        "not committed into another level"
    );
    assert_eq!(g.teleport_counter, counter, "and not corrected");
}

#[test]
fn a_player_who_left_has_nothing_pending_and_a_late_ruling_is_ignored() {
    let mut hub = standing(true, vec![at(0.5)]);
    let conn = hub.guests[0].conn;
    drain(&mut hub);
    let gone = hub.remove_guest(0, "gone");
    assert!(gone.pending_move.is_some(), "it leaves with the guest");
    hub.resolve_move(conn, true);
    hub.resolve_move(conn, false);
    assert!(hub.pending_move_report(conn).is_none());
}
