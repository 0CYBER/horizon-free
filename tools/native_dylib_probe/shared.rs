//! Exact-build probe dependency, deliberately loaded once for both consumers.
use std::sync::atomic::{AtomicUsize, Ordering};
pub static STATE: AtomicUsize = AtomicUsize::new(0);
pub fn bump() -> usize {
    STATE.fetch_add(1, Ordering::SeqCst)
}
pub fn identity() -> [usize; 2] {
    std::hint::black_box(horizon_mod_host::main_with as *const ());
    [
        &STATE as *const _ as usize,
        &mod_points::__private::POINTS as *const _ as usize,
    ]
}
pub fn exchange(value: Vec<u8>) -> Vec<u8> {
    assert_eq!(value, [1, 2, 3]);
    mod_api::content::crash::set_detail("linkage probe", "shared dependency");
    vec![4, 5, 6]
}
