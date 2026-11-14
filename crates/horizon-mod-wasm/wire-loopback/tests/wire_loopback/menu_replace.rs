use super::*;
use mod_api::__private::game::BackendMutRefOf;

#[test]
fn menu_replace_round_trips_both_choices() {
    for (choice, ordinal) in [
        (mod_api::MenuReplace::CloseFirst, 0_u32),
        (mod_api::MenuReplace::InPlace, 1_u32),
    ] {
        let mut out = Writer::new();
        out.put(&choice);
        assert_eq!(out.bytes(), ordinal.to_le_bytes());
        assert_eq!(
            Reader::decode::<mod_api::MenuReplace>(out.bytes()).unwrap(),
            choice
        );
    }
}

/// A server whose `player_open_mod_menu` records the close choice it was given.
#[derive(Default)]
struct Server {
    replaces: Vec<mod_api::MenuReplace>,
}
impl mod_api::Host for Server {}
mod_api::impl_host_areas!(Server);
impl mod_api::ServerHost for Server {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(Server);
impl mod_api::Backend for Server {}
mod_api::impl_backend_areas!(Server);
impl mod_api::BackendMut for Server {}
mod_api::impl_backend_mut_areas!(Server, except(players));
impl mod_api::prelude::PlayersBackendMut for Server {
    fn player_open_mod_menu(
        &mut self,
        _: mod_api::PlayerRef,
        _: mod_api::MenuKey,
        _: mod_api::Text,
        extra: Vec<u8>,
        _: Option<[i32; 3]>,
        replace: mod_api::MenuReplace,
    ) -> Result<Option<i32>, mod_api::OpenMenuError> {
        assert_eq!(extra, [7]);
        self.replaces.push(replace);
        Ok(Some(self.replaces.len() as i32 + 1))
    }
}

#[test]
fn a_wasm_mod_menu_open_carries_the_close_choice() {
    mod_api::__private::initialize().unwrap();
    let _ = mod_api::__private::resolve_menus(["minecraft:generic_9x1"]);
    let menu = mod_api::__private::menu_keys()
        .into_iter()
        .find(|key| {
            mod_api::__private::menu_key_id(*key).as_deref() == Some("minecraft:generic_9x1")
        })
        .unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut server = Server::default();
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut server);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    let requests = Rc::new(std::cell::RefCell::new(Vec::<Vec<u8>>::new()));
    let seen = requests.clone();
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        seen.borrow_mut().push(bytes.to_vec());
        let mut out = Writer::new();
        // SAFETY: server and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let epoch = mod_api::WorldEpoch::new(7);
    let player = mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 5), 123);
    for (replace, window) in [
        (mod_api::MenuReplace::CloseFirst, 2),
        (mod_api::MenuReplace::InPlace, 3),
    ] {
        assert_eq!(
            player.open_mod_menu(
                &mut guest::Guest,
                menu,
                mod_api::Text::literal("new"),
                vec![7],
                None,
                replace,
            ),
            Ok(Some(window))
        );
    }
    guest::set_transport(None);
    // The choice is the call's last argument: its ordinal ends each request.
    let requests = requests.borrow();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0][requests[0].len() - 4..], [0, 0, 0, 0]);
    assert_eq!(requests[1][requests[1].len() - 4..], [1, 0, 0, 0]);
    assert_eq!(
        server.replaces,
        [
            mod_api::MenuReplace::CloseFirst,
            mod_api::MenuReplace::InPlace
        ]
    );
}
