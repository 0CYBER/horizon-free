use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::prelude::*;

struct World;
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(levels));
impl LevelsBackend for World {
    fn level_entity_ticking(&self, level: mod_api::LevelRef, pos: [i32; 3]) -> Option<bool> {
        (level.epoch() == mod_api::WorldEpoch::new(7)).then_some(pos == [-17, 64, 29])
    }
    fn level_should_tick_blocks_at(&self, level: mod_api::LevelRef, pos: [i32; 3]) -> Option<bool> {
        self.level_entity_ticking(level, pos).map(|value| !value)
    }
}
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);

#[test]
fn ticking_reads_round_trip_positions_and_unavailable_answers() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = World;
    let mut frame = boundary::Frame::default();
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: world and its frame remain live for this synchronous transport.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    for (epoch, pos, expected) in [
        (7, [-17, 64, 29], Some(true)),
        (7, [0, 64, 0], Some(false)),
        (8, [-17, 64, 29], None),
    ] {
        let level = mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(epoch),
            mod_api::DimensionKey::OVERWORLD,
        );
        assert_eq!(level.entity_ticking(&guest::Guest, pos), expected);
        assert_eq!(
            level.should_tick_blocks_at(&guest::Guest, pos),
            expected.map(|value| !value)
        );
    }
    guest::set_transport(None);
}
