//! Registered damage value effects, resolved when enchantment content loads.
use crate::{EntityRef, Identifier, ItemStack, LivingRef, ServerHost};
use std::sync::Arc;

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// Context of one value node in ordinary enchantment damage computation.
// exhaustive: the game constructs the hit context with a literal; mods only read it
pub struct EnchantmentDamageContext<'a> {
    /// The actual attacker of this hit.
    pub attacker: LivingRef,
    /// The actual victim of this hit.
    pub victim: EntityRef,
    /// Weapon snapshot taken before the hit.
    pub weapon: &'a ItemStack,
    /// Enchantment level on the weapon.
    pub level: u32,
    /// This effect node's content, converted once to NBT at load, including its type and custom fields.
    pub config: &'a horizon_core::nbt::Tag,
}

/// A registered value transformation. Called only for nodes naming this effect.
/// The host is read-only; callbacks may be nested and must not retain hit context.
pub trait EnchantmentDamageEffect: Send + Sync + 'static {
    /// Transform the value at this node, in vanilla enchantment and node order.
    /// NaN retains the input. Other results, including infinities, are clamped
    /// to 0..=1024 before the next node, bounding per-unit built-in work.
    fn apply(
        &self,
        host: &dyn ServerHost,
        context: &EnchantmentDamageContext<'_>,
        value: f32,
    ) -> f32;
}

/// Startup registrar for namespaced enchantment damage effects.
pub trait EnchantmentDamageEffects {
    /// Register a non-vanilla identifier. Duplicate and vanilla identifiers are refused.
    fn register(
        &mut self,
        id: Identifier,
        effect: Arc<dyn EnchantmentDamageEffect>,
    ) -> Result<(), String>;
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// Engine-owned storage behind the startup registrar.
#[derive(Default)]
#[non_exhaustive]
pub struct EnchantmentDamageRegistry {
    entries: Vec<(Identifier, Arc<dyn EnchantmentDamageEffect>)>,
}
impl EnchantmentDamageEffects for EnchantmentDamageRegistry {
    fn register(
        &mut self,
        id: Identifier,
        effect: Arc<dyn EnchantmentDamageEffect>,
    ) -> Result<(), String> {
        if id.to_string().starts_with("minecraft:")
            || self.entries.iter().any(|(key, _)| key == &id)
        {
            return Err(format!(
                "invalid or duplicate enchantment damage effect {id}"
            ));
        }
        self.entries.push((id, effect));
        Ok(())
    }
}
impl EnchantmentDamageRegistry {
    /// Engine-owned registration entries, in slot order.
    // hidden: the engine consumes registrations after startup; not part of the mod API
    #[doc(hidden)]
    pub fn into_entries(self) -> Vec<(Identifier, Arc<dyn EnchantmentDamageEffect>)> {
        self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Identity;
    impl EnchantmentDamageEffect for Identity {
        fn apply(&self, _: &dyn ServerHost, _: &EnchantmentDamageContext<'_>, value: f32) -> f32 {
            value
        }
    }
    #[test]
    fn damage_effect_registration_refuses_vanilla_and_duplicate_ids() {
        let mut effects = EnchantmentDamageRegistry::default();
        assert!(
            effects
                .register(
                    Identifier::parse("minecraft:add").unwrap(),
                    Arc::new(Identity)
                )
                .is_err()
        );
        assert!(
            effects
                .register(
                    Identifier::parse("test:damage").unwrap(),
                    Arc::new(Identity)
                )
                .is_ok()
        );
        assert!(
            effects
                .register(
                    Identifier::parse("test:damage").unwrap(),
                    Arc::new(Identity)
                )
                .is_err()
        );
        assert_eq!(effects.into_entries().len(), 1);
    }
}
