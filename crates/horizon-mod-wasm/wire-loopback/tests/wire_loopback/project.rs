use super::*;
use mod_api::prelude::*;

struct Client;
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(client_view));
impl ClientViewClientHost for Client {
    fn project(&self, at: [f64; 3]) -> Option<[f32; 3]> {
        if at == [3.0, 70.0, -5.0] {
            Some([0.25, -0.5, 0.125])
        } else {
            None
        }
    }
}

#[test]
fn project_preserves_optional_coordinates_across_the_host_call() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client;
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the frame and client outlive this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    for (at, expected) in [
        ([3.0, 70.0, -5.0], Some([0.25, -0.5, 0.125])),
        ([0.0; 3], None),
    ] {
        assert_eq!(client.project(at), expected);
        assert_eq!(guest::Guest.project(at), expected);
    }
    guest::set_transport(None);
}
