use super::*;
use mod_api::prelude::*;
use std::cell::RefCell;

#[test]
fn a_client_prediction_refuses_server_commands_by_name() {
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl mod_api::ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut client = Client;
    let mut frame = boundary::Frame::default();
    frame.set_client_host_mut(&mut client);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = RefCell::new(cx);
    let refused = Rc::new(RefCell::new(Vec::new()));
    let observed = refused.clone();
    let previous = LOOPBACK_TRANSPORT.with(|transport| transport.borrow().clone());
    let _restore = TransportScope(previous);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        if id == mod_api::wire::HOST_LOG {
            return Vec::new();
        }
        let mut out = Writer::new();
        // SAFETY: client and its lent frame remain alive throughout the synchronous call.
        let error =
            unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap_err();
        observed.borrow_mut().push(error.to_string());
        out.into_bytes()
    })));
    let call = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        guest::Guest
            .commands()
            .run_command("say forbidden client write")
    }));
    assert!(
        call.is_err(),
        "a refused write must not report command success"
    );
    assert_eq!(&*refused.borrow(), &["Missing(\"server_host\")"]);
}
