#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
pub use crate::parts::tests as part_tests;
pub use mod_vocab::__private::*;

/// Validate registrations above the engine, deriving lookup indexes below it.
pub fn install_parts(
    mods: &[crate::ModEntry],
    schema: &RuleSchema<'_>,
    dimension_shape: fn(&crate::DimensionSettings) -> bool,
) -> Result<(), Vec<String>> {
    initialize().map_err(|reason| vec![reason.to_owned()])?;
    crate::parts::install_parts(mods, schema, dimension_shape)
}
#[path = "profiler.rs"]
pub mod profiler;

/// Install platform instrumentation before any mod registration or dispatch.
/// A first guard entry seals the socket; late initialization is refused without
/// replacing a backend or disturbing an in-flight call.
pub fn initialize() -> Result<(), &'static str> {
    crate::content::crash::ensure_installed()?;
    crate::wire::initialize()
}

/// Install instrumentation before publishing the process-wide mod list.
/// This engine-only boundary treats conflicting initialization as an invariant
/// failure. User-facing startup checks `initialize` first and refuses to launch.
pub fn install(mods: crate::Mods) {
    initialize().expect("platform initialization must succeed before publishing mods");
    mod_vocab::__private::install(mods);
}

#[doc(hidden)]
pub use crate::load_state::{install_packages, install_registration_data};
#[cfg(any(test, feature = "testing"))]
#[doc(hidden)]
pub use crate::load_state::{with_test_packages, with_test_registration_data};

pub use crate::equip_reads::{
    EquipReadGuard, EquipReadSet, cached_equip_alive, cached_equip_equipment,
    cached_equip_hand_item, cached_equip_health, cached_equip_kind, cached_equip_living,
    cached_equip_max_health, cached_equip_position, enter_equip_reads,
};
