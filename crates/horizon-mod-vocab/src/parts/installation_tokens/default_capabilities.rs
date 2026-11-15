//! Engine access to the same type-bound default providers registration queries observe.
/// This block's first enabled item-container declaration.
pub fn block_container(block: &str) -> Option<crate::ModContainer> {
    crate::parts::DefaultCapabilitiesCapabilityProviders::block_container(
        &crate::InstalledProviders,
        block,
    )
}
/// This block's first enabled fluid-tank declaration.
pub fn block_fluids(block: &str) -> Option<crate::ModFluidTanks> {
    crate::parts::DefaultCapabilitiesCapabilityProviders::block_fluids(
        &crate::InstalledProviders,
        block,
    )
}
/// This block's first enabled energy-buffer declaration.
pub fn block_energy(block: &str) -> Option<crate::ModEnergyBuffer> {
    crate::parts::DefaultCapabilitiesCapabilityProviders::block_energy(
        &crate::InstalledProviders,
        block,
    )
}
/// Run only this item's registered dynamic callbacks at the normal runtime lookup moment.
pub fn item_container_slots(host: &dyn crate::Host, item: &str) -> Option<u32> {
    let table = host.capability_providers();
    let mut index = 0;
    while let Some(provider) = table.item_container_provider(item, &mut index) {
        if let Some(slots) = provider.slots(host) {
            return Some(slots);
        }
    }
    None
}
