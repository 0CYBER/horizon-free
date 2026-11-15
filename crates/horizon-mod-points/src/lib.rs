//! The game's injection points (arismoko/horizon#589), for mods that listen at them. Every engine function the
//! staging pass admits with an API record is a point: a mod listens at its head (before the body
//! runs), at its return (after), or replaces its outcome
//! through manifest `[[inject]]` functions, with no game rebuild. Each listener gets the
//! point's record: the arguments the API carries, and `cancel` or `result` where the function's
//! return allows. Each declaration names the target, access and listener function.
//!
//! Depend on this crate only to listen at points: it is large (one record per point), and every
//! crate that depends on it compiles a few seconds slower. Hooks, handles and parts are in
//! `mod_api`.
#![deny(missing_docs)]
// Its tests are integration tests, so the unit-test build (`cargo test --lib`) is empty rather than
// a second compile of the generated points.
#![cfg(not(test))]

include!(concat!(env!("OUT_DIR"), "/points_generated.rs"));

mod registry;
#[cfg(target_arch = "wasm32")]
pub use guest::register;
#[cfg(not(target_arch = "wasm32"))]
pub use registry::register;
pub use registry::{RegisterError, stop};

/// A `replace` listener at a point's head: the function is the listener's, as with a mixin
/// overwrite, so the body never runs and the listener produces the outcome. It runs after the
/// place's other API head listeners whatever its priority (they may change its inputs, not its
/// outcome).
///
/// `replace!(constructor, returns, host, mod_id, priority, inject_id, listener)`: `constructor`
/// is the place's typed listener constructor in its table (`mod_points::listeners::tick`). `returns` is the
/// point's `returns` in `points.json`: at a `cancel` point the listener returns `()` and the cancel
/// is forced; at a `result` point it returns the record's `result` type, which the function
/// returns. A point whose return has no API form (`none`) has no replace listener. `host` is `host`
/// when the point's API listeners take one, `none` otherwise. The slot is marked
/// [`__private::SlotListener::replacing`], so a second replace listener at the place is refused.
///
/// The launcher's generated `main` and `cargo horizon-mod build --wasm` both emit it, so native
/// and wasm mods share one contract.
///
#[macro_export]
macro_rules! replace {
    ($constructor:path, cancel, host, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(cancel, host, $listener),
        )
        .replacing()
    };
    ($constructor:path, cancel, none, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(cancel, none, $listener),
        )
        .replacing()
    };
    ($constructor:path, result, host, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(result, host, $listener),
        )
        .replacing()
    };
    ($constructor:path, result, none, $id:expr, $priority:expr, $inject:expr, $listener:path) => {
        $constructor(
            $id,
            $priority,
            $inject,
            mod_api::replacement_listener!(result, none, $listener),
        )
        .replacing()
    };
}

#[cfg(any(target_arch = "wasm32", feature = "wasm-guest"))]
pub mod guest;

// hidden: the game's side of the points, not part of the API a mod is written against
#[doc(hidden)]
pub mod __private {
    pub use crate::registry::{POINTS, TABLE, seal};
    pub use mod_api::__private::points::{ApiKind, Forward, PointHost, SlotListener};
}
