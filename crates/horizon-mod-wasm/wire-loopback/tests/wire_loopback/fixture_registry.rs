use super::*;

pub(super) struct FixtureRegistryHooks;
impl FixtureRegistryHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        cx.warn("wire_test.warn.optional", &["café", "jei"]);
        REGISTERED_ITEM.set(Some(registrar.item_key("minecraft:stone")));
        REGISTERED_BLOCK.set(Some(registrar.block_key("minecraft:stone")));
        REGISTERED_EFFECT.set(Some(registrar.effect_key("minecraft:poison")));
        let kind = registrar.block_entity_type("counter").blocks(&[]).build();
        REGISTERED_KIND.set(Some(kind));
        registrar.block_entity_part(&kind, MachineBlock).unwrap();
    }
}
