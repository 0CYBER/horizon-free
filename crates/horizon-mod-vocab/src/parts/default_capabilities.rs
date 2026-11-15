//! Type-bound declarations consumed by the game's default capability providers.
use super::*;
use crate::wire::{Reader, Wire, WireError, Writer};

/// A registered item's dynamic slot count. The item type is bound at registration; this
/// callback runs only for an actual runtime lookup, never to infer whether it is registered.
pub trait ItemContainerProvider: Send + Sync {
    /// The item's slot count now, or none to let the next registration answer.
    fn slots(&self, host: &dyn Host) -> Option<u32>;
}

crate::wire::wire_handlers::handler!(dyn ItemContainerProvider, "ItemContainerProvider");

impl<F: Fn(&dyn Host) -> Option<u32> + Send + Sync> ItemContainerProvider for F {
    fn slots(&self, host: &dyn Host) -> Option<u32> {
        self(host)
    }
}

impl dyn Registrar + '_ {
    /// Register the dynamic default item-container provider for this item type.
    pub fn item_container(
        &mut self,
        item: &crate::ItemKey,
        provider: impl ItemContainerProvider + 'static,
    ) {
        self.register_item_container(item, Arc::new(provider));
    }
}

/// Explicit domains of the game's default item, fluid and energy providers.
pub trait DefaultCapabilitiesRegistrar {
    /// Declare this mod block's item slots and optional menu. Vanilla blocks must use
    /// `block_capability` instead of replacing the game's block entity ownership.
    /// The normal game table builder validates slots and faces; a provider with no slots is registered.
    fn block_container(&mut self, block: &crate::BlockKey, container: crate::ModContainer);
    /// Declare this block's fluid tanks, validated by the normal game table builder.
    fn block_fluids(&mut self, block: &crate::BlockKey, fluids: crate::ModFluidTanks);
    /// Declare this block's energy buffer, validated by the normal game table builder.
    fn block_energy(&mut self, block: &crate::BlockKey, energy: crate::ModEnergyBuffer);
    /// Register the type-bound callback used by `item_container`.
    fn register_item_container(
        &mut self,
        item: &crate::ItemKey,
        provider: Arc<dyn ItemContainerProvider>,
    );
}

/// Default-provider operations available to both native and portable hosts.
// hidden: the game reads the installed default providers through it; mods declare them through the registrar.
#[doc(hidden)]
pub trait DefaultCapabilitiesCapabilityProviders {
    /// First enabled declaration of this block's item container.
    fn block_container(&self, block: &str) -> Option<crate::ModContainer>;
    /// First enabled declaration of this block's fluid tanks.
    fn block_fluids(&self, block: &str) -> Option<crate::ModFluidTanks>;
    /// First enabled declaration of this block's energy buffer.
    fn block_energy(&self, block: &str) -> Option<crate::ModEnergyBuffer>;
    /// Registered item callbacks in registration order, without a global mod scan.
    fn item_container_provider(
        &self,
        item: &str,
        index: &mut u32,
    ) -> Option<Arc<dyn ItemContainerProvider>>;
}

impl DefaultCapabilitiesCapabilityProviders for InstalledProviders {
    fn block_container(&self, block: &str) -> Option<crate::ModContainer> {
        first_enabled(installed()?.keys.default_block_containers.get(block)?)
    }
    fn block_fluids(&self, block: &str) -> Option<crate::ModFluidTanks> {
        first_enabled(installed()?.keys.default_block_fluids.get(block)?)
    }
    fn block_energy(&self, block: &str) -> Option<crate::ModEnergyBuffer> {
        first_enabled(installed()?.keys.default_block_energy.get(block)?)
    }
    fn item_container_provider(
        &self,
        item: &str,
        index: &mut u32,
    ) -> Option<Arc<dyn ItemContainerProvider>> {
        let providers = installed()?.keys.default_item_containers.get(item)?;
        while (*index as usize) < providers.len() {
            let at = *index as usize;
            *index += 1;
            let (mod_index, provider) = &providers[at];
            if crate::mod_index_on(*mod_index) {
                return Some(provider.clone());
            }
        }
        None
    }
}

fn first_enabled<T: Clone>(declarations: &[(usize, T)]) -> Option<T> {
    declarations
        .iter()
        .find(|(index, _)| crate::mod_index_on(*index))
        .map(|(_, declaration)| declaration.clone())
}
