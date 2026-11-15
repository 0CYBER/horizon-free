use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::prelude::*;
use mod_api::{MobRef, WantedPositionError};

#[derive(Default)]
struct World {
    wish: Option<([f64; 3], f64)>,
}
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(mobs));
impl MobsBackend for World {
    fn mob_wanted_position(&self, _: MobRef) -> Option<[f64; 3]> {
        self.wish.map(|v| v.0)
    }
    fn mob_speed_modifier(&self, _: MobRef) -> Option<f64> {
        self.wish.map(|v| v.1)
    }
}
impl mod_api::BackendMut for World {}
mod_api::impl_backend_mut_areas!(World, except(mobs));
impl MobsBackendMut for World {
    fn mob_set_wanted_position(
        &mut self,
        mob: MobRef,
        at: [f64; 3],
        speed: f64,
    ) -> Result<bool, WantedPositionError> {
        if at.iter().any(|v| !v.is_finite()) {
            return Err(WantedPositionError::NonFinitePosition);
        }
        if !speed.is_finite() {
            return Err(WantedPositionError::NonFiniteSpeed);
        }
        if mob.entity().id() != 12 {
            return Ok(false);
        }
        self.wish = Some((at, speed));
        Ok(true)
    }
}
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);
impl mod_api::ServerHost for World {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(World);

#[test]
fn move_control_wish_and_named_refusals_cross_the_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut world = World::default();
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the host outlives the synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mob = MobRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 12));
    assert_eq!(mob.wanted_position(&guest::Guest), None);
    assert_eq!(mob.speed_modifier(&guest::Guest), None);
    assert_eq!(
        mob.set_wanted_position(&mut guest::Guest, [10.5, 65.0, -3.5], -1.25),
        Ok(true)
    );
    assert_eq!(mob.wanted_position(&guest::Guest), Some([10.5, 65.0, -3.5]));
    assert_eq!(mob.speed_modifier(&guest::Guest), Some(-1.25));
    assert_eq!(
        mob.set_wanted_position(&mut guest::Guest, [f64::NAN, 0.0, 0.0], 1.0),
        Err(WantedPositionError::NonFinitePosition)
    );
    assert_eq!(
        mob.set_wanted_position(&mut guest::Guest, [0.0; 3], f64::INFINITY),
        Err(WantedPositionError::NonFiniteSpeed)
    );
    let stale = MobRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 99));
    assert_eq!(
        stale.set_wanted_position(&mut guest::Guest, [0.0; 3], 1.0),
        Ok(false)
    );
    guest::set_transport(None);
}
