//! StrictMath-exact operations, pure on native and wasm, with identical bits.
//!
//! The fdlibm `sin`, `cos`, `tan`, `exp` and `pow` algorithms use ordered IEEE
//! double operations without FMA or platform/libm calls. They are linked into
//! each guest: no operation crosses a Host import. NaN payloads are unspecified.
pub use horizon_core::jmath::fdlibm::{cbrt, cos, exp, hypot, log, log10, pow, sin, tan};
pub use horizon_core::jmath::{acos, asin, atan, atan2};
#[cfg(test)]
#[path = "strict_math_tests.rs"]
mod tests;
