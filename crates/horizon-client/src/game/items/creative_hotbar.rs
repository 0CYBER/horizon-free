//! The creative toolbars, `hotbar.nbt` beside `options.txt`: hold the save or
//! load key and press a hotbar key (`Minecraft.handleKeybinds`).
use super::Game;
use horizon_client::{hotbar, items::ItemStack, keybinds, text::Text};
use horizon_core::nbt::Tag;

impl Game<'_> {
    /// Saved toolbars as this world takes their stacks in (`ItemStack.CODEC`,
    /// `applyPatch` against its item prototypes), so they stack with its own.
    fn clean_hotbar_rows(&self, rows: &mut [Vec<Option<ItemStack>>]) {
        for stack in rows.iter_mut().flatten().flatten() {
            self.clean_stack(stack);
        }
    }

    /// Whether the key bound to `mapping` is held, or the controller button bound to it.
    pub(super) fn mapped_key_down(&self, mapping: &str) -> bool {
        keybinds::key_code(self.settings.keys.get(mapping))
            .is_some_and(|k| self.physical_keys.contains(&k))
            || self.input_holds(mapping)
    }

    /// `CreativeModeInventoryScreen.selectTab` on the HOTBAR tab: each saved
    /// row's nine stacks, or for an empty row a locked paper in that row's own
    /// slot, named after the keys that save it (`inventory.hotbarInfo`).
    pub(super) fn refresh_saved_hotbars(&mut self) {
        if !self.creative_screen.tab().hotbar {
            return;
        }
        let path = Self::options_path().with_file_name("hotbar.nbt");
        let known = |id: &str| self.res.items().get(id).is_some();
        let mut rows = hotbar::load(&path, &known);
        self.clean_hotbar_rows(&mut rows);
        let key = |mapping: &str| keybinds::display_name(self.settings.keys.get(mapping)).plain();
        let empty = || ItemStack::new("minecraft:air", 0);
        let mut items = Vec::with_capacity(hotbar::GROUPS * hotbar::SIZE);
        for (i, row) in rows.iter().enumerate().take(hotbar::GROUPS) {
            if row.iter().any(Option::is_some) {
                items.extend(row.iter().map(|s| s.clone().unwrap_or_else(empty)));
                continue;
            }
            for j in 0..hotbar::SIZE {
                if j != i {
                    items.push(empty());
                    continue;
                }
                let with = [
                    key("key.saveToolbarActivator"),
                    key(&format!("key.hotbar.{}", i + 1)),
                ];
                let mut paper = ItemStack::new("minecraft:paper", 1);
                let data = paper.data_mut();
                data.set_other(
                    "minecraft:creative_slot_lock",
                    Some(Tag::compound(Vec::<(&str, Tag)>::new())),
                );
                data.set_other(
                    "minecraft:item_name",
                    Some(Tag::compound([
                        ("translate", Tag::String("inventory.hotbarInfo".into())),
                        (
                            "with",
                            Tag::List(
                                with.iter()
                                    .map(|s| Tag::String(s.as_str().into()))
                                    .collect(),
                            ),
                        ),
                    ])),
                );
                items.push(paper);
            }
        }
        self.creative_screen.set_saved_hotbars(items);
    }
    /// `CreativeModeInventoryScreen.handleHotbarLoadOrSave` for the hotbar key
    /// `index`: load puts the saved row in the hotbar (the creative slot
    /// packets follow from the inventory change), save stores the hotbar and
    /// says so on the action bar.
    pub(super) fn hotbar_load_or_save(&mut self, index: usize, load: bool) {
        let path = Self::options_path().with_file_name("hotbar.nbt");
        if load {
            self.load_hotbar_row(&path, index);
            return;
        }
        if let Err(e) = self.save_hotbar_row(&path, index) {
            // saveHotbar sets the message inside its try, so a failed save says nothing.
            log::warn!("the creative hotbar was not saved: {e}");
            return;
        }
        let key = |mapping: &str| keybinds::display_name(self.settings.keys.get(mapping));
        let message = Text::translate(
            "inventory.hotbarSaved",
            &[
                key("key.loadToolbarActivator"),
                key(&format!("key.hotbar.{}", index + 1)),
            ],
        );
        // handleHotbarLoadOrSave: the overlay, said at once.
        self.say_system_now(&message);
        self.titles.set_overlay(message, false);
    }
    /// The saved row `index` into the hotbar, its stacks taken in against this
    /// world's item prototypes.
    fn load_hotbar_row(&mut self, path: &std::path::Path, index: usize) {
        let known = |id: &str| self.items().get(id).is_some();
        let mut rows = hotbar::load(path, &known);
        let mut row = std::mem::take(&mut rows[index]);
        for stack in row.iter_mut().flatten() {
            self.clean_stack(stack);
        }
        for (slot, stack) in row.into_iter().enumerate() {
            self.inventory[slot] = stack;
        }
    }
    /// The hotbar into the saved row `index`. The other rows are written back
    /// as they were read: a value this world's prototypes leave out may be
    /// another world's override.
    fn save_hotbar_row(&self, path: &std::path::Path, index: usize) -> horizon_client::Result<()> {
        let known = |id: &str| self.items().get(id).is_some();
        let mut rows = hotbar::load(path, &known);
        rows[index] = self.inventory[..hotbar::SIZE].to_vec();
        hotbar::save(path, &rows)
    }
}

#[cfg(test)]
mod saved_row_tests {
    use crate::game::game_tests::tests::server_game;
    use horizon_client::{hotbar, items::ItemStack};

    /// A saved toolbar's stacks are taken in against the world's item prototypes,
    /// so a loaded one stacks with the inventory's.
    #[test]
    #[ignore = "daily"]
    fn saved_toolbar_stacks_are_the_worlds() {
        let mut resources = None;
        let game = server_game(&mut resources);
        let saved = horizon_core::nbt::parse_snbt(
            r#"{id:"minecraft:iron_ore",count:3,components:{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap()
        .0;
        let mut rows: Vec<hotbar::Hotbar> = vec![vec![horizon_client::save::item_from_nbt(&saved)]];
        game.clean_hotbar_rows(&mut rows);
        assert_eq!(rows[0][0], Some(ItemStack::new("minecraft:iron_ore", 3)));
    }

    /// Saving one toolbar keeps the others as the file had them, a value this
    /// world's prototypes leave out included; loading one takes it in.
    #[test]
    #[ignore = "daily"]
    fn saving_a_toolbar_keeps_the_others_as_saved() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("hotbar.nbt");
        let saved = horizon_core::nbt::parse_snbt(
            r#"{id:"minecraft:iron_ore",count:3,components:{"minecraft:max_stack_size":64}}"#,
        )
        .unwrap()
        .0;
        let overridden = horizon_client::save::item_from_nbt(&saved);
        let mut rows: Vec<hotbar::Hotbar> = (0..hotbar::GROUPS)
            .map(|_| vec![None; hotbar::SIZE])
            .collect();
        rows[1][0] = overridden.clone();
        hotbar::save(&path, &rows).unwrap();
        let known = |_: &str| true;

        game.save_hotbar_row(&path, 0).unwrap();
        assert_eq!(hotbar::load(&path, &known)[1][0], overridden);

        game.load_hotbar_row(&path, 1);
        assert_eq!(
            game.inventory[0],
            Some(ItemStack::new("minecraft:iron_ore", 3))
        );
        assert_eq!(hotbar::load(&path, &known)[1][0], overridden);
    }
}
