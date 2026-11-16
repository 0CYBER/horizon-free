//! Host assertions shared by the native and real-member integration executables.
use std::sync::atomic::{AtomicUsize, Ordering};
static BODIES: AtomicUsize = AtomicUsize::new(0);

#[horizon_mod_api_macros::point(slot = 6, name = "listener_fixture_sum", free)]
fn sum(a: u32, b: &u32) -> u32 {
    BODIES.fetch_add(1, Ordering::SeqCst);
    a + *b
}

pub fn baseline() {
    let borrowed = 7;
    assert_eq!(sum(3, &borrowed), 10);
    assert_eq!(borrowed, 7);
    assert_eq!(BODIES.swap(0, Ordering::SeqCst), 1);
}

pub fn check() {
    __pointa_sum();
    let borrowed = 7;
    assert_eq!(sum(3, &borrowed), 12);
    assert_eq!(borrowed, 7);
    assert_eq!(BODIES.swap(0, Ordering::SeqCst), 1);

    let off = std::collections::BTreeMap::from([("lit".into(), "false".into())]);
    let on = std::collections::BTreeMap::from([("lit".into(), "true".into())]);
    // The compiled fixture mod also asks for the water cauldron (its host-read checks), so the
    // completed registry holds it; this harness gives every block the same two-state lamp shape.
    mod_api::__private::resolve_block_keys(["fixture:lamp", "minecraft:water_cauldron"], |_| {
        horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&off, &on], 0)
            .map_err(|error| format!("{error:?}"))
    })
    .unwrap();
    let block = mod_api::__private::block_key("fixture:lamp").unwrap();
    let first = mod_api::BlockState::from(mod_api::BlockStateRecord::new(block, 0));
    let second = mod_api::BlockState::from(mod_api::BlockStateRecord::new(block, 1));
    let text = mod_api::Text::literal("fixture: café 水");
    let mut args = mod_points::ListenerFixtureMirrorsArgs::__new(text.clone(), first, second);
    horizon_engine_slots::dispatch::listener_fixture_mirrors(&mut args);
    assert!(args.cancel, "the mirror listener must have executed");
    assert_eq!(args.text, mod_api::Text::literal("fixture: café 水"));
    let metadata = mod_api::__private::block_definitions().unwrap();
    for (state, ordinal, lit) in [(args.first, 0, "false"), (args.second, 1, "true")] {
        assert_eq!(state.ordinal(), ordinal);
        assert_eq!(
            mod_api::BlockMetadata::name(metadata, state.block()),
            Ok("fixture:lamp")
        );
        assert_eq!(state.property(metadata, "lit"), Ok(Some(lit)));
    }
    assert_eq!(text, mod_api::Text::literal("fixture: café 水"));
}

fn __fnr_read_slot_6() -> Option<fn(u32, &u32) -> u32> {
    let pointer = crate::__REPLACE[6].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(u32, &u32) -> u32>(pointer) })
    }
}
fn __fnr_read_body_6() -> Option<fn(u32, &u32) -> u32> {
    let pointer = crate::__REPLACE[7].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(u32, &u32) -> u32>(pointer) })
    }
}
