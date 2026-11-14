use super::*;
use mod_api::MappingError;
use mod_api::prelude::*;
use std::cell::RefCell;

/// A client that answers the presses as the game does for these names, and records what reached
/// it.
#[derive(Default)]
struct Client {
    calls: RefCell<Vec<(String, Option<bool>)>>,
}
impl Client {
    fn answer(&self, key: &str, down: Option<bool>) -> Result<(), MappingError> {
        self.calls.borrow_mut().push((key.into(), down));
        match key {
            "key.use" | "key.attack" => Ok(()),
            "key.screenshot" => Err(MappingError::NotGameplay { key: key.into() }),
            "key.forward" => Err(MappingError::NoWorld),
            _ => Err(MappingError::Unknown { key: key.into() }),
        }
    }
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_input));
impl ClientInputClientHost for Client {
    fn set_mapping_down(&mut self, key: &str, down: bool) -> Result<(), MappingError> {
        self.answer(key, Some(down))
    }
    fn click_mapping(&mut self, key: &str) -> Result<(), MappingError> {
        self.answer(key, None)
    }
}

#[test]
fn a_wasm_mod_presses_mappings_and_gets_the_refusals_by_name() {
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
    assert_eq!(guest::Guest.set_mapping_down("key.use", true), Ok(()));
    assert_eq!(guest::Guest.set_mapping_down("key.use", false), Ok(()));
    assert_eq!(guest::Guest.click_mapping("key.attack"), Ok(()));
    assert_eq!(
        guest::Guest.click_mapping("key.screenshot"),
        Err(MappingError::NotGameplay {
            key: "key.screenshot".into()
        })
    );
    assert_eq!(
        guest::Guest.set_mapping_down("nope", true),
        Err(MappingError::Unknown { key: "nope".into() })
    );
    assert_eq!(
        guest::Guest.set_mapping_down("key.forward", true),
        Err(MappingError::NoWorld)
    );
    guest::set_transport(None);
    assert_eq!(
        *client.calls.borrow(),
        [
            ("key.use".to_owned(), Some(true)),
            ("key.use".to_owned(), Some(false)),
            ("key.attack".to_owned(), None),
            ("key.screenshot".to_owned(), None),
            ("nope".to_owned(), Some(true)),
            ("key.forward".to_owned(), Some(true)),
        ]
    );
}
