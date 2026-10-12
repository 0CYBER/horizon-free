//! The recipe book of the inventory, crafting table and furnace screens
//! (`RecipeBookComponent`): the green book button, the tabs by category, the
//! search box, the craftable filter, the pages of recipe buttons with the
//! alternatives overlay, ghost recipes, and placing a recipe in the grid
//! (`ServerPlaceRecipe`). Which recipes are known comes from the advancement
//! rewards; the open and filter settings are saved with the player.
use super::{Game, Screen};
use horizon_client::{
    containers::{BlockEntity, FurnaceKind},
    hud::{HudBatch, Rect, rgba},
    items::ItemStack,
    menu::place::{PostPlace, Stacked},
    recipe_book::{Book, BookType, Collection, Display, Entry, Tab},
    ui::EditBox,
};
use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
};

const WIDTH: f32 = 147.0;
const HEIGHT: f32 = 166.0;
/// `RecipeBookComponent.TICKS_TO_SWAP_SLOT`.
const SWAP_TICKS: f32 = 30.0;
/// Widest GUI at which the book sits next to the menu (`widthTooNarrow`).
const NARROW: f32 = 379.0;

/// A ghost recipe slot: the items it cycles through.
struct Ghost {
    slot: usize,
    items: Vec<ItemStack>,
    result: bool,
}
/// The alternatives overlay of a recipe button (`OverlayRecipeComponent`).
struct Overlay {
    x: f32,
    y: f32,
    /// (entry index, craftable)
    entries: Vec<(usize, bool)>,
}
/// One recipe button: the collection's recipes shown in the current view.
#[derive(Clone, Default)]
struct Button {
    /// (entry index, craftable)
    entries: Vec<(usize, bool)>,
}
impl Button {
    fn craftable(&self) -> bool {
        self.entries.iter().any(|e| e.1)
    }
}
#[derive(Default)]
struct View {
    /// Visible tabs.
    tabs: Vec<Tab>,
    buttons: Vec<Button>,
}

#[derive(Default)]
pub(super) struct RecipeUi {
    book: Option<Book>,
    tab: Option<Tab>,
    search: Option<EditBox>,
    page: usize,
    /// Ticks, for the ghost and button cycling (`RecipeBookComponent.time`).
    time: f32,
    ghost: Vec<Ghost>,
    last_placed: Option<String>,
    overlay: Option<Overlay>,
    view: View,
    view_key: u64,
    /// Squeeze animations of new recipes: ticks left per button index and tab.
    button_anim: Vec<(usize, f32)>,
    tab_anim: Vec<(Tab, f32)>,
}

impl RecipeUi {
    /// A new screen starts without a ghost recipe, overlay or search focus.
    pub(super) fn reset_screen(&mut self) {
        self.ghost.clear();
        self.last_placed = None;
        self.overlay = None;
        self.tab = None;
        self.page = 0;
        if let Some(search) = self.search.as_mut() {
            search.focused = false;
            search.set_value("");
        }
        self.view_key = 0;
    }
}

fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
fn sprite(name: &str) -> String {
    format!("gui/sprites/recipe_book/{name}")
}

