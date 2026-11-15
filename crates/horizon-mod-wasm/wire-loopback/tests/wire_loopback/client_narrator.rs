//! `ClientNarratorClientHost::narrate` from a wasm mod reaches the client's
//! narrator with the line and its `Narration` intact, and each named refusal
//! comes back as the native call returns it (arismoko/horizon#4532).
use super::*;
use mod_api::prelude::*;
use mod_api::{NarrateError, Narration, Text, TextFormError};
use std::cell::RefCell;

/// A client that answers as the narrator would for these lines, and records
/// what reached it.
#[derive(Default)]
struct Client {
    calls: RefCell<Vec<(Text, Narration)>>,
}
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_narrator));
impl ClientNarratorClientHost for Client {
    fn narrate(&mut self, text: &Text, when: Narration) -> Result<(), NarrateError> {
        self.calls.borrow_mut().push((text.clone(), when));
        match text {
            t if *t == Text::literal("") => Err(NarrateError::Empty),
            t if *t == Text::literal("off") => Err(NarrateError::Off),
            t if *t == Text::literal("mute") => Err(NarrateError::Unavailable),
            t if *t == Text::literal("deep") => Err(NarrateError::Text(TextFormError::TooDeep)),
            _ => Ok(()),
        }
    }
}

#[test]
fn a_wasm_mods_line_reaches_the_narrator_and_gets_each_refusal_by_name() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client::default();
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
    let mut target = Text::translate("block.minecraft.oak_log", vec![]);
    target.style.bold = Some(true);
    let refused = [
        ("", NarrateError::Empty),
        ("off", NarrateError::Off),
        ("mute", NarrateError::Unavailable),
        ("deep", NarrateError::Text(TextFormError::TooDeep)),
    ];
    assert_eq!(guest::Guest.narrate(&target, Narration::Now), Ok(()));
    assert_eq!(guest::Guest.narrate(&target, Narration::Queued), Ok(()));
    for (line, refusal) in &refused {
        let answer = guest::Guest.narrate(&Text::literal(*line), Narration::Now);
        assert_eq!(answer, Err(refusal.clone()));
    }
    guest::set_transport(None);
    let mut reached = vec![(target.clone(), Narration::Now)];
    reached.push((target, Narration::Queued));
    for (line, _) in &refused {
        reached.push((Text::literal(*line), Narration::Now));
    }
    assert_eq!(*client.calls.borrow(), reached);
}
