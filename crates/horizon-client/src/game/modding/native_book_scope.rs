//! The live player and menu witness used across mod callbacks.
use super::*;
use horizon_client::{
    containers::ModMenuType,
    lan::menu::{ModCraft, Slots},
    recipe_book::placement_sync::Refusal,
};
use std::collections::BTreeSet;
use std::result::Result;

// no wire: server-local state compared before publishing a detached placement.
#[derive(Clone, PartialEq)]
pub(super) struct MenuSnapshot {
    pub(super) conn: u32,
    pub(super) uuid: u128,
    pub(super) entity_id: i32,
    pub(super) placed_at: u64,
    pub(super) dimension: String,
    pub(super) instance: u64,
    pub(super) window: i32,
    pub(super) kind: &'static ModMenuType,
    pub(super) slots: Slots,
    pub(super) pos: Option<(i32, i32, i32)>,
    pub(super) size: usize,
    pub(super) hidden: Vec<usize>,
    pub(super) links: Vec<(i32, i32, i32)>,
    pub(super) craft: Option<ModCraft>,
    pub(super) inventory: Vec<Option<ItemStack>>,
    pub(super) armor: [Option<ItemStack>; 4],
    pub(super) offhand: Option<ItemStack>,
    pub(super) carried: Option<ItemStack>,
    pub(super) selected: usize,
    pub(super) game_mode: i32,
    pub(super) health: u32,
    pub(super) xp: (i32, u32, i32, i32),
    pub(super) known: BTreeSet<String>,
    block: (u32, Option<u64>, String, usize),
}
impl Game<'_> {
    pub(super) fn mod_book_menu_snapshot(
        &self,
        conn: u32,
        window: i32,
        menu: &str,
    ) -> Result<MenuSnapshot, Refusal> {
        let host = self.lan.as_ref().ok_or(Refusal::Unavailable)?;
        let g = host
            .hub
            .guest(conn)
            .filter(|g| g.placed && host.hub.here(g) && g.health > 0.0 && g.game_mode != 3)
            .ok_or(Refusal::Denied)?;
        let m = g
            .menu
            .as_ref()
            .filter(|m| window != 0 && m.window == window && &*m.registry_id == menu)
            .ok_or(Refusal::Stale)?;
        let t = m
            .mod_type
            .filter(|t| t.id == menu && t.slots == m.size)
            .ok_or(Refusal::Unavailable)?;
        if !m.still_valid(&host.hub.loop_context, &self.world, g) {
            return Err(Refusal::Stale);
        }
        let Slots::Block(pos) = m.slots else {
            return Err(Refusal::Unavailable);
        };
        let Some(BlockEntity::Mod(block)) = self.world.block_entities.get(&pos) else {
            return Err(Refusal::Unavailable);
        };
        if block.id().is_empty() {
            return Err(Refusal::Unavailable);
        }
        Ok(MenuSnapshot {
            conn,
            uuid: g.uuid,
            entity_id: g.entity_id,
            placed_at: g.placed_at,
            dimension: g.dimension.clone(),
            instance: g.menu_generation,
            window,
            kind: t,
            slots: m.slots,
            pos: m.pos,
            size: m.size,
            hidden: m.hidden.clone(),
            links: m.links.clone(),
            craft: m.craft.clone(),
            inventory: g.inventory.clone(),
            armor: g.armor.clone(),
            offhand: g.offhand.clone(),
            carried: g.carried.clone(),
            selected: g.selected,
            game_mode: g.game_mode,
            health: g.health.to_bits(),
            xp: (g.xp_level, g.xp_progress.to_bits(), g.xp_total, g.xp_seed),
            known: g.recipe_book.recipes.clone(),
            block: (
                self.world.block(pos.0, pos.1, pos.2).0,
                self.world.block_entities.generation(&pos),
                block.id().to_owned(),
                block.items.len(),
            ),
        })
    }
    pub(super) fn mod_book_menu_unchanged(&self, before: &MenuSnapshot) -> bool {
        let Some(host) = &self.lan else {
            return false;
        };
        let Some(g) = host.hub.guest(before.conn) else {
            return false;
        };
        let Some(m) = &g.menu else {
            return false;
        };
        if !g.placed
            || !host.hub.here(g)
            || g.uuid != before.uuid
            || g.entity_id != before.entity_id
            || g.placed_at != before.placed_at
            || g.dimension != before.dimension
            || g.menu_generation != before.instance
            || m.window != before.window
            || &*m.registry_id != before.kind.id
            || m.mod_type != Some(before.kind)
            || m.slots != before.slots
            || m.pos != before.pos
            || m.size != before.size
            || m.hidden != before.hidden
            || m.links != before.links
            || m.craft != before.craft
            || g.inventory != before.inventory
            || g.armor != before.armor
            || g.offhand != before.offhand
            || g.carried != before.carried
            || g.selected != before.selected
            || g.game_mode != before.game_mode
            || g.health.to_bits() != before.health
            || (g.xp_level, g.xp_progress.to_bits(), g.xp_total, g.xp_seed) != before.xp
            || g.recipe_book.recipes != before.known
            || !m.still_valid(&host.hub.loop_context, &self.world, g)
        {
            return false;
        }
        let Slots::Block(pos) = m.slots else {
            return false;
        };
        let Some(BlockEntity::Mod(block)) = self.world.block_entities.get(&pos) else {
            return false;
        };
        self.world.block(pos.0, pos.1, pos.2).0 == before.block.0
            && self.world.block_entities.generation(&pos) == before.block.1
            && block.id() == before.block.2
            && block.items.len() == before.block.3
    }
}
