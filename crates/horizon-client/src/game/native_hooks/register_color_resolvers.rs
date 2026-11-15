use super::*;

impl Game<'_> {
    /// The colour resolvers block tints blend over the biomes around a block (NeoForge's
    /// `RegisterColorHandlersEvent.ColorResolvers`): `register(resolver)` adds the mod's own
    /// `BiomeColorResolver` and returns the `ColorResolver` its block tint sources pass to
    /// `BlockAndTintGetter::block_tint`. Asked once at start on the client, before
    /// `register_block_tint_sources` and before resources load.
    pub(in crate::game) fn native_register_color_resolvers(
        context: &mod_api::RegistrationContext,
        resolvers: &mut dyn ColorResolvers,
    ) {
        Self::fire_register_color_resolvers(context, resolvers);
    }
}
