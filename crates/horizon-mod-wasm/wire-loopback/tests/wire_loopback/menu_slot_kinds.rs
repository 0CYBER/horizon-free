use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::prelude::*;

struct Client;
impl mod_api::Host for Client {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client);
impl mod_api::Backend for Client {}
mod_api::impl_backend_areas!(Client, except(menus));
impl MenusBackend for Client {}
impl MenusClientBackend for Client {
    fn menu_slot_kinds(&self, _: mod_api::MenuRef) -> Option<Vec<mod_api::MenuSlotKind>> {
        use mod_api::MenuSlotKind::*;
        Some(vec![
            Player(40),
            Storage,
            CraftingGrid,
            Result,
            Equipment,
            Other,
        ])
    }
}

#[test]
fn menu_slot_kinds_native_and_guest_round_trip() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let menu = mod_api::__private::game::menu_ref(
        mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 5),
            123,
        ),
        0,
        1,
    );
    let mut client = Client;
    let expected = menu.slot_kinds(&client);
    for kind in expected.as_ref().unwrap() {
        let mut writer = Writer::new();
        writer.put(kind);
        assert_eq!(
            Reader::decode::<mod_api::MenuSlotKind>(writer.bytes()).unwrap(),
            *kind
        );
    }
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: frame and client remain live throughout this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    assert_eq!(menu.slot_kinds(&guest::Guest), expected);
    guest::set_transport(None);
}
