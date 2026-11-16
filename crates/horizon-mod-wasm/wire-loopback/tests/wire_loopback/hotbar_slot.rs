use super::*;
use mod_api::HotbarSlotError;
use mod_api::prelude::*;
use std::cell::RefCell;

/// A window that records the slots it was asked for: it selects 0 to 8, refuses 9 and above by
/// name, answers `NoWorld` until it has one, and `Refused` while it spectates.
#[derive(Default)]
struct Client {
    asked: RefCell<Vec<u8>>,
    has_world: std::cell::Cell<bool>,
    spectator: std::cell::Cell<bool>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(players));
impl PlayersClientHost for Client {
    fn select_hotbar_slot(&mut self, slot: u8) -> Result<(), HotbarSlotError> {
        self.asked.borrow_mut().push(slot);
        if slot > 8 {
            return Err(HotbarSlotError::OutOfRange { slot });
        }
        if !self.has_world.get() {
            return Err(HotbarSlotError::NoWorld);
        }
        if self.spectator.get() {
            return Err(HotbarSlotError::Refused);
        }
        Ok(())
    }
}

#[test]
fn a_hotbar_selection_and_its_refusals_cross_the_wire() {
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
    assert_eq!(
        guest::Guest.select_hotbar_slot(4),
        Err(HotbarSlotError::NoWorld)
    );
    client.has_world.set(true);
    assert_eq!(guest::Guest.select_hotbar_slot(4), Ok(()));
    assert_eq!(
        guest::Guest.select_hotbar_slot(9),
        Err(HotbarSlotError::OutOfRange { slot: 9 })
    );
    client.spectator.set(true);
    assert_eq!(
        guest::Guest.select_hotbar_slot(6),
        Err(HotbarSlotError::Refused)
    );
    guest::set_transport(None);
    assert_eq!(*client.asked.borrow(), [4, 4, 9, 6]);
}
