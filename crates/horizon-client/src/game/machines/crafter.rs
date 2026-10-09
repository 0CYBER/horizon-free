//! The crafter (`CrafterBlock`, `CrafterBlockEntity`, `CrafterMenu`,
//! `CrafterScreen`): redstone-triggered crafting of its 3x3 grid, ejecting
//! the result and remainders into the container in front or as items, the
//! crafting animation ticks, and the menu with its toggleable slots.
use super::*;
use horizon_client::{
    advancements as adv,
    containers::Crafter,
    hud::{HudBatch, rgba},
    particles::{ParticleOptions, ParticleType},
};
use serde_json::Value;

/// `CrafterBlock`'s crafting animation length.
const CRAFTING_TICKS: i32 = 6;
const RESULT_SLOT: usize = 309;

fn center(pos: Pos) -> [f64; 3] {
    [
        f64::from(pos.0) + 0.5,
        f64::from(pos.1) + 0.5,
        f64::from(pos.2) + 0.5,
    ]
}
/// Whether an advancement condition's `recipes` entry names the recipe.
fn recipe_matches(condition: &Value, id: &str) -> bool {
    match condition {
        Value::String(s) => s == id,
        Value::Array(list) => list.iter().any(|v| v.as_str() == Some(id)),
        _ => true,
    }
}

