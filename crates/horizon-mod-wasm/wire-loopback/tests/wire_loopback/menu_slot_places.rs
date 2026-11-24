use super::*;
use mod_api::__private::game::BackendMutRefOf;

#[test]
fn menu_slot_places_round_trip() {
    let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(3), 9);
    for place in [
        mod_api::MenuSlotPlace::HeldItem {
            hand: mod_api::Hand::Off,
            slot: 26,
        },
        mod_api::MenuSlotPlace::EntityItems { entity, slot: 4 },
        mod_api::MenuSlotPlace::ModItem {
            provider: "example:back",
            key: 7,
            slot: 3,
        },
    ] {
        let mut out = Writer::new();
        out.put(&place);
        assert_eq!(
            Reader::decode::<mod_api::MenuSlotPlace>(out.bytes()).unwrap(),
            place
        );
        let backing = mod_api::MenuSlotBacking::Place(place);
        let mut out = Writer::new();
        out.put(&backing);
        assert_eq!(
            Reader::decode::<mod_api::MenuSlotBacking>(out.bytes()).unwrap(),
            backing
        );
    }
}

/// A server whose `player_back_mod_menu_slots` records the places it was given.
#[derive(Default)]
struct Server {
    backed: Vec<(i32, Vec<mod_api::MenuSlotPlace>)>,
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
    fn player_back_mod_menu_slots(
        &mut self,
        _: mod_api::PlayerRef,
        window: i32,
        places: Vec<mod_api::MenuSlotPlace>,
    ) -> Result<(), mod_api::MenuSlotBackRefusal> {
        // Window 2 is backed; window 3 names its refusal, which crosses back as sent.
        self.backed.push((window, places));
        match window {
            2 => Ok(()),
            _ => Err(mod_api::MenuSlotBackRefusal::PlaceUnresolved { index: 4 }),
        }
    }
}

#[test]
fn a_wasm_mod_backs_its_menu_slots_with_places() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut server = Server::default();
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut server);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: server and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let epoch = mod_api::WorldEpoch::new(7);
    let player = mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 5), 123);
    let places: Vec<_> = (0..27)
        .map(|slot| mod_api::MenuSlotPlace::HeldItem {
            hand: mod_api::Hand::Main,
            slot,
        })
        .collect();
    assert_eq!(
        player.back_mod_menu_slots(&mut guest::Guest, 2, places.clone()),
        Ok(())
    );
    assert_eq!(
        player.back_mod_menu_slots(&mut guest::Guest, 3, Vec::new()),
        Err(mod_api::MenuSlotBackRefusal::PlaceUnresolved { index: 4 })
    );
    guest::set_transport(None);
    assert_eq!(server.backed, [(2, places), (3, Vec::new())]);
}
