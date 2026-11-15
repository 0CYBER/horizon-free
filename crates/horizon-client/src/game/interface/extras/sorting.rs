//! Sorting (`sort`, `sortKey`; arismoko/horizon#332, `docs/extras.md`), off by default: a small button by a
//! menu's storage slots and one by the inventory's (an optional key sorts the part under the
//! cursor). The clicks come from `inv_sort::plan` and go through the Mouse Tweaks' ordinary slot
//! clicks (`mouse_tweaks.rs`), so they work wherever vanilla's do.
use crate::game::{Game, Screen};
use horizon_client::{
    containers::BlockEntity,
    hud::{HudBatch, rgba},
    inv_sort,
    items::ItemStack,
    keybinds,
    menu::Input,
    stations::hover_name,
};
use winit::{event::MouseButton, keyboard::KeyCode};

/// A sort button's side in GUI pixels.
const SORT_BUTTON: f32 = 9.0;

impl Game<'_> {
    /// The sort buttons of the open screen, with the extra on: their
    /// corner (from the screen's) and the slots each sorts. The container's
    /// (chest-like ones only) sits by its title, the main inventory's by
    /// the "Inventory" label's line.
    fn sort_buttons(&self) -> Vec<((f32, f32), Vec<usize>)> {
        if !self.settings.extras.sort
            || self.screen != Screen::Inventory
            || self.station.is_some()
            || self.mount_active()
        {
            return Vec::new();
        }
        let chest_like = if self.remote.is_some() {
            if !self.remote_storage_open() {
                return Vec::new();
            }
            true
        } else {
            match self.container_kind() {
                None => false,
                Some(BlockEntity::Chest(_)) => {
                    self.chest_rows().is_some() || self.container_block_is_shulker()
                }
                Some(_) => return Vec::new(),
            }
        };
        let mut buttons = Vec::new();
        let container: Vec<usize> = self
            .slots()
            .into_iter()
            .filter(|s| (300..500).contains(s))
            .collect();
        if chest_like && !container.is_empty() {
            buttons.push(((160.0, 5.0), container));
        }
        buttons.push(((160.0, self.screen_height() - 95.0), (9..36).collect()));
        buttons
    }
    /// The sort buttons: a bordered square with three bars, lighter under
    /// the cursor.
    pub(in crate::game) fn draw_sort_buttons(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let atlas = &self.gui.atlas;
        for ((x, y), _) in self.sort_buttons() {
            let (x, y) = (ox + x, oy + y);
            let (cx, cy) = self.cursor;
            let hovered = cx >= x && cx < x + SORT_BUTTON && cy >= y && cy < y + SORT_BUTTON;
            let face = if hovered {
                rgba(220, 220, 220, 255)
            } else {
                rgba(160, 160, 160, 255)
            };
            batch.fill(atlas, x, y, SORT_BUTTON, SORT_BUTTON, rgba(55, 55, 55, 255));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                SORT_BUTTON - 2.0,
                SORT_BUTTON - 2.0,
                face,
            );
            for (i, w) in [5.0, 4.0, 3.0].into_iter().enumerate() {
                let by = y + 2.0 + 2.0 * i as f32;
                batch.fill(atlas, x + 2.0, by, w, 1.0, rgba(55, 55, 55, 255));
            }
        }
    }
    /// A left press on a sort button sorts its slots (true).
    pub(in crate::game) fn sort_press(&mut self) -> bool {
        let Some(r) = self.renderer.as_ref() else {
            return false;
        };
        let batch = HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        );
        let (ox, oy) = self.inventory_origin(&batch);
        let (cx, cy) = self.cursor;
        let hit = self.sort_buttons().into_iter().find(|((x, y), _)| {
            let (x, y) = (ox + x, oy + y);
            cx >= x && cx < x + SORT_BUTTON && cy >= y && cy < y + SORT_BUTTON
        });
        let Some((_, slots)) = hit else {
            return false;
        };
        self.sort_slots(&slots);
        true
    }
    /// The sort key: sorts the part under the cursor (the container when
    /// the cursor is over no slot).
    pub(in crate::game) fn sort_key_press(&mut self, key: KeyCode) -> bool {
        keybinds::key_name(key).is_some_and(|name| self.sort_input(name))
    }
    pub(in crate::game) fn sort_mouse_press(&mut self, button: MouseButton) -> bool {
        self.sort_input(&keybinds::mouse_name(button))
    }
    fn sort_input(&mut self, name: &str) -> bool {
        let e = &self.settings.extras;
        if !e.sort || e.sort_key != name || self.recipe_search_focused() {
            return false;
        }
        let buttons = self.sort_buttons();
        let hovered = self.inventory_target().0;
        let slots = buttons
            .iter()
            .find(|(_, slots)| hovered.is_some_and(|h| slots.contains(&h)))
            .or(buttons.first())
            .map(|(_, slots)| slots.clone());
        let Some(slots) = slots else {
            return false;
        };
        self.sort_slots(&slots);
        true
    }
    /// Sorts `slots` with ordinary left clicks (needs an empty hand).
    fn sort_slots(&mut self, slots: &[usize]) {
        if self.carried.is_some() {
            return;
        }
        let views: Vec<Option<ItemStack>> = slots.iter().map(|&s| self.slot_view(s)).collect();
        let clicks = {
            let rank = |s: &ItemStack| {
                let order = self.creative_screen.order_of(s).unwrap_or(usize::MAX);
                (order, hover_name(&self.pack.names, s))
            };
            let max = |s: &ItemStack| self.max_stack(s);
            inv_sort::plan(&views, &rank, &max)
        };
        for i in clicks {
            self.tweak_click(slots[i], 0, Input::Pickup);
        }
    }
}
