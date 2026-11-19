// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
// It calls the places' dispatch itself, so it needs a build whose table has sites.
#![cfg(all(feature = "wasm-guest", feature = "listener-fixture"))]

use mod_api::wire::{Reader, Record, Writer};
use mod_points::{
    __private::{Forward, PointHost, SlotListener},
    PointId,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl mod_api::ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Quiet {}

#[unsafe(no_mangle)]
pub fn __horizon_mod_hooks() -> Box<dyn mod_api::Hooks> {
    Box::new(Quiet)
}

static HEAD: AtomicUsize = AtomicUsize::new(0);
static RETURN: AtomicUsize = AtomicUsize::new(0);
fn head(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    HEAD.fetch_add(1, Ordering::Relaxed);
    args.cancel = true;
}
fn tail(_: mod_api::HostSide<'_>, args: &mut mod_points::TickArgs) {
    RETURN.fetch_add(1, Ordering::Relaxed);
    args.cancel = false;
}

struct Loopback(u32);
impl Forward for Loopback {
    fn forward(&self, id: u32, host: PointHost<'_>, record: &mut dyn Record) {
        let mut input = Writer::new();
        input.put(&self.0);
        if let PointHost::Side(side) = host {
            input.put(&matches!(side, mod_api::HostSide::Server(_)));
        }
        loopback(id, input, record);
    }
}

fn loopback(id: u32, mut input: Writer, record: &mut dyn Record) {
    record.encode(&mut input);
    let pointer = mod_api::wire::guest::input(input.bytes().len());
    // SAFETY: input() reserves this many writable bytes until run().
    unsafe {
        std::ptr::copy_nonoverlapping(input.bytes().as_ptr(), pointer, input.bytes().len());
    }
    let (pointer, len) = mod_api::wire::guest::run(id);
    assert_ne!(len, u32::MAX);
    // SAFETY: run() retains this reply until the next call at this depth.
    let reply = unsafe { std::slice::from_raw_parts(pointer, len as usize) };
    let mut reply = Reader::new(reply);
    reply.take::<()>().unwrap();
    let top = id == mod_api::wire::POINTS + PointId::GameServerLinksListTop as u32;
    let top_return = id == mod_api::wire::POINTS + PointId::GameServerLinksListTopReturn as u32;
    if (top && BAD_HEAD.swap(false, Ordering::Relaxed))
        || (top_return && BAD_RETURN.swap(false, Ordering::Relaxed))
    {
        assert!(
            record
                .apply(&reply.rest()[..reply.rest().len() - 1])
                .is_err()
        );
    } else {
        record.apply(reply.rest()).unwrap();
    }
}

static BAD_HEAD: AtomicBool = AtomicBool::new(false);
static BAD_RETURN: AtomicBool = AtomicBool::new(false);
static TOP: AtomicUsize = AtomicUsize::new(0);
static TOP_RETURN: AtomicUsize = AtomicUsize::new(0);
static BODY: AtomicUsize = AtomicUsize::new(0);

/// The same listener source a native mod registers: it changes both by-value inputs, and on its
/// second call answers at once.
fn top(args: &mut mod_points::GameServerLinksListTopArgs) {
    assert_eq!((args.h, args.n, args.result), (3.0, 7, None));
    args.h = 999.0;
    args.n = 999;
    if TOP.fetch_add(1, Ordering::Relaxed) == 1 {
        args.result = Some(42.5);
    }
}

fn top_return(args: &mut mod_points::GameServerLinksListTopArgs) {
    let call = TOP_RETURN.fetch_add(1, Ordering::Relaxed);
    let value = args.result.unwrap();
    assert_eq!(value, if call == 1 { 10.0 } else { 1998.0 });
    args.result = Some(value + 1.0);
}

#[horizon_mod_api_macros::point(slot = 0, name = "game_server_links_list_top", free)]
fn fixture_list_top(h: f32, n: usize) -> f32 {
    BODY.fetch_add(1, Ordering::Relaxed);
    h + n as f32
}

static LOOPBACK_HEAD: Loopback = Loopback(0);
static LOOPBACK_RETURN: Loopback = Loopback(1);
static LOOPBACK_TOP: Loopback = Loopback(2);
static LOOPBACK_TOP_RETURN: Loopback = Loopback(3);

#[test]
fn head_and_return_share_the_hook_arena_and_native_listener_list() {
    mod_points::guest::register(&mod_points::listeners::tick("guest", 0, "head", head)).unwrap();
    mod_points::guest::register(&mod_points::listeners::tick_return(
        "guest", 0, "return", tail,
    ))
    .unwrap();
    // Each subscription names its place and carries that place's identity, which the game
    // looks up by name: a head and its return are two places.
    let listed = mod_points::guest::subscriptions();
    let expected = [(PointId::Tick, "head"), (PointId::TickReturn, "return")];
    assert_eq!(listed.len(), expected.len());
    for ((place, name, identity, inject), (id, want)) in listed.iter().zip(expected) {
        let (point_name, point_identity) = mod_points::point_identity(id);
        assert_eq!(*place, id as u32);
        assert_eq!(
            (name.as_str(), *identity, inject.as_str()),
            (point_name, point_identity, want)
        );
        assert_eq!(mod_points::point_by_name(point_name), Some(id));
    }
    assert_ne!(listed[0].1, listed[1].1);
    for (place, endpoint) in [
        (PointId::Tick, &LOOPBACK_HEAD),
        (PointId::TickReturn, &LOOPBACK_RETURN),
    ] {
        mod_api::__private::points::register(&SlotListener::forwarded(
            "guest",
            place.place(),
            endpoint,
            mod_api::wire::POINTS + place as u32,
        ))
        .unwrap();
    }
    mod_points::guest::register(&mod_points::listeners::game_server_links_list_top(
        "guest", 0, "top", top,
    ))
    .unwrap();
    mod_points::guest::register(&mod_points::listeners::game_server_links_list_top_return(
        "guest",
        0,
        "top_return",
        top_return,
    ))
    .unwrap();
    for (place, endpoint) in [
        (PointId::GameServerLinksListTop, &LOOPBACK_TOP),
        (PointId::GameServerLinksListTopReturn, &LOOPBACK_TOP_RETURN),
    ] {
        mod_api::__private::points::register(&SlotListener::forwarded(
            "guest",
            place.place(),
            endpoint,
            mod_api::wire::POINTS + place as u32,
        ))
        .unwrap();
    }
    mod_points::__private::seal();
    __pointa_fixture_list_top();
    let mut args = mod_points::TickArgs::__new();
    horizon_engine_slots::dispatch::tick(mod_api::HostSide::Server(&mut Quiet), &mut args);
    assert!(args.cancel);
    horizon_engine_slots::dispatch::tick_return(mod_api::HostSide::Server(&mut Quiet), &mut args);
    assert!(!args.cancel);
    assert_eq!(
        (HEAD.load(Ordering::Relaxed), RETURN.load(Ordering::Relaxed)),
        (1, 1)
    );
    assert_eq!(
        fixture_list_top(3.0, 7),
        1999.0,
        "head writes replace by-value inputs; the return listener changes the outcome"
    );
    assert_eq!(
        fixture_list_top(3.0, 7),
        42.5,
        "a head result skips the body"
    );
    assert_eq!(BODY.load(Ordering::Relaxed), 1);
    assert_eq!(TOP.load(Ordering::Relaxed), 2);
    BAD_HEAD.store(true, Ordering::Relaxed);
    assert_eq!(
        fixture_list_top(3.0, 7),
        11.0,
        "a rejected head reply applies no writes; the valid return listener still adds one"
    );
    assert_eq!(BODY.load(Ordering::Relaxed), 2);
    BAD_RETURN.store(true, Ordering::Relaxed);
    assert_eq!(
        fixture_list_top(3.0, 7),
        1998.0,
        "a rejected return reply does not replace the outcome"
    );
    assert_eq!(BODY.load(Ordering::Relaxed), 3);
    assert_eq!(TOP_RETURN.load(Ordering::Relaxed), 3);
    mod_points::stop("guest");
    horizon_engine_slots::dispatch::tick(mod_api::HostSide::Server(&mut Quiet), &mut args);
    assert!(!args.cancel);
    assert_eq!(HEAD.load(Ordering::Relaxed), 1);
    assert_eq!(fixture_list_top(3.0, 7), 10.0);
    assert_eq!(TOP.load(Ordering::Relaxed), 4);
    assert_eq!(TOP_RETURN.load(Ordering::Relaxed), 3);
}

#[test]
fn malformed_writeback_is_atomic() {
    let mut args = mod_points::TickArgs::__new();
    assert!(
        args.apply(&[1, 0]).is_err(),
        "trailing bytes are not a partial update"
    );
    assert!(!args.cancel);
    assert!(args.apply(&[]).is_err());
    assert!(!args.cancel);
    args.apply(&[1]).unwrap();
    assert!(args.cancel);
}

static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 2] =
    [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 2];
fn __fnr_read_slot_0() -> Option<fn(f32, usize) -> f32> {
    let pointer = crate::__REPLACE[0].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(f32, usize) -> f32>(pointer) })
    }
}
fn __fnr_read_body_0() -> Option<fn(f32, usize) -> f32> {
    let pointer = crate::__REPLACE[1].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(f32, usize) -> f32>(pointer) })
    }
}

/// Every API place, head and return, has a name the game finds it by and an identity.
#[test]
fn every_place_is_found_by_its_name() {
    let mut names = std::collections::HashSet::new();
    for &(place, ..) in mod_points::API_PLACES {
        let (name, _) = mod_points::point_identity(place);
        assert!(!name.is_empty(), "{place:?}");
        assert!(names.insert(name), "{name} twice");
        assert_eq!(mod_points::point_by_name(name), Some(place), "{name}");
    }
    assert!(names.iter().any(|name| name.ends_with("_return")));
    assert_eq!(mod_points::point_by_name("no_such_place"), None);
}
