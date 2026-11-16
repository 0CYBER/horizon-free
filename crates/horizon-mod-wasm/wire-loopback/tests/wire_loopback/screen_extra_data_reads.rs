use super::*;
use mod_api::prelude::*;
use std::cell::RefCell;

/// A client whose mod screen holds the bytes a server's `open_mod_menu` sent
/// with no block: the owned extra bytes, and no position.
#[derive(Default)]
struct Client {
    extra: RefCell<Option<Vec<u8>>>,
    pos: RefCell<Option<[i32; 3]>>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(screens));
impl ScreensClientHost for Client {
    fn screen_extra_data(&self) -> Option<Vec<u8>> {
        self.extra.borrow().clone()
    }
    fn screen_pos(&self) -> Option<[i32; 3]> {
        *self.pos.borrow()
    }
}

#[test]
fn a_wasm_mod_reads_the_owned_extra_bytes_and_the_optional_position() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client {
        extra: RefCell::new(Some(vec![1, 2, 3])),
        pos: RefCell::new(None),
    };
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: client and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    // The bytes of the issue's test, read through the owned call on wasm as on native.
    assert_eq!(guest::Guest.screen_extra_data(), Some(vec![1, 2, 3]));
    // A blockless open sends no position, and a block open's does.
    assert_eq!(guest::Guest.screen_pos(), None);
    *client.pos.borrow_mut() = Some([3, 70, -5]);
    assert_eq!(guest::Guest.screen_pos(), Some([3, 70, -5]));
    // Nothing written reads as empty, not None, while a mod screen is open.
    *client.extra.borrow_mut() = Some(Vec::new());
    assert_eq!(guest::Guest.screen_extra_data(), Some(Vec::new()));
    guest::set_transport(None);
}
