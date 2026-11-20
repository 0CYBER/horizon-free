//! `ScreensClientHost::request_screenshot` from a wasm mod reaches the client, and
//! each named refusal comes back as the native call returns it (#9606).
use super::*;
use mod_api::ScreenshotError;
use mod_api::prelude::*;
use std::cell::{Cell, RefCell};

/// A client that takes one screenshot at a time and has room for two, and counts
/// each request that reached it.
#[derive(Default)]
struct Client {
    asked: RefCell<u32>,
    in_progress: Rc<Cell<bool>>,
    taken: RefCell<u32>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(screens));
impl ScreensClientHost for Client {
    fn request_screenshot(&mut self) -> Result<(), ScreenshotError> {
        *self.asked.borrow_mut() += 1;
        if self.in_progress.replace(true) {
            return Err(ScreenshotError::Busy);
        }
        if *self.taken.borrow() == 2 {
            return Err(ScreenshotError::BudgetExhausted);
        }
        *self.taken.borrow_mut() += 1;
        Ok(())
    }
}

#[test]
fn a_wasm_mods_screenshot_crosses_the_wire_and_each_refusal_comes_back_by_name() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client::default();
    let in_progress = client.in_progress.clone();
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: client and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    // The client's screenshot is written: the next one may start.
    let finish = || in_progress.set(false);
    assert_eq!(guest::Guest.request_screenshot(), Ok(()));
    assert_eq!(
        guest::Guest.request_screenshot(),
        Err(ScreenshotError::Busy)
    );
    finish();
    assert_eq!(guest::Guest.request_screenshot(), Ok(()));
    finish();
    assert_eq!(
        guest::Guest.request_screenshot(),
        Err(ScreenshotError::BudgetExhausted)
    );
    guest::set_transport(None);
    assert_eq!(*client.asked.borrow(), 4);
}
