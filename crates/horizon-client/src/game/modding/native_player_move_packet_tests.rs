//! `player_move_packet`: the mods hear a network player's validated movement packet with the
//! packet's flags, and nothing when no mod listens or the guest is gone.
use super::native_guest_turn_tests::with_guests;
use mod_api::{Hooks, PlayerMovePacketEvent, ServerHost};
use std::sync::{Arc, Mutex};

type Heard = Arc<Mutex<Vec<(u128, [f64; 3], [f32; 2], bool, bool, bool)>>>;

struct Recorder(Heard);

#[mod_api::__subscriptions]
impl Hooks for Recorder {
    fn player_move_packet(&self, _: &mut dyn ServerHost, event: &mut PlayerMovePacketEvent) {
        self.0.lock().unwrap().push((
            event.player,
            event.to,
            event.to_rotation,
            event.on_ground,
            event.has_position,
            event.has_rotation,
        ));
    }
}

#[test]
#[ignore = "daily"]
fn a_packet_is_heard_with_its_flags_and_a_gone_guest_is_not() {
    let heard = Heard::default();
    let mods = vec![mod_api::ModEntry::new(
        "recorder",
        Box::new(Recorder(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            game.announce_player_move_packet(
                13,
                [1.0, 64.0, 2.0],
                [30.0, 5.0],
                (true, false, true),
            );
            let heard_now = std::mem::take(&mut *heard.lock().unwrap());
            assert_eq!(heard_now.len(), 1);
            let (_, to, rotation, on_ground, has_position, has_rotation) = heard_now[0];
            assert_eq!((to, rotation), ([1.0, 64.0, 2.0], [30.0, 5.0]));
            assert_eq!((on_ground, has_position, has_rotation), (true, false, true));
            game.announce_player_move_packet(9999, [0.0; 3], [0.0; 2], (true, true, true));
            assert!(heard.lock().unwrap().is_empty(), "no such guest");
        });
    });
}
