use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Override an entity's outline colour.
    pub(in crate::game) fn native_entity_outline(
        &self,
        entity_id: u32,
        kind: &str,
        outline: &mut Option<u32>,
    ) {
        self.fire_entity_outline(entity_id, kind, outline);
    }
}
