use super::*;
#[path = "../../../tests/fixture/src/bulk_files.rs"]
mod consumer;
#[path = "../../../tests/support/bulk_files.rs"]
mod support;

#[test]
fn file_capabilities_keep_roots_and_refusals_on_native_and_guest() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let world = support::World::default();
    consumer::check(&world);
    let mut frame = boundary::Frame::default();
    frame.set_server_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        // SAFETY: the host and its frame live through all synchronous guest calls.
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let _transport = TransportScope(None);
    consumer::check(&guest::Guest);
    world
        .world
        .no_world
        .store(true, std::sync::atomic::Ordering::Relaxed);
    consumer::no_world(&world);
    consumer::no_world(&guest::Guest);
}

#[test]
fn world_file_capability_requires_server_host_and_names_wrong_phase() {
    mod_api::__private::initialize().unwrap();
    let world = support::World::default();
    let mut frame = boundary::Frame::default();
    // A client or generic Host must not acquire the world service merely by selecting it.
    frame.set_host(&world);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let id = boundary::import_id("ConfigFiles", "read").unwrap();
    let mut input = Writer::new();
    input.put(&2u8);
    input.put(&mod_api::ConfigPath::new("tiles/a.bin").unwrap());
    let mut out = Writer::new();
    {
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        assert!(matches!(
            unsafe { boundary::serve(id, &mut cx, input.bytes(), &mut out) },
            Err(boundary::ServeError::Missing("server_host"))
        ));
        assert!(out.bytes().is_empty());
    }
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Packages);
    assert!(matches!(
        unsafe { boundary::serve(id, &mut cx, input.bytes(), &mut out) },
        Err(boundary::ServeError::Phase {
            operation: "world_data_files",
            phase: mod_api::__private::ModPhase::Packages,
        })
    ));
    assert!(out.bytes().is_empty());
}

#[test]
fn file_capability_selector_is_bounded_before_host_access() {
    let id = boundary::import_id("ConfigFiles", "read").unwrap();
    let mut cx = boundary::Cx::default();
    for root in 3..=u8::MAX {
        let mut input = Writer::new();
        input.put(&root);
        let mut out = Writer::new();
        assert!(matches!(
            unsafe { boundary::serve(id, &mut cx, input.bytes(), &mut out) },
            Err(boundary::ServeError::Decode(WireError::Invalid(
                "file capability root"
            )))
        ));
        assert!(out.bytes().is_empty());
    }
}
