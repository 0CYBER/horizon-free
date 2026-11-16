// Test module extracted without changing its logical path.
use super::super::native_entity_refs::tests::with_game;
use super::super::native_item_places::tests::{guest, open_to_lan};
use horizon_client::items::ItemStack;
use mod_api::{Host, HostSide, PlayerRef};
use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq)]
struct Snapshot {
    phase: &'static str,
    side: &'static str,
    uuid: u128,
    diamonds: u32,
    xp: i32,
    cooldown: Option<i32>,
}

struct Log(Arc<Mutex<Vec<Snapshot>>>);
impl Log {
    fn record(&self, side: HostSide<'_>, player: PlayerRef, phase: &'static str) {
        let (host, side, cooldown): (&dyn Host, _, _) = match side {
            HostSide::Server(host) => {
                assert_eq!(
                    host.players().acting_player().unwrap().uuid(),
                    player.uuid()
                );
                let cooldown = player
                    .item_cooldowns(host)
                    .unwrap()
                    .first()
                    .map(|c| c.ticks_left);
                (host, "server", cooldown)
            }
            HostSide::Client(host) => {
                if host.local_player() != Some(player) {
                    assert_eq!(player.uuid(), 789);
                    assert_eq!(player.entity().id(), 66);
                    assert!(player.inventory(host).is_none());
                }
                (host, "client", None)
            }
        };
        self.0.lock().unwrap().push(Snapshot {
            phase,
            side,
            uuid: player.uuid(),
            diamonds: player
                .inventory(host)
                .unwrap_or_default()
                .iter()
                .filter(|s| {
                    mod_api::__private::item_key_id(s.item()).unwrap() == "minecraft:diamond"
                })
                .map(|s| s.count())
                .sum(),
            xp: player.experience(host).map_or(0, |xp| xp.total),
            cooldown,
        });
    }
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Log {
    fn player_tick_pre(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPreEvent) {
        self.record(host, event.player, "pre");
    }
    fn player_tick_post(&self, host: HostSide<'_>, event: &mut mod_api::PlayerTickPostEvent) {
        self.record(host, event.player, "post");
    }
    fn can_continue_sleeping(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::CanContinueSleepingEvent<'_>,
    ) -> Option<bool> {
        // The sleep rule runs inside the tick, never ahead of Pre.
        let heard = self.0.lock().unwrap();
        assert_eq!(heard.last().unwrap().phase, "pre");
        assert_eq!(heard.last().unwrap().uuid, event.entity);
        Some(false)
    }
}

#[test]
#[ignore = "daily"]
fn guest_ticks_before_player_loaded_and_after_its_timeout() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "log",
        Box::new(Log(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            let mut g = guest(1);
            assert!(!g.loaded);
            assert!(!g.client_loaded());
            g.food = 10;
            g.saturation = 5.0;
            g.effects
                .add(horizon_client::effects::EffectInstance::new("speed", 10, 0));
            let mut motion = horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::ZERO,
            )
            .unwrap();
            motion.health.food.exhaustion = 5.0;
            g.motion = Some(Box::new(motion));
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.push(g);
            host.play
                .entry(1)
                .or_default()
                .cooldowns
                .add("minecraft:ender_pearl".into(), 3);

            // A new join/respawn waits 60 ticks. A dimension change preserves
            // the connection's load timer, including an already elapsed one.
            for (timeout, effect_ticks, cooldown) in [(60, 9, 2), (0, 8, 1)] {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.load_timeout = timeout);
                heard.lock().unwrap().clear();
                game.tick_guests();
                let observations = heard.lock().unwrap();
                assert_eq!(
                    observations
                        .iter()
                        .map(|s| (s.phase, s.uuid))
                        .collect::<Vec<_>>(),
                    [("pre", 123), ("post", 123)]
                );
                assert_eq!(observations[0].cooldown, Some(cooldown + 1));
                assert_eq!(observations[1].cooldown, Some(cooldown));
                let g = &game.lan.as_ref().unwrap().hub.guests[0];
                assert!(!g.loaded);
                assert_eq!(g.load_timeout, timeout);
                assert_eq!(g.client_loaded(), timeout == 0);
                assert_eq!(g.position, [0.0; 3]);
                assert_eq!(g.effects.get("speed").unwrap().duration, effect_ticks);
                assert_eq!((g.food, g.saturation), (10, 4.0));
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn guest_tick_brackets_sleep_cooldowns_pickups_and_death() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "log",
        Box::new(Log(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            let mut g = guest(1);
            g.loaded = true;
            g.bed = Some([0, 0, 0]);
            g.sleeping = true;
            g.sleep_timer = 4;
            let mut other = guest(2);
            other.loaded = true;
            other.position = [50.0, 0.0, 50.0];
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.extend([g, other]);
            host.play
                .entry(1)
                .or_default()
                .cooldowns
                .add("minecraft:ender_pearl".into(), 3);
            let diamond = ItemStack::new("minecraft:diamond", 2);
            game.spawn_item(diamond.clone(), [0.0; 3], [0.0; 3], 0);
            let level = game.level("minecraft:overworld").unwrap();
            assert!(game.level_award_experience(level, 0.0, 0.0, 0.0, 7));
            game.tick_guests();
            let observations = heard.lock().unwrap();
            assert_eq!(
                observations
                    .iter()
                    .map(|s| (s.phase, s.uuid))
                    .collect::<Vec<_>>(),
                [("pre", 123), ("post", 123), ("pre", 124), ("post", 124)]
            );
            assert_eq!(
                observations[0],
                Snapshot {
                    phase: "pre",
                    side: "server",
                    uuid: 123,
                    diamonds: 0,
                    xp: 0,
                    cooldown: Some(3)
                }
            );
            assert_eq!(
                observations[1],
                Snapshot {
                    phase: "post",
                    side: "server",
                    uuid: 123,
                    diamonds: 2,
                    xp: 7,
                    cooldown: Some(2)
                }
            );
            assert_eq!(observations[2].diamonds, 0);
            drop(observations);
            // The missing bed is a living-entity continuation failure: immediate wake.
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].sleep_timer, 0);
            assert!(game.lan.as_ref().unwrap().hub.guests[0].bed.is_none());
            assert!(game.inventory.iter().all(Option::is_none));
            heard.lock().unwrap().clear();
            // A real zero-health death-screen tick still gets both endpoints,
            // without use/cooldown/pickup work re-enabled.
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.dead = true;
                g.health = 0.0;
            });
            game.spawn_item(diamond, [0.0; 3], [0.0; 3], 0);
            assert!(game.level_award_experience(level, 0.0, 0.0, 0.0, 7));
            game.tick_guests();
            let observations = heard.lock().unwrap();
            assert_eq!(
                observations[0],
                Snapshot {
                    phase: "pre",
                    side: "server",
                    uuid: 123,
                    diamonds: 2,
                    xp: 7,
                    cooldown: Some(2)
                }
            );
            assert_eq!(
                observations[1],
                Snapshot {
                    phase: "post",
                    side: "server",
                    uuid: 123,
                    diamonds: 2,
                    xp: 7,
                    cooldown: Some(2)
                }
            );
            assert_eq!(observations.len(), 4);
            assert_eq!(game.dropped.len(), 1);
        })
    });
}

