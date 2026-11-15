//! `player_start_spectating_entity` and `player_stop_spectating_entity` heard through a guest's
//! camera (`ServerPlayer.setCamera`): one start per new target, one stop when the view returns,
//! nothing for a target that is gone.
use horizon_client::lan::{Joined, PlayerSender, guest::Guest};
use horizon_protocol::packets::configuration::ClientInformation;
use mod_api::{
    Hooks, ModEntry, PlayerStartSpectatingEntityEvent, PlayerStopSpectatingEntityEvent, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, AtomicUsize},
};

/// One heard moment: start or stop, the spectator's UUID and name, the target's id, and where
/// the spectator's entity stood when the listener ran.
type Heard = (&'static str, u128, String, i32, Option<[f64; 3]>);

struct Probe(Arc<Mutex<Vec<Heard>>>);

/// Where the spectator `player` (their UUID is their entity id in these tests) stands.
fn at(host: &dyn ServerHost, player: u128) -> Option<[f64; 3]> {
    host.entity(u32::try_from(player).ok()?)?.position(host)
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn player_start_spectating_entity(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerStartSpectatingEntityEvent<'_>,
    ) {
        let position = at(host, event.player);
        self.0.lock().unwrap().push((
            "start",
            event.player,
            event.name.to_owned(),
            event.entity_id,
            position,
        ));
    }
    fn player_stop_spectating_entity(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerStopSpectatingEntityEvent<'_>,
    ) {
        let position = at(host, event.player);
        self.0.lock().unwrap().push((
            "stop",
            event.player,
            event.name.to_owned(),
            event.entity_id,
            position,
        ));
    }
}

/// Guest 14 joins the two of `with_guests` (12 and 13, both at 3, 64, 5), standing at `position`.
fn third_guest(game: &mut super::Game<'_>, position: [f64; 3]) {
    let (tx, _rx) = std::sync::mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 14,
            uuid: 14,
            name: "guest14".to_owned(),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
            expected: false,
        },
        14,
        position,
        10,
    );
    guest.placed = true;
    game.lan.as_mut().unwrap().hub.guests.push(guest);
}

/// Runs `test` with the probe listening and three guests, guest 14 standing apart at `FAR`.
fn with_probe(test: impl FnOnce(&mut super::Game<'_>, &dyn Fn() -> Vec<Heard>)) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("spectating", Box::new(Probe(heard.clone())))];
    mod_platform::testing::with_mods(mods.into(), || {
        super::native_guest_turn_tests::with_guests(|game| {
            third_guest(game, FAR);
            let take = || std::mem::take(&mut *heard.lock().unwrap());
            test(game, &take);
        });
    });
}

const FAR: [f64; 3] = [9.5, 70.0, 9.5];
const NEAR: [f64; 3] = [3.0, 64.0, 5.0];

fn camera(game: &super::Game<'_>, conn: u32) -> Option<i32> {
    game.cmds.guest_cameras.get(&conn).map(|c| c.id)
}

#[test]
#[ignore = "daily"]
fn a_guests_camera_is_heard_starting_once_and_stopping() {
    with_probe(|game, take| {
        // Guest 12 (index 0) watches guest 13's entity.
        game.guest_set_camera(0, Some(13));
        assert_eq!(
            take(),
            [("start", 12, "guest12".to_owned(), 13, Some(NEAR))]
        );
        // The same target again changes nothing and is not heard.
        game.guest_set_camera(0, Some(13));
        assert!(take().is_empty());
        // Their own entity is no camera: it is the return to the player.
        game.guest_set_camera(0, Some(12));
        assert_eq!(take(), [("stop", 12, "guest12".to_owned(), 13, Some(NEAR))]);
        // Without a camera, returning to the player hears nothing.
        game.guest_set_camera(0, None);
        assert!(take().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn switching_straight_to_another_target_is_one_start_and_no_stop() {
    with_probe(|game, take| {
        game.guest_set_camera(0, Some(13));
        take();
        game.guest_set_camera(0, Some(14));
        assert_eq!(take(), [("start", 12, "guest12".to_owned(), 14, Some(FAR))]);
        assert_eq!(camera(game, 12), Some(14));
    });
}

#[test]
#[ignore = "daily"]
fn a_target_that_is_gone_sets_no_camera_and_is_not_heard() {
    with_probe(|game, take| {
        game.guest_set_camera(0, Some(9999));
        assert!(take().is_empty());
        assert_eq!(camera(game, 12), None);
        // Nor while watching someone: the camera stays where it was.
        game.guest_set_camera(0, Some(13));
        take();
        game.guest_set_camera(0, Some(9999));
        assert!(take().is_empty());
        assert_eq!(camera(game, 12), Some(13));
    });
}

#[test]
#[ignore = "daily"]
fn the_start_is_heard_once_the_camera_has_moved() {
    with_probe(|game, take| {
        // Guest 12 stands at NEAR; guest 14 at FAR. The listener reads 12 already at FAR: the
        // view moved to the target before the hook ran.
        game.guest_set_camera(0, Some(14));
        assert_eq!(take(), [("start", 12, "guest12".to_owned(), 14, Some(FAR))]);
        assert_eq!(camera(game, 12), Some(14));
    });
}
