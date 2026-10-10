//! Shulker boxes (`ShulkerBoxBlockEntity`: 27 slots that refuse other boxes,
//! kept in the dropped item's `container` component) and ender chests (the
//! player's `PlayerEnderChestContainer`, saved as `EnderItems`).
use super::Game;
use horizon_client::{containers::BlockEntity, items::ItemStack};

pub(super) fn is_shulker_box(name: &str) -> bool {
    name.ends_with("shulker_box")
}
impl Game<'_> {
    /// Block-entity contents on removal: shulker boxes keep theirs for the
    /// dropped box (`copy_components`), other containers spill them.
    pub(super) fn release_contents(&mut self, pos: [i32; 3], entity: BlockEntity, block: &str) {
        if let Some(name) = self
            .world
            .block_names
            .remove(&(pos[0], pos[1], pos[2]))
            .filter(|_| !matches!(entity, BlockEntity::Banner(_)))
        {
            self.pending_names.insert(pos, (block.to_owned(), name));
        }
        if let BlockEntity::Banner(banner) = entity {
            self.pending_banner.insert(pos, banner);
            return;
        }
        if is_shulker_box(block) {
            if let BlockEntity::Chest(items) = entity {
                let kept = items
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, s)| Some((i as u8, s?)))
                    .collect();
                self.pending_shulker.insert(pos, kept);
            }
            return;
        }
        for stack in entity.contents() {
            self.pop_resource(pos, stack);
        }
    }
    /// The contents a shulker box dropped at `pos` carries.
    pub(super) fn attach_shulker_contents(&mut self, pos: [i32; 3], stack: &mut ItemStack) {
        if !is_shulker_box(&stack.item) {
            return;
        }
        if let Some(contents) = self.pending_shulker.remove(&pos)
            && !contents.is_empty()
        {
            stack.data_mut().container = contents;
        }
    }
    /// The custom name a broken container drops with (`copy_components`).
    pub(super) fn attach_container_name(&mut self, pos: [i32; 3], stack: &mut ItemStack) {
        if self
            .pending_names
            .get(&pos)
            .is_some_and(|(item, _)| **item == *stack.item)
            && let Some((_, name)) = self.pending_names.remove(&pos)
        {
            stack.data_mut().custom_name = Some(name);
        }
    }
    /// A placed container takes the item's custom name.
    pub(super) fn name_placed_container(&mut self, pos: [i32; 3], stack: &ItemStack) {
        let key = (pos[0], pos[1], pos[2]);
        if let Some(name) = stack.custom_name()
            && !matches!(
                self.world.block_entities.get(&key),
                Some(BlockEntity::Banner(_) | BlockEntity::Sign(_))
            )
        {
            self.world.block_names.insert(key, name.to_owned());
        }
    }
    /// The custom name of the open container, or its default title
    /// (a double chest is named by whichever half has a name).
    pub(super) fn container_title(&self, default: String) -> String {
        self.container_parts()
            .iter()
            .find_map(|(key, _)| self.world.block_names.get(key))
            .cloned()
            .unwrap_or(default)
    }
    /// A placed shulker box takes the item's contents.
    pub(super) fn fill_placed_box(&mut self, pos: [i32; 3], stack: &ItemStack) {
        let Some(data) = stack.data() else { return };
        if !is_shulker_box(&stack.item) || data.container.is_empty() {
            return;
        }
        if let Some(BlockEntity::Chest(items)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            for (slot, s) in &data.container {
                if let Some(cell) = items.get_mut(usize::from(*slot)) {
                    *cell = Some(s.clone());
                }
            }
        }
    }
    /// ShulkerBoxSlot.mayPlace: no shulker box inside a shulker box.
    pub(super) fn refuses(&self, slot: usize, stack: &ItemStack) -> bool {
        (300..400).contains(&slot)
            && is_shulker_box(&stack.item)
            && self.container.is_some_and(|k| {
                is_shulker_box(&self.res.facts.get(self.world.block(k.0, k.1, k.2)).name)
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
    fn chest_partner(&self, key: (i32, i32, i32)) -> Option<((i32, i32, i32), bool)> {
        let registry = self.res.assets.registry;
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
                .facts
                .get(self.world.block(k.0, k.1, k.2))
                .name
                .clone(),
        )
    }
    /// Rows of the open generic 9-wide container (`ChestMenu`), or None for
    /// the other menus (hoppers, dispensers, shulker boxes, furnaces...).
    pub(super) fn chest_rows(&self) -> Option<usize> {
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
    fn chest_title(&self, rows: usize) -> String {
        let (key, fallback) = match self.container_block().as_deref() {
            Some("minecraft:barrel") => ("container.barrel", "Barrel"),
            Some("minecraft:ender_chest") => ("container.enderchest", "Ender Chest"),
            _ if self.open_cart_kind().is_some() => {
                ("entity.minecraft.chest_minecart", "Minecart with Chest")
            }
            _ if rows > 3 => ("container.chestDouble", "Large Chest"),
            _ => ("container.chest", "Chest"),
        };
        self.container_title(self.translate(key, fallback))
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
        let atlas = &self.res.atlas;
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
            batch.text(atlas, "Inventory", ox + 8.0, oy + 73.0, grey, false);
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
        batch.text(
            atlas,
            &self.chest_title(rows),
            ox + 8.0,
            oy + 6.0,
            grey,
            false,
        );
        batch.text(
            atlas,
            "Inventory",
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
        use horizon_client::menu::{Env, Input, Kind, Menu};
        let size = self.container_size();
        let (kind, count) = match self.container_kind() {
            Some(BlockEntity::Chest(_)) => {
                let shulker = self.container.is_some_and(|k| {
                    let id = self.world.block(k.0, k.1, k.2);
                    is_shulker_box(&self.res.facts.get(id).name)
                });
                let kind = match size {
                    5 => Kind::Hopper,
                    9 => Kind::Dispenser,
                    27 if shulker => Kind::Shulker,
                    n => Kind::Chest((n / 9).clamp(1, 6)),
                };
                (kind, size)
            }
            Some(BlockEntity::Furnace(f)) => (
                Kind::Furnace(
                    match f.kind.recipe_type().trim_start_matches("minecraft:") {
                        "blasting" => "blasting",
                        "smoking" => "smoking",
                        _ => "smelting",
                    },
                ),
                3,
            ),
            _ => return false,
        };
        // Menu index <-> game slot: container 300.., main inventory 9..36, hotbar 0..9.
        let to_menu = |g: usize| match g {
            300.. if g < 300 + count => Some(g - 300),
            9..36 => Some(count + g - 9),
            0..9 => Some(count + 27 + g),
            _ => None,
        };
        let from_menu = |i: usize| match i {
            _ if i < count => 300 + i,
            _ if i < count + 27 => 9 + i - count,
            _ => i - count - 27,
        };
        let Some(index) = to_menu(slot) else {
            return false;
        };
        let mut menu = Menu::new(kind);
        let total = menu.slot_count();
        let before: Vec<Option<ItemStack>> = (0..total)
            .map(|i| self.slot_mut(from_menu(i)).clone())
            .collect();
        let after: Vec<Option<ItemStack>> = {
            let env = Env {
                items: &self.res.items,
                recipes: &self.res.recipes,
                brewing: &self.res.stations.brewing,
            };
            for (i, stack) in before.iter().enumerate() {
                menu.set_slot(&env, i, stack.clone());
            }
            if menu.click(&env, index as i32, 0, Input::QuickMove).is_err() {
                return true;
            }
            (0..total).map(|i| menu.slot(i).cloned()).collect()
        };
        for (i, stack) in after.into_iter().enumerate() {
            *self.slot_mut(from_menu(i)) = stack;
        }
        if matches!(kind, Kind::Furnace(_))
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            f.cooking_timer = 0;
        }
        true
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
