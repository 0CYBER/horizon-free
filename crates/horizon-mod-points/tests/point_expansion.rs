//! The API-only point boundary copies inputs and outputs and skips an unlistened path.
use horizon_mod_api_macros::point;
use std::sync::atomic::Ordering;

extern crate self as horizon_engine_slots;
pub use mod_points::*;

mod mod_points {
    pub struct DoubleArgs {
        pub value: i64,
        pub result: Option<i64>,
    }
    impl DoubleArgs {
        pub fn __new(value: i64) -> Self {
            Self {
                value,
                result: None,
            }
        }
    }
    pub enum PointId {
        Double,
        DoubleReturn,
    }
    pub mod __private {
        pub use mod_api::__private::points::cold;
        use std::sync::atomic::AtomicBool;
        pub static POINTS: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];
    }
    pub mod dispatch {
        pub fn double(args: &mut super::DoubleArgs) {
            args.value += 1;
            if args.value < 0 {
                args.result = Some(42);
            }
        }
        pub fn double_return(args: &mut super::DoubleArgs) {
            args.result = args.result.map(|n| n + 10);
        }
    }
}

#[point(slot = 0, name = "double", free)]
fn double(value: i64) -> i64 {
    value * 2
}

#[test]
fn api_head_body_return_and_early_result() {
    use mod_points::__private::POINTS;
    assert_eq!(double(2), 4);
    POINTS[0].store(true, Ordering::Release);
    __pointa_double();
    assert_eq!(double(2), 6);
    POINTS[1].store(true, Ordering::Release);
    assert_eq!(double(2), 16);
    assert_eq!(
        double(-2),
        42,
        "head result skips body and return listeners"
    );
}

static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 2] =
    [const { std::sync::atomic::AtomicPtr::new(std::ptr::null_mut()) }; 2];
fn __fnr_read_slot_0() -> Option<fn(i64) -> i64> {
    let pointer = crate::__REPLACE[0].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(i64) -> i64>(pointer) })
    }
}
fn __fnr_read_body_0() -> Option<fn(i64) -> i64> {
    let pointer = crate::__REPLACE[1].load(std::sync::atomic::Ordering::Relaxed);
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute::<*mut (), fn(i64) -> i64>(pointer) })
    }
}
