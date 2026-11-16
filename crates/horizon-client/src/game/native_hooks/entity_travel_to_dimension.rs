use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `EntityTravelToDimensionEvent`: `entity` is about to travel to the level
    /// `dimension` because of `cause` (`nether_portal`, `end_portal`,
    /// `end_gateway` or `teleport`), before it moves. Set `cancel` to keep it
    /// where it is (`server_travel.rs`).
    pub(in crate::game) fn native_entity_travel_to_dimension(
        &mut self,
        entity: EntityRef,
        dimension: &str,
        cause: &str,
        cancel: &mut bool,
    ) {
        self.fire_entity_travel_to_dimension(entity, dimension, cause, cancel);
    }
}