#[test]
#[ignore = "daily"]
fn client_player_ticks_on_the_death_screen_too() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "log",
        Box::new(Log(Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            let (client, server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "tick-client").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.profile = 456;
            remote.entity_id = 42;
            game.player = horizon_client::motion::PlayerMotion::new(
                game.motion,
                horizon_core::geometry::Vec3::ZERO,
            )
            .ok();
            game.remote.as_mut().unwrap().level_players.insert(66, 789);
            let kind = game
                .remote
                .as_ref()
                .unwrap()
                .player_type
                .as_ref()
                .unwrap()
                .clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                66,
                kind,
                horizon_core::geometry::Vec3::ZERO,
                0.0,
            ));
            game.cooldowns.add("minecraft:ender_pearl".into(), 3);
            game.sleep_timer = 4;
            // LocalPlayer.tick waits for the client load and its chunk: only
            // the other player's pair runs, and the level pass still ticks.
            game.tick_remote().unwrap();
            // Player.tick owns both counters, so they wait with it.
            assert_eq!(game.cooldowns.remaining().next().unwrap().1, 3);
            assert_eq!(game.sleep_timer, 4);
            let phases = || {
                std::mem::take(&mut *heard.lock().unwrap())
                    .iter()
                    .map(|s| (s.phase, s.side, s.uuid))
                    .collect::<Vec<_>>()
            };
            assert_eq!(phases(), [("pre", "client", 789), ("post", "client", 789)]);
            game.remote.as_mut().unwrap().loaded_sent = true;
            game.streaming
                .insert_remote(&mut game.world, horizon_client::chunk::Chunk::empty(0, 0));
            // The other player's tick ends before the local player's starts.
            game.tick_remote().unwrap();
            assert_eq!(game.cooldowns.remaining().next().unwrap().1, 2);
            assert_eq!(game.sleep_timer, 5);
            game.screen = super::super::Screen::Dead;
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            game.tick_remote().unwrap();
            assert_eq!(
                phases(),
                [
                    ("pre", "client", 789),
                    ("post", "client", 789),
                    ("pre", "client", 456),
                    ("post", "client", 456),
                    ("pre", "client", 789),
                    ("post", "client", 789),
                    ("pre", "client", 456),
                    ("post", "client", 456)
                ]
            );
            server.shutdown();
        })
    });
}

