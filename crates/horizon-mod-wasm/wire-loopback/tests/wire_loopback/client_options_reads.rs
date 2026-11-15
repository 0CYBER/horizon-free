use super::*;
use mod_api::prelude::*;

/// A client whose GUI scale is the value a stubbed window resolved.
struct Client(Option<u32>);
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_options));
impl ClientOptionsClientHost for Client {
    fn gui_scale(&self) -> Option<u32> {
        self.0
    }
}

/// The same literal values the native resolution test asserts reach a wasm
/// mod unchanged through the generated wire proxy.
#[test]
fn a_wasm_mod_reads_the_gui_scale_the_window_applies() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    for expected in [Some(3), Some(4), Some(2), Some(1), None] {
        let client = Client(expected);
        let mut frame = boundary::Frame::default();
        frame.set_client_host(&client);
        let mut cx = boundary::Cx::default();
        cx.frames.push(frame);
        let cx = std::cell::RefCell::new(cx);
        guest::set_transport(Some(Rc::new(move |id, bytes| {
            let mut out = Writer::new();
            // SAFETY: client and its frame remain live for this synchronous transport.
            unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
            out.into_bytes()
        })));
        assert_eq!(guest::Guest.gui_scale(), expected);
    }
    guest::set_transport(None);
}
