//! Shulker boxes (`ShulkerBoxBlockEntity`: 27 slots that refuse other boxes,
//! kept in the dropped item's `container` component) and ender chests (the
//! player's `PlayerEnderChestContainer`, saved as `EnderItems`).
use super::Game;
use horizon_client::{containers::BlockEntity, items::ItemStack, text::Text};

pub(super) fn is_shulker_box(name: &str) -> bool {
    name.ends_with("shulker_box")
}
impl Game<'_> {
    /// The custom name of the open container, or its default title
    /// (a double chest is named by whichever half has a name).
    pub(super) fn container_title(&self, default: String) -> String {
        self.container_parts()
            .iter()
            .find_map(|(key, _)| self.world.block_names.get(key))
            .cloned()
            .unwrap_or(default)
    }
    /// ShulkerBoxSlot.mayPlace: no shulker box inside a shulker box.
    pub(super) fn refuses(&self, slot: usize, stack: &ItemStack) -> bool {
        (300..400).contains(&slot)
            && is_shulker_box(&stack.item)
            && self.container.is_some_and(|k| {
                is_shulker_box(&self.res.facts().get(self.world.block(k.0, k.1, k.2)).name)
            })
    }
    /// Opening an ender chest shows the player's ender inventory in its place.
    pub(super) fn open_ender_chest(&mut self, key: (i32, i32, i32)) {
        self.world
            .block_entities
            .insert(key, BlockEntity::Chest(self.ender_items.clone()));
        self.container = Some(key);
        self.ender_open = Some(key);
    }
    /// Closing it copies the items back and leaves no block-entity contents.
    pub(super) fn close_ender_chest(&mut self) {
        let Some(key) = self.ender_open.take() else {
            return;
        };
        if let Some(BlockEntity::Chest(items)) = self.world.block_entities.remove(&key) {
            self.ender_items = items;
        }
    }
}

// ---- chest menus: double chests, titles and shift-click ----

/// `Direction.getClockWise` for horizontal facings.
fn clockwise(facing: &str) -> &'static str {
    match facing {
        "north" => "east",
        "east" => "south",
        "south" => "west",
        _ => "north",
    }
}
fn counter_clockwise(facing: &str) -> &'static str {
    match clockwise(facing) {
        "east" => "west",
        "south" => "north",
        "west" => "east",
        _ => "south",
    }
}
fn facing_step(facing: &str) -> (i32, i32) {
    match facing {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (1, 0),
    }
}

