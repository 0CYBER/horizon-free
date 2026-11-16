//! The player limit a mod sets (`set_max_players`) is the one the server list, the Login
//! packet and the login check all read, not the fixed `max-players` the server started with.
use super::pending_move_tests::with_input_context;
use super::*;

const ADDRESS: &str = "127.0.0.1:25565";

/// The `max_players` the next Login packet to guest 0 carries.
fn login_limit(hub: &mut Hub, rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> i32 {
    while rx.try_recv().is_ok() {}
    hub.guests.update(0, |g| g.placed = false);
    with_input_context(|ctx| hub.place_new_player(1, ctx));
    let mut limit = None;
    while let Ok(crate::lan::Out::Packet(body)) = rx.try_recv() {
        let mut reader = horizon_protocol::buf::Reader::new(&body);
        let Ok(id) = reader.varint() else { continue };
        if let Ok(C::Login(login)) = C::decode(id, reader.rest()) {
            limit = Some(login.max_players);
        }
    }
    limit.expect("a Login packet")
}

/// The `max` of the server list's players, as a ping reads it.
fn status_limit(hub: &Hub) -> i64 {
    let status: serde_json::Value = serde_json::from_str(&hub.lan.shared.status_json()).unwrap();
    status["players"]["max"].as_i64().unwrap()
}

/// The hub starts with 2 places on the network and 20 in its own config, so a Login packet
/// that read the config would say 20.
#[test]
fn the_login_packet_and_the_status_answer_state_the_live_limit() {
    let (mut hub, rx) = hub_with_a_guest();
    assert_eq!(hub.config.max_players, 20);
    assert_eq!(login_limit(&mut hub, &rx), 2);
    assert_eq!(status_limit(&hub), 2);
    hub.lan.shared.set_max_players(7);
    assert_eq!(login_limit(&mut hub, &rx), 7);
    assert_eq!(status_limit(&hub), 7);
    hub.lan.shared.set_max_players(0);
    assert_eq!(login_limit(&mut hub, &rx), 0);
    assert_eq!(status_limit(&hub), 0);
    hub.lan.shared.set_max_players(usize::MAX);
    assert_eq!(login_limit(&mut hub, &rx), i32::MAX);
}

/// A limit lowered below the players on refuses the next login as full and leaves both players
/// on; a raised limit takes logins again.
#[test]
fn a_lowered_limit_refuses_logins_and_keeps_the_players_on() {
    let (hub, _rx) = hub_with_a_guest();
    let address = ADDRESS.parse().unwrap();
    let shared = &hub.lan.shared;
    assert!(shared.try_admit(address, 11, "First").is_none());
    assert!(shared.try_admit(address, 12, "Second").is_none());
    assert!(
        shared.try_admit(address, 13, "Third").is_some(),
        "2 places, 2 on"
    );
    shared.set_max_players(1);
    assert_eq!(shared.online(), 2, "both stay on");
    let status: serde_json::Value = serde_json::from_str(&shared.status_json()).unwrap();
    assert_eq!(status["players"]["max"], 1);
    assert_eq!(status["players"]["online"], 2);
    assert!(shared.try_admit(address, 13, "Third").is_some(), "full");
    shared.set_max_players(3);
    assert!(shared.try_admit(address, 13, "Third").is_none());
    assert_eq!(shared.online(), 3);
}
