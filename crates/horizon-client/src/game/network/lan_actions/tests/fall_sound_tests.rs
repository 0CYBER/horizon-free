use super::*;
use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_item_places::tests::{guest, open_to_lan};
use horizon_client::{
    chunk::Chunk,
    lan::{Out, PlayerSender},
};
use std::sync::{Arc, mpsc};

fn run(distance: f64, expected: usize) {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| {
            open_to_lan(game);
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 63, 1, stone).unwrap();
            let mut landing = guest(1);
            landing.position = [1.5, 64.0, 1.5];
            landing.loaded = true;
            landing.moves_from_ground = false;
            landing.moves.push(([0.0, -distance, 0.0], true));
            let mut motion = PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap();
            // The replayed downward move accumulates the fall distance.
            motion.fall_distance = 0.0;
            landing.motion = Some(Box::new(motion));
            let (tx, own) = mpsc::channel();
            landing.sender = PlayerSender::new(tx, Arc::default());
            let mut observer = guest(2);
            observer.position = landing.position;
            observer.loaded = true;
            let (tx, other) = mpsc::channel();
            observer.sender = PlayerSender::new(tx, Arc::default());
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .extend([landing, observer]);
            let target = match game.lan.as_ref().unwrap().hub.sound_packet(
                "minecraft:block.stone.fall",
                [1.5, 64.0, 1.5],
                0.5,
                0.75,
                4,
                0,
            ) {
                C::Sound(s) => s.sound,
                _ => unreachable!(),
            };
            game.guest_server_tick(1);
            let sounds = |rx: mpsc::Receiver<Out>| {
                rx.try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(b) => vec![b],
                        Out::Packets(b) => b,
                        _ => vec![],
                    })
                    .filter_map(|body| {
                        let mut r = horizon_protocol::buf::Reader::new(&body);
                        let id = r.varint().unwrap();
                        match C::decode(id, r.rest()).unwrap() {
                            C::Sound(s) if s.sound == target => Some((s.source, s.volume, s.pitch)),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            // Source 7 is `SoundSource.PLAYERS`.
            assert_eq!(sounds(other), vec![(7, 0.5, 0.75); expected]);
            assert!(sounds(own).is_empty(), "landing guest must be excluded");
        })
    });
}

#[test]
#[ignore = "daily"]
fn five_block_fall_sends_stone_once_except_guest() {
    run(5.0, 1);
}
#[test]
#[ignore = "daily"]
fn three_block_fall_sends_no_stone_sound() {
    run(3.0, 0);
}
