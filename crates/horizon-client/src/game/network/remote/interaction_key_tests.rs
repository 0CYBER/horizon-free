use super::*;
use mod_api::{
    ClientHost, InteractionAttempt, InteractionKey, InteractionKeyMappingTriggeredEvent,
};

struct KeyControl {
    cancel: bool,
    swing: bool,
    hand: Hand,
    calls: Arc<Mutex<Vec<Hand>>>,
    packets: Arc<Mutex<Vec<String>>>,
}
#[mod_api::__subscriptions]
impl Hooks for KeyControl {
    fn client_packet_send(
        &self,
        _: &dyn ClientHost,
        e: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        self.packets.lock().unwrap().push(e.packet.id.to_string());
        Flow::Continue
    }
    fn interaction_key_mapping_triggered(
        &self,
        _: &mut dyn ClientHost,
        e: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        self.calls.lock().unwrap().push(e.hand);
        if e.hand == self.hand {
            e.swing_hand = self.swing;
            if self.cancel {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn interaction_key_literal_matrix_preserves_actions_and_swing_independently() {
    for (cancel, swing, actions, swings) in [
        (false, false, 1, 0),
        (false, true, 1, 1),
        (true, false, 0, 0),
        (true, true, 0, 1),
    ] {
        for (key, hand, attempt) in [
            (InteractionKey::Use, Hand::Main, InteractionAttempt::Press),
            (InteractionKey::Use, Hand::Off, InteractionAttempt::Press),
            (
                InteractionKey::Attack,
                Hand::Main,
                InteractionAttempt::Press,
            ),
            (InteractionKey::Attack, Hand::Main, InteractionAttempt::Held),
        ] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let packets = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![mod_api::ModEntry::new(
                "key_matrix",
                Box::new(KeyControl {
                    cancel,
                    swing,
                    hand,
                    calls: calls.clone(),
                    packets: packets.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_interaction_game(true, |game| {
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    let lever = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:lever")
                        .unwrap();
                    game.world.set(0, 63, 0, stone).unwrap();
                    game.world.set(0, 64, 0, lever).unwrap();
                    game.forced_hit = Some(hit());
                    let before = game.remote.as_ref().unwrap().sequence;
                    let before_swings = game.swings_started;
                    if key == InteractionKey::Attack && attempt == InteractionAttempt::Press {
                        game.remote_start_attack();
                    } else if key == InteractionKey::Attack {
                        game.remote_continue_attack(true);
                    } else if hand == Hand::Main {
                        game.remote_use(attempt);
                        if cancel {
                            assert_eq!(*calls.lock().unwrap(), [Hand::Main]);
                        }
                    } else {
                        // Enter the off-hand turn after the main-hand item has passed.
                        game.inventory[game.selected] = None;
                        game.offhand = Some(ItemStack::new("minecraft:stone", 1));
                        game.world.set(0, 64, 0, stone).unwrap();
                        game.remote_use(attempt);
                    }
                    assert_eq!(
                        game.remote.as_ref().unwrap().sequence - before,
                        actions + i32::from(hand == Hand::Off),
                        "{key:?} {hand:?} {attempt:?} cancel={cancel} swing={swing}"
                    );
                    let sent = packets.lock().unwrap();
                    let punch = sent.iter().filter(|p| p.ends_with(":punch")).count();
                    let uses = sent
                        .iter()
                        .filter(|p| p.ends_with(":use_item_on") || p.ends_with(":use_item"))
                        .count();
                    if key == InteractionKey::Use {
                        assert_eq!(punch, 0, "use never sends an extra client swing");
                        assert_eq!(uses, usize::from(!cancel) + usize::from(hand == Hand::Off));
                        if cancel && hand == Hand::Main {
                            assert!(sent.is_empty(), "cancelled use sends nothing: {sent:?}");
                        }
                    } else {
                        assert_eq!(
                            punch,
                            usize::from(swing && (!cancel || attempt == InteractionAttempt::Held))
                        );
                    }
                    drop(sent);
                    assert_eq!(
                        game.swings_started - before_swings,
                        swings,
                        "{key:?} {hand:?} {attempt:?} cancel={cancel} swing={swing}"
                    );
                })
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn pick_keeps_packet_veto_and_never_swings() {
    for cancel in [false, true] {
        let packets = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "pick",
            Box::new(KeyControl {
                cancel,
                swing: true,
                hand: Hand::Main,
                calls: Arc::default(),
                packets: packets.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_interaction_game(true, |game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                use horizon_client::interaction_input::InteractionWorld;
                super::super::RemoteInputWorld(game).pick();
                assert_eq!(
                    packets
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|p| p.ends_with("pick_item_from_block"))
                        .count(),
                    usize::from(!cancel)
                );
                assert_eq!(game.swings_started, 0);
            })
        });
    }
}

#[test]
#[ignore = "daily"]
fn no_listener_preserves_natural_swing_and_unsuccessful_use_stays_quiet() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_interaction_game(true, |game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let lever = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lever")
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.forced_hit = Some(hit());
            let sequence = game.remote.as_ref().unwrap().sequence;
            game.remote_use(InteractionAttempt::Press);
            assert_eq!(game.remote.as_ref().unwrap().sequence - sequence, 1);
            assert_eq!(game.swings_started, 1);
            assert_eq!(
                game.remote.as_ref().unwrap().interaction_swing,
                [true, true]
            );
        })
    });
    let mods = vec![mod_api::ModEntry::new(
        "natural",
        Box::new(KeyControl {
            cancel: false,
            swing: true,
            hand: Hand::Main,
            calls: Arc::default(),
            packets: Arc::default(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            game.inventory[game.selected] = None;
            game.offhand = None;
            game.forced_hit = None;
            game.remote_use(InteractionAttempt::Press);
            assert_eq!(game.swings_started, 0);
        })
    });
}

struct FirstSilent(std::sync::atomic::AtomicBool);
#[mod_api::__subscriptions]
impl Hooks for FirstSilent {
    fn interaction_key_mapping_triggered(
        &self,
        _: &mut dyn ClientHost,
        e: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        if !self.0.swap(true, std::sync::atomic::Ordering::Relaxed) {
            e.swing_hand = false;
        }
        Flow::Continue
    }
}
#[test]
#[ignore = "daily"]
fn silent_policy_does_not_leak_into_next_ordinary_use() {
    let mods = vec![mod_api::ModEntry::new(
        "first_silent",
        Box::new(FirstSilent(std::sync::atomic::AtomicBool::new(false))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_interaction_game(true, |game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let lever = game
                .pack
                .assets
                .registry
                .default_state("minecraft:lever")
                .unwrap();
            game.world.set(0, 63, 0, stone).unwrap();
            game.world.set(0, 64, 0, lever).unwrap();
            game.forced_hit = Some(hit());
            game.remote_use(InteractionAttempt::Press);
            assert_eq!(game.swings_started, 0);
            assert_eq!(
                game.remote.as_ref().unwrap().interaction_swing,
                [true, true]
            );
            game.remote_use(InteractionAttempt::Press);
            assert_eq!(game.swings_started, 1);
            assert_eq!(
                game.remote.as_ref().unwrap().interaction_swing,
                [true, true]
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn no_interaction_listener_attack_mining_offhand_and_pick() {
    for path in 0..4 {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_interaction_game(true, |game| {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 63, 0, stone).unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                game.forced_hit = Some(hit());
                let sequence = game.remote.as_ref().unwrap().sequence;
                let swings = game.swings_started;
                match path {
                    0 => {
                        game.remote_start_attack();
                    }
                    1 => game.remote_continue_attack(true),
                    2 => {
                        game.inventory[game.selected] = None;
                        game.offhand = Some(ItemStack::new("minecraft:stone", 1));
                        game.remote_use(InteractionAttempt::Press);
                    }
                    _ => {
                        use horizon_client::interaction_input::InteractionWorld;
                        super::super::RemoteInputWorld(game).pick();
                    }
                }
                assert_eq!(
                    game.remote.as_ref().unwrap().sequence - sequence,
                    if path == 3 {
                        0
                    } else if path == 2 {
                        2
                    } else {
                        1
                    },
                    "path {path}"
                );
                assert_eq!(
                    game.swings_started - swings,
                    if path == 3 { 0 } else { 1 },
                    "path {path}"
                );
                assert_eq!(
                    game.remote.as_ref().unwrap().interaction_swing,
                    [true, true]
                );
            })
        });
    }
}
