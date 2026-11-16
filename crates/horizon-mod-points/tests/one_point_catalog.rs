//! One point catalog (arismoko/horizon#3643): when `horizon-engine-slots` has a `PointId` of its own, it is
//! `mod_points`' very type, re-exported, never a second generation, or this test does not
//! compile. Today the engine crate has none (`build.rs` sets `engine_slots_point_id` once it
//! does), so `mod_points`' generated enum is the only one.
#[cfg(engine_slots_point_id)]
const _: fn(horizon_engine_slots::PointId) -> mod_points::PointId = |id| id;

#[test]
fn the_point_id_is_mod_points_own_until_the_engine_crate_has_one() {
    let name = std::any::type_name::<mod_points::PointId>();
    if cfg!(engine_slots_point_id) {
        assert!(name.starts_with("horizon_engine_slots::"), "{name}");
    } else {
        assert!(name.starts_with("mod_points::"), "{name}");
    }
}
