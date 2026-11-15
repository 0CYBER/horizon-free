//! The wasm fixture's shutdown listener, registered natively: the same answers as `real_member`.
#[path = "fixture/src/lending.rs"]
mod lending;
#[path = "fixture/src/matrix.rs"]
mod matrix;
#[path = "support/checks.rs"]
mod matrix_checks;
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[path = "fixture/src/replace.rs"]
mod replace;
#[path = "fixture/src/shutdown.rs"]
mod shutdown;
#[path = "fixture/src/turn.rs"]
mod turn;

/// `horizon_client::game::server_lifecycle::remember_shutdown`, as the staging pass marks it.
#[horizon_mod_api_macros::point(slot = 0, name = "game_server_lifecycle_remember_shutdown", free)]
fn remember_shutdown(failure: &mut Option<String>, message: String) {
    if failure.is_none() {
        *failure = Some(message);
    }
}

/// `horizon_client::game::beacon::average`, as the staging pass marks it.
#[horizon_mod_api_macros::point(slot = 2, name = "game_beacon_average", free)]
fn average(a: u32, b: u32) -> u32 {
    replace::BODY_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    (a + b) / 2
}

/// `horizon_client::game::commands::wrap_degrees` (`Mth.wrapDegrees`), as the staging pass
/// marks it.
#[horizon_mod_api_macros::point(slot = 4, name = "game_commands_wrap_degrees", free)]
fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = angle % 360.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

#[test]
fn the_shared_listener_source_answers_natively_as_in_wasm() {
    // Before any dispatch: a first dispatch without the platform seals instrumentation off
    // for the whole process, and the other test of this binary (`with_mods`) then refuses.
    mod_api::__private::initialize().unwrap();
    matrix_checks::baseline();
    // Without a listener the body wraps the angle as given.
    assert_eq!(wrap_degrees(10.0), 10.0);
    // Without a listener the body averages.
    assert_eq!(average(3, 7), 5);
    assert_eq!(
        replace::BODY_CALLS.swap(0, std::sync::atomic::Ordering::SeqCst),
        1
    );
    // Without a listener the body runs as written.
    let mut failure = None;
    remember_shutdown(&mut failure, "disk full".into());
    assert_eq!(failure.as_deref(), Some("disk full"));
    mod_api::__private::points::register(
        &mod_points::listeners::game_server_lifecycle_remember_shutdown(
            "native",
            0,
            "shutdown",
            shutdown::on_shutdown,
        )
        .ordered(0, "shutdown"),
    )
    .unwrap();
    mod_api::__private::points::register(&mod_points::replace!(
        mod_points::listeners::game_beacon_average,
        result,
        none,
        "native",
        0,
        "average",
        replace::own_average
    ))
    .unwrap();
    mod_api::__private::points::register(&mod_points::listeners::listener_fixture_sum_return(
        "native",
        0,
        "sum",
        matrix::sum_return,
    ))
    .unwrap();
    mod_api::__private::points::register(&mod_points::listeners::listener_fixture_mirrors(
        "native",
        0,
        "mirrors",
        matrix::mirrors,
    ))
    .unwrap();
    // A head listener changing its argument (mixin `@ModifyVariable` on a parameter).
    mod_api::__private::points::register(&mod_points::listeners::game_commands_wrap_degrees(
        "native",
        0,
        "turn",
        turn::turn_angle,
    ))
    .unwrap();
    mod_points::__private::seal();
    __pointa_remember_shutdown();
    __pointa_average();
    __pointa_wrap_degrees();
    matrix_checks::check();
    for (before, message, after) in shutdown::CASES {
        let mut failure = before.map(String::from);
        remember_shutdown(&mut failure, message.into());
        assert_eq!(failure.as_deref(), after, "{message}");
    }
    for (a, b, value) in replace::CASES {
        assert_eq!(average(a, b), value, "{a} {b}");
        assert_eq!(
            replace::BODY_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }
    for (angle, wrapped) in turn::CASES {
        assert_eq!(wrap_degrees(angle), wrapped, "{angle}");
    }
}

static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 8] =
    [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 8];
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
fn __fnr_read_slot_4() -> Option<fn(f32) -> f32> {
    let pointer = crate::__REPLACE[4].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(f32) -> f32>(pointer) })
    }
}
fn __fnr_read_body_4() -> Option<fn(f32) -> f32> {
    let pointer = crate::__REPLACE[5].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(f32) -> f32>(pointer) })
    }
}

#[test]
fn collect_positions_copy_in_and_out_natively_as_in_the_real_guest() {
    struct Quiet;
    impl mod_api::Host for Quiet {}
    mod_api::impl_host_areas!(Quiet, except(items));
    impl mod_api::prelude::ItemsHost for Quiet {
        fn item_key(&self, id: &mod_api::Identifier) -> Option<mod_api::ItemKey> {
            mod_api::__private::item_key_by_identifier(id)
        }
    }
    impl mod_api::ServerHost for Quiet {}
    mod_api::impl_server_host_areas!(Quiet);
    mod_api::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "native_lender",
            Box::new(lending::Lender),
        )]
        .into(),
        || {
            mod_api::__private::mark_game_thread();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            lending::check(&mut Quiet);
            mod_api::__private::resolve_items(["minecraft:bow"]).unwrap();
            lending::check_death(
                &mut Quiet,
                mod_api::__private::item_key("minecraft:bow").unwrap(),
            );
        },
    );
}
