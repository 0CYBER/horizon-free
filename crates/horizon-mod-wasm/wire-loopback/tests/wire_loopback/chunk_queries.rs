use super::*;
use mod_api::__private::game::BackendRefOf;
use mod_api::prelude::*;

struct World;
impl mod_api::Backend for World {}
mod_api::impl_backend_areas!(World, except(levels));
impl LevelsBackend for World {
    fn chunk_is_slime_chunk(&self, chunk: mod_api::ChunkRef) -> Option<bool> {
        (chunk.level().epoch() == mod_api::WorldEpoch::new(7)).then_some(chunk.pos() == [-17, 29])
    }
}
impl mod_api::Host for World {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}
mod_api::impl_host_areas!(World);

#[test]
fn slime_chunk_query_round_trips_true_false_and_unavailable() {
    // The platform phase this scope sets is process-wide state another test may not have installed yet.
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
        (7, [-17, 29], Some(true)),
        (7, [0, 0], Some(false)),
        (8, [-17, 29], None),
    ] {
        let level = mod_api::LevelRef::new(
            mod_api::WorldEpoch::new(epoch),
            mod_api::DimensionKey::OVERWORLD,
        );
        let chunk = mod_api::ChunkRef::new(level, pos);
        assert_eq!(chunk.is_slime_chunk(&guest::Guest), expected);
    }
    guest::set_transport(None);
}
