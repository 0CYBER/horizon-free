//! The schemas of before 1.14 (keys below 1800): V99 to V1510 as far as the fixes
//! of those versions look. [`Walker::legacy_children`] answers for the types whose
//! shape differs from the general code of [`super`]; the other types fall through.
use super::{Key, Ty, Walker, as_str, key, namespaced};
use crate::nbt::Tag;

/// The schemas below this key are described here.
pub(super) const END: Key = key(1800, 0);

impl Walker<'_> {
    /// `children` for a schema below [`END`]; false when the general code applies.
    pub(super) fn legacy_children(&mut self, ty: Ty, v: &mut Tag) -> bool {
        let k = self.k;
        match ty {
            Ty::Entity => {
                self.legacy_entity(v);
                true
            }
            // STATS is a plain map until V1451_6 turns it into typed counters.
            Ty::Stats if k < key(1451, 6) => true,
            Ty::Objective if k < key(1451, 6) => {
                self.field(Ty::TextComponent, v, "DisplayName");
                true
            }
            _ => false,
        }
    }

    /// `ENTITY` of the schemas V705 to V1510 (the ids are `minecraft:...` since V704).
    fn legacy_entity(&mut self, v: &mut Tag) {
        let k = self.k;
        self.list(Ty::ItemStack, v, "ArmorItems");
        self.list(Ty::ItemStack, v, "HandItems");
        self.field(Ty::TextComponent, v, "CustomName");
        let id = namespaced(&as_str(v.get("id")).unwrap_or_default());
        // V1451_3 flattens the blocks of entities; V1510 renames some entities.
        let blocks_as_names = k < key(1451, 3);
        let renamed = k >= key(1510, 0);
        let block = |w: &mut Self, v: &mut Tag, old: &str, new: &str| {
            if blocks_as_names {
                w.field(Ty::BlockName, v, old);
            } else {
                w.field(Ty::BlockState, v, new);
            }
        };
        match id.as_str() {
            "minecraft:area_effect_cloud" => self.field(Ty::Particle, v, "Particle"),
            "minecraft:arrow" | "minecraft:spectral_arrow" => block(self, v, "inTile", "inBlockState"),
            "minecraft:egg"
            | "minecraft:ender_pearl"
            | "minecraft:fireball"
            | "minecraft:small_fireball"
            | "minecraft:snowball"
            | "minecraft:wither_skull" => {
                if blocks_as_names {
                    self.field(Ty::BlockName, v, "inTile");
                }
            }
            "minecraft:xp_bottle" if !renamed => {
                if blocks_as_names {
                    self.field(Ty::BlockName, v, "inTile");
                }
            }
            "minecraft:potion" => {
                self.field(Ty::ItemStack, v, "Potion");
                if blocks_as_names {
                    self.field(Ty::BlockName, v, "inTile");
                }
            }
            "minecraft:trident" => {
                self.field(Ty::BlockState, v, "inBlockState");
                self.field(Ty::ItemStack, v, "Trident");
            }
            "minecraft:chest_minecart" | "minecraft:hopper_minecart" => {
                block(self, v, "DisplayTile", "DisplayState");
                self.list(Ty::ItemStack, v, "Items");
            }
            "minecraft:minecart" | "minecraft:furnace_minecart" | "minecraft:tnt_minecart" => {
                block(self, v, "DisplayTile", "DisplayState");
            }
            "minecraft:commandblock_minecart" if !renamed => {
                block(self, v, "DisplayTile", "DisplayState");
                self.field(Ty::TextComponent, v, "LastOutput");
            }
            "minecraft:command_block_minecart" if renamed => {
                block(self, v, "DisplayTile", "DisplayState");
                self.field(Ty::TextComponent, v, "LastOutput");
            }
            "minecraft:spawner_minecart" => {
                block(self, v, "DisplayTile", "DisplayState");
                self.spawner(v);
            }
            "minecraft:enderman" => block(self, v, "carried", "carriedBlockState"),
            "minecraft:falling_block" => {
                block(self, v, "Block", "BlockState");
                self.field(Ty::BlockEntity, v, "TileEntityData");
            }
            "minecraft:fireworks_rocket" if !renamed => self.field(Ty::ItemStack, v, "FireworksItem"),
            "minecraft:firework_rocket" if renamed => self.field(Ty::ItemStack, v, "FireworksItem"),
            "minecraft:item" | "minecraft:item_frame" => self.field(Ty::ItemStack, v, "Item"),
            "minecraft:horse" => {
                self.field(Ty::ItemStack, v, "ArmorItem");
                self.field(Ty::ItemStack, v, "SaddleItem");
            }
            "minecraft:donkey" | "minecraft:mule" => {
                self.list(Ty::ItemStack, v, "Items");
                self.field(Ty::ItemStack, v, "SaddleItem");
            }
            "minecraft:llama" => {
                self.list(Ty::ItemStack, v, "Items");
                self.field(Ty::ItemStack, v, "SaddleItem");
                self.field(Ty::ItemStack, v, "DecorItem");
            }
            "minecraft:skeleton_horse" | "minecraft:zombie_horse" => {
                self.field(Ty::ItemStack, v, "SaddleItem");
            }
            "minecraft:villager" => {
                self.list(Ty::ItemStack, v, "Inventory");
                if let Some(o) = v.get_mut("Offers") {
                    self.list(Ty::VillagerTrade, o, "Recipes");
                }
            }
            "minecraft:zombie_villager" => {
                if let Some(o) = v.get_mut("Offers") {
                    self.list(Ty::VillagerTrade, o, "Recipes");
                }
            }
            _ => {}
        }
    }
}

/// Whether the block entity id is registered in the schema at `k` (before 1.14).
pub(super) fn legacy_block_entity_known(id: &str, k: Key) -> bool {
    match id {
        "minecraft:furnace"
        | "minecraft:chest"
        | "minecraft:ender_chest"
        | "minecraft:jukebox"
        | "minecraft:dispenser"
        | "minecraft:dropper"
        | "minecraft:sign"
        | "minecraft:mob_spawner"
        | "minecraft:piston"
        | "minecraft:brewing_stand"
        | "minecraft:enchanting_table"
        | "minecraft:end_portal"
        | "minecraft:beacon"
        | "minecraft:skull"
        | "minecraft:daylight_detector"
        | "minecraft:hopper"
        | "minecraft:comparator"
        | "minecraft:banner"
        | "minecraft:structure_block"
        | "minecraft:end_gateway"
        | "minecraft:command_block"
        | "minecraft:shulker_box" => true,
        // Registered by V1125 and dropped by V1451_5.
        "minecraft:bed" => k >= key(1125, 0),
        "minecraft:noteblock" | "minecraft:flower_pot" => k < key(1451, 5),
        "minecraft:trapped_chest" => k >= key(1451, 0),
        "DUMMY" => k >= key(1466, 0),
        "minecraft:conduit" => k >= key(1481, 0),
        _ => false,
    }
}
