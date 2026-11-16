use super::*;
use mod_api::__private::game::{BackendMutRefOf, BackendRefOf};
use mod_api::prelude::*;
#[derive(Default)]
struct World([bool; 3]);
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(mobs));
impl MobsBackend for World {
    fn mob_can_open_doors(&self, mob: mod_api::MobRef) -> Option<bool> {
        (mob.entity().id() == 99).then_some(self.0[0])
    }
    fn mob_can_pass_doors(&self, mob: mod_api::MobRef) -> Option<bool> {
        (mob.entity().id() == 99).then_some(self.0[1])
    }
    fn mob_can_float(&self, mob: mod_api::MobRef) -> Option<bool> {
        (mob.entity().id() == 99).then_some(self.0[2])
    }
}
impl mod_api::BackendMut for World {}
mod_api::impl_backend_mut_areas!(World, except(mobs));
impl MobsBackendMut for World {
    fn mob_set_can_open_doors(&mut self, mob: mod_api::MobRef, value: bool) -> bool {
        if mob.entity().id() != 99 {
            return false;
        }
        self.0[0] = value;
        true
    }
    fn mob_set_can_pass_doors(&mut self, mob: mod_api::MobRef, value: bool) -> bool {
        if mob.entity().id() != 99 {
            return false;
        }
        self.0[1] = value;
        true
    }
    fn mob_set_can_float(&mut self, mob: mod_api::MobRef, value: bool) -> bool {
        if mob.entity().id() != 99 {
            return false;
        }
        self.0[2] = value;
        true
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
fn navigation_flags_round_trip_values_and_refusals() {
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
        // SAFETY: world remains live throughout these synchronous calls.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let mob = mod_api::MobRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 99));
    let stale = mod_api::MobRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 100));
    assert_eq!(guest::Guest.mob_can_open_doors(mob), Some(false));
    assert!(guest::Guest.mob_set_can_open_doors(mob, true));
    assert_eq!(guest::Guest.mob_can_open_doors(mob), Some(true));
    assert!(guest::Guest.mob_set_can_open_doors(mob, false));
    assert_eq!(guest::Guest.mob_can_open_doors(mob), Some(false));
    assert_eq!(guest::Guest.mob_can_open_doors(stale), None);
    assert!(!guest::Guest.mob_set_can_open_doors(stale, true));
    assert_eq!(guest::Guest.mob_can_pass_doors(mob), Some(false));
    assert!(guest::Guest.mob_set_can_pass_doors(mob, true));
    assert_eq!(guest::Guest.mob_can_pass_doors(mob), Some(true));
    assert!(guest::Guest.mob_set_can_pass_doors(mob, false));
    assert_eq!(guest::Guest.mob_can_pass_doors(mob), Some(false));
    assert_eq!(guest::Guest.mob_can_pass_doors(stale), None);
    assert!(!guest::Guest.mob_set_can_pass_doors(stale, true));
    assert_eq!(guest::Guest.mob_can_float(mob), Some(false));
    assert!(guest::Guest.mob_set_can_float(mob, true));
    assert_eq!(guest::Guest.mob_can_float(mob), Some(true));
    assert!(guest::Guest.mob_set_can_float(mob, false));
    assert_eq!(guest::Guest.mob_can_float(mob), Some(false));
    assert_eq!(guest::Guest.mob_can_float(stale), None);
    assert!(!guest::Guest.mob_set_can_float(stale, true));
    guest::set_transport(None);
}