impl Game<'_> {
    /// The other half of the open double chest and whether the open one is
    /// the right half (`ChestBlock.getConnectedDirection`).
    pub(super) fn chest_partner(&self, key: (i32, i32, i32)) -> Option<((i32, i32, i32), bool)> {
        let registry = self.pack.assets.registry;
        let state = registry.state(self.world.block(key.0, key.1, key.2))?;
        if !matches!(
            state.name.as_str(),
            "minecraft:chest" | "minecraft:trapped_chest"
        ) {
            return None;
        }
        let kind = state.properties.get("type")?.as_str();
        if kind == "single" {
            return None;
        }
        let facing = state.properties.get("facing")?.as_str();
        let dir = if kind == "left" {
            clockwise(facing)
        } else {
            counter_clockwise(facing)
        };
        let (dx, dz) = facing_step(dir);
        let other = (key.0 + dx, key.1, key.2 + dz);
        let other_state = registry.state(self.world.block(other.0, other.1, other.2))?;
        let other_kind = other_state.properties.get("type")?.as_str();
        let paired = other_state.name == state.name
            && other_kind != "single"
            && other_kind != kind
            && matches!(
                self.world.block_entities.get(&other),
                Some(BlockEntity::Chest(items)) if items.len() == 27
            );
        paired.then_some((other, kind == "right"))
    }
    /// The other half of a double chest at `key`, for neighbour updates.
    pub(super) fn chest_other_half(&self, key: (i32, i32, i32)) -> Option<(i32, i32, i32)> {
        self.chest_partner(key).map(|p| p.0)
    }
    /// The block entities of the open container in menu order, with their
    /// sizes: a double chest is its right half then its left half.
    pub(super) fn container_parts(&self) -> Vec<((i32, i32, i32), usize)> {
        let Some(key) = self.container else {
            return Vec::new();
        };
        let Some(entity) = self.world.block_entities.get(&key) else {
            return Vec::new();
        };
        let own = (key, entity.slots().len());
        match self.chest_partner(key) {
            Some((other, true)) => vec![own, (other, 27)],
            Some((other, false)) => vec![(other, 27), own],
            None => vec![own],
        }
    }
    /// `Container.stillValidBlockEntity`: the open block container closes once
    /// the player's eyes are out of reach of any of its blocks (a 4-block buffer).
    pub(super) fn tick_container_reach(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() {
            return;
        }
        let Some(eye) = self.eye_position() else {
            return;
        };
        if self
            .container
            .is_none_or(|k| k.1 == super::minecarts::CART_KEY_Y)
        {
            return;
        }
        let range = self.reach() + 4.0;
        let near = self.container_parts().iter().all(|&((x, y, z), _)| {
            horizon_client::lan::guest::within_block_range(eye, range, [x, y, z])
        });
        if !near {
            self.close_container();
            self.screen = super::Screen::Playing;
            self.capture(true);
        }
    }
    /// `CraftingMenu.stillValid`: the open crafting table closes once its
    /// block is no longer a crafting table or the player's eyes are out of
    /// reach (a 4-block buffer).
    pub(super) fn tick_craft_table(&mut self) {
        // A server's menu ends when the server closes it.
        if self.remote.is_some() || self.craft_size != 3 {
            return;
        }
        let (Some(pos), Some(eye)) = (self.craft_table, self.eye_position()) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let table = &*self.res.facts().get(state).name == "minecraft:crafting_table";
        let range = self.reach() + 4.0;
        if !(table && horizon_client::lan::guest::within_block_range(eye, range, pos)) {
            self.close_container();
            self.screen = super::Screen::Playing;
            self.capture(true);
        }
    }
    /// The number of slots of the open container.
    pub(super) fn container_size(&self) -> usize {
        self.container_parts().iter().map(|p| p.1).sum()
    }
    /// Container slot `n` (menu slot `300 + n`).
    pub(super) fn container_slot_mut(&mut self, mut n: usize) -> &mut Option<ItemStack> {
        let mut target = None;
        for (pos, size) in self.container_parts() {
            if n < size {
                target = Some((pos, n));
                break;
            }
            n -= size;
        }
        match target.and_then(|(pos, i)| {
            self.world
                .block_entities
                .get_mut(&pos)
                .and_then(|e| e.slots_mut().get_mut(i))
        }) {
            Some(slot) => slot,
            None => &mut self.scratch_slot,
        }
    }
    pub(super) fn container_block_is_shulker(&self) -> bool {
        self.container_block()
            .as_deref()
            .is_some_and(is_shulker_box)
    }
    fn container_block(&self) -> Option<std::sync::Arc<str>> {
        let k = self.container?;
        Some(
            self.res
                .facts()
                .get(self.world.block(k.0, k.1, k.2))
                .name
                .clone(),
        )
    }
    /// Rows of the open generic 9-wide container (`ChestMenu`), or None for
    /// the other menus (hoppers, dispensers, shulker boxes, furnaces...).
    pub(super) fn chest_rows(&self) -> Option<usize> {
        if let Some((rows, _)) = self.remote_chest_menu() {
            return Some(rows);
        }
        if !matches!(self.container_kind()?, BlockEntity::Chest(_)) {
            return None;
        }
        let generic = match self.container_block().as_deref() {
            Some(
                "minecraft:chest"
                | "minecraft:trapped_chest"
                | "minecraft:barrel"
                | "minecraft:ender_chest",
            ) => true,
            _ => self.open_cart_kind() == Some(super::minecarts::CartKind::Chest),
        };
        generic.then(|| self.container_size() / 9)
    }
    /// The title of the open chest-like menu.
    fn chest_title(&self, rows: usize) -> Text {
        if let Some((_, title)) = self.remote_chest_menu() {
            return title;
        }
        let (key, fallback) = match self.container_block().as_deref() {
            Some("minecraft:barrel") => ("container.barrel", "Barrel"),
            Some("minecraft:ender_chest") => ("container.enderchest", "Ender Chest"),
            _ if self.open_cart_kind().is_some() => {
                ("entity.minecraft.chest_minecart", "Minecart with Chest")
            }
            _ if rows > 3 => ("container.chestDouble", "Large Chest"),
            _ => ("container.chest", "Chest"),
        };
        Text::literal(self.container_title(self.translate(key, fallback))).resolve_legacy()
    }
    /// `ContainerScreen` / `ShulkerBoxScreen`: the background with its
    /// container rows, the title and the inventory label.
    pub(super) fn draw_chest_screen(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) {
        use horizon_client::hud::rgba;
        let atlas = &self.gui.atlas;
        let grey = rgba(64, 64, 64, 255);
        if self
            .container_block()
            .as_deref()
            .is_some_and(is_shulker_box)
        {
            batch.sprite_region(
                atlas,
                "gui/container/shulker_box",
                [ox, oy, 176.0, 167.0],
                256.0,
                256.0,
            );
            let title = self.translate("container.shulkerBox", "Shulker Box");
            batch.text(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
            let inventory = self.translate("container.inventory", "Inventory");
            batch.text(atlas, &inventory, ox + 8.0, oy + 73.0, grey, false);
            return;
        }
        let rows = self.chest_rows().unwrap_or(3);
        let top = rows as f32 * 18.0 + 17.0;
        batch.sprite_region(
            atlas,
            "gui/container/generic_54",
            [ox, oy, 176.0, top],
            256.0,
            256.0,
        );
        // The player half of the texture starts at v = 126.
        if let Some(mut r) = atlas.sprites.get("gui/container/generic_54").copied() {
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            r.v0 += 126.0 * dv;
            r.v1 = r.v0 + 96.0 * dv;
            r.u1 = r.u0 + 176.0 * du;
            batch.rect_uv(ox, oy + top, 176.0, 96.0, r, u32::MAX);
        }
        batch.text_styled(
            atlas,
            &self.chest_title(rows),
            ox + 8.0,
            oy + 6.0,
            grey,
            false,
        );
        let inventory = self.translate("container.inventory", "Inventory");
        batch.text(
            atlas,
            &inventory,
            ox + 8.0,
            oy + 114.0 + rows as f32 * 18.0 - 94.0,
            grey,
            false,
        );
    }
}

impl Game<'_> {
    /// `quickMoveStack` of the open chest-like, hopper, dispenser or furnace
    /// menu for a shift-click on `slot`, run by the verified menu engine
    /// (`horizon_client::menu`); false when the menu has none.
    pub(super) fn container_quick_move(&mut self, slot: usize) -> bool {
        self.menu_click(Some(slot), 0, horizon_client::menu::Input::QuickMove)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_chest_halves_connect_around_the_facing() {
        // A left half facing north connects east, a right half west.
        assert_eq!(facing_step(clockwise("north")), (1, 0));
        assert_eq!(facing_step(counter_clockwise("north")), (-1, 0));
        assert_eq!(facing_step(clockwise("south")), (-1, 0));
        assert_eq!(facing_step(counter_clockwise("east")), (0, -1));
    }
    #[test]
    fn shulker_boxes_are_recognised_by_name() {
        assert!(is_shulker_box("minecraft:red_shulker_box"));
        assert!(is_shulker_box("minecraft:shulker_box"));
        assert!(!is_shulker_box("minecraft:chest"));
    }
}
