//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    use super::*;
    #[cfg(test)]
    use super::{Attached, Parts};

    #[cfg(test)]
    use crate::parts::tests::{Level, game_thread};
    use crate::*;

    include!(concat!(env!("OUT_DIR"), "/blocks_fixtures.rs"));
}

impl BlocksRegistrar for PartRegistrar {
    fn block_key(&mut self, id: &str) -> crate::BlockKey {
        match self
            .registry_keys
            .borrow_mut()
            .blocks
            .request(self.mod_id, id)
        {
            Ok(slot) => mod_vocab::__private::parts_installation::block_key(slot),
            Err(error) => {
                self.refused.push(error);
                mod_vocab::__private::parts_installation::block_key(u32::MAX)
            }
        }
    }

    fn attach_block_part(
        &mut self,
        block: crate::BlockKey,
        kind: &'static str,
        part: Arc<dyn BlockPart>,
    ) {
        let id = self
            .registry_keys
            .borrow()
            .blocks
            .id(mod_vocab::__private::parts_installation::block_key_slot(
                block,
            ))
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {}: invalid block key slot {}",
                self.mod_id,
                mod_vocab::__private::parts_installation::block_key_slot(block)
            ));
            return;
        };
        self.attach(|p| &mut p.blocks, kind, &id, part);
    }

    fn attach_block_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn BlockPart>) {
        let part = self.tagged(kind, tag, part);
        self.tags.blocks.extend(part);
    }

    fn provide_block_capability(
        &mut self,
        name: &'static str,
        block: &crate::BlockKey,
        before_game: bool,
        provider: BlockProviderOf,
    ) {
        let slot = mod_vocab::__private::parts_installation::block_key_slot(*block);
        let id = self
            .registry_keys
            .borrow()
            .blocks
            .id(slot)
            .map(str::to_owned);
        let Some(id) = id else {
            self.refused.push(format!(
                "mod {}: invalid block key slot {slot}",
                self.mod_id
            ));
            return;
        };
        self.capability(
            |p| &mut p.block_capabilities,
            Some(|p| &p.capabilities),
            name,
            ProviderTarget::Id(&id),
            before_game,
            {
                let handler =
                    mod_vocab::__private::parts_installation::block_provider_handler(&provider);
                (provider, handler)
            },
        );
    }
}
