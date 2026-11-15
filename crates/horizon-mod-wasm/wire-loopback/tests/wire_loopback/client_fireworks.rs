use super::*;
use mod_api::prelude::*;
use mod_api::{FireworkExplosion, FireworkShape, FireworksRequest};
use std::cell::RefCell;

/// A client that keeps every fireworks request it receives and accepts the audible ones.
#[derive(Default)]
struct Client(RefCell<Vec<FireworksRequest>>);
impl mod_api::Host for Client {}
mod_api::impl_host_areas!(Client);
impl mod_api::ClientHost for Client {}
mod_api::impl_client_host_areas!(Client, except(effects));
impl EffectsClientHost for Client {
    fn create_fireworks(&mut self, request: &FireworksRequest) -> bool {
        self.0.borrow_mut().push(request.clone());
        request.audible
    }
}

#[test]
fn fireworks_request_round_trips_with_audible_on_and_off() {
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
    let sent: Vec<_> = [true, false]
        .into_iter()
        .map(|audible| FireworksRequest {
            at: [0.5, 70.25, -3.5],
            velocity: [0.0, 0.05, -0.125],
            explosions: vec![
                FireworkExplosion {
                    shape: FireworkShape::Creeper,
                    colors: vec![0xFF5555, 0x55FF55],
                    fade_colors: vec![0xFFFFFF],
                    trail: true,
                    twinkle: false,
                },
                FireworkExplosion {
                    shape: FireworkShape::Burst,
                    colors: vec![],
                    fade_colors: vec![],
                    trail: false,
                    twinkle: true,
                },
            ],
            audible,
        })
        .collect();
    for request in &sent {
        assert_eq!(guest::Guest.create_fireworks(request), request.audible);
    }
    guest::set_transport(None);
    assert_eq!(*client.0.borrow(), sent);
}
