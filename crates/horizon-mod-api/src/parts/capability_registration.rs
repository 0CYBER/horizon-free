//! One live registration table for provider type coverage and forwarding metadata.
use super::*;

impl PartRegistrar {
    pub(crate) fn capability_parts() -> Parts {
        let mut parts = Parts::default();
        for (name, handler) in
            mod_vocab::__private::parts_installation::builtin_block_capabilities()
        {
            parts.block_capability_order.push(name);
            parts
                .block_capability_metadata
                .insert(name, (Arc::from(handler), None));
        }
        parts
    }

    /// A name has one handler type whether first encountered by a provider, a query or metadata.
    pub(crate) fn note_block_capability(&mut self, name: &'static str, handler: Arc<str>) -> bool {
        if !namespaced(name) {
            self.refused.push(format!(
                "mod {} used capability {name:?}, which is not a namespaced name",
                self.mod_id
            ));
            return false;
        }
        if let Some((known, _)) = self.parts.block_capability_metadata.get(name) {
            if *known != handler {
                self.refused.push(format!("capability {name} is answered with two handler types, {known} and {handler} by {}", self.mod_id));
                return false;
            }
            return true;
        }
        self.parts.block_capability_order.push(name);
        self.parts
            .block_capability_metadata
            .insert(name, (handler, None));
        true
    }

    fn block_capability_covers(&self, name: &str, id: &str) -> bool {
        let declared_default = if name == crate::ITEMS.name {
            self.parts.default_block_containers.contains_key(id)
        } else if name == crate::FLUIDS.name {
            self.parts.default_block_fluids.contains_key(id)
        } else if name == crate::ENERGY.name {
            self.parts.default_block_energy.contains_key(id)
        } else {
            false
        };
        declared_default
            || self
                .parts
                .block_capabilities
                .get(name)
                .and_then(|types| types.get(id))
                .is_some_and(|providers| !providers.is_empty())
            || mod_vocab::__private::parts_installation::game_capability_registered(
                ProvidedBy::Block,
                name,
                id,
            )
            || self.parts.capabilities.get(name).is_some_and(|types| {
                types.iter().any(|(kind, providers)| {
                    !providers.is_empty()
                        && (self.block_entity_types.iter().any(|declaration| {
                            declaration.id == **kind
                                && declaration.blocks.iter().any(|block| block == id)
                        }) || mod_vocab::__private::parts_installation::block_entity_covers(
                            kind, id,
                        ))
                })
            })
            || mod_vocab::__private::parts_installation::block_capability_dependency(name)
                .is_some_and(|source| self.block_capability_covers(source, id))
    }
}

impl CapabilityRegistrationRegistrar for PartRegistrar {
    fn entity_type_key(&mut self, id: &str) -> crate::EntityTypeKey {
        match self
            .registry_keys
            .borrow_mut()
            .entity_types
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::entity_type_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::entity_type_key(u32::MAX)
            }
        }
    }

    fn block_capability_registered(
        &mut self,
        name: &'static str,
        handler: &'static str,
        block: &crate::BlockKey,
    ) -> bool {
        if !self.note_block_capability(name, Arc::from(handler)) {
            return false;
        }
        let keys = self.registry_keys.borrow();
        let Some(id) = keys
            .blocks
            .id(mod_vocab::__private::parts_installation::block_key_slot(
                *block,
            ))
        else {
            return false;
        };
        self.block_capability_covers(name, id)
    }

    fn entity_capability_registered(&self, name: &str, kind: &crate::EntityTypeKey) -> bool {
        let keys = self.registry_keys.borrow();
        let Some(id) = keys
            .entity_types
            .id(mod_vocab::__private::parts_installation::entity_type_key_slot(*kind))
        else {
            return false;
        };
        self.parts
            .entity_capabilities
            .get(name)
            .and_then(|types| types.get(id))
            .is_some_and(|providers| !providers.is_empty())
            || mod_vocab::__private::parts_installation::game_capability_registered(
                ProvidedBy::Entity,
                name,
                id,
            )
    }

    fn item_capability_registered(&self, name: &str, item: &crate::ItemKey) -> bool {
        let keys = self.registry_keys.borrow();
        let Some(id) = keys
            .items
            .id(mod_vocab::__private::parts_installation::item_key_slot(
                *item,
            ))
        else {
            return false;
        };
        self.parts
            .item_capabilities
            .get(name)
            .and_then(|types| types.get(id))
            .is_some_and(|providers| !providers.is_empty())
            || mod_vocab::__private::parts_installation::game_capability_registered(
                ProvidedBy::Item,
                name,
                id,
            )
            || (name == crate::ITEM_ITEMS.name
                && self.parts.default_item_containers.contains_key(id))
    }

    fn set_block_capability_proxyable(
        &mut self,
        name: &'static str,
        handler: &'static str,
        proxyable: bool,
    ) {
        if !self.note_block_capability(name, Arc::from(handler)) {
            return;
        }
        let (_, known) = self
            .parts
            .block_capability_metadata
            .get_mut(name)
            .expect("declared above");
        if known.is_some_and(|known| known != proxyable) {
            self.refused.push(format!("mod {} marked capability {name} proxyable={proxyable}, contradicting its earlier explicit mark", self.mod_id));
            return;
        }
        *known = Some(proxyable);
    }

    fn block_capability_names(&self) -> Vec<String> {
        self.parts
            .block_capability_order
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }

    fn block_capability_proxyable(&self, name: &str) -> bool {
        self.parts
            .block_capability_metadata
            .get(name)
            .is_some_and(|(_, proxyable)| *proxyable == Some(true))
    }
}
