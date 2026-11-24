//! A native mod's `request_title` as the title runs it: Create New World from the main page comes back to the main
//! page, and a join goes as Direct Connection's does.
use super::tests::Fixture;
use super::*;

#[test]
fn create_world_from_the_main_page_comes_back_to_it() {
    let mut f = Fixture::new();
    // A save exists, so its Cancel would go to the world list had it come from there.
    f.world("Saved", "Saved");
    f.title.open_new_world();
    assert!(f.title.preparing());
    f.title.finish_preparing();
    assert!(matches!(f.title.mode, Mode::Create { .. }));
    f.title.cancel_create();
    assert!(matches!(f.title.mode, Mode::Main));
}

#[test]
fn a_join_is_direct_connections_and_keeps_the_address() {
    let mut f = Fixture::new();
    assert!(f.title.direct_join("bad host:x").is_none());
    let action = f.title.direct_join("play.example.net:25566");
    assert!(matches!(
        action,
        Some(Action::Connect { ref address, lan: false, realm: false }) if address == "play.example.net:25566"
    ));
    // Kept as the last direct address, a hidden entry of the server list.
    let (shown, hidden) =
        horizon_protocol::servers::load_all(&crate::multiplayer::servers_file(&f.title.saves));
    assert!(shown.is_empty());
    assert!(hidden.iter().any(|s| s.ip == "play.example.net:25566"));
    // And Direct Connection opens on it next time.
    assert_eq!(f.title.last_server, "play.example.net:25566");
}
