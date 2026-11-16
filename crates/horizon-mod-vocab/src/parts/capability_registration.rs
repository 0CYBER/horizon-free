//! Registration-time type coverage and block capability forwarding metadata.
use super::*;

impl dyn Registrar + '_ {
    /// Whether the game or an earlier/current registration covers this block type.
    /// This is type coverage, not a promise that a lookup at a particular position answers.
    pub fn is_block_capability_registered<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        block: &crate::BlockKey,
    ) -> bool {
        self.block_capability_registered(key.name, type_name::<T>(), block)
    }

    /// Whether the game or an earlier/current registration covers this entity type.
    pub fn is_entity_capability_registered<T: ?Sized + 'static, C: 'static>(
        &mut self,
        key: &EntityCapabilityKey<T, C>,
        kind: &crate::EntityTypeKey,
    ) -> bool {
        self.entity_capability_registered(key.name, kind)
    }

    /// Whether the game or an earlier/current registration covers this item type.
    pub fn is_item_capability_registered<T: ?Sized + 'static>(
        &mut self,
        key: &ItemCapabilityKey<T>,
        item: &crate::ItemKey,
    ) -> bool {
        self.item_capability_registered(key.name, item)
    }

    /// Mark whether forwarding this capability to another block is safe. Unmarked keys are
    /// not proxyable. Repeating the same mark is allowed; contradicting an explicit mark
    /// refuses registration, as NeoForge 26.3's `setProxyable`/`setNonProxyable` do.
    pub fn set_capability_proxyable<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        proxyable: bool,
    ) {
        self.set_block_capability_proxyable(key.name, type_name::<T>(), proxyable);
    }
}

/// Registrar operations for capability type coverage and forwarding metadata.
pub trait CapabilityRegistrationRegistrar {
    /// Request an entity type by canonical registry id; unknown names are refused at resolution.
    fn entity_type_key(&mut self, id: &str) -> crate::EntityTypeKey;
    /// Type-only block coverage, including the game's default provider and registrations so far.
    fn block_capability_registered(
        &mut self,
        name: &'static str,
        handler: &'static str,
        block: &crate::BlockKey,
    ) -> bool;
    /// Type-only entity coverage, including the game's default provider and registrations so far.
    fn entity_capability_registered(&self, name: &str, kind: &crate::EntityTypeKey) -> bool;
    /// Type-only item coverage, including the game's default provider and registrations so far.
    fn item_capability_registered(&self, name: &str, item: &crate::ItemKey) -> bool;
    /// Mark a named block capability; contradictory explicit marks refuse registration.
    fn set_block_capability_proxyable(
        &mut self,
        name: &'static str,
        handler: &'static str,
        proxyable: bool,
    );
    /// Known block capabilities in discovery order: built-ins, then registrations so far.
    fn block_capability_names(&self) -> Vec<String>;
    /// Whether a known block capability has explicitly been marked proxyable.
    fn block_capability_proxyable(&self, name: &str) -> bool;
}

/// Known published block capability names, in discovery order. The returned snapshot is owned.
pub fn block_capability_names(host: &dyn Host) -> Vec<String> {
    host.capability_providers().block_capability_names()
}

/// Whether a published capability is safe to forward to another block. Unknown and unmarked
/// capabilities are not proxyable. This metadata never changes provider dispatch.
pub fn block_capability_proxyable(host: &dyn Host, name: &str) -> bool {
    host.capability_providers().block_capability_proxyable(name)
}

/// An owned snapshot of the known published capabilities safe to forward to another block.
pub fn block_capability_proxyable_names(host: &dyn Host) -> Vec<String> {
    host.capability_providers()
        .block_capability_proxyable_names()
}
