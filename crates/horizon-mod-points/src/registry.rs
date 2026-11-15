//! The game's slot table, in the one registry every point table shares
//! (`mod_api::__private::points`).
use crate::PointId;
use mod_api::__private::points::{Place, Table};

#[cfg(not(target_arch = "wasm32"))]
pub use mod_api::__private::points::register;
pub use mod_api::__private::points::{RegisterError, stop};

/// Seal listener registration after installing the engine's point table.
pub fn seal() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // SAFETY: the table accepts typed listeners only through registrations whose
        // contract checks the catalog index, host, and canonical record signature.
        unsafe { horizon_engine_slots::install_points(&TABLE) }
            .expect("installing the game's point provider");
    });
    mod_api::__private::points::seal();
}

/// Each place's slot: set once, at the seal, where a mod listens. A point that nothing listens at
/// costs one load of it.
pub use horizon_engine_slots::__private::POINTS;
/// The game's API point table. Only the prebuilt game has the points' sites; any other build's
/// table refuses every listener by mod and target.
pub static TABLE: Table = if crate::STAGED {
    Table::new(&POINTS)
} else {
    Table::without_sites(&POINTS, crate::POINT_TARGETS)
};

impl PointId {
    /// This place in the game's table, for a listener's constructor.
    pub const fn place(self) -> Place {
        Place::new(&TABLE, self as usize)
    }
}
