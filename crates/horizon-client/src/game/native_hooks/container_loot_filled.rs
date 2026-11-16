use super::*;
use crate::game::container_loot::LootTarget;
use horizon_world::loot::LootStack;

// Admission preserves the list's counts through every listener. Only placement
// trims them; reuse ordinary validation to obtain the effective maximum.
fn trim_placement_stacks(drops: &mut [ItemStack], items: &horizon_client::items::Items) {
    for stack in drops {
        let Ok(value) = stack.try_into_api() else {
            continue;
        };
        if let Err(mod_api::StackWriteError::CountExceeded { maximum }) =
            value.try_into_engine(items)
        {
            stack.count = maximum;
        }
    }
}

impl Game<'_> {
    pub(in crate::game) fn container_fill_hook(
        &mut self,
        table: &str,
        target: LootTarget,
        origin: [f64; 3],
        player: Option<mod_api::PlayerRef>,
        drops: &mut Vec<LootStack>,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ContainerLootFilled) {
            return;
        }
        self.with_level(|_, level, _| {
            level.container_fill_hook(table, target, origin, player, drops)
        });
    }
}
impl GameLevel<'_> {
    pub(in crate::game) fn native_container_loot_filled(
        &self,
        context: &ContainerLootFilled<'_>,
        drops: &mut Vec<ItemStack>,
    ) {
        self.fire_container_loot_filled(context, drops);
        drops.retain(|stack| stack.count > 0 && &*stack.item != "minecraft:air");
    }
}
impl GameLevel<'_> {
    pub(in crate::game) fn container_fill_hook(
        &mut self,
        table: &str,
        target: LootTarget,
        origin: [f64; 3],
        player: Option<mod_api::PlayerRef>,
        drops: &mut Vec<LootStack>,
    ) {
        if !self.native_authoritative
            || !mod_api::__private::listened(mod_api::HookId::ContainerLootFilled)
        {
            return;
        }
        let dimension = self.dimension.key().id().to_owned();
        let context =
            mod_api::ContainerLootFilled::from(mod_api::__private::built::ContainerLootFilled {
                table,
                dimension: &dimension,
                origin,
                block: match target {
                    LootTarget::Block((x, y, z)) => Some([x, y, z]),
                    _ => None,
                },
                entity: match target {
                    LootTarget::Cart(id) | LootTarget::Boat(id) => Some(id),
                    _ => None,
                },
                opener: player.map(|p| p.uuid()),
            });
        let mut stacks: Vec<_> = drops
            .iter()
            .map(horizon_client::structure_blocks::loot_item_with_components)
            .collect();
        let original = stacks.clone();
        self.native_container_loot_filled(&context, &mut stacks);
        if stacks != original {
            trim_placement_stacks(&mut stacks, self.items);
            *drops = stacks
                .into_iter()
                .map(|stack| {
                    let mut loot = LootStack {
                        item: stack.item.to_string(),
                        count: 0,
                        damage: stack.damage.min(i32::MAX as u32) as i32,
                        enchantments: Vec::new(),
                        potion: None,
                        map: false,
                        exploration: None,
                        ominous_amplifier: None,
                        stew_effects: Vec::new(),
                        names: Vec::new(),
                        components: Vec::new(),
                    };
                    loot.count = stack.count as i32;
                    if let Some(horizon_core::nbt::Tag::Compound(patch)) =
                        horizon_client::item_data::encode_components(&stack)
                    {
                        loot.components = patch
                            .into_iter()
                            .map(|(id, tag)| {
                                let id = id.to_string_lossy();
                                if let Some(removed) = id.strip_prefix('!') {
                                    (removed.to_owned(), None)
                                } else {
                                    (id, Some(tag))
                                }
                            })
                            .collect();
                    }
                    loot
                })
                .collect();
        }
    }
}
