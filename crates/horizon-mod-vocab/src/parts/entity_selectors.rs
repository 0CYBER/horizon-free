//! Entity selector tokens mods register (`Registrar::register_entity_selector`), kept for the
//! command registries built at every world open.
use super::*;
use std::sync::Arc;

/// A selector token a mod registered: the word after `@` and its type.
#[derive(Clone)]
pub(crate) struct EntitySelectorInfo {
    pub(crate) name: Box<str>,
    pub(crate) selector: Arc<dyn crate::EntitySelectorType>,
    pub(crate) base: crate::SelectorBase,
}

/// The selector tokens every installed mod registered, in token-name order.
pub(crate) fn installed_entity_selectors() -> &'static [EntitySelectorInfo] {
    installed().map_or(&[], |installed| installed.keys.entity_selectors.as_slice())
}

impl dyn Registrar + '_ {
    /// Register selector token `name` (`allies` gives `@allies`), once, from `Hooks::register`.
    /// A name that is empty, not lowercase letters, digits and `_`, one of vanilla's
    /// `p a r s e n`, or registered by any mod already refuses the mods at startup, naming the
    /// mod. Names must fit 32767 ASCII bytes. The base is read once here; a panic,
    /// zero limit or tooltip exceeding 32767 UTF-16 units also refuses registration.
    /// Horizon bounds the complete encoded selector metadata to 256 KiB; a token that
    /// would exceed that total is refused by name without changing earlier records.
    pub fn register_entity_selector<T: crate::EntitySelectorType>(
        &mut self,
        name: &str,
        selector: T,
    ) {
        self.add_entity_selector(name, Arc::new(selector));
    }
}

/// Registrar operations for entity selector tokens.
pub trait EntitySelectorsRegistrar {
    /// Register selector token `name` with `selector`; mods reach it through
    /// `register_entity_selector`.
    fn add_entity_selector(&mut self, name: &str, selector: Arc<dyn crate::EntitySelectorType>);
}
