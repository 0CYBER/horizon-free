//! Registration's type-only view of the game's stock capability providers.
//!
//! These queries share the factories/catalogs used by the runtime defaults, not a
//! second inventory of vanilla types. They never need a world, position or stack.
use super::{
    armor_stands, boats, composter::is_composter, machines::cauldron_contents, minecarts::CartKind,
};
use horizon_client::{containers::BlockEntity, mobs::species};
use mod_api::ProvidedBy;

/// Coverage of a built-in provider, independent of whether a particular runtime
/// instance is loaded, simulated, chested or reachable from the requested side.
pub(super) fn builtin(by: ProvidedBy, capability: &str, kind: &str) -> bool {
    match by {
        ProvidedBy::Block => {
            let capability =
                mod_api::__private::parts_installation::block_capability_dependency(capability)
                    .unwrap_or(capability);
            if !kind.starts_with("minecraft:") {
                return false;
            }
            if capability == mod_api::ITEMS.name {
                // Composters expose state-backed ports rather than a block entity.
                BlockEntity::has_builtin_item_container(kind) || is_composter(kind)
            } else if capability == mod_api::FLUIDS.name {
                cauldron_contents::vanilla_block_has_fluids(kind)
            } else {
                // ENERGY's default reads only a mod block entity's declared buffer.
                false
            }
        }
        ProvidedBy::Entity => {
            if capability == mod_api::ENTITY_AUTOMATION.name {
                automation_container(kind)
            } else if capability == mod_api::ENTITY_ITEMS.name {
                // These are the runtime default's three owners: the player inventory,
                // living equipment (the MobTypes catalog and armor stands), and the
                // container vehicles. Chested mounts already belong to the mob catalog.
                mod_api::__private::parts_installation::builtin_player_inventory_type(kind)
                    || kind == armor_stands::TYPE_ID
                    || kind.strip_prefix("minecraft:").is_some_and(|name| {
                        species::ALL
                            .iter()
                            .any(|definition| definition.name == name)
                    })
                    || automation_container(kind)
            } else {
                // ENTITY_ENERGY and ENTITY_FLUIDS have no game default provider.
                false
            }
        }
        ProvidedBy::Item => {
            mod_api::__private::parts_installation::builtin_item_capability_registered(
                capability, kind,
            )
        }
        // Registration asks for block coverage, not block-entity type coverage.
        ProvidedBy::BlockEntity => false,
    }
}

/// The canonical stock type relation used by wire encoding and runtime save identity.
/// The factory identity is the same fallback `save::block_entity_id` uses where no
/// block-specific identity exists (including copper chests).
pub(super) fn block_entity(kind: &str, block: &str) -> bool {
    block.starts_with("minecraft:")
        && horizon_client::lan::chunks::block_entity_type(block)
            .or_else(|| BlockEntity::builtin_item_container_type(block))
            == Some(kind)
}

