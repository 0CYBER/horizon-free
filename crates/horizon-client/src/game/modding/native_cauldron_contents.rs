//! Mod reads share the fluid capability's resource-rebuilt content table.
use super::Game;
use crate::game::machines::cauldron_contents::{self, CauldronContent};

fn export(content: &CauldronContent) -> Option<mod_api::CauldronContent> {
    Some(mod_api::CauldronContent::new(
        mod_api::__private::block_key(&content.block)?,
        mod_api::__private::fluid_key(&content.fluid)?,
        content.total,
        content.levels(),
        content.level.as_ref().map(|(name, _)| name.clone()),
    ))
}

impl Game<'_> {
    pub(super) fn cauldron_content_for_block(
        &self,
        block: mod_api::BlockKey,
    ) -> Option<mod_api::CauldronContent> {
        let id = mod_api::__private::block_key_id(block)?;
        let contents = cauldron_contents::current_if_built()?;
        export(contents.of_block(id)?.1)
    }

    pub(super) fn cauldron_content_for_fluid(
        &self,
        fluid: mod_api::FluidKey,
    ) -> Option<mod_api::CauldronContent> {
        let id = mod_api::__private::fluid_id(fluid)?;
        let contents = cauldron_contents::current_if_built()?;
        export(contents.of_fluid(id)?.1)
    }

    pub(super) fn cauldron_level(
        &self,
        state: horizon_world::registry::BlockStateId,
    ) -> Option<u32> {
        let info = self.pack.assets.registry.state(state)?;
        let contents = cauldron_contents::current_if_built()?;
        let content = contents.of_block(&info.name)?.1;
        match &content.level {
            Some((property, _)) => info.properties.get(property.as_str())?.parse().ok(),
            None => Some(content.levels()),
        }
    }
}

#[cfg(test)]
#[path = "native_cauldron_contents/tests.rs"]
mod tests;
