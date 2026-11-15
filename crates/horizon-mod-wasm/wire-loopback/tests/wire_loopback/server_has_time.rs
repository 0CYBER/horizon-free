use super::*;
use std::cell::Cell;

struct World(Cell<bool>);
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World, except(config));
impl mod_api::prelude::ConfigHost for World {
    fn server_has_time(&self) -> bool {
        self.0.get()
    }
}

#[test]
fn server_has_time_crosses_the_wire_both_ways() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = World(Cell::new(false));
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let id = boundary::import_id("Host", "server_has_time").unwrap();
    for spare in [false, true] {
        world.0.set(spare);
        let mut out = Writer::new();
        unsafe { boundary::serve(id, &mut cx, &[], &mut out) }.unwrap();
        assert_eq!(Reader::decode::<bool>(out.bytes()).unwrap(), spare);
    }
}
