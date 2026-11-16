//! `player_move`: the hook hears a held report with the player's state swapped in, and its
//! veto decides what the hub does with it. The ordered cases run the real game tick.
use super::native_guest_turn_tests::with_guests;
use horizon_client::lan::hub::{Accepted, PendingMove};
use mod_api::{Flow, Hooks, PlayerMoveEvent, ServerHost};
use std::sync::{Arc, Mutex};

type Heard = Arc<Mutex<Vec<(u128, [f64; 3], [f32; 2], [f64; 3], [f32; 2])>>>;

/// Records each report and vetoes one that goes past x = 10, as a world wall does.
struct Wall(Heard);

#[mod_api::__subscriptions]
impl Hooks for Wall {
    fn player_move(&self, _: &mut dyn ServerHost, event: &mut PlayerMoveEvent) -> Flow {
        self.0.lock().unwrap().push((
            event.player,
            event.from,
            event.from_rotation,
            event.to,
            event.to_rotation,
        ));
        if event.to[0] > 10.0 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn hold(game: &mut super::Game<'_>, conn: u32, to: [f64; 3], yaw: f32) {
    let hub = &mut game.lan.as_mut().unwrap().hub;
    let i = hub.guests.iter().position(|g| g.conn == conn).unwrap();
    let (teleports, dimension) = (
        hub.guests[i].teleport_counter,
        hub.guests[i].dimension.clone(),
    );
    let report = Accepted {
        to,
        yaw,
        pitch: 0.0,
        flags: 1,
        stands: true,
        floating: false,
    };
    let held = PendingMove {
        report,
        since: hub.ticks,
        teleports,
        dimension,
        from: hub.guests[i].last_reported,
        paused: true,
    };
    hub.guests.update(i, |g| g.pending_move = Some(held));
}

#[test]
#[ignore = "daily"]
fn a_veto_corrects_the_player_and_a_pass_commits_the_report() {
    let heard = Heard::default();
    let mods = vec![mod_api::ModEntry::new(
        "wall",
        Box::new(Wall(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            // A report inside the wall: heard with the exact values, then committed.
            hold(game, 13, [5.0, 64.0, 5.0], 40.0);
            game.lan_player_move(13);
            assert_eq!(
                std::mem::take(&mut *heard.lock().unwrap()),
                [(
                    13,
                    [3.0, 64.0, 5.0],
                    [0.0, 0.0],
                    [5.0, 64.0, 5.0],
                    [40.0, 0.0]
                )]
            );
            let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!((g.position, g.yaw), ([5.0, 64.0, 5.0], 40.0));
            assert!(g.pending_move.is_none());
            // A report past it: vetoed, the player stays and the client is corrected.
            hold(game, 13, [20.0, 64.0, 5.0], 90.0);
            game.lan_player_move(13);
            assert_eq!(heard.lock().unwrap().len(), 1);
            let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
            assert_eq!((g.position, g.yaw), ([5.0, 64.0, 5.0], 40.0));
            assert!(
                g.awaiting_teleport.is_some(),
                "corrected through the teleport"
            );
            // Nothing pending any more: the mods are not asked again.
            heard.lock().unwrap().clear();
            game.lan_player_move(13);
            assert!(heard.lock().unwrap().is_empty());
        });
    });
}

/// Records each report and rules on it the same way every time.
struct Ruling(Heard, Flow);

#[mod_api::__subscriptions]
impl Hooks for Ruling {
    fn player_move(&self, _: &mut dyn ServerHost, event: &mut PlayerMoveEvent) -> Flow {
        self.0.lock().unwrap().push((
            event.player,
            event.from,
            event.from_rotation,
            event.to,
            event.to_rotation,
        ));
        self.1
    }
}

/// Guest 13 stands where it was placed, loaded, long past its first tick, with two turns of its
/// head (yaw 10, then 20) in its inbox, both in the one client tick.
fn two_turns_waiting(game: &mut super::Game<'_>) {
    use horizon_client::lan::guest::InboxItem;
    use horizon_protocol::packets::play::{self, Serverbound as S};
    let hub = &mut game.lan.as_mut().unwrap().hub;
    hub.ticks = 10;
    hub.update_guest(13, |g| {
        g.load_timeout = 0;
        g.first_good = g.position;
        (g.yaw, g.pitch) = (0.0, 0.0);
        g.level_entered = 0;
        g.last_reported = (g.position, [0.0, 0.0]);
        for yaw in [10.0, 20.0] {
            let turn = S::MovePlayerRot(play::MovePlayerRot {
                yaw,
                pitch: 0.0,
                flags: 1,
            });
            g.inbox.push_back(InboxItem::Packet(turn, None));
        }
    });
}

/// One server tick of the real game: the poll, the guests' packets, and the events the hub tells
/// the game, with the mods' ruling in them.
fn game_tick(game: &mut super::Game<'_>) {
    game.level_pass.packets_drained = false;
    game.tick_lan_packets();
}

/// The accepted look of guest 13, its inbox length, and whether a correction waits for it.
fn state(game: &super::Game<'_>) -> ([f32; 2], usize, bool) {
    let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
    (
        [g.yaw, g.pitch],
        g.inbox.len(),
        g.awaiting_teleport.is_some(),
    )
}

const AT: [f64; 3] = [3.0, 64.0, 5.0];

#[test]
#[ignore = "daily"]
fn two_reports_in_one_tick_are_heard_in_order_each_from_the_one_before_when_both_pass() {
    let heard = Heard::default();
    let mods = vec![mod_api::ModEntry::new(
        "pass",
        Box::new(Ruling(heard.clone(), Flow::Continue)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            two_turns_waiting(game);
            game_tick(game);
            assert_eq!(
                std::mem::take(&mut *heard.lock().unwrap()),
                [
                    (13, AT, [0.0, 0.0], AT, [10.0, 0.0]),
                    (13, AT, [10.0, 0.0], AT, [20.0, 0.0])
                ],
                "both in the one tick, the second from the first's destination"
            );
            assert_eq!(state(game), ([20.0, 0.0], 0, false));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_veto_of_the_first_of_two_reports_in_one_tick_corrects_and_the_second_is_not_heard() {
    let heard = Heard::default();
    let mods = vec![mod_api::ModEntry::new(
        "veto",
        Box::new(Ruling(heard.clone(), Flow::Cancel)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            two_turns_waiting(game);
            game_tick(game);
            assert_eq!(
                std::mem::take(&mut *heard.lock().unwrap()),
                [(13, AT, [0.0, 0.0], AT, [10.0, 0.0])],
                "only the first: the second is not heard while the correction is unconfirmed"
            );
            assert_eq!(
                state(game),
                ([0.0, 0.0], 0, true),
                "corrected and not applied; the second report met the unconfirmed teleport"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn two_reports_in_one_tick_with_no_listener_are_committed_in_the_one_pass() {
    with_guests(|game| {
        two_turns_waiting(game);
        game_tick(game);
        assert_eq!(state(game), ([20.0, 0.0], 0, false));
        let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert!(g.pending_move.is_none(), "nothing was held");
    });
}
