//! Real guest host-call fixture. The harness host deliberately refuses launch;
//! this checks transport and refusal, not a simulated Game or wasm gameplay parity.
pub struct OwnerLaunch;
#[mod_api::horizon_part]
impl mod_api::BlockPart for OwnerLaunch {}
#[mod_api::horizon_part]
impl mod_api::BlockEntityPart for OwnerLaunch {
    fn block_entity_tick(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::BlockEntityTickEvent<'_>,
    ) {
        let id = mod_api::Identifier::parse("minecraft:snowball").unwrap();
        let Some(item) = host.item_key(&id) else {
            event.entity.data = vec![1];
            return;
        };
        let stack = mod_api::ItemStack::new(item, 16).unwrap();
        let owner =
            mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 21));
        let shot = host.spawn_projectile_from_rotation(owner, &id, &stack, 0.0, 1.5, 1.0);
        event.entity.data = vec![if shot.is_none() { 2 } else { 3 }];
    }
}
