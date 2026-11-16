//! `replace` listeners (arismoko/horizon#2368) at real points, through `mod_points::replace!`, the expression the
//! launcher and `cargo horizon-mod build --wasm` both emit: the body never runs, the listener's
//! value reaches the caller, and a second replace at the same place is refused.
// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
use mod_api::__private::points::register;
use mod_points::RegisterError;
use std::sync::atomic::{AtomicUsize, Ordering};

static SHUTDOWN_BODY: AtomicUsize = AtomicUsize::new(0);
static AVERAGE_BODY: AtomicUsize = AtomicUsize::new(0);
static REPLACED: AtomicUsize = AtomicUsize::new(0);

/// `horizon_client::game::server_lifecycle::remember_shutdown`, as the staging pass marks it: a
/// unit-returning point, so its record has `cancel`.
#[horizon_mod_api_macros::point(slot = 0, name = "game_server_lifecycle_remember_shutdown", free)]
fn remember_shutdown(failure: &mut Option<String>, message: String) {
    SHUTDOWN_BODY.fetch_add(1, Ordering::Relaxed);
    if failure.is_none() {
        *failure = Some(message);
    }
}

/// `horizon_client::game::blocks::beacon::average`, as the staging pass marks it: a point that
/// returns an API value, so its record has `result`.
#[horizon_mod_api_macros::point(slot = 2, name = "game_beacon_average", free)]
fn average(a: u32, b: u32) -> u32 {
    AVERAGE_BODY.fetch_add(1, Ordering::Relaxed);
    (a + b) / 2
}

/// Leaves `cancel` alone: the replace contract, not the listener, keeps the body from running.
fn own_shutdown(_: &mut mod_points::GameServerLifecycleRememberShutdownArgs) {
    REPLACED.fetch_add(1, Ordering::Relaxed);
}

/// Produces the function's value from its inputs.
fn own_average(args: &mut mod_points::GameBeaconAverageArgs) -> u32 {
    args.a.max(args.b) * 10
}

fn other_average(_: &mut mod_points::GameBeaconAverageArgs) -> u32 {
    0
}

/// An ordinary head listener before the replace (higher priority): it may change the inputs,
/// and the replace sees the change.
fn raise_b(args: &mut mod_points::GameBeaconAverageArgs) {
    args.b += 1;
}

/// Ordinary head listeners after the replace's priority, trying to undo it: the replace runs
/// last whatever its priority, so they can't.
fn clear_average(args: &mut mod_points::GameBeaconAverageArgs) {
    args.result = None;
}

fn uncancel_shutdown(args: &mut mod_points::GameServerLifecycleRememberShutdownArgs) {
    args.cancel = false;
}

#[test]
fn a_replace_owns_the_function_and_only_one_may() {
    register(&mod_points::replace!(
        mod_points::listeners::game_server_lifecycle_remember_shutdown,
        cancel,
        none,
        "owner",
        0,
        "own_shutdown",
        own_shutdown
    ))
    .unwrap();
    register(&mod_points::replace!(
        mod_points::listeners::game_beacon_average,
        result,
        none,
        "owner",
        0,
        "own_average",
        own_average
    ))
    .unwrap();
    assert_eq!(
        register(&mod_points::replace!(
            mod_points::listeners::game_beacon_average,
            result,
            none,
            "rival",
            5,
            "other_average",
            other_average
        )),
        Err(RegisterError::Replaced {
            first: ("owner", "own_average"),
            second: ("rival", "other_average"),
        })
    );
    assert_eq!(
        RegisterError::Replaced {
            first: ("owner", "own_average"),
            second: ("rival", "other_average"),
        }
        .to_string(),
        "rival:other_average replaces a function that owner:own_average already replaces; \
         only one replace listener can own it"
    );
    register(&mod_points::listeners::game_beacon_average(
        "raiser", 100, "raise", raise_b,
    ))
    .unwrap();
    register(&mod_points::listeners::game_beacon_average(
        "clearer",
        -100,
        "clear",
        clear_average,
    ))
    .unwrap();
    register(
        &mod_points::listeners::game_server_lifecycle_remember_shutdown(
            "uncanceller",
            -100,
            "uncancel",
            uncancel_shutdown,
        ),
    )
    .unwrap();
    mod_points::__private::seal();
    __pointa_remember_shutdown();
    __pointa_average();

    let mut failure = None;
    remember_shutdown(&mut failure, "disk full".into());
    assert_eq!(REPLACED.load(Ordering::Relaxed), 1);
    assert_eq!(
        SHUTDOWN_BODY.load(Ordering::Relaxed),
        0,
        "the replaced body ran"
    );
    assert_eq!(failure, None);

    // b raised to 8 before the replace: 8 * 10, not the body's 5 nor a cleared result.
    assert_eq!(average(3, 7), 80);
    assert_eq!(
        AVERAGE_BODY.load(Ordering::Relaxed),
        0,
        "the replaced body ran"
    );
}

static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 4] =
    [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 4];
fn __fnr_read_slot_0() -> Option<fn(&mut Option<String>, String)> {
    let pointer = crate::__REPLACE[0].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(&mut Option<String>, String)>(pointer) })
    }
}
fn __fnr_read_body_0() -> Option<fn(&mut Option<String>, String)> {
    let pointer = crate::__REPLACE[1].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(&mut Option<String>, String)>(pointer) })
    }
}
fn __fnr_read_slot_2() -> Option<fn(u32, u32) -> u32> {
    let pointer = crate::__REPLACE[2].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(u32, u32) -> u32>(pointer) })
    }
}
fn __fnr_read_body_2() -> Option<fn(u32, u32) -> u32> {
    let pointer = crate::__REPLACE[3].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(u32, u32) -> u32>(pointer) })
    }
}