// `player_name_format` on the name tag over a player (`Player.getDisplayName`).
/// Renames "Steve" and counts how often the hook is asked.
struct Captain(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Captain {
    fn player_name_format(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::PlayerNameFormatEvent<'_>,
    ) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if *event.username == mod_api::Text::literal("Steve") {
            *event.displayname = mod_api::Text::literal("Captain Steve");
        }
    }
}

/// A client game with Steve (entity 66) in the level, and the names its tags show.
fn tags_over_steve(mods: mod_api::Mods) -> Vec<String> {
    let mut shown = Vec::new();
    mod_platform::testing::with_mods(mods, || {
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.options.execution_role = super::super::execution_role::ExecutionRole::Client;
            let (client, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "name-tag").unwrap();
            let remote = game.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.profile = 456;
            remote.entity_id = 42;
            remote.level_players.insert(66, 789);
            remote.level_player_names.insert(66, "Steve".into());
            let kind = remote.player_type.as_ref().unwrap().clone();
            game.mobs.push(horizon_client::mobs::Mob::new(
                66,
                kind,
                horizon_core::geometry::Vec3::new(0.0, 64.0, 2.0),
                0.0,
            ));
            shown = game.shown_name_tags(glam::DVec3::new(0.0, 64.0, 0.0));
        });
    });
    shown
}

#[test]
#[ignore = "daily"]
fn the_name_tag_over_a_player_shows_the_name_a_mod_formats() {
    let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "captain",
        Box::new(Captain(asked.clone())),
    )]
    .into();
    assert_eq!(tags_over_steve(mods), ["Captain Steve"]);
    assert!(asked.load(std::sync::atomic::Ordering::Relaxed) > 0);
}

#[test]
#[ignore = "daily"]
fn a_mod_that_does_not_listen_is_never_asked_for_the_name_tag() {
    struct Other;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Other {
        fn register(&self, _: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {}
    }
    let mods = vec![mod_api::ModEntry::new("other", Box::new(Other))].into();
    assert_eq!(tags_over_steve(mods), ["Steve"]);
}
