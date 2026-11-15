use super::*;
use mod_api::prelude::*;
use mod_api::{BlockPos, SignEditView};
use std::cell::RefCell;

/// A client whose screen of its own opens and closes, whose inventory opens,
/// and whose sign editor is on a sign: the three results a mod reads cross the
/// wire as their native values.
#[derive(Default)]
struct Client {
    screen_open: RefCell<bool>,
    inventory: RefCell<bool>,
    sign: RefCell<Option<SignEditView>>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(screens));
impl ScreensClientHost for Client {
    fn close_screen(&mut self) -> bool {
        let open = !*self.screen_open.borrow();
        self.screen_open.replace(open);
        open
    }
    fn open_inventory(&mut self) -> bool {
        let opened = !*self.inventory.borrow();
        self.inventory.replace(opened);
        opened
    }
    fn sign_editor(&self) -> Option<SignEditView> {
        *self.sign.borrow()
    }
}

#[test]
fn a_wasm_mod_closes_opens_and_reads_the_sign_through_the_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client::default();
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
    assert_eq!(guest::Guest.close_screen(), true);
    assert_eq!(guest::Guest.close_screen(), false);
    assert_eq!(guest::Guest.open_inventory(), true);
    assert_eq!(guest::Guest.open_inventory(), false);
    assert_eq!(guest::Guest.sign_editor(), None);
    *client.sign.borrow_mut() = Some(SignEditView {
        pos: BlockPos::new(3, 64, -2),
        front: true,
    });
    assert_eq!(
        guest::Guest.sign_editor(),
        Some(SignEditView {
            pos: BlockPos::new(3, 64, -2),
            front: true,
        })
    );
    guest::set_transport(None);
}
