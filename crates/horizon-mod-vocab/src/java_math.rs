//! Engine paths used by vanilla Math call sites, including machine-dependent rounding.
//! These mirror horizon_core::jmath; they are not a promise of universal HotSpot
//! bit equality. Use strict_math for established portable StrictMath operations.
#[cfg(not(target_arch = "wasm32"))]
pub use horizon_core::jmath::{
    acos, asin, atan, atan2, cbrt, cos, exp, hypot, log, log10, pow, round, round_f32, sin, tan,
};
#[cfg(target_arch = "wasm32")]
mod unavailable {
    #![allow(missing_docs, unused_variables)]
    #[diagnostic::on_unimplemented(
        message = "java_math is native only; use mod_api::strict_math on wasm"
    )]
    // hidden: the unimplementable bound that makes wasm calls fail to compile with the message above.
    #[doc(hidden)]
    pub trait NativeOnly: private::Sealed {}
    mod private {
        pub trait Sealed {}
    }
    pub fn round_f32(a: impl NativeOnly) -> i32 {
        unreachable!()
    }
    pub fn round(x: impl NativeOnly) -> i64 {
        unreachable!()
    }
    pub fn sin(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn cos(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn tan(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn exp(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn log(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn log10(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn cbrt(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn atan(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn asin(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn acos(x: impl NativeOnly) -> f64 {
        unreachable!()
    }
    pub fn pow(x: impl NativeOnly, y: f64) -> f64 {
        unreachable!()
    }
    pub fn atan2(x: impl NativeOnly, y: f64) -> f64 {
        unreachable!()
    }
    pub fn hypot(x: impl NativeOnly, y: f64) -> f64 {
        unreachable!()
    }
}
#[cfg(target_arch = "wasm32")]
pub use unavailable::*;
