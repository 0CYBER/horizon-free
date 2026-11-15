//! `experience_orb_merge` (version 2) and `set_experience_orb` cross the wire with their literal
//! signed values and counts: the guest listener reads what the host sent and cancels, and the
//! setter's inputs and its refusal reach the host unchanged. The refusal rules themselves are the
//! host's (`orb_merge_tests`).
use super::*;
use mod_api::prelude::*;
use std::cell::RefCell;

/// The guest's `experience_orb_merge` listener: it reads the edge values of a resident merge and cancels it; any
/// other dimension is left alone.
pub(super) struct ExperienceOrbMergeHooks;
impl ExperienceOrbMergeHooks {
    pub(super) fn experience_orb_merge(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ExperienceOrbMergeEvent<'_>,
    ) -> mod_api::Flow {
        if event.dimension != "test:orb_merge" {
            return mod_api::Flow::Continue;
        }
        assert_eq!((event.into, event.from), (1, 41));
        assert_eq!(
            (event.into_value, event.into_count),
            (-32768, i32::MAX as u32)
        );
        assert_eq!((event.from_value, event.from_count), (5, 1));
        mod_api::Flow::Cancel
    }
}

#[test]
fn wire_loopback_orb_merge_delivers_signed_values_and_counts_and_honours_cancel() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    install_counter();
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    let event = |dimension: &'static str| mod_api::ExperienceOrbMergeEvent {
        dimension,
        into: 1,
        from: 41,
        into_value: -32768,
        into_count: i32::MAX as u32,
        from_value: 5,
        from_count: 1,
        cancelled: false,
    };
    assert_eq!(
        module.experience_orb_merge(&mut World, &mut event("test:orb_merge")),
        mod_api::Flow::Cancel
    );
    assert_eq!(
        module.experience_orb_merge(&mut World, &mut event("test:elsewhere")),
        mod_api::Flow::Continue
    );
}

/// Records what `set_experience_orb` is asked and refuses a write to orb 2.
#[derive(Default)]
struct Level(RefCell<Vec<(String, u32, Option<i32>, Option<u32>, Option<u32>)>>);
impl mod_api::Host for Level {}
mod_api::impl_host_areas!(Level);
impl mod_api::ServerHost for Level {}
mod_api::impl_server_host_areas!(Level, except(entities));
impl EntitiesServerHost for Level {
    fn set_experience_orb(
        &mut self,
        dimension: &str,
        id: u32,
        value: Option<i32>,
        count: Option<u32>,
        age: Option<u32>,
    ) -> bool {
        self.0
            .borrow_mut()
            .push((dimension.to_owned(), id, value, count, age));
        id != 2
    }
}

#[test]
fn wire_loopback_set_experience_orb_carries_its_limits_and_its_refusal() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut level = Level::default();
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut level);
    let mut cx = Box::new(boundary::Cx::default());
    cx.frames.push(frame);
    let pointer = &mut *cx as *mut boundary::Cx;
    // SAFETY: cx outlives the synchronous calls below; the transport is cleared before it drops.
    guest::set_transport(Some(unsafe { host_dispatch::transport(pointer) }));
    let overworld = "minecraft:overworld";
    let answers = [
        guest::Guest.set_experience_orb(overworld, 1, Some(-32768), Some(1), None),
        guest::Guest.set_experience_orb(
            overworld,
            1,
            Some(32767),
            Some(i32::MAX as u32),
            Some(5999),
        ),
        guest::Guest.set_experience_orb(overworld, 1, Some(40_000), None, None),
        guest::Guest.set_experience_orb(overworld, 1, None, Some(0), None),
        guest::Guest.set_experience_orb(overworld, 2, None, Some(u32::MAX), None),
    ];
    guest::set_transport(None);
    assert_eq!(answers, [true, true, true, true, false]);
    assert_eq!(
        *level.0.borrow(),
        [
            (overworld.to_owned(), 1, Some(-32768), Some(1), None),
            (
                overworld.to_owned(),
                1,
                Some(32767),
                Some(i32::MAX as u32),
                Some(5999)
            ),
            (overworld.to_owned(), 1, Some(40_000), None, None),
            (overworld.to_owned(), 1, None, Some(0), None),
            (overworld.to_owned(), 2, None, Some(u32::MAX), None),
        ]
    );
}
