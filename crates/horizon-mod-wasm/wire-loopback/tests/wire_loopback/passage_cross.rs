use super::*;
use mod_api::prelude::*;
use mod_api::{PassageCrossError, PassageCrossRefusal, PassageId};
use std::cell::RefCell;

/// A window that keeps the passages it is asked to cross: it crosses 1 and refuses the
/// rest as too far, and holds the server's refusal of passage 1.
#[derive(Default)]
struct Client(RefCell<Vec<PassageId>>);
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(players));
impl PlayersClientHost for Client {
    fn push_player(&mut self, _: [f32; 3]) {}
    fn cross_passage(&mut self, passage: PassageId) -> Result<(), PassageCrossError> {
        self.0.borrow_mut().push(passage);
        if passage == PassageId::from_raw(1) {
            Ok(())
        } else {
            Err(PassageCrossError::TooFar { limit: 20 })
        }
    }
    fn last_passage_refusal(&self) -> Option<(PassageId, PassageCrossRefusal)> {
        Some((PassageId::from_raw(1), PassageCrossRefusal::Vetoed))
    }
}

#[test]
fn a_crossing_and_the_servers_refusal_cross_the_wire() {
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
    let (one, far) = (PassageId::from_raw(1), PassageId::from_raw(u64::MAX));
    assert_eq!(guest::Guest.cross_passage(one), Ok(()));
    assert_eq!(
        guest::Guest.cross_passage(far),
        Err(PassageCrossError::TooFar { limit: 20 })
    );
    assert_eq!(
        guest::Guest.last_passage_refusal(),
        Some((one, PassageCrossRefusal::Vetoed))
    );
    guest::set_transport(None);
    assert_eq!(*client.0.borrow(), [one, far]);
}
