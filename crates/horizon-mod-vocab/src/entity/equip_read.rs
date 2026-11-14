//! Read facts transferred once with a prepared equipment decision.
use crate::*;
/// One target's immutable read view; equipment uses EquipmentSlot order.
// hidden: engine/guest preparation metadata, not an additional public Host read API.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, horizon_mod_api_macros::Wire)]
// exhaustive: transport data for one scoped equipment decision
pub struct EquipReadRow {
    /// The target handle captured before traversal.
    pub entity: EntityRef,
    /// Its resolved entity-type key, if available.
    pub kind: Option<EntityTypeKey>,
    /// Its position in blocks, if available.
    pub position: Option<[f64; 3]>,
    /// Current health from the prepared view.
    pub health: Option<f32>,
    /// Maximum health from the prepared view.
    pub max_health: Option<f32>,
    /// Whether the prepared target is alive.
    pub alive: Option<bool>,
    /// Whether the target exposes living-entity reads.
    pub living: bool,
    /// Eight equipment slots in EquipmentSlot order; empty or unsupported slots are None.
    pub equipment: [Option<ItemStack>; 8],
}
impl EquipReadRow {
    /// The row for `entity` with nothing read yet: no kind, position or health, not living,
    /// every equipment slot empty. The engine fills in what it prepared.
    pub fn new(entity: EntityRef) -> Self {
        Self {
            entity,
            kind: None,
            position: None,
            health: None,
            max_health: None,
            alive: None,
            living: false,
            equipment: Default::default(),
        }
    }
}
