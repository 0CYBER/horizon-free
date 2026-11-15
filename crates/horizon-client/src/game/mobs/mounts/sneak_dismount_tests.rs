//! A sneaking rider gets off its mount (`Player.rideTick`: `wantsToStopRiding`, which is
//! `isShiftKeyDown`), unless an `entity_mount` listener cancels the dismount (NeoForge's
//! `EntityMountEvent`). Kept on, the mount still reads that tick's input, sneak included
//! (`LivingEntity.getRiddenInput` reads the controller), on the local player's route and on a
//! server player's (arismoko/horizon#2739).
use super::camel_jump_tests::game;
use crate::game::Game;
use horizon_client::{mobs::equine, sprint::Keys};
use mod_api::{EntityMountEvent, Flow, Hooks, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

/// Cancels every dismount while `keep` is set, and counts the dismounts it hears.
struct KeepRider {
    keep: Arc<AtomicBool>,
    leaves: Arc<AtomicU32>,
}
#[mod_api::__subscriptions]
impl Hooks for KeepRider {
    fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
        if event.mounting {
            return Flow::Continue;
        }
        self.leaves.fetch_add(1, Ordering::SeqCst);
        if self.keep.load(Ordering::SeqCst) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Runs `test` on a server game with a horse (its id) and the `KeepRider` mod.
pub(in crate::game) fn with_horse(test: impl FnOnce(&mut Game<'_>, u32, &AtomicBool, &AtomicU32)) {
    let keep = Arc::new(AtomicBool::new(true));
    let leaves = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "keep_rider",
        Box::new(KeepRider {
            keep: keep.clone(),
            leaves: leaves.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut game = game();
        // A server hears its mounts' `entity_mount`.
        game.options.dedicated = Some(Vec::new());
        assert!(game.run_command("summon horse 8.5 100 8.5 {NoAI:1b,Silent:1b,Tame:1b}"));
        let horse = game
            .mobs
            .iter()
            .find(|m| &*m.kind.id == "minecraft:horse")
            .unwrap()
            .id;
        test(&mut game, horse, &keep, &leaves);
    });
}

fn rider(game: &Game<'_>, horse: u32) -> Option<equine::Rider> {
    game.mob_object(horse).unwrap().rider
}

#[test]
#[ignore = "daily"]
fn underwater_dismount_obeys_the_listener_without_sneak() {
    with_horse(|game, horse, keep, leaves| {
        game.riding = Some(horse);
        assert!(game.run_command("fill 0 100 0 15 104 15 water"));
        assert!(game.rider_eyes_in_water(), "exercise the underwater branch");
        let keys = Keys {
            forward: true,
            ..Keys::default()
        };
        assert!(game.tick_mount_riding(keys));
        assert_eq!(leaves.load(Ordering::SeqCst), 1);
        assert_eq!(game.riding, Some(horse));
        let input = rider(game, horse).unwrap();
        assert_eq!(input.forward, 1.0);
        assert!(!input.sneak);
        keep.store(false, Ordering::SeqCst);
        assert!(game.tick_mount_riding(keys));
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
        assert_eq!(game.riding, None);
        assert!(rider(game, horse).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn a_dead_mount_never_receives_input_even_when_dismount_is_cancelled() {
    with_horse(|game, horse, keep, leaves| {
        game.riding = Some(horse);
        let i = game.mount_index().unwrap();
        game.mobs[i].health = 0.0;
        for cancelled in [true, false] {
            keep.store(cancelled, Ordering::SeqCst);
            assert!(game.tick_mount_riding(Keys {
                forward: true,
                jump: true,
                ..Keys::default()
            }));
            assert!(rider(game, horse).is_none(), "dead mounts receive no input");
            assert_eq!(game.riding, cancelled.then_some(horse));
        }
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
    });
}

#[test]
#[ignore = "daily"]
fn a_kept_local_rider_still_drives_and_sneaks() {
    with_horse(|game, horse, keep, leaves| {
        game.riding = Some(horse);
        let keys = Keys {
            forward: true,
            left: true,
            jump: true,
            shift: true,
            ..Keys::default()
        };
        assert!(game.tick_mount_riding(keys));
        assert_eq!(leaves.load(Ordering::SeqCst), 1, "the dismount was asked");
        assert_eq!(game.riding, Some(horse), "the listener kept the rider on");
        let r = rider(game, horse).expect("the kept mount reads the rider");
        assert_eq!((r.strafe, r.forward), (1.0, 1.0));
        assert!(r.jumping, "the jump key reaches the mount");
        assert!(r.sneak, "the mount sees sneak held");
        assert!(!r.client, "the local player's mount moves here");

        // Without the listener's veto, sneaking gets the rider off as before.
        keep.store(false, Ordering::SeqCst);
        assert!(game.tick_mount_riding(Keys {
            shift: true,
            ..Keys::default()
        }));
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
        assert_eq!(game.riding, None);
        assert!(rider(game, horse).is_none(), "the mount lost its rider");
    });
}

#[test]
#[ignore = "daily"]
fn a_kept_server_player_still_drives_and_sneaks() {
    // `PlayerInput` flags: forward 1, left 4, jump 16, shift 32.
    const INPUT: u8 = 1 | 4 | 16 | 32;
    const SHIFT: u8 = 32;
    with_horse(|game, horse, keep, leaves| {
        let set_input = |game: &mut Game<'_>, input: u8| {
            let guests = &mut game.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                (g.vehicle, g.vehicle_order, g.input) = (Some(horse), 1, input);
            });
        };
        let vehicle = |game: &Game<'_>| game.lan.as_ref().unwrap().hub.guests[0].vehicle;
        set_input(game, INPUT);
        game.guest_vehicle_tick(2);
        assert_eq!(leaves.load(Ordering::SeqCst), 1, "the dismount was asked");
        assert_eq!(vehicle(game), Some(horse), "the listener kept the guest on");
        let r = rider(game, horse).expect("the kept mount reads the guest");
        assert_eq!((r.strafe, r.forward), (1.0, 1.0));
        assert!(r.jumping, "the jump key reaches the mount");
        assert!(r.sneak, "the mount sees sneak held");
        assert!(r.client, "the guest's own client moves its horse");

        // Without the listener's veto, sneaking gets the guest off as before.
        keep.store(false, Ordering::SeqCst);
        set_input(game, SHIFT);
        game.guest_vehicle_tick(2);
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
        assert_eq!(vehicle(game), None);
        assert!(rider(game, horse).is_none(), "the mount lost its rider");
    });
}