impl Game<'_> {
    /// `Level.setBlock` of one boolean property of the block at `pos`.
    fn set_bool_prop(&mut self, pos: Pos, key: &str, value: bool, flags: i32) {
        let registry = self.res.assets.registry;
        let old = self.world.block(pos.0, pos.1, pos.2);
        let Some(mut state) = registry.state(old).cloned() else {
            return;
        };
        if !state.properties.contains_key(key) {
            return;
        }
        state.properties.insert(key.to_owned(), value.to_string());
        let Ok(id) = registry.resolve(&state) else {
            return;
        };
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, BlockPos::new(pos.0, pos.1, pos.2), id, flags)
        });
        let _ = self.process_level_events();
    }
    fn crafter_at(&self, pos: Pos) -> Option<&Crafter> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::Crafter(c) => Some(c),
            _ => None,
        }
    }
    /// `CrafterBlockEntity.serverTick` for every crafter, and the powered
    /// flag following the block state.
    pub(in crate::game) fn tick_crafters(&mut self) {
        let positions: Vec<Pos> = self
            .world
            .block_entities
            .iter()
            .filter(|(_, e)| matches!(e, BlockEntity::Crafter(_)))
            .map(|(p, _)| *p)
            .collect();
        for pos in positions {
            let state = self.world.block(pos.0, pos.1, pos.2);
            let triggered = self.containers().prop(state, "triggered") == Some("true");
            let mut finished = false;
            if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&pos) {
                c.triggered = triggered;
                let remaining = c.crafting_ticks - 1;
                if remaining >= 0 {
                    c.crafting_ticks = remaining;
                    finished = remaining == 0;
                }
            }
            if finished {
                self.set_bool_prop(
                    pos,
                    "crafting",
                    false,
                    horizon_world::level::flags::UPDATE_ALL,
                );
            }
        }
    }
    /// `CrafterBlock.dispenseFrom` (the scheduled tick after a redstone pulse).
    pub(super) fn crafter_dispense(&mut self, pos: BlockPos, state: BlockStateId) -> Result<()> {
        let key = (pos.x, pos.y, pos.z);
        let Some(crafter) = self.crafter_at(key) else {
            return Ok(());
        };
        let grid: Vec<Option<ItemStack>> = crafter.items.to_vec();
        let found = self
            .res
            .recipes
            .find(&grid, 3)
            .map(|(name, result)| (name.to_owned(), result))
            .filter(|(_, result)| result.count > 0);
        let Some((recipe, result)) = found else {
            // LevelEvent 1050: CRAFTER_FAIL.
            self.audio
                .play("minecraft:block.crafter.fail", Some(center(key)), 1.0, 1.0);
            return Ok(());
        };
        let Some(facing) = self
            .containers()
            .prop(state, "orientation")
            .and_then(|o| o.split('_').next())
            .and_then(dir_from_name)
        else {
            return Ok(());
        };
        if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&key) {
            c.crafting_ticks = CRAFTING_TICKS;
        }
        self.set_bool_prop(
            key,
            "crafting",
            true,
            horizon_world::level::flags::UPDATE_CLIENTS,
        );
        let remainders = self.res.recipes.remainders(&self.res.items, &grid, &result);
        self.crafter_eject(key, facing, result, &recipe);
        for rest in remainders.into_iter().flatten() {
            self.crafter_eject(key, facing, rest, &recipe);
        }
        if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&key) {
            for slot in c.items.iter_mut() {
                if let Some(s) = slot {
                    s.count -= 1;
                    if s.count == 0 {
                        *slot = None;
                    }
                }
            }
        }
        self.container_changed(key);
        Ok(())
    }
    /// `CrafterBlock.dispenseItem`: into the container in front, else out as an item.
    fn crafter_eject(&mut self, pos: Pos, facing: Direction, stack: ItemStack, recipe: &str) {
        let target = offset(pos, facing);
        let mut rest = stack;
        if let Some(into) = self.containers().view(target, false) {
            let one_by_one = self.crafter_at(target).is_some()
                || rest.count > self.containers().max_stack(&rest);
            while rest.count > 0 {
                let before = rest.count;
                if one_by_one {
                    let one = ItemStack {
                        count: 1,
                        ..rest.clone()
                    };
                    if self
                        .add_item(None, &into, one, Some(facing.opposite()))
                        .is_some()
                    {
                        break;
                    }
                    rest.count -= 1;
                } else {
                    match self.add_item(None, &into, rest.clone(), Some(facing.opposite())) {
                        Some(left) => rest = left,
                        None => rest.count = 0,
                    }
                    if rest.count == before {
                        break;
                    }
                }
            }
        }
        if rest.count == 0 {
            return;
        }
        self.spawn_dispensed(pos, facing, rest, 6);
        // LevelEvent 1049 (CRAFTER_CRAFT) and 2010 (white smoke shot out the front).
        self.audio
            .play("minecraft:block.crafter.craft", Some(center(pos)), 1.0, 1.0);
        let s = step(facing);
        for _ in 0..10 {
            let r = &mut self.world.random;
            let pow = r.next_double() * 0.2 + 0.01;
            let at = [
                f64::from(pos.0)
                    + f64::from(s[0]) * 0.6
                    + 0.5
                    + f64::from(s[0]) * 0.01
                    + (r.next_double() - 0.5) * f64::from(s[2]) * 0.5,
                f64::from(pos.1)
                    + f64::from(s[1]) * 0.6
                    + 0.5
                    + f64::from(s[1]) * 0.01
                    + (r.next_double() - 0.5) * f64::from(s[1]) * 0.5,
                f64::from(pos.2)
                    + f64::from(s[2]) * 0.6
                    + 0.5
                    + f64::from(s[2]) * 0.01
                    + (r.next_double() - 0.5) * f64::from(s[0]) * 0.5,
            ];
            let v = [
                f64::from(s[0]) * pow + r.next_gaussian() * 0.01,
                f64::from(s[1]) * pow + r.next_gaussian() * 0.01,
                f64::from(s[2]) * pow + r.next_gaussian() * 0.01,
            ];
            self.particles
                .add(&ParticleOptions::simple(ParticleType::WhiteSmoke), at, v);
        }
        // CrafterRecipeCraftedTrigger for players within the 17-block box.
        let near = self.eye_position().is_some_and(|e| {
            let c = center(pos);
            (0..3).all(|i| (e[i] - c[i]).abs() <= 8.5 + 1.0)
        });
        if near {
            let id = format!("minecraft:{recipe}");
            let me = self.player_entity();
            self.trigger("minecraft:crafter_recipe_crafted", move |c, w| {
                let ctx = adv::Entities {
                    this: Some(&me),
                    origin: me.position,
                    tool: None,
                    block_pos: None,
                };
                adv::conditions(&c["player"], &ctx, w) && recipe_matches(&c["recipes"], &id)
            });
        }
    }

    // ---- the menu ----

    /// The grid's current recipe result (`CrafterMenu.refreshRecipeResult`).
    pub(in crate::game) fn crafter_result(&self) -> Option<ItemStack> {
        let key = self.container?;
        let c = self.crafter_at(key)?;
        self.res.recipes.craft(&c.items, 3)
    }
    fn crafter_open(&self) -> bool {
        matches!(self.container_kind(), Some(BlockEntity::Crafter(_)))
    }
    /// `CrafterMenu` slot layout: the grid at (26, 17), the result at (134, 35).
    pub(in crate::game) fn crafter_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        if !self.crafter_open() {
            return None;
        }
        match i {
            300..309 => Some((
                26.0 + ((i - 300) % 3) as f32 * 18.0,
                17.0 + ((i - 300) / 3) as f32 * 18.0,
            )),
            RESULT_SLOT => Some((134.0, 35.0)),
            _ => None,
        }
    }
    /// `CrafterScreen`'s background, title, disabled slots and powered arrow.
    pub(in crate::game) fn draw_crafter_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some(c) = self.container.and_then(|k| self.crafter_at(k)) else {
            return;
        };
        let atlas = &self.res.atlas;
        batch.sprite_region(
            atlas,
            "gui/container/crafter",
            [ox, oy, 176.0, 166.0],
            256.0,
            256.0,
        );
        let grey = rgba(64, 64, 64, 255);
        let title = self.translate("container.crafter", "Crafter");
        let tw = HudBatch::text_width(atlas, &title);
        batch.text(
            atlas,
            &title,
            ox + (176.0 - tw) / 2.0,
            oy + 6.0,
            grey,
            false,
        );
        batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, grey, false);
        for slot in 0..9 {
            if c.disabled[slot] {
                let (x, y) = self.crafter_slot_position(300 + slot).unwrap_or_default();
                batch.sprite(
                    atlas,
                    "gui/sprites/container/crafter/disabled_slot",
                    ox + x - 1.0,
                    oy + y - 1.0,
                );
            }
        }
        let powered = self
            .container
            .map(|k| self.world.block(k.0, k.1, k.2))
            .is_some_and(|state| self.containers().prop(state, "triggered") == Some("true"));
        let arrow = if powered {
            "powered_redstone"
        } else {
            "unpowered_redstone"
        };
        batch.sprite(
            atlas,
            &format!("gui/sprites/container/crafter/{arrow}"),
            ox + 97.0,
            oy + 35.0,
        );
    }
    /// `gui.togglable_slot` over an empty, enabled grid slot.
    pub(in crate::game) fn crafter_tooltip(&self, slot: usize) -> Option<String> {
        let c = self.container.and_then(|k| self.crafter_at(k))?;
        let i = slot.checked_sub(300).filter(|i| *i < 9)?;
        (self.carried.is_none() && c.items[i].is_none() && !c.disabled[i] && !self.spectator())
            .then(|| self.translate("gui.togglable_slot", "Click to disable slot"))
    }
    /// A click in the crafter menu before the generic handling; true when
    /// it was fully handled (the result, shift-clicks).
    pub(in crate::game) fn crafter_click(&mut self, slot: usize, shift: bool) -> bool {
        if !self.crafter_open() {
            return false;
        }
        if slot == RESULT_SLOT {
            return true;
        }
        if (300..309).contains(&slot) {
            // CrafterScreen.slotClicked: toggle an empty slot.
            let i = slot - 300;
            if !shift && !self.spectator() && self.slot_mut(slot).is_none() {
                let carried_empty = self.carried.is_none();
                let key = self.container;
                let mut toggled = None;
                if let Some(BlockEntity::Crafter(c)) =
                    key.and_then(|k| self.world.block_entities.get_mut(&k))
                {
                    if c.disabled[i] {
                        c.set_slot_state(i, true);
                        toggled = Some(true);
                    } else if carried_empty {
                        c.set_slot_state(i, false);
                        toggled = Some(false);
                    }
                }
                if let Some(enabled) = toggled {
                    let pitch = if enabled { 1.0 } else { 0.75 };
                    self.audio
                        .play("minecraft:ui.button.click", None, 0.4, pitch);
                }
            }
        }
        if shift {
            self.crafter_quick_move(slot);
            return true;
        }
        false
    }
    /// `CrafterMenu.quickMoveStack`.
    fn crafter_quick_move(&mut self, slot: usize) {
        if slot >= 309 {
            return;
        }
        let Some(mut stack) = self.slot_mut(slot).take() else {
            return;
        };
        if slot >= 300 {
            let player: Vec<usize> = (9..36).chain(0..9).collect();
            self.move_to(&mut stack, &player, true);
        } else {
            let grid: Vec<usize> = (300..309).collect();
            self.move_to(&mut stack, &grid, false);
        }
        if stack.count > 0 {
            *self.slot_mut(slot) = Some(stack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack::new(item, count))
    }
    #[test]
    fn signal_counts_filled_and_disabled_slots() {
        let mut c = Crafter::default();
        assert_eq!(c.redstone_signal(), 0);
        c.items[0] = stack("minecraft:stick", 3);
        c.set_slot_state(4, false);
        assert_eq!(c.redstone_signal(), 2);
        // A filled slot cannot be disabled.
        c.set_slot_state(0, false);
        assert!(!c.disabled[0]);
        c.set_slot_state(4, true);
        assert_eq!(c.redstone_signal(), 1);
    }
    #[test]
    fn hoppers_fill_the_smaller_stacks_first() {
        let mut c = Crafter::default();
        let stick = ItemStack::new("minecraft:stick", 1);
        c.items[0] = stack("minecraft:stick", 2);
        c.items[1] = stack("minecraft:stick", 1);
        // A later empty slot counts as smaller: stacks level out front to back.
        assert!(!c.can_place(0, &stick, 64));
        assert!(!c.can_place(1, &stick, 64));
        assert!(c.can_place(2, &stick, 64));
        c.disabled[5] = true;
        assert!(!c.can_place(5, &stick, 64));
        // With every later slot at least as big, a slot accepts again.
        for i in 2..9 {
            c.items[i] = stack("minecraft:stick", 2);
        }
        assert!(c.can_place(1, &stick, 64));
        // A full stack accepts nothing.
        c.items[2] = stack("minecraft:stick", 64);
        assert!(!c.can_place(2, &stick, 64));
    }
    #[test]
    fn recipes_condition_accepts_a_name_or_a_list() {
        assert!(recipe_matches(
            &serde_json::json!("minecraft:crafter"),
            "minecraft:crafter"
        ));
        assert!(!recipe_matches(
            &serde_json::json!(["minecraft:stick"]),
            "minecraft:crafter"
        ));
        assert!(recipe_matches(&Value::Null, "minecraft:crafter"));
    }
}
