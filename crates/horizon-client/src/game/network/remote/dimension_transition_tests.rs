use crate::game::{Game, native_entity_refs::tests::with_client_game};
use horizon_client::hud::HudBatch;
use horizon_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound},
};
use mod_api::{
    ClientHost, DimensionTransition, DimensionTransitionCause as Cause,
    DimensionTransitionReason as Reason, DimensionTransitionScreen,
    DimensionTransitionScreenDrawEvent, Hooks, ModEntry,
};
use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
};

const OVERWORLD: &str = "minecraft:overworld";
const END: &str = "minecraft:the_end";
const NETHER: &str = "minecraft:the_nether";
const END_SCREEN: &str = "transition_test:end";
const RETURN_SCREEN: &str = "transition_test:return";
const NETHER_SCREEN: &str = "transition_test:nether";

#[derive(Debug, PartialEq)]
struct Drawing {
    screen: String,
    reason: Reason,
    cause: Option<Cause>,
}

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Vec<Drawing>>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn dimension_transition_screen_draw(
        &self,
        host: &dyn ClientHost,
        event: &mut DimensionTransitionScreenDrawEvent<'_>,
    ) {
        self.0.lock().unwrap().push(Drawing {
            screen: event.screen.to_owned(),
            reason: event.reason,
            cause: event.cause(host),
        });
    }
}

fn spawn(dimension: &str) -> play::SpawnInfo {
    play::SpawnInfo {
        dimension_type: 0,
        dimension: dimension.to_owned(),
        seed: 0,
        game_type: 0,
        previous_game_type: None,
        debug: false,
        flat: true,
        last_death: None,
        portal_cooldown: 0,
        sea_level: 63,
    }
}

fn login(game: &mut Game<'_>) {
    game.handle_packet(Clientbound::Login(play::Login {
        player_id: 7,
        hardcore: false,
        levels: vec![OVERWORLD.into(), END.into(), NETHER.into()],
        max_players: 1,
        chunk_radius: 8,
        simulation_distance: 8,
        reduced_debug_info: false,
        show_death_screen: true,
        limited_crafting: false,
        spawn: spawn(END),
        online_mode: false,
        enforces_secure_chat: false,
    }))
    .unwrap();
}

fn position(game: &mut Game<'_>, teleport: i32) {
    game.handle_packet(Clientbound::PlayerPosition(play::PlayerPosition {
        teleport,
        position: [0.5, 65.0, 0.5],
        delta: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        relatives: 0,
    }))
    .unwrap();
}

fn die(game: &mut Game<'_>) {
    game.handle_packet(Clientbound::SetHealth(play::SetHealth {
        health: 0.0,
        food: 20,
        saturation: 5.0,
    }))
    .unwrap();
}

fn respawn(game: &mut Game<'_>, destination: &str, keep: i8) {
    game.handle_packet(Clientbound::Respawn(play::Respawn {
        spawn: spawn(destination),
        keep,
    }))
    .unwrap();
}

fn expect_draw(game: &Game<'_>, probe: &Probe, screen: &str, reason: Reason, cause: Cause) {
    let load = &game.remote.as_ref().unwrap().level_load;
    let mut batch = HudBatch::scratch([320, 240]);
    assert!(game.draw_dimension_transition_screen(&mut batch, load, None));
    assert_eq!(
        std::mem::take(&mut *probe.0.lock().unwrap()),
        [Drawing {
            screen: screen.to_owned(),
            reason,
            cause: Some(cause),
        }]
    );
}

fn check_packet_transitions(game: &mut Game<'_>, probe: &Probe) {
    horizon_client_render::dimension_transition::register(vec![
        DimensionTransitionScreen {
            screen: END_SCREEN.into(),
            transition: DimensionTransition::Incoming(END.into()),
        },
        DimensionTransitionScreen {
            screen: RETURN_SCREEN.into(),
            transition: DimensionTransition::Conditional {
                from: END.into(),
                to: OVERWORLD.into(),
            },
        },
        DimensionTransitionScreen {
            screen: NETHER_SCREEN.into(),
            transition: DimensionTransition::Incoming(NETHER.into()),
        },
    ]);
    login(game);
    expect_draw(game, probe, END_SCREEN, Reason::Other, Cause::Login);

    position(game, 1);
    die(game);
    respawn(game, END, 0);
    expect_draw(game, probe, END_SCREEN, Reason::Other, Cause::DeathRespawn);

    position(game, 2);
    die(game);
    respawn(game, OVERWORLD, 0);
    expect_draw(
        game,
        probe,
        RETURN_SCREEN,
        Reason::Other,
        Cause::DeathRespawn,
    );
    assert_eq!(game.dimension_transition_cause(END_SCREEN), None);

    position(game, 3);
    respawn(game, END, 3);
    expect_draw(game, probe, END_SCREEN, Reason::EndPortal, Cause::Travel);
    position(game, 4);
    respawn(game, OVERWORLD, 3);
    expect_draw(game, probe, RETURN_SCREEN, Reason::EndPortal, Cause::Travel);

    position(game, 5);
    respawn(game, NETHER, 3);
    expect_draw(
        game,
        probe,
        NETHER_SCREEN,
        Reason::NetherPortal,
        Cause::Travel,
    );
    assert_eq!(game.dimension_transition_cause(RETURN_SCREEN), None);

    game.remote.as_mut().unwrap().level_load.finish();
    assert_eq!(game.dimension_transition_cause(NETHER_SCREEN), None);
    let load = &game.remote.as_ref().unwrap().level_load;
    assert_eq!(load.transition_cause(), None);
    let mut batch = HudBatch::scratch([320, 240]);
    assert!(!game.draw_dimension_transition_screen(&mut batch, load, None));
    assert!(probe.0.lock().unwrap().is_empty());
}

#[test]
#[ignore = "daily"]
fn dimension_transition_draw_distinguishes_login_death_respawn_and_travel() {
    let probe = Probe::default();
    let mods = vec![ModEntry::new("transition_test", Box::new(probe.clone()))].into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "TransitionTest").unwrap();
            check_packet_transitions(game, &probe);
        });
        with_client_game(|game| {
            let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            game.connect_remote_session(
                &server.local_addr().unwrap().to_string(),
                "TransitionTest",
                None,
                None,
                false,
                false,
            )
            .unwrap();
            check_packet_transitions(game, &probe);
        });
    });
}
