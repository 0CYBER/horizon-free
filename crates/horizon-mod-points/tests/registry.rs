//! The registry's lifecycle, registering, sealing and stopping, on the real points: an integration
//! test, so the generated points compile once for the tests.
// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
use mod_api::__private::points::register;
use mod_points::__private::POINTS;
use mod_points::{PointId, RegisterError, stop};
use std::sync::atomic::Ordering;

fn tick(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    assert_eq!(
        mod_api::content::crash::current_native_mod(),
        Some("tick_mod")
    );
    args.cancel = true;
}

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl mod_api::ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Quiet {}

/// Stopping from the mod's subscription callback still applies to slots registered later.
struct SelfStop;
impl mod_api::Hooks for SelfStop {
    fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        stop("self_stopping");
        &[]
    }
}

#[test]
fn one_list_registered_sealed_and_stopped() {
    mod_api::__private::install(
        ["tick_mod", "stopped_early", "self_stopping"]
            .into_iter()
            .map(|id| {
                mod_api::ModEntry::new(
                    id,
                    if id == "self_stopping" {
                        Box::new(SelfStop) as Box<dyn mod_api::Hooks>
                    } else {
                        Box::new(Quiet)
                    },
                )
            })
            .collect(),
    );
    assert!(mod_api::__private::set_mod_enabled("tick_mod", false));
    assert_eq!(
        register(&mod_points::listeners::tick("tick_mod", 0, "head", tick)),
        Ok(())
    );
    assert_eq!(
        register(&mod_points::listeners::tick_return(
            "tick_mod", 0, "tail", tick
        )),
        Ok(())
    );
    assert_eq!(
        register(&mod_points::listeners::tick(
            "stopped_early",
            0,
            "head",
            tick
        )),
        Ok(())
    );
    stop("stopped_early");
    assert_eq!(
        register(&mod_points::listeners::tick(
            "self_stopping",
            0,
            "head",
            tick
        )),
        Ok(())
    );
    assert!(
        !POINTS[PointId::Tick as usize].load(Ordering::Acquire),
        "nothing listens before the seal"
    );
    horizon_engine_slots::install_seal(mod_points::__private::seal).unwrap();
    horizon_engine_slots::seal().unwrap();
    assert_eq!(
        horizon_engine_slots::seal(),
        Err(horizon_engine_slots::Refusal::Sealed)
    );
    assert_eq!(
        register(&mod_points::listeners::tick("late", 0, "head", tick)),
        Err(RegisterError::Sealed)
    );
    assert!(POINTS[PointId::Tick as usize].load(Ordering::Acquire));
    assert!(POINTS[PointId::TickReturn as usize].load(Ordering::Acquire));
    let mut heard = 0;
    horizon_engine_slots::__private::fire_api(horizon_engine_slots::PointId::Tick, &mut |_| {
        heard += 1
    });
    assert_eq!(heard, 0, "disabled before sealing");
    assert!(mod_api::__private::set_mod_enabled("tick_mod", true));
    horizon_engine_slots::__private::fire_api(horizon_engine_slots::PointId::Tick, &mut |_| {
        heard += 1
    });
    assert_eq!(heard, 1);
    let mut args = mod_points::TickArgs::__new();
    horizon_engine_slots::dispatch::tick(mod_api::HostSide::Server(&mut Quiet), &mut args);
    assert!(
        args.cancel,
        "the checked function runs under its owner's guard and writes back"
    );
    assert_eq!(mod_api::content::crash::current_native_mod(), None);
    assert!(mod_api::__private::set_mod_enabled("tick_mod", false));
    horizon_engine_slots::__private::fire_api(horizon_engine_slots::PointId::Tick, &mut |_| {
        heard += 1
    });
    assert_eq!(heard, 1);
    assert!(mod_api::__private::set_mod_enabled("tick_mod", true));
    horizon_engine_slots::__private::fire_api(horizon_engine_slots::PointId::Tick, &mut |_| {
        heard += 1
    });
    assert_eq!(heard, 2);
    stop("tick_mod");
    assert!(mod_api::__private::set_mod_enabled("tick_mod", false));
    assert!(mod_api::__private::set_mod_enabled("tick_mod", true));
    horizon_engine_slots::__private::fire_api(horizon_engine_slots::PointId::Tick, &mut |_| {
        heard += 1
    });
    assert_eq!(heard, 2, "an enable switch cannot undo a stop");
}
