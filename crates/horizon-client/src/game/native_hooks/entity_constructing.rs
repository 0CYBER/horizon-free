use super::*;

impl Game<'_> {
    /// `EntityEvent.EntityConstructing`: a new entity of type `kind`, whose attachments are
    /// `holder`'s, is being made on this side (see `modding/entity_initialization.rs`).
    pub(in crate::game) fn native_entity_constructing(
        &mut self,
        holder: mod_api::Holder,
        kind: &str,
    ) {
        self.fire_entity_constructing(holder, kind);
    }
}
