use super::*;

impl Game<'_> {
    /// The tint sources of blocks (NeoForge's `RegisterColorHandlersEvent.BlockTintSources`,
    /// vanilla's `BlockColors.register`): `register(layers, blocks)`, and a model face with
    /// `tintindex` i is multiplied by layer i of its block's list; a face past the list is not
    /// tinted. A layer is one of vanilla's sources (`VanillaTint`) or the mod's own
    /// `BlockTintSource`. Blocks are ids, a mod's or vanilla's, and a later registration for a
    /// block replaces the earlier one, vanilla's own colours included. Asked once at start on the
    /// client, before resources load; every resource load, a pack reload included, applies the
    /// registrations again. An id no block has is logged and skipped.
    pub(in crate::game) fn native_register_block_tint_sources(
        context: &mod_api::RegistrationContext,
        sources: &mut dyn BlockTintSources,
    ) {
        Self::fire_register_block_tint_sources(context, sources);
    }
}
