use super::*;

impl Game<'_> {
    /// The authoritative tick of one general entity row of type `kind`
    /// (`mymod:robot`) with ID `entity` (arismoko/horizon#792 stage 4): `ServerLevel
    /// .tickNonPassenger`'s `entity.tick()` for a row the type registry owns,
    /// its rider subtree ticked after it. Read and change the row's data
    /// through its attachments; a part hears only the type it is attached
    /// to. Vanilla families never reach this call. Server-side.
    pub(in crate::game) fn native_general_entity_tick(
        &mut self,
        kind: &str,
        entity: mod_api::EntityRef,
    ) {
        self.fire_general_entity_tick(kind, entity);
    }
}