impl Game<'_> {
    /// The book the open screen has, if any.
    fn active_book(&self) -> Option<BookType> {
        if self.screen != Screen::Inventory || self.remote.is_some() || self.station.is_some() {
            return None;
        }
        if self.mount_active() {
            return None;
        }
        match self.container_kind() {
            None if self.container.is_none() => Some(BookType::Crafting),
            Some(BlockEntity::Furnace(f)) => Some(match f.kind {
                FurnaceKind::Furnace => BookType::Furnace,
                FurnaceKind::BlastFurnace => BookType::BlastFurnace,
                FurnaceKind::Smoker => BookType::Smoker,
            }),
            _ => None,
        }
    }
    fn book_open(&self, book: BookType) -> bool {
        self.level.recipe_book.open[book.index()]
    }
    fn book_filtering(&self, book: BookType) -> bool {
        self.level.recipe_book.filtering[book.index()]
    }
    /// How far the open book pushes the menu to the right
    /// (`RecipeBookComponent.updateScreenPosition`).
    pub(super) fn recipe_book_shift(&self, width: f32) -> f32 {
        match self.active_book() {
            Some(book) if self.book_open(book) && width >= NARROW => 77.0,
            _ => 0.0,
        }
    }
    /// Whether typed keys go to the search box.
    pub(super) fn recipe_search_focused(&self) -> bool {
        self.active_book().is_some_and(|b| self.book_open(b))
            && self.recipe_ui.search.as_ref().is_some_and(|s| s.focused)
    }
    /// `keyChat` focuses the search box.
    pub(super) fn recipe_focus_search(&mut self) {
        if self.active_book().is_some_and(|b| self.book_open(b))
            && let Some(s) = self.recipe_ui.search.as_mut()
        {
            s.focused = true;
            s.reset_blink();
        }
    }
    /// Text typed (or one backspace) into the search box.
    pub(super) fn recipe_text(&mut self, text: Option<&str>, backspace: bool) {
        let Some(search) = self.recipe_ui.search.as_mut() else {
            return;
        };
        if backspace {
            search.delete_chars(-1);
        } else if let Some(text) = text {
            search.insert_text(text);
        }
    }
    /// A key press while the search box has focus.
    pub(super) fn recipe_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::{KeyCode, PhysicalKey};
        let ctrl = self.keys.contains(&KeyCode::ControlLeft);
        let shift = self.keys.contains(&KeyCode::ShiftLeft);
        let Some(search) = self.recipe_ui.search.as_mut() else {
            return;
        };
        let handled = match event.physical_key {
            PhysicalKey::Code(code) => search.key(code, ctrl, shift),
            _ => false,
        };
        if !handled
            && !ctrl
            && let Some(text) = &event.text
        {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            search.insert_text(&text);
        }
        search.reset_blink();
    }

    /// The toast for newly unlocked recipes: one for a burst of unlocks.
    pub(super) fn recipe_toast(&mut self, recipes: &[String]) {
        if recipes.is_empty() || self.adv.quiet {
            return;
        }
        self.ensure_book();
        let icon = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| recipes.iter().find_map(|id| b.entry(id)))
            .map(|e| e.result.item.to_string());
        let Some(icon) = icon else { return };
        let fresh = self
            .adv
            .toasts
            .back()
            .is_some_and(|t| t.is_recipe() && !t.was_shown());
        if !fresh {
            self.adv
                .toasts
                .push_back(super::advancements::Toast::recipe(&icon));
        }
    }

    // ---- recipes and what can be crafted ----

    /// The grid size of the open crafting menu.
    fn book_grid(&self) -> (usize, usize) {
        (self.craft_size, self.craft_size)
    }
    /// The items the book counts (`RecipeBookComponent.updateStackedContents`):
    /// the inventory and the menu's crafting slots, as the menu engine counts them.
    fn book_available(&mut self, _book: BookType) -> Stacked {
        let mut out = Stacked::default();
        let stock = &mut out;
        self.menu_session(|menu, env, _| {
            *stock = menu.stacked_contents(env);
            Some(false)
        });
        out
    }
    fn unlocked_recipes(&self) -> BTreeSet<String> {
        self.level.recipe_book.recipes.clone()
    }
    fn ensure_book(&mut self) {
        if self.recipe_ui.book.is_none() {
            self.recipe_ui.book = Some(Book::build(&self.res.recipes, &self.res.items));
        }
        if self.recipe_ui.search.is_none() {
            let mut search = EditBox::new(0.0, 0.0, 81.0, 14.0);
            search.max_length = 50;
            search.focused = false;
            search.text_color = u32::MAX;
            self.recipe_ui.search = Some(search);
        }
    }
    /// Rebuilds the tab and button lists when anything they depend on changed.
    fn refresh_book_view(&mut self, book_type: BookType) {
        self.ensure_book();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for s in self.inventory.iter().flatten() {
            (&s.item, s.count, s.damage).hash(&mut h);
        }
        for slot in [
            100usize, 101, 102, 103, 104, 105, 106, 107, 108, 300, 301, 302,
        ] {
            let stack =
                if slot < 109 && self.active_book() == Some(BookType::Crafting) || slot >= 300 {
                    self.slot_mut(slot).clone()
                } else {
                    None
                };
            stack.map(|s| (s.item, s.count)).hash(&mut h);
        }
        self.craft_size.hash(&mut h);
        self.level.recipe_book.recipes.len().hash(&mut h);
        self.book_filtering(book_type).hash(&mut h);
        self.recipe_ui.tab.map(|t| format!("{t:?}")).hash(&mut h);
        self.recipe_ui
            .search
            .as_ref()
            .map(|s| s.value.clone())
            .hash(&mut h);
        book_type.index().hash(&mut h);
        let key = h.finish();
        if key == self.recipe_ui.view_key && !self.recipe_ui.view.tabs.is_empty() {
            return;
        }
        let ui = std::mem::take(&mut self.recipe_ui);
        let view = self.build_view(&ui, book_type);
        self.recipe_ui = ui;
        self.recipe_ui.view = view;
        self.recipe_ui.view_key = key;
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        if self.recipe_ui.page >= pages.max(1) {
            self.recipe_ui.page = 0;
        }
        self.start_new_recipe_animations(book_type);
    }
    /// `RecipeBookTabButton.startAnimation` and `RecipeButton.init`: tabs
    /// holding a recipe still to be displayed squeeze, and the buttons of the
    /// shown page squeeze once and clear their recipes' highlight.
    fn start_new_recipe_animations(&mut self, book_type: BookType) {
        if self.level.recipe_book.to_be_displayed.is_empty() {
            return;
        }
        let Some(book) = self.recipe_ui.book.take() else {
            return;
        };
        let unlocked = self.unlocked_recipes();
        let filtering = self.book_filtering(book_type);
        let (gw, gh) = self.book_grid();
        let available = self.book_available(book_type);
        let shown = |e: &Entry| {
            (book_type != BookType::Crafting || e.fits(gw, gh))
                && (!filtering || book.can_craft(e, &available))
        };
        let new = |level: &Self, e: &Entry| level.level.recipe_book.to_be_displayed.contains(&e.id);
        let mut tabs = Vec::new();
        for tab in self.recipe_ui.view.tabs.clone() {
            if tab == Tab::Search {
                continue;
            }
            let hit = book
                .collections(&unlocked, book_type, tab)
                .iter()
                .flat_map(|c| c.entries.iter())
                .any(|&i| shown(&book.entries[i]) && new(self, &book.entries[i]));
            if hit {
                tabs.push((tab, 15.0));
            }
        }
        self.recipe_ui.tab_anim = tabs;
        let first = self.recipe_ui.page * 20;
        let mut buttons = Vec::new();
        let mut cleared = Vec::new();
        for (n, button) in self
            .recipe_ui
            .view
            .buttons
            .iter()
            .skip(first)
            .take(20)
            .enumerate()
        {
            let ids: Vec<&String> = button
                .entries
                .iter()
                .map(|e| &book.entries[e.0].id)
                .filter(|id| self.level.recipe_book.to_be_displayed.contains(*id))
                .collect();
            if !ids.is_empty() {
                buttons.push((first + n, 15.0));
                cleared.extend(ids.into_iter().cloned());
            }
        }
        self.level
            .recipe_book
            .to_be_displayed
            .retain(|id| !cleared.contains(id));
        self.recipe_ui.button_anim = buttons;
        self.recipe_ui.book = Some(book);
    }
    fn build_view(&mut self, ui: &RecipeUi, book_type: BookType) -> View {
        let Some(book) = ui.book.as_ref() else {
            return View::default();
        };
        let unlocked = self.unlocked_recipes();
        let available = self.book_available(book_type);
        let (gw, gh) = self.book_grid();
        let selected = |e: &Entry| match book_type {
            BookType::Crafting => e.fits(gw, gh),
            _ => true,
        };
        let craftable = |e: &Entry| selected(e) && book.can_craft(e, &available);
        let filtering = self.book_filtering(book_type);
        let mut tabs = Vec::new();
        for tab in book_type.tabs() {
            let visible = tab == Tab::Search
                || book
                    .collections(&unlocked, book_type, tab)
                    .iter()
                    .any(|c| c.entries.iter().any(|&i| selected(&book.entries[i])));
            if visible {
                tabs.push(tab);
            }
        }
        let tab = ui.tab.filter(|t| tabs.contains(t)).unwrap_or(Tab::Search);
        let query = ui
            .search
            .as_ref()
            .map(|s| s.value.clone())
            .unwrap_or_default();
        let collections: Vec<Collection> = book.collections(&unlocked, book_type, tab);
        let mut buttons = Vec::new();
        for c in collections {
            if !c.entries.iter().any(|&i| selected(&book.entries[i])) {
                continue;
            }
            if !query.is_empty()
                && !book.collection_matches(&c.entries, &query, &|stack| {
                    self.stack_tooltip(stack).into_iter().map(|l| l.0).collect()
                })
            {
                continue;
            }
            let entries: Vec<(usize, bool)> = c
                .entries
                .iter()
                .filter(|&&i| selected(&book.entries[i]))
                .map(|&i| (i, craftable(&book.entries[i])))
                .collect();
            if filtering && !entries.iter().any(|e| e.1) {
                continue;
            }
            let entries = if filtering {
                entries.into_iter().filter(|e| e.1).collect()
            } else {
                entries
            };
            buttons.push(Button { entries });
        }
        View { tabs, buttons }
    }

    // ---- geometry ----

    fn book_origin(&self, batch: &HudBatch) -> (f32, f32) {
        (
            ((batch.width - WIDTH) / 2.0).floor() - 86.0,
            ((batch.height - HEIGHT) / 2.0).floor(),
        )
    }
    /// The green book button's top-left corner.
    fn book_button_pos(&self, book: BookType, ox: f32, batch: &HudBatch) -> (f32, f32) {
        let mid = (batch.height / 2.0).floor();
        match book {
            BookType::Crafting if self.craft_size == 2 => (ox + 104.0, mid - 22.0),
            BookType::Crafting => (ox + 5.0, mid - 49.0),
            _ => (ox + 20.0, mid - 49.0),
        }
    }
    fn tab_pos(&self, xo: f32, yo: f32, index: usize) -> (f32, f32) {
        (xo - 30.0, yo + 3.0 + 27.0 * index as f32)
    }
    fn button_pos(xo: f32, yo: f32, i: usize) -> (f32, f32) {
        (
            xo + 11.0 + 25.0 * (i % 5) as f32,
            yo + 31.0 + 25.0 * (i / 5) as f32,
        )
    }
    /// `RecipeBookComponent.slotSelectTime`.
    fn swap_index(&self) -> usize {
        (self.recipe_ui.time / SWAP_TICKS).floor() as usize
    }
    fn batch_for_size(&self) -> Option<HudBatch> {
        let r = self.renderer.as_ref()?;
        Some(HudBatch::new([r.config.width, r.config.height]))
    }

    // ---- clicks ----

    /// A click while a screen with a recipe book is open; true when the book
    /// used it.
    pub(super) fn recipe_book_click(&mut self, right: bool) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        self.ensure_book();
        let cursor = self.cursor;
        let (ox, _) = self.inventory_origin(&batch);
        // Clicking a crafting slot drops the ghost recipe (`slotClicked`).
        if let Some(slot) = self.slot_at(&batch, cursor.0, cursor.1)
            && self.is_book_slot(book_type, slot)
        {
            self.recipe_ui.last_placed = None;
            self.recipe_ui.ghost.clear();
        }
        let (bx, by) = self.book_button_pos(book_type, ox, &batch);
        if inside(cursor, bx, by, 20.0, 18.0) {
            let open = !self.book_open(book_type);
            self.level.recipe_book.open[book_type.index()] = open;
            if !open {
                self.recipe_ui.overlay = None;
            }
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if !self.book_open(book_type) || self.spectator() {
            return false;
        }
        self.refresh_book_view(book_type);
        let (xo, yo) = self.book_origin(&batch);
        // The alternatives overlay captures the click first.
        if let Some(overlay) = self.recipe_ui.overlay.take() {
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            for (n, (entry, _)) in overlay.entries.iter().enumerate() {
                let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                if !right && inside(cursor, x, y, 24.0, 24.0) {
                    let shift = self.shift_down();
                    self.try_place(*entry, shift);
                    return true;
                }
            }
            return true;
        }
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        // Page arrows.
        if pages > 1 {
            if self.recipe_ui.page + 1 < pages && inside(cursor, xo + 93.0, yo + 137.0, 12.0, 17.0)
            {
                self.recipe_ui.page += 1;
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
            if self.recipe_ui.page > 0 && inside(cursor, xo + 38.0, yo + 137.0, 12.0, 17.0) {
                self.recipe_ui.page -= 1;
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        // Recipe buttons.
        let first = self.recipe_ui.page * 20;
        for n in 0..20 {
            let Some(button) = self.recipe_ui.view.buttons.get(first + n).cloned() else {
                break;
            };
            let (x, y) = Self::button_pos(xo, yo, n);
            if !inside(cursor, x, y, 25.0, 25.0) {
                continue;
            }
            if right {
                if button.entries.len() > 1 {
                    self.open_overlay(&button, x, y, xo, yo);
                }
            } else {
                let step = self.swap_index();
                let (entry, _) = button.entries[step % button.entries.len()];
                let shift = self.shift_down();
                self.try_place(entry, shift);
            }
            return true;
        }
        // Search box.
        if let Some(search) = self.recipe_ui.search.as_mut() {
            search.x = xo + 25.0;
            search.y = yo + 13.0;
            let magnifier = inside(cursor, xo + 8.0, yo + 13.0, 17.0, 14.0);
            if magnifier || inside(cursor, search.x, search.y, search.width, search.height) {
                search.focused = true;
                search.reset_blink();
                if !magnifier {
                    let idx = search.index_at(&self.res.atlas, cursor.0);
                    search.set_cursor(idx, false);
                }
                return true;
            }
            search.focused = false;
        }
        // The craftable filter.
        if inside(cursor, xo + 110.0, yo + 12.0, 26.0, 16.0) {
            let filtering = !self.book_filtering(book_type);
            self.level.recipe_book.filtering[book_type.index()] = filtering;
            self.recipe_ui.page = 0;
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        // Tabs.
        for (n, tab) in self.recipe_ui.view.tabs.clone().into_iter().enumerate() {
            let (x, y) = self.tab_pos(xo, yo, n);
            if inside(cursor, x, y, 35.0, 27.0) {
                if self.recipe_ui.tab.unwrap_or(Tab::Search) != tab {
                    self.recipe_ui.tab = Some(tab);
                    self.recipe_ui.page = 0;
                }
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                return true;
            }
        }
        false
    }
    fn shift_down(&self) -> bool {
        use winit::keyboard::KeyCode;
        self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight)
    }
    /// The grid, result and furnace slots clicks on which end a ghost recipe.
    fn is_book_slot(&self, book: BookType, slot: usize) -> bool {
        match book {
            BookType::Crafting => (100..109).contains(&slot) || slot == 200,
            _ => (300..303).contains(&slot),
        }
    }
    /// `OverlayRecipeComponent.init`.
    fn open_overlay(&mut self, button: &Button, bx: f32, by: f32, xo: f32, yo: f32) {
        let mut entries: Vec<(usize, bool)> =
            button.entries.iter().copied().filter(|e| e.1).collect();
        entries.extend(button.entries.iter().copied().filter(|e| !e.1));
        let total = entries.len();
        let maxrow = if total <= 16 { 4 } else { 5 };
        let rows = total.div_ceil(maxrow);
        let (mut x, mut y) = (bx, by);
        let (center_x, center_y) = (xo + WIDTH / 2.0, yo + 13.0 + HEIGHT / 2.0);
        let right = x + (total.min(maxrow) * 25) as f32;
        let max_left = center_x + 50.0;
        if right > max_left {
            x -= 25.0 * ((right - max_left) / 25.0).floor();
        }
        let bottom = y + rows as f32 * 25.0;
        let max_bottom = center_y + 50.0;
        if bottom > max_bottom {
            y -= 25.0 * ((bottom - max_bottom) / 25.0).ceil();
        }
        let max_top = center_y - 100.0;
        if y < max_top {
            y -= 25.0 * ((y - max_top) / 25.0).ceil();
        }
        self.recipe_ui.overlay = Some(Overlay { x, y, entries });
    }

    // ---- placing a recipe ----

    /// `RecipeBookComponent.tryPlaceRecipe`.
    fn try_place(&mut self, entry_index: usize, use_max: bool) {
        let Some(book_type) = self.active_book() else {
            return;
        };
        let Some(id) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .map(|e| e.id.clone())
        else {
            return;
        };
        let button_craftable = self
            .recipe_ui
            .view
            .buttons
            .iter()
            .flat_map(|b| b.entries.iter())
            .any(|e| e.0 == entry_index && e.1);
        if !button_craftable && self.recipe_ui.last_placed.as_deref() == Some(id.as_str()) {
            return;
        }
        self.recipe_ui.last_placed = Some(id);
        self.recipe_ui.ghost.clear();
        self.place_recipe(book_type, entry_index, use_max);
    }
    /// `ServerboundPlaceRecipePacket`: the server places the recipe
    /// (`ServerPlaceRecipe` on the menu engine) and answers with a ghost
    /// recipe when the items are missing.
    fn place_recipe(&mut self, book_type: BookType, entry_index: usize, use_max: bool) {
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .cloned()
        else {
            return;
        };
        let creative = self.creative;
        let mut post = None;
        let answer = &mut post;
        self.menu_session(|menu, env, _| {
            // handlePlaceRecipe: an impossible recipe is refused outright.
            if env
                .recipes
                .placement(&entry.id)
                .is_none_or(|p| p.impossible())
            {
                return Some(false);
            }
            *answer = Some(menu.place_recipe(env, &entry.id, use_max, creative, None));
            Some(true)
        });
        if post == Some(PostPlace::Ghost) {
            self.fill_ghost(book_type, &entry);
        }
        self.recipe_book_changed();
    }
    /// The container changed under the book: comparators and saving.
    fn recipe_book_changed(&mut self) {
        if let Some(key) = self.container {
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
            self.container_changed(key);
        }
    }

    // ---- ghost recipe ----

    /// `fillGhostRecipe`: the recipe's ingredients faded into the slots.
    fn fill_ghost(&mut self, book_type: BookType, entry: &Entry) {
        let Some(book) = self.recipe_ui.book.as_ref() else {
            return;
        };
        let stacks = |ingredient: &horizon_client::crafting::Ingredient| -> Vec<ItemStack> {
            book.items_of(ingredient)
                .iter()
                .map(|i| ItemStack::new(i, 1))
                .collect()
        };
        let mut ghost = Vec::new();
        match book_type {
            BookType::Crafting => {
                let (gw, gh) = (self.craft_size, self.craft_size);
                ghost.push(Ghost {
                    slot: 200,
                    items: vec![entry.result.clone()],
                    result: true,
                });
                let requirements = entry.requirements();
                for (ingredient, grid_index) in entry.grid_cells(gw, gh) {
                    if let Some(i) = ingredient {
                        let items = stacks(requirements[i]);
                        if !items.is_empty() {
                            ghost.push(Ghost {
                                slot: 100 + grid_index,
                                items,
                                result: false,
                            });
                        }
                    }
                }
            }
            _ => {
                ghost.push(Ghost {
                    slot: 302,
                    items: vec![entry.result.clone()],
                    result: true,
                });
                if let Display::Furnace(ingredient) = &entry.display {
                    ghost.push(Ghost {
                        slot: 300,
                        items: stacks(ingredient),
                        result: false,
                    });
                    if self.slot_mut(301).is_none() {
                        let mut fuels: Vec<&std::sync::Arc<str>> = self
                            .res
                            .items
                            .order
                            .iter()
                            .filter(|i| self.res.items.get(i).is_some_and(|i| i.fuel.is_some()))
                            .collect();
                        fuels.dedup();
                        ghost.push(Ghost {
                            slot: 301,
                            items: fuels.iter().map(|i| ItemStack::new(i, 1)).collect(),
                            result: false,
                        });
                    }
                }
            }
        }
        self.recipe_ui.ghost = ghost;
    }

    // ---- drawing ----

    /// The book button, ghost recipe and the open book.
    pub(super) fn draw_recipe_book(&mut self, batch: &mut HudBatch) {
        let Some(book_type) = self.active_book() else {
            return;
        };
        self.ensure_book();
        if !self.shift_down_or_ctrl() {
            self.recipe_ui.time += self.frame_ticks;
        }
        let (ox, _) = self.inventory_origin(batch);
        let cursor = self.cursor;
        let (bx, by) = self.book_button_pos(book_type, ox, batch);
        let hover = inside(cursor, bx, by, 20.0, 18.0);
        batch.sprite(
            &self.res.atlas,
            &sprite(if hover {
                "button_highlighted"
            } else {
                "button"
            }),
            bx,
            by,
        );
        self.draw_ghost(batch, book_type);
        if !self.book_open(book_type) {
            return;
        }
        self.refresh_book_view(book_type);
        self.draw_book_panel(batch, book_type);
    }
    fn shift_down_or_ctrl(&self) -> bool {
        use winit::keyboard::KeyCode;
        self.keys.contains(&KeyCode::ControlLeft) || self.keys.contains(&KeyCode::ControlRight)
    }
    fn draw_ghost(&mut self, batch: &mut HudBatch, book_type: BookType) {
        if self.recipe_ui.ghost.is_empty() {
            return;
        }
        // Ghosts show only while their slots are empty.
        let (ox, oy) = self.inventory_origin(batch);
        let step = self.swap_index();
        let big_result = book_type != BookType::Crafting || self.craft_size == 3;
        let ghosts: Vec<(usize, ItemStack, bool)> = self
            .recipe_ui
            .ghost
            .iter()
            .filter(|g| !g.items.is_empty())
            .map(|g| (g.slot, g.items[step % g.items.len()].clone(), g.result))
            .collect();
        for (slot, stack, result) in ghosts {
            let (sx, sy) = self.slot_position(slot);
            let (x, y) = (ox + sx, oy + sy);
            let atlas = &self.res.atlas;
            let red = rgba(255, 0, 0, 0x30);
            if result && big_result {
                batch.fill(atlas, x - 4.0, y - 4.0, 24.0, 24.0, red);
            } else {
                batch.fill(atlas, x, y, 16.0, 16.0, red);
            }
            self.draw_item(batch, &stack, x, y);
            batch.fill(&self.res.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 0x30));
        }
    }
    fn draw_book_panel(&mut self, batch: &mut HudBatch, book_type: BookType) {
        let (xo, yo) = self.book_origin(batch);
        let cursor = self.cursor;
        let atlas_name = "gui/recipe_book";
        // Background: the 147x166 page at (1, 1) of the texture.
        if let Some(r) = self.res.atlas.sprites.get(atlas_name).copied() {
            batch.rect_uv(
                xo,
                yo,
                WIDTH,
                HEIGHT,
                sub_rect(r, [1.0, 1.0, WIDTH, HEIGHT]),
                u32::MAX,
            );
        }
        // Search box.
        if let Some(search) = self.recipe_ui.search.as_mut() {
            search.x = xo + 25.0;
            search.y = yo + 13.0;
            search.draw(batch, &self.res.atlas, None);
            if search.value.is_empty() && !search.focused {
                let hint = self
                    .res
                    .stations
                    .enchantments
                    .translate("gui.recipebook.search_hint")
                    .unwrap_or("Search...")
                    .to_owned();
                batch.text(
                    &self.res.atlas,
                    &hint,
                    search.text_x(),
                    search.text_y(),
                    rgba(170, 170, 170, 255),
                    true,
                );
            }
        }
        // Tabs.
        let tabs = self.recipe_ui.view.tabs.clone();
        let current = self
            .recipe_ui
            .tab
            .filter(|t| tabs.contains(t))
            .unwrap_or(Tab::Search);
        let frame = self.frame_ticks;
        for (n, tab) in tabs.iter().enumerate() {
            let (x, y) = self.tab_pos(xo, yo, n);
            let selected = *tab == current;
            let name = if selected { "tab_selected" } else { "tab" };
            let sx = if selected { x - 2.0 } else { x };
            let start = batch.vertices.len();
            batch.sprite(&self.res.atlas, &sprite(name), sx, y);
            let (a, b) = tab.icons();
            let shift = if selected { -2.0 } else { 0.0 };
            match b {
                Some(b) => {
                    self.draw_item(batch, &ItemStack::new(a, 1), x + 3.0 + shift, y + 5.0);
                    self.draw_item(batch, &ItemStack::new(b, 1), x + 14.0 + shift, y + 5.0);
                }
                None => self.draw_item(batch, &ItemStack::new(a, 1), x + 9.0 + shift, y + 5.0),
            }
            if let Some(entry) = self.recipe_ui.tab_anim.iter_mut().find(|e| e.0 == *tab) {
                let squeeze = 1.0 + 0.1 * (entry.1 / 15.0 * std::f32::consts::PI).sin();
                batch.scale_since(start, x + 8.0, y + 12.0, 1.0, squeeze);
                entry.1 -= frame;
            }
        }
        self.recipe_ui.tab_anim.retain(|e| e.1 > 0.0);
        // Craftable filter.
        let filtering = self.book_filtering(book_type);
        let hover = inside(cursor, xo + 110.0, yo + 12.0, 26.0, 16.0);
        let prefix = if book_type == BookType::Crafting {
            ""
        } else {
            "furnace_"
        };
        let state = if filtering {
            "filter_enabled"
        } else {
            "filter_disabled"
        };
        let name = format!("{prefix}{state}{}", if hover { "_highlighted" } else { "" });
        batch.sprite(&self.res.atlas, &sprite(&name), xo + 110.0, yo + 12.0);
        // Recipe buttons.
        let buttons = self.recipe_ui.view.buttons.clone();
        let pages = buttons.len().div_ceil(20);
        let step = self.swap_index();
        let first = self.recipe_ui.page * 20;
        let mut tooltip: Option<Vec<(String, u32)>> = None;
        let overlay_open = self.recipe_ui.overlay.is_some();
        for n in 0..20 {
            let Some(button) = buttons.get(first + n) else {
                break;
            };
            let (x, y) = Self::button_pos(xo, yo, n);
            let anim_start = batch.vertices.len();
            let many = button.entries.len() > 1;
            let name = match (button.craftable(), many) {
                (true, true) => "slot_many_craftable",
                (true, false) => "slot_craftable",
                (false, true) => "slot_many_uncraftable",
                (false, false) => "slot_uncraftable",
            };
            batch.sprite(&self.res.atlas, &sprite(name), x, y);
            let shown = button.entries[step % button.entries.len()].0;
            let Some(stack) = self
                .recipe_ui
                .book
                .as_ref()
                .map(|b| ItemStack::new(&b.entries[shown].result.item, 1))
            else {
                continue;
            };
            let same_result = button.entries.iter().all(|e| {
                self.recipe_ui
                    .book
                    .as_ref()
                    .is_some_and(|b| b.entries[e.0].result.item == b.entries[shown].result.item)
            });
            let mut offset = 4.0;
            if many && same_result {
                self.draw_item_plain(batch, &stack, x + offset + 1.0, y + offset + 1.0);
                offset -= 1.0;
            }
            self.draw_item_plain(batch, &stack, x + offset, y + offset);
            if let Some(entry) = self
                .recipe_ui
                .button_anim
                .iter_mut()
                .find(|e| e.0 == first + n)
            {
                let squeeze = 1.0 + 0.1 * (entry.1 / 15.0 * std::f32::consts::PI).sin();
                batch.scale_since(anim_start, x + 8.0, y + 12.0, squeeze, squeeze);
                entry.1 -= self.frame_ticks;
            }
            if inside(cursor, x, y, 25.0, 25.0) && !overlay_open {
                let mut lines = self.stack_tooltip(&stack);
                if many {
                    lines.push((
                        self.translate("gui.recipebook.moreRecipes", "Right Click for More"),
                        u32::MAX,
                    ));
                }
                tooltip = Some(lines);
            }
        }
        self.recipe_ui.button_anim.retain(|e| e.1 > 0.0);
        // Page arrows and number.
        if pages > 1 {
            let page_text = self
                .translate("gui.recipebook.page", "%s/%s")
                .replacen("%s", &(self.recipe_ui.page + 1).to_string(), 1)
                .replacen("%s", &pages.to_string(), 1);
            let w = HudBatch::text_width(&self.res.atlas, &page_text);
            batch.text(
                &self.res.atlas,
                &page_text,
                xo - (w / 2.0).floor() + 73.0,
                yo + 141.0,
                u32::MAX,
                true,
            );
            if self.recipe_ui.page + 1 < pages {
                let hover = inside(cursor, xo + 93.0, yo + 137.0, 12.0, 17.0);
                let name = if hover {
                    "page_forward_highlighted"
                } else {
                    "page_forward"
                };
                batch.sprite(&self.res.atlas, &sprite(name), xo + 93.0, yo + 137.0);
                if hover {
                    tooltip = Some(vec![(
                        self.translate("gui.recipebook.next_page", "Next Page"),
                        u32::MAX,
                    )]);
                }
            }
            if self.recipe_ui.page > 0 {
                let hover = inside(cursor, xo + 38.0, yo + 137.0, 12.0, 17.0);
                let name = if hover {
                    "page_backward_highlighted"
                } else {
                    "page_backward"
                };
                batch.sprite(&self.res.atlas, &sprite(name), xo + 38.0, yo + 137.0);
                if hover {
                    tooltip = Some(vec![(
                        self.translate("gui.recipebook.previous_page", "Previous Page"),
                        u32::MAX,
                    )]);
                }
            }
        }
        if inside(cursor, xo + 110.0, yo + 12.0, 26.0, 16.0) {
            let key = if !filtering {
                "gui.recipebook.toggleRecipes.all"
            } else {
                match book_type {
                    BookType::Crafting => "gui.recipebook.toggleRecipes.craftable",
                    BookType::Furnace => "gui.recipebook.toggleRecipes.smeltable",
                    BookType::BlastFurnace => "gui.recipebook.toggleRecipes.blastable",
                    BookType::Smoker => "gui.recipebook.toggleRecipes.smokable",
                }
            };
            tooltip = Some(vec![(self.translate(key, key), u32::MAX)]);
        }
        self.draw_overlay(batch, book_type);
        if let Some(lines) = tooltip {
            self.draw_tooltip_lines(batch, &lines, cursor.0, cursor.1);
        }
    }
    /// An item without count or bars (`GuiGraphics.fakeItem`).
    fn draw_item_plain(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        let one = ItemStack {
            count: 1,
            damage: 0,
            ..stack.clone()
        };
        self.draw_item(batch, &one, x, y);
    }
    fn draw_overlay(&mut self, batch: &mut HudBatch, book_type: BookType) {
        let Some(overlay) = self.recipe_ui.overlay.as_ref() else {
            return;
        };
        let (ox, oy, entries) = (overlay.x, overlay.y, overlay.entries.clone());
        let maxrow = if entries.len() <= 16 { 4 } else { 5 };
        let width = entries.len().min(maxrow) as f32 * 25.0 + 8.0;
        let height = entries.len().div_ceil(maxrow) as f32 * 25.0 + 8.0;
        horizon_client::ui::nine_slice(
            batch,
            &self.res.atlas,
            &sprite("overlay_recipe"),
            ox,
            oy,
            width,
            height,
            u32::MAX,
        );
        let cursor = self.cursor;
        let step = self.swap_index();
        let (gw, gh) = if book_type == BookType::Crafting {
            (3, 3)
        } else {
            (1, 1)
        };
        for (n, (index, craftable)) in entries.into_iter().enumerate() {
            let x = ox + 4.0 + 25.0 * (n % maxrow) as f32;
            let y = oy + 5.0 + 25.0 * (n / maxrow) as f32;
            let hover = inside(cursor, x, y, 24.0, 24.0);
            let family = if book_type == BookType::Crafting {
                "crafting"
            } else {
                "furnace"
            };
            let state = if craftable { "" } else { "_disabled" };
            let name = format!(
                "{family}_overlay{state}{}",
                if hover { "_highlighted" } else { "" }
            );
            batch.sprite(&self.res.atlas, &sprite(&name), x, y);
            let Some(entry) = self
                .recipe_ui
                .book
                .as_ref()
                .map(|b| b.entries[index].clone())
            else {
                continue;
            };
            // The recipe's layout in miniature, ingredients at 3/8 scale.
            let mut cells: Vec<(f32, f32, ItemStack)> = Vec::new();
            if let Some(book) = self.recipe_ui.book.as_ref() {
                let requirements = entry.requirements();
                match &entry.display {
                    Display::Furnace(ingredient) => {
                        let items = book.items_of(ingredient);
                        if !items.is_empty() {
                            let item = &items[step % items.len()];
                            cells.push((3.0 + 7.0, 3.0 + 7.0, ItemStack::new(item, 1)));
                        }
                    }
                    Display::Shaped { .. } => {
                        for (ingredient, grid_index) in entry.grid_cells(gw, gh) {
                            if let Some(i) = ingredient {
                                let items = book.items_of(requirements[i]);
                                if !items.is_empty() {
                                    let item = &items[step % items.len()];
                                    cells.push((
                                        3.0 + (grid_index % 3) as f32 * 7.0,
                                        3.0 + (grid_index / 3) as f32 * 7.0,
                                        ItemStack::new(item, 1),
                                    ));
                                }
                            }
                        }
                    }
                    Display::Shapeless(list) => {
                        for (i, ingredient) in list.iter().enumerate() {
                            let items = book.items_of(ingredient);
                            if !items.is_empty() {
                                let item = &items[step % items.len()];
                                cells.push((
                                    3.0 + (i % 3) as f32 * 7.0,
                                    3.0 + (i / 3) as f32 * 7.0,
                                    ItemStack::new(item, 1),
                                ));
                            }
                        }
                    }
                }
            }
            for (cx, cy, stack) in cells {
                // Draw at the origin, then shrink around the cell centre.
                let start = batch.vertices.len();
                self.draw_item_plain(batch, &stack, 0.0, 0.0);
                let (px, py) = (x + 2.0 + cx, y + 2.0 + cy);
                for v in &mut batch.vertices[start..] {
                    v.pos = [px + (v.pos[0] - 8.0) * 0.375, py + (v.pos[1] - 8.0) * 0.375];
                }
            }
        }
    }
}

/// A sub-rectangle of a sprite given in texels.
fn sub_rect(r: Rect, [x, y, w, h]: [f32; 4]) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}
