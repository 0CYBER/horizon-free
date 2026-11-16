//! Process-lifetime math selection, shared by native and wasm hosts.
#![allow(unused_variables)]
use crate::*;
/// Math selection reads available through [`Host`] on both sides.
#[horizon_mod_api_macros::host_area]
pub trait MathHost {
    /// Side: both.
    /// The native engine's fixed selection for the seven platform-dependent
    /// Java Math operations. This read allocates nothing on the native path.
    /// It identifies behavior, not the machine, and does not implement guest
    /// math. See [`MathProfile`] for the exact mixed paths and guest limitations.
    #[access(Metadata)]
    fn math_profile(&self) -> MathProfile;
}