fn automation_container(kind: &str) -> bool {
    CartKind::from_id(kind).is_some_and(|cart| cart.slots() > 0)
        || boats::any_boat_wood(kind).is_some_and(|(_, chest)| chest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_type_relation_preserves_runtime_save_identities() {
        for (block, kind) in [
            ("minecraft:chest", "minecraft:chest"),
            ("minecraft:trapped_chest", "minecraft:trapped_chest"),
            ("minecraft:barrel", "minecraft:barrel"),
            ("minecraft:ender_chest", "minecraft:ender_chest"),
            ("minecraft:dropper", "minecraft:dropper"),
            ("minecraft:red_shulker_box", "minecraft:shulker_box"),
            ("minecraft:oxidized_copper_chest", "minecraft:chest"),
            ("minecraft:furnace", "minecraft:furnace"),
            ("minecraft:oak_shelf", "minecraft:shelf"),
        ] {
            assert!(block_entity(kind, block), "{kind} {block}");
            let entity = BlockEntity::for_block(block).unwrap();
            assert_eq!(horizon_client::save::block_entity_id(&entity, block), kind);
        }
        assert!(!block_entity("minecraft:chest", "minecraft:trapped_chest"));
        assert!(!block_entity("minecraft:chest", "minecraft:barrel"));
        assert!(!block_entity("minecraft:chest", "minecraft:stone"));
        assert!(!block_entity("minecraft:chest", "test:chest"));
    }

    #[test]
    fn block_defaults_cover_container_types_without_a_world_or_mod() {
        for capability in [mod_api::ITEMS.name, mod_api::STORAGE.name] {
            for kind in [
                "minecraft:chest",
                "minecraft:trapped_chest",
                "minecraft:oxidized_copper_chest",
                "minecraft:barrel",
                "minecraft:ender_chest",
                "minecraft:red_shulker_box",
                "minecraft:furnace",
                "minecraft:blast_furnace",
                "minecraft:smoker",
                "minecraft:hopper",
                "minecraft:dispenser",
                "minecraft:dropper",
                "minecraft:crafter",
                "minecraft:brewing_stand",
                "minecraft:jukebox",
                "minecraft:chiseled_bookshelf",
                "minecraft:oak_shelf",
                "minecraft:composter",
            ] {
                assert!(
                    builtin(ProvidedBy::Block, capability, kind),
                    "{capability} {kind}"
                );
            }
            for kind in [
                "minecraft:stone",
                "minecraft:campfire",
                "minecraft:decorated_pot",
                "minecraft:lectern",
                "minecraft:beehive",
                "minecraft:suspicious_sand",
                "test:chest",
            ] {
                assert!(
                    !builtin(ProvidedBy::Block, capability, kind),
                    "{capability} {kind}"
                );
            }
        }
    }

    #[test]
    fn entity_defaults_distinguish_inventory_equipment_and_automation() {
        for kind in [
            "minecraft:player",
            "minecraft:armor_stand",
            "minecraft:zombie",
            "minecraft:donkey",
            "minecraft:mule",
            "minecraft:llama",
            "minecraft:trader_llama",
        ] {
            assert!(
                builtin(ProvidedBy::Entity, mod_api::ENTITY_ITEMS.name, kind),
                "{kind}"
            );
            assert!(
                !builtin(ProvidedBy::Entity, mod_api::ENTITY_AUTOMATION.name, kind),
                "{kind}"
            );
        }
        for kind in [
            "minecraft:chest_minecart",
            "minecraft:hopper_minecart",
            "minecraft:oak_chest_boat",
            "minecraft:bamboo_chest_raft",
        ] {
            for capability in [mod_api::ENTITY_ITEMS.name, mod_api::ENTITY_AUTOMATION.name] {
                assert!(
                    builtin(ProvidedBy::Entity, capability, kind),
                    "{capability} {kind}"
                );
            }
        }
        for kind in [
            "minecraft:minecart",
            "minecraft:furnace_minecart",
            "minecraft:oak_boat",
            "minecraft:item",
            "test:zombie",
        ] {
            for capability in [mod_api::ENTITY_ITEMS.name, mod_api::ENTITY_AUTOMATION.name] {
                assert!(
                    !builtin(ProvidedBy::Entity, capability, kind),
                    "{capability} {kind}"
                );
            }
        }
    }

    #[test]
    fn every_builtin_capability_routes_to_its_actual_default() {
        assert!(builtin(
            ProvidedBy::Block,
            mod_api::FLUIDS.name,
            "minecraft:cauldron"
        ));
        assert!(builtin(
            ProvidedBy::Block,
            mod_api::FLUIDS.name,
            "minecraft:water_cauldron"
        ));
        assert!(builtin(
            ProvidedBy::Block,
            mod_api::FLUIDS.name,
            "minecraft:lava_cauldron"
        ));
        assert!(!builtin(
            ProvidedBy::Block,
            mod_api::FLUIDS.name,
            "minecraft:powder_snow_cauldron"
        ));
        assert!(!builtin(
            ProvidedBy::Block,
            mod_api::ENERGY.name,
            "minecraft:chest"
        ));
        assert!(!builtin(
            ProvidedBy::Entity,
            mod_api::ENTITY_ENERGY.name,
            "minecraft:player"
        ));
        assert!(!builtin(
            ProvidedBy::Entity,
            mod_api::ENTITY_FLUIDS.name,
            "minecraft:player"
        ));
        for kind in [
            "minecraft:bucket",
            "minecraft:water_bucket",
            "minecraft:lava_bucket",
        ] {
            assert!(
                builtin(ProvidedBy::Item, mod_api::ITEM_FLUIDS.name, kind),
                "{kind}"
            );
        }
        for kind in [
            "minecraft:milk_bucket",
            "minecraft:powder_snow_bucket",
            "minecraft:cod_bucket",
        ] {
            assert!(
                !builtin(ProvidedBy::Item, mod_api::ITEM_FLUIDS.name, kind),
                "{kind}"
            );
        }
        assert!(!builtin(
            ProvidedBy::Item,
            mod_api::ITEM_ITEMS.name,
            "minecraft:shulker_box"
        ));
        assert!(!builtin(
            ProvidedBy::Item,
            mod_api::ITEM_ENERGY.name,
            "minecraft:bucket"
        ));
        assert!(!builtin(
            ProvidedBy::Block,
            "test:capability",
            "minecraft:chest"
        ));
        assert!(!builtin(
            ProvidedBy::Item,
            "test:capability",
            "minecraft:bucket"
        ));
        assert!(!builtin(
            ProvidedBy::Entity,
            "test:capability",
            "minecraft:player"
        ));
    }
}
