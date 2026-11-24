//! The recipe book of the inventory, crafting table and furnace screens
//! (`RecipeBookComponent`): the green book button, the tabs by category, the
//! search box, the craftable filter, the pages of recipe buttons with the
//! alternatives overlay, ghost recipes, and placing a recipe in the grid
//! (`ServerPlaceRecipe`). Which recipes are known comes from the advancement
//! rewards; the open and filter settings are saved with the player.
#[path = "recipe_book/mod_screen.rs"]
mod mod_screen;

#[cfg(test)]
#[path = "recipe_book/animation_draw_tests.rs"]
mod animation_draw_tests;

use super::{Game, Screen};
use horizon_client::{
    containers::{BlockEntity, FurnaceKind},
    hud::{HudAtlas, HudBatch, Rect, rgba},
    items::ItemStack,
    menu::place::{PostPlace, Stacked},
    random_source,
    recipe_book::{Book, BookType, ClientBook, Collection, Entry, GhostTarget, Tab},
    recipe_book_screen::{
        self as screen, ButtonView, Call, Drawn, GhostView, ItemCall, OverlayRecipe, OverlayView,
        Screen as Layout, Search, TabView,
    },
    text::{Style, Text},
    ui::{EditBox, blit_sprite},
};
use mod_api::{RecipeTransferCheck, RecipeTransferResult};
use std::{
    collections::BTreeSet,
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};

const WIDTH: f32 = 147.0;
const HEIGHT: f32 = 166.0;
/// `RecipeBookComponent.TICKS_TO_SWAP_SLOT`.
const SWAP_TICKS: f32 = 30.0;
/// Widest GUI at which the book sits next to the menu (`widthTooNarrow`).
const NARROW: f32 = 379.0;

/// `RecipeBookComponent.isOffsetNextToMainGUI`: false in a window narrower than
/// `NARROW`, where the open book covers the menu. Escape closes only the book
/// there (`keyPressed`), not the container.
pub(super) fn escape_closes_book(width: f32, open: bool) -> bool {
    open && width < NARROW
}

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
    /// Explicit viewport for renderer-less resource-backed fixtures.
    #[cfg(test)]
    pub(super) test_viewport: Option<[u32; 2]>,
    custom: mod_screen::ModRecipeUi,
    /// The last authoritative property-set and stonecutter generation.
    pub(super) server_properties: crate::recipe_book::ServerRecipeProperties,
    book: Option<Book>,
    tab: Option<Tab>,
    pub(super) search: Option<EditBox>,
    page: usize,
    /// Ticks, for the ghost and button cycling (`RecipeBookComponent.time`).
    time: f32,
    ghost: Vec<Ghost>,
    last_placed: Option<String>,
    overlay: Option<Overlay>,
    /// A press landed in the search box, so a drag selects text in it.
    search_drag: bool,
    view: View,
    view_key: u64,
    /// Squeeze animations of new recipes: ticks left per button index and tab.
    button_anim: Vec<(usize, f32)>,
    tab_anim: Vec<(Tab, f32)>,
    /// The client's copy of the known recipes (`ClientRecipeBook`), its
    /// order deciding the buttons', and the recipes it was last synced with.
    client: ClientBook,
    client_known: Option<BTreeSet<String>>,
    client_version: u64,
    /// The entries a server's packets carried, by display id: the book is
    /// made with them over its own (`Book::use_server_entries`).
    server_entries:
        std::collections::BTreeMap<i32, horizon_protocol::packets::recipes::RecipeBookEntry>,
    /// The display a click last placed (`lastRecipe`), which Enter and Space
    /// place again; slot clicks keep it.
    last_clicked: Option<i32>,
    /// The server's checks of viewer transfers (`transfer_recipe_check`).
    checks: TransferChecks,
}

/// The recipe viewer channels the server said it answers: `horizon:recipe_transfer`
/// and `horizon:recipe_check` (`lan::recipes::advertise_viewer_channels`).
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::game) struct ViewerChannels {
    pub(in crate::game) transfer: bool,
    pub(in crate::game) check: bool,
}

/// The server's answers to `horizon:recipe_check`, for what the client last
/// saw of the open menu: its window, its state id, and the inventory and
/// grid. Any change there forgets them.
#[derive(Default)]
struct TransferChecks {
    seen: Option<(i32, i32, Vec<Option<ItemStack>>)>,
    answers: Vec<CheckAnswer>,
    /// The one request out: its display id and when it went.
    asked: Option<(i32, i32, Instant)>,
}

impl TransferChecks {
    fn ask(&mut self, display_id: i32, now: Instant) -> Option<i32> {
        static NEXT_TAG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
        let tag = NEXT_TAG
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |tag| tag.checked_add(1),
            )
            .ok()?;
        self.asked = Some((tag, display_id, now));
        Some(tag)
    }
}

/// One answer of `TransferChecks`.
struct CheckAnswer {
    display_id: i32,
    /// When it came.
    at: Instant,
    /// How many times it was asked again for this view of the menu.
    refreshes: u32,
    check: RecipeTransferCheck,
}

/// How old an answer is asked again while it is wanted (chests beside a
/// station change without the client seeing it), and how many times for one
/// view of the menu.
const CHECK_REFRESH: Duration = Duration::from_secs(2);
const CHECK_REFRESHES: u32 = 15;
/// A request with no answer this long is given up, and another may go.
const CHECK_WAIT: Duration = Duration::from_millis(500);
/// The answers kept for one view of the menu.
const CHECK_KEPT: usize = 64;

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
        self.last_clicked = None;
        self.custom = mod_screen::ModRecipeUi::default();
    }
}

fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
fn sprite(name: &str) -> String {
    format!("gui/sprites/recipe_book/{name}")
}

/// `ImageButton.extractContents`: draw the sprite in the 20×18 bounds set by
/// `AbstractRecipeBookScreen.initButton`, not at its atlas dimensions.
fn draw_book_button(batch: &mut HudBatch, atlas: &HudAtlas, x: f32, y: f32, hovered: bool) {
    let sprite = if hovered {
        "gui/sprites/recipe_book/button_highlighted"
    } else {
        "gui/sprites/recipe_book/button"
    };
    blit_sprite(batch, atlas, sprite, x, y, 20.0, 18.0, u32::MAX);
}

impl Game<'_> {
    /// A viewer's transfer of a recipe by id, into a mod menu or a grid.
    pub(in crate::game) fn transfer_recipe_id(&mut self, recipe: &str, max: bool) {
        self.transfer_viewer_recipe_id(recipe, max);
    }

    /// The vanilla book the open screen has, if any.
    pub(super) fn active_book(&self) -> Option<BookType> {
        if self.screen != Screen::Inventory || self.station.is_some() {
            return None;
        }
        // Explicitly assigned mod panels use their own identity and flags.
        if self.mount_active() || self.native_screen_height().is_some() {
            return None;
        }
        match self.container_kind() {
            // The inventory's grid, or a crafting table's (`CraftingScreen`);
            // a server's other menus without a container have no book.
            None if self.container.is_none()
                && self.remote.as_ref().is_none_or(|r| {
                    r.menu
                        .as_ref()
                        .is_none_or(|(_, menu)| menu == "minecraft:crafting")
                }) =>
            {
                Some(BookType::Crafting)
            }
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
        if self.mod_book_open() {
            return self.mod_book_shift(width);
        }
        match self.active_book() {
            Some(book) if self.book_open(book) && width >= NARROW => 77.0,
            _ => 0.0,
        }
    }
    /// `AbstractRecipeBookScreen.isHovering` is false while the book is
    /// visible in a window narrower than `NARROW`: the book covers the menu,
    /// so no slot is hovered or clicked.
    pub(super) fn book_covers_menu(&self, width: f32) -> bool {
        (self.mod_book_open() && self.mod_book_overlay(width))
            || (width < NARROW && self.active_book().is_some_and(|b| self.book_open(b)))
    }
    /// Whether typed keys go to the search box.
    pub(super) fn recipe_search_focused(&self) -> bool {
        (self.mod_book_open() || self.active_book().is_some_and(|b| self.book_open(b)))
            && self.recipe_ui.search.as_ref().is_some_and(|s| s.focused)
    }
    /// `keyChat` focuses the search box.
    pub(super) fn recipe_focus_search(&mut self) {
        if (self.mod_book_open() || self.active_book().is_some_and(|b| self.book_open(b)))
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
    /// Ctrl+A/C/V/X in the search box.
    pub(super) fn recipe_shortcut(&mut self, shortcut: horizon_client::clipboard::EditShortcut) {
        if let Some(search) = self.recipe_ui.search.as_mut() {
            search.shortcut(shortcut, &mut self.clipboard);
        }
    }
    /// A key press while the search box has focus.
    pub(super) fn recipe_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::PhysicalKey;
        if let Some(shortcut) = self.edit_shortcut(event) {
            self.recipe_shortcut(shortcut);
            return;
        }
        let ctrl = self.has_control_down();
        let shift = self.has_shift_down();
        let types_text = self.types_text();
        let Some(search) = self.recipe_ui.search.as_mut() else {
            return;
        };
        let handled = match event.physical_key {
            PhysicalKey::Code(code) => search.key(code, ctrl, shift),
            _ => false,
        };
        if !handled
            && types_text
            && let Some(text) = &event.text
        {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            search.insert_text(&text);
        }
        search.reset_blink();
    }

    /// `RecipeToast.addOrUpdate` for each display of the recipes just
    /// unlocked: its crafting station and result join the toast still
    /// queued or shown, or a new one.
    pub(super) fn recipe_toast(&mut self, recipes: &[String]) {
        if recipes.is_empty() || self.adv.quiet {
            return;
        }
        self.ensure_book();
        let index = self.display_index();
        let book = self.recipe_ui.book.as_ref();
        // SmithingTrimDemoSlotDisplay draws from a random of its own.
        let mut random = random_source::entity();
        let mut items = Vec::new();
        for id in recipes {
            let Some(station) = index.toast_station(id) else {
                continue;
            };
            for display in index.display_ids(id) {
                // The book's displays carry their resolved results; the
                // other recipe types name theirs, and trims are demos.
                let result = book
                    .and_then(|b| {
                        b.entry_of_display(display)
                            .map(|i| b.entries[i].result.clone())
                    })
                    .or_else(|| index.result(id).map(|r| ItemStack::new(r, 1)))
                    .or_else(|| {
                        self.stations().recipes.trim_demo(
                            &self.stations().enchantments,
                            id,
                            &mut |n| {
                                horizon_core::random::RandomSource::next_int_bound(
                                    &mut random,
                                    n as i32,
                                )
                                .unwrap_or(0) as usize
                            },
                        )
                    });
                if let Some(result) = result {
                    items.push((station.to_owned(), result));
                }
            }
        }
        if items.is_empty() {
            return;
        }
        match self.adv.toasts.iter_mut().find(|t| t.is_recipe()) {
            Some(toast) => toast.add_recipes(items),
            None => self.add_advancement_toast(super::advancements::Toast::recipe(items)),
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
        if self.remote.is_some() {
            // A server's menu: the slots it last sent, counted the same way.
            use horizon_client::menu::{Env, Kind, Menu};
            let table = self.craft_size == 3;
            let grid: Vec<_> = (0..self.craft_size * self.craft_size)
                .map(|k| self.slot_mut(100 + k).clone())
                .collect();
            let mut menu = Menu::new(if table {
                Kind::Crafting
            } else {
                Kind::Inventory
            });
            let env = Env {
                crafting_remainder: Some(self),
                player: None,
                player_entity_id: None,
                loop_context: &self.loop_context,
                items: self.items(),
                recipes: self.recipes(),
                brewing: &self.stations().brewing,
                banners: &self.stations().banners,
                station_recipes: &self.stations().recipes,
                enchantments: &self.stations().enchantments,
                names: &self.pack.names,
                trade: None,
                maps: None,
                limited_crafting: self.limited_crafting(),
                can_equip: None,
                known_recipes: Some(&self.level.recipe_book),
                slot_rules: Some(self),
            };
            for i in 0..36 {
                menu.set_player_item(i, self.inventory[i].clone());
            }
            for (k, stack) in grid.into_iter().enumerate() {
                menu.set_slot(&env, 1 + k, stack);
            }
            return menu.stacked_contents(&env);
        }
        let mut out = Stacked::default();
        let stock = &mut out;
        self.menu_session(|menu, env, _| {
            *stock = menu.stacked_contents(env);
            Some(false)
        });
        out
    }
    /// Sends the client book what changed in the server's: the first time a
    /// replacing batch (vanilla's server iterates an identity hash set; this
    /// one goes in id order), then each unlock and removal.
    fn sync_client_book(&mut self) {
        // On a server the book is the server's packets' (remote_recipe_book_*).
        if self.remote.is_some() {
            return;
        }
        let known = &self.level.recipe_book.recipes;
        let index = &world_stations!(self).recipe_index;
        let ui = &mut self.recipe_ui;
        match ui.client_known.as_ref() {
            Some(previous) if previous == known => return,
            Some(previous) => {
                for id in previous.difference(known) {
                    for d in index.display_ids(id) {
                        ui.client.remove(d);
                    }
                }
                for id in known.difference(previous) {
                    for d in index.display_ids(id) {
                        ui.client.add(d);
                    }
                }
            }
            None => {
                ui.client.clear();
                for id in known {
                    for d in index.display_ids(id) {
                        ui.client.add(d);
                    }
                }
            }
        }
        ui.client_known = Some(known.clone());
        ui.client_version += 1;
    }
    /// Every entry of the book, in its order (`native_recipes`).
    pub(super) fn recipe_book_entries(&mut self) -> Vec<Entry> {
        self.ensure_book();
        self.recipe_ui
            .book
            .as_ref()
            .map_or_else(Vec::new, |b| b.entries.clone())
    }
    /// The display ids the player's book holds, and a number that changes
    /// whenever they do (`known_recipes`).
    pub(super) fn recipe_book_displays(&mut self) -> (u64, Vec<i32>) {
        self.sync_client_book();
        (
            self.recipe_ui.client_version,
            self.recipe_ui.client.displays(),
        )
    }
    /// The book's model is made again from `Game::recipes` when next needed:
    /// the server's recipes arrived, or a new session began.
    pub(super) fn forget_book(&mut self) {
        self.recipe_ui.book = None;
        self.recipe_ui.custom = mod_screen::ModRecipeUi::default();
        self.recipe_tables.books = None;
        self.invalidate_mod_book_placement_catalogues();
    }
    /// Puts a server's entries over `book` (`Book::use_server_entries`). The
    /// names come from the connection's registries; a tag's items are the ones
    /// the client knows for it (the server's tags are not read).
    fn server_entries_over<'a>(
        &self,
        book: &mut Book,
        entries: impl IntoIterator<Item = &'a horizon_protocol::packets::recipes::RecipeBookEntry>,
    ) {
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        let items = self.items();
        let name_of = |registry: &str, id: i32| {
            r.builtin
                .get(registry)
                .and_then(|ids| ids.name(id))
                .or_else(|| r.registries.name(registry, id))
                .map(str::to_owned)
        };
        let item = |id: i32| name_of("minecraft:item", id);
        let category = |id: i32| {
            name_of("minecraft:recipe_book_category", id).or_else(|| {
                horizon_client::recipe_book::Category::from_id(id).map(|c| c.name().to_owned())
            })
        };
        let tag = |name: &str| items.item_tag_order.get(name).cloned().unwrap_or_default();
        book.use_server_entries(
            entries,
            &horizon_client::recipe_book::PacketNames {
                item: &item,
                category: &category,
                tag: &tag,
            },
        );
    }
    fn ensure_book(&mut self) {
        if self.recipe_ui.book.is_none() {
            let mut book = Book::build(self.recipes(), self.items());
            book.set_display_ids(self.display_index());
            // A server numbers and describes its recipes itself.
            self.server_entries_over(&mut book, self.recipe_ui.server_entries.values());
            self.recipe_ui.book = Some(book);
        }
        self.ensure_recipe_search();
    }
    fn ensure_recipe_search(&mut self) {
        if self.recipe_ui.search.is_none() {
            let mut search = EditBox::new(0.0, 0.0, 81.0, 14.0);
            search.message = super::screen_info::tr("itemGroup.search", &[]);
            search.max_length = 50;
            search.focused = false;
            search.text_color = u32::MAX;
            self.recipe_ui.search = Some(search);
        }
    }
    /// Rebuilds the tab and button lists when anything they depend on changed.
    fn refresh_book_view(&mut self, book_type: BookType) {
        self.ensure_book();
        self.sync_client_book();
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
        self.recipe_ui.client_version.hash(&mut h);
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
            let hit = self
                .recipe_ui
                .client
                .collections(&book, book_type, tab)
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
                || ui
                    .client
                    .collections(book, book_type, tab)
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
        let collections: Vec<Collection> = ui.client.collections(book, book_type, tab);
        let mut buttons = Vec::new();
        for c in collections {
            if !c.entries.iter().any(|&i| selected(&book.entries[i])) {
                continue;
            }
            if !query.is_empty()
                && !book.collection_matches(&c.entries, &query, &|stack| {
                    horizon_client::item_tooltip::lines(
                        stack,
                        self.items().get(&stack.item),
                        &crate::game::native_hooks::HookedAttributeTooltips::new(self, None),
                        self.item_enchantments(),
                        &|k| self.pack.names.display_get(k),
                        &|attribute| self.attribute_style(attribute, false),
                    )
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

    /// `RecipeBookComponent.getXOrigin` / `getYOrigin` (`widthTooNarrow`
    /// centres the book over the menu), moved left with the window by the
    /// room the mods ask beside it (`screen_room`).
    fn book_origin(&self, batch: &HudBatch) -> (f32, f32) {
        let (x, y) = self.book_origin_in_place(batch);
        (x - self.screen_room_shift(batch), y)
    }
    /// The visible left edge, including the selected tab, of the book beside
    /// the window before room moves it; none while no book is open or it covers the
    /// window of a narrow screen.
    pub(super) fn open_book_left(&self, batch: &HudBatch) -> Option<f32> {
        if !self.mod_book_open() {
            self.active_book().filter(|b| self.book_open(*b))?;
        }
        (!self.book_covers_menu(batch.width)).then(|| self.book_origin_in_place(batch).0 - 32.0)
    }
    /// `book_origin` before the mods' room moves it.
    fn book_origin_in_place(&self, batch: &HudBatch) -> (f32, f32) {
        let narrow = if self.native_screen_book().is_some() {
            self.mod_book_overlay(batch.width)
        } else {
            batch.width < NARROW
        };
        let offset = if narrow { 0.0 } else { 86.0 };
        (
            ((batch.width - WIDTH) / 2.0).floor() - offset,
            ((batch.height - HEIGHT) / 2.0).floor(),
        )
    }
    /// The open book's panel, `[x, y, width, height]` in GUI pixels; none
    /// while the screen has no book or it is closed.
    pub(super) fn recipe_book_rect(&self, batch: &HudBatch) -> Option<[f32; 4]> {
        if !self.mod_book_open() {
            self.active_book().filter(|b| self.book_open(*b))?;
        }
        let (x, y) = self.book_origin(batch);
        Some([x, y, WIDTH, HEIGHT])
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
        #[cfg(test)]
        if let Some(size) = self.recipe_ui.test_viewport {
            return Some(HudBatch::new(size, self.settings.gui_scale_inputs()));
        }
        let r = self.renderer.as_ref()?;
        Some(HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        ))
    }

    // ---- clicks ----

    /// A click while a screen with a recipe book is open; true when the book
    /// used it.
    pub(super) fn recipe_book_click(&mut self, right: bool) -> bool {
        if self.native_screen_shown() {
            return self.mod_book_click(right);
        }
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
            // The open book moves the menu beside it: the build follows now,
            // so this click's hit tests use the new place.
            self.build_screen();
            self.send_book_settings(book_type);
            if !open {
                self.recipe_ui.overlay = None;
            }
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if !self.book_open(book_type) {
            return false;
        }
        // AbstractRecipeBookScreen.mouseClicked: in a narrow window the visible
        // book takes every click, even one it does not use itself.
        let narrow = batch.width < NARROW;
        if self.spectator() {
            return narrow;
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
                    if self.try_place(*entry, shift) {
                        self.remember_click(*entry);
                        self.close_narrow_book(book_type, &batch);
                    }
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
                if self.try_place(entry, shift) {
                    self.remember_click(entry);
                    self.close_narrow_book(book_type, &batch);
                }
            }
            return true;
        }
        // Search box.
        if let Some(search) = self.recipe_ui.search.as_mut() {
            search.x = xo + 25.0;
            search.y = yo + 13.0;
            // RecipeBookComponent's magnifierIconPlacement: from the book's
            // left edge plus eight to the search box's own left edge.
            let magnifier = inside(cursor, xo + 8.0, yo + 13.0, search.x - xo, 14.0);
            if magnifier || inside(cursor, search.x, search.y, search.width, search.height) {
                self.recipe_ui.search_drag = true;
                search.focused = true;
                search.reset_blink();
                if !magnifier {
                    let idx = search.index_at(&self.gui.atlas, cursor.0);
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
            self.send_book_settings(book_type);
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
        narrow
    }
    /// `RecipeBookComponent.mouseClicked` after a placed recipe: it becomes
    /// `lastRecipe`.
    fn remember_click(&mut self, entry: usize) {
        self.recipe_ui.last_clicked = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry))
            .map(|e| e.display_id);
    }
    /// `RecipeBookComponent.keyPressed`: Enter or Space (`isSelection`) places
    /// the recipe clicked last again, key repeats included, as many as fit
    /// with Shift. The focused search box takes the key first.
    pub(super) fn recipe_book_key(&mut self, key: winit::keyboard::KeyCode) -> bool {
        use winit::keyboard::KeyCode;
        if !matches!(key, KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) {
            return false;
        }
        let Some(book_type) = self.active_book() else {
            return false;
        };
        if !self.book_open(book_type) || self.spectator() || self.recipe_search_focused() {
            return false;
        }
        let Some(display) = self.recipe_ui.last_clicked else {
            return false;
        };
        // The craft just taken changed what is craftable.
        self.refresh_book_view(book_type);
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entry_of_display(display))
        else {
            return false;
        };
        self.audio
            .play("minecraft:ui.button.click", None, 0.25, 1.0);
        let shift = self.shift_down();
        self.try_place(entry, shift)
    }
    /// `RecipeBookComponent.mouseDragged`: while the search box is focused a
    /// drag moves its selection end to the pointer (`EditBox.onDrag`).
    pub(super) fn recipe_book_drag(&mut self) {
        if !self.recipe_ui.search_drag
            || !(self.mod_book_open() || self.active_book().is_some_and(|b| self.book_open(b)))
        {
            return;
        }
        let x = self.cursor.0;
        if let Some(search) = self.recipe_ui.search.as_mut()
            && search.focused
        {
            search.drag_to(&self.gui.atlas, x);
        }
    }
    /// A mouse button let go: a drag no longer selects in the search box.
    pub(super) fn recipe_book_release(&mut self) {
        self.recipe_ui.search_drag = false;
    }
    /// `RecipeBookComponent.sendUpdateSettings`: the server owns the saved book.
    fn send_book_settings(&self, book_type: BookType) {
        self.send(
            horizon_protocol::packets::play::Serverbound::RecipeBookChangeSettings(
                horizon_protocol::packets::recipes::RecipeBookChangeSettings {
                    book_type: book_type.index() as i32,
                    open: self.book_open(book_type),
                    filtering: self.book_filtering(book_type),
                },
            ),
        );
    }
    pub(super) fn recipe_book_flags(&self) -> Option<(bool, bool)> {
        if self.native_screen_shown() {
            return self.mod_book_flags();
        }
        let book = self.active_book()?;
        Some((self.book_open(book), self.book_filtering(book)))
    }
    /// How many displays the client's book holds.
    pub(super) fn recipe_book_known(&self) -> usize {
        self.recipe_ui.client.displays().len()
    }
    /// The next or previous page, as the arrows turn it: nothing past either end, or with a
    /// pointer button held.
    pub(super) fn pad_recipe_page(&mut self, next: bool) {
        if self.pad.menu.held != [false; 2] {
            return;
        }
        if self.native_screen_shown() {
            if self.mod_book_open() && !self.spectator() {
                self.recipe_ui.custom.turn_page(next);
            }
            return;
        }
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        let page = self.recipe_ui.page;
        let Some(to) = (if next {
            (page + 1 < pages).then_some(page + 1)
        } else {
            page.checked_sub(1)
        }) else {
            return;
        };
        self.recipe_ui.page = to;
        self.audio
            .play("minecraft:ui.button.click", None, 0.25, 1.0);
    }
    /// The crafting book's entry of the recipe button under the pointer, in its overlay if open.
    pub(super) fn recipe_under_pointer(&self) -> Option<usize> {
        let batch = self.batch_for_size()?;
        let book_type = self.active_book()?;
        if !self.book_open(book_type) || self.spectator() {
            return None;
        }
        let cursor = self.cursor;
        if let Some(overlay) = &self.recipe_ui.overlay {
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            return overlay.entries.iter().enumerate().find_map(|(n, e)| {
                let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                inside(cursor, x, y, 24.0, 24.0).then_some(e.0)
            });
        }
        let (xo, yo) = self.book_origin(&batch);
        let first = self.recipe_ui.page * 20;
        (0..20).find_map(|n| {
            let button = self.recipe_ui.view.buttons.get(first + n)?;
            let (x, y) = Self::button_pos(xo, yo, n);
            inside(cursor, x, y, 25.0, 25.0)
                .then(|| button.entries[self.swap_index() % button.entries.len()].0)
        })
    }
    /// The recipe button under the pointer has others to show (`gui.recipebook.moreRecipes`).
    pub(super) fn recipe_has_alternatives_under_pointer(&self) -> bool {
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        if self.recipe_ui.overlay.is_some() {
            return false;
        }
        let (xo, yo) = self.book_origin(&batch);
        let first = self.recipe_ui.page * 20;
        (0..20).any(|n| {
            let (x, y) = Self::button_pos(xo, yo, n);
            inside(self.cursor, x, y, 25.0, 25.0)
                && self
                    .recipe_ui
                    .view
                    .buttons
                    .get(first + n)
                    .is_some_and(|button| button.entries.len() > 1)
        })
    }
    /// The pointer is on one of the book's own controls, a recipe or not.
    pub(super) fn recipe_book_under_pointer(&self) -> bool {
        self.recipe_book_targets().iter().any(|t| {
            let [x, y, w, h] = t.rect;
            inside(self.cursor, x, y, w, h)
        })
    }
    /// Whether the crafting book's entry can be crafted from what the menu holds now.
    pub(super) fn recipe_craftable(&mut self, entry_index: usize) -> bool {
        let available = self.book_available(BookType::Crafting);
        self.recipe_ui.book.as_ref().is_some_and(|book| {
            book.entries
                .get(entry_index)
                .is_some_and(|entry| book.can_craft(entry, &available))
        })
    }
    /// The crafting book's entry by what a rebuilt book keeps of it (`pad_craft::RecipeKey`).
    pub(super) fn recipe_stable_key(
        &self,
        entry_index: usize,
    ) -> Option<super::pad_craft::RecipeKey> {
        let entry = self.recipe_ui.book.as_ref()?.entries.get(entry_index)?;
        Some(super::pad_craft::RecipeKey {
            id: entry.id.clone(),
            display: entry.display_id,
        })
    }
    /// Where the crafting book holds the recipe now: none once the book has it no more.
    pub(super) fn recipe_entry_index(&self, key: &super::pad_craft::RecipeKey) -> Option<usize> {
        self.recipe_ui
            .book
            .as_ref()?
            .entries
            .iter()
            .position(|e| e.id == key.id && e.display_id == key.display)
    }
    /// What the crafting book's entry makes: none when the book has no such entry.
    pub(super) fn recipe_results(&self, entry_index: usize) -> &[ItemStack] {
        self.recipe_ui
            .book
            .as_ref()
            .and_then(|book| book.entries.get(entry_index))
            .map_or(&[], Entry::results)
    }
    /// Places one set of the crafting book's entry, as a click on its button places it.
    pub(super) fn recipe_place_once(&mut self, entry_index: usize) {
        self.recipe_ui.last_placed = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .map(|e| e.id.clone());
        self.recipe_ui.ghost.clear();
        self.recipe_ui.overlay = None;
        self.place_book_recipe(BookType::Crafting, entry_index, false);
    }
    /// The recipe book's buttons a controller can focus: the book button,
    /// recipes, page arrows, the craftable filter and the tabs (GUI pixels;
    /// the search box registers itself as a field).
    pub(super) fn recipe_book_targets(&self) -> Vec<horizon_client::widgets::Target> {
        if self.native_screen_shown() {
            return self.mod_book_targets();
        }
        use horizon_client::widgets::{Kind, Target};
        let (Some(book_type), Some(batch)) = (self.active_book(), self.batch_for_size()) else {
            return Vec::new();
        };
        let button = |rect| Target {
            rect,
            kind: Kind::Button,
        };
        let (ox, _) = self.inventory_origin(&batch);
        let (bx, by) = self.book_button_pos(book_type, ox, &batch);
        let mut out = vec![button([bx, by, 20.0, 18.0])];
        if !self.book_open(book_type) || self.spectator() {
            return out;
        }
        let (xo, yo) = self.book_origin(&batch);
        if self.recipe_ui.overlay.is_none() {
            // The search box is painted by hand: a field, for the pad's keyboard.
            out.push(Target {
                rect: [xo + 25.0, yo + 13.0, 81.0, 14.0],
                kind: Kind::Field,
            });
        }
        if let Some(overlay) = &self.recipe_ui.overlay {
            // The alternatives overlay takes every click while open.
            let maxrow = if overlay.entries.len() <= 16 { 4 } else { 5 };
            return (0..overlay.entries.len())
                .map(|n| {
                    let x = overlay.x + 4.0 + 25.0 * (n % maxrow) as f32;
                    let y = overlay.y + 5.0 + 25.0 * (n / maxrow) as f32;
                    button([x, y, 24.0, 24.0])
                })
                .collect();
        }
        let first = self.recipe_ui.page * 20;
        let shown = self
            .recipe_ui
            .view
            .buttons
            .len()
            .saturating_sub(first)
            .min(20);
        out.extend((0..shown).map(|n| {
            let (x, y) = Self::button_pos(xo, yo, n);
            button([x, y, 25.0, 25.0])
        }));
        let pages = self.recipe_ui.view.buttons.len().div_ceil(20);
        if pages > 1 && self.recipe_ui.page + 1 < pages {
            out.push(button([xo + 93.0, yo + 137.0, 12.0, 17.0]));
        }
        if pages > 1 && self.recipe_ui.page > 0 {
            out.push(button([xo + 38.0, yo + 137.0, 12.0, 17.0]));
        }
        out.push(button([xo + 110.0, yo + 12.0, 26.0, 16.0]));
        let selected = self.recipe_ui.tab.unwrap_or(Tab::Search);
        for (n, tab) in self.recipe_ui.view.tabs.iter().enumerate() {
            let (x, y) = self.tab_pos(xo, yo, n);
            out.push(Target {
                rect: [x, y, 35.0, 27.0],
                kind: Kind::Tab(*tab == selected),
            });
        }
        out
    }
    fn shift_down(&self) -> bool {
        self.has_shift_down()
    }
    /// The open screen's book for `hasClickedOutside`: (visible, selected tab
    /// hovered); None when the screen has no book.
    pub(super) fn recipe_book_area(&self, batch: &HudBatch) -> Option<(bool, bool)> {
        if self.native_screen_shown() {
            return self.mod_book_area(batch);
        }
        let book = self.active_book()?;
        if !self.book_open(book) {
            return Some((false, false));
        }
        let (xo, yo) = self.book_origin(batch);
        let tabs = &self.recipe_ui.view.tabs;
        let selected = self.recipe_ui.tab.unwrap_or(Tab::Search);
        let hovered = tabs.iter().position(|t| *t == selected).is_some_and(|n| {
            let (x, y) = self.tab_pos(xo, yo, n);
            inside(self.cursor, x, y, 35.0, 27.0)
        });
        Some((true, hovered))
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
        let (x, y) =
            screen::overlay_origin(bx as i32, by as i32, entries.len(), xo as i32, yo as i32);
        let (x, y) = (x as f32, y as f32);
        self.recipe_ui.overlay = Some(Overlay { x, y, entries });
    }

    // ---- placing a recipe ----

    /// What the book's click handler does once a recipe was placed: a book
    /// that is not offset next to the menu (`!isOffsetNextToMainGUI`, a window
    /// narrower than `NARROW`) closes with `setVisible(false)`.
    fn close_narrow_book(&mut self, book_type: BookType, batch: &HudBatch) {
        if batch.width >= NARROW {
            return;
        }
        self.level.recipe_book.open[book_type.index()] = false;
        self.build_screen();
        self.send_book_settings(book_type);
        self.recipe_ui.overlay = None;
    }
    /// `RecipeBookComponent.keyPressed`: Escape closes the book, not the
    /// screen, while it covers a narrow menu; a wide window's Escape reaches
    /// the container.
    pub(super) fn recipe_book_escape(&mut self) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(batch) = self.batch_for_size() else {
            return false;
        };
        if !escape_closes_book(batch.width, self.book_open(book_type)) {
            return false;
        }
        self.close_narrow_book(book_type, &batch);
        true
    }
    /// `RecipeBookComponent.tryPlaceRecipe`: false when the click is refused.
    fn try_place(&mut self, entry_index: usize, use_max: bool) -> bool {
        let Some(book_type) = self.active_book() else {
            return false;
        };
        let Some(id) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .map(|e| e.id.clone())
        else {
            return false;
        };
        self.recipe_ui.last_placed = Some(id);
        self.recipe_ui.ghost.clear();
        self.place_book_recipe(book_type, entry_index, use_max);
        true
    }
    /// Whether the open screen takes `native_place_recipe`: a crafting grid's
    /// book (the inventory's or a crafting table's), or a mod menu type's
    /// crafting grid.
    pub(super) fn mod_recipe_screen(&self) -> bool {
        self.screen == Screen::Inventory
            && (self.active_book() == Some(BookType::Crafting) || self.mod_crafting_open())
            && !self.spectator()
    }
    /// Whether the open menu is a mod menu type with a crafting grid.
    fn mod_crafting_open(&self) -> bool {
        self.remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .and_then(|(_, menu)| horizon_client::containers::mod_menu_type(menu))
            .is_some_and(|t| t.crafting.is_some())
    }
    /// `native_place_recipe`: the book's click on the recipe with this display
    /// id, refused as `handlePlaceRecipe` refuses it (no crafting screen,
    /// not in the player's book; an impossible one in `place_recipe`).
    pub(super) fn mod_place_recipe(&mut self, display_id: i32, all: bool) {
        if !self.mod_recipe_screen() || !self.recipe_ui.client.displays().contains(&display_id) {
            return;
        }
        self.ensure_book();
        let Some(index) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.iter().position(|e| e.display_id == display_id))
        else {
            return;
        };
        self.recipe_ui.ghost.clear();
        self.place_book_recipe(BookType::Crafting, index, all);
    }
    /// Whether the open menu is a content mod's menu type (a machine's, with or without a grid):
    /// the server's menu on a server, the block's own in a solo world or on a LAN host.
    fn mod_menu_open(&self) -> bool {
        self.remote_menu_type()
            .and_then(horizon_client::containers::mod_menu_type)
            .is_some()
            || self.local_mod_menu().is_some()
    }
    /// The content mod menu type and block of the menu this player has open when the game is
    /// its own server (a solo world, a LAN host's own player): the block container's menu type.
    fn local_mod_menu(&self) -> Option<(&'static str, [i32; 3])> {
        if self.remote.is_some() {
            return None;
        }
        let k = self.container?;
        Some((self.mod_menu_at(k)?, [k.0, k.1, k.2]))
    }
    /// `menu_transfer` for this player's own mod menu when the game is its own server: the same
    /// server-side ask a guest's transfer gets (`guest_menu_transfer`), on the same thread.
    /// None when no part listens (vanilla menus never ask) or no mod menu is open here.
    fn local_menu_transfer(
        &mut self,
        recipe: &str,
        max: bool,
        dry_run: bool,
    ) -> Option<RecipeTransferResult> {
        if !mod_api::__private::has_part_hook(mod_api::HookId::MenuTransfer) {
            return None;
        }
        let (menu, pos) = self.local_mod_menu()?;
        if !mod_api::__private::menu_listens(mod_api::HookId::MenuTransfer, menu) {
            return None;
        }
        let mut result = None;
        self.native_menu_transfer(mod_api::__private::MenuTransferCall {
            player: self.own_uuid(),
            menu,
            pos: Some(pos),
            recipe,
            max,
            dry_run,
            result: &mut result,
        });
        result
    }
    /// A viewer's transfer of a recipe by id (`transfer_recipe_id`): sent to the server, whose
    /// `menu_transfer` parts on a mod menu answer first, then its grid transfer. Only a server
    /// that takes viewer transfers is asked: elsewhere it shows as not into this menu.
    pub(super) fn transfer_viewer_recipe_id(&mut self, recipe: &str, max: bool) {
        self.panel.transfer_warning = None;
        let open = self.screen == Screen::Inventory
            && !self.spectator()
            && (self.mod_menu_open() || self.mod_recipe_screen());
        if !open {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        }
        if self.remote.is_none() {
            // The game is its own server: the mod menu's part answers first, as on a server,
            // then the grid transfer.
            if let Some(answer) = self.local_menu_transfer(recipe, max, false) {
                self.show_transfer_result(answer);
                return;
            }
            self.local_grid_transfer(recipe, max);
            return;
        }
        if !self
            .remote
            .as_ref()
            .is_some_and(|r| r.recipe_viewer.transfer)
        {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        }
        let mut request = horizon_protocol::buf::Writer::new();
        request.varint(self.remote_menu_id());
        request.string(recipe);
        request.u8(u8::from(max));
        self.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
            horizon_protocol::packets::configuration::CustomPayload {
                channel: horizon_client::lan::recipes::TRANSFER_CHANNEL.into(),
                data: request.data,
            },
        ));
    }
    /// The display ids of `recipe_id` that the server sent and this book
    /// numbers the same way (`Book::confirmed_ids`): the entries
    /// `transfer_viewer_recipe` and `transfer_recipe_check` find. A server
    /// numbers the recipes it sent itself, so the display index's ids are not
    /// evidence of its own.
    pub(in crate::game) fn viewer_book_ids(&mut self, recipe_id: &str) -> std::sync::Arc<[i32]> {
        self.ensure_book();
        self.recipe_ui
            .book
            .as_ref()
            .map_or_else(Default::default, |book| book.confirmed_ids(recipe_id))
    }
    /// A viewer's transfer of one set of ingredients.
    pub(super) fn transfer_viewer_recipe(&mut self, display_id: i32) {
        self.panel.transfer_warning = None;
        if !self.mod_recipe_screen()
            || (self.creative && self.craft_size != 3 && !self.mod_crafting_open())
        {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        }
        if self
            .remote
            .as_ref()
            .is_some_and(|r| !r.recipe_viewer.transfer)
        {
            // Older Horizon and vanilla servers still accept book clicks.
            if self.recipe_place(display_id, false).is_err() {
                self.panel.transfer_warning = Some((
                    "horizon.recipe_transfer.unsupported",
                    "Server requires a known recipe",
                ));
            }
            return;
        }
        self.ensure_book();
        let Some(id) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|book| {
                book.entries
                    .iter()
                    .find(|entry| entry.display_id == display_id)
            })
            .map(|entry| entry.id.clone())
        else {
            self.show_transfer_result(RecipeTransferResult::NotIntoMenu);
            return;
        };
        if self.remote.is_some() {
            let mut request = horizon_protocol::buf::Writer::new();
            request.varint(self.remote_menu_id());
            request.string(&id);
            self.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
                horizon_protocol::packets::configuration::CustomPayload {
                    channel: horizon_client::lan::recipes::TRANSFER_CHANNEL.into(),
                    data: request.data,
                },
            ));
            return;
        }
        // The game is its own server: a mod menu's part answers first, as on a server.
        if let Some(answer) = self.local_menu_transfer(&id, false, false) {
            self.show_transfer_result(answer);
            return;
        }
        self.local_grid_transfer(&id, false);
    }
    /// The game's own grid transfer of `id` into this player's menu when the game is its own
    /// server (what a server runs when no `menu_transfer` part answers).
    fn local_grid_transfer(&mut self, id: &str, max: bool) {
        use horizon_client::menu::place::TransferError;
        let mut result = Err(TransferError::Unavailable);
        self.menu_session(|menu, env, _| {
            result = menu.transfer_recipe(env, id, max);
            Some(result.is_ok())
        });
        self.show_transfer_result(horizon_client::lan::recipes::transfer_result(result));
        if result.is_ok() {
            self.recipe_book_changed();
        }
    }

    /// A payload on one of the recipe viewer's channels
    /// (`lan::recipes::viewer_channel`); with no data, the server's word that
    /// it answers that channel.
    pub(super) fn remote_recipe_viewer(&mut self, channel: &str, data: &[u8]) {
        use horizon_client::lan::recipes::TRANSFER_CHANNEL;
        if data.is_empty() {
            if let Some(remote) = &mut self.remote {
                let channels = &mut remote.recipe_viewer;
                *(if channel == TRANSFER_CHANNEL {
                    &mut channels.transfer
                } else {
                    &mut channels.check
                }) = true;
            }
        } else if channel == TRANSFER_CHANNEL {
            self.remote_recipe_transfer(data);
        } else {
            self.remote_recipe_check(data);
        }
    }

    fn remote_recipe_transfer(&mut self, data: &[u8]) {
        if let Ok(Some((container, result))) =
            horizon_client::lan::recipes::decode_transfer_reply(data)
            && container == self.remote_menu_id()
            && (self.mod_recipe_screen() || self.mod_menu_open())
            && self.panel.recipes.is_some()
        {
            match horizon_client::lan::recipes::byte_result(result) {
                Some(result) => self.show_transfer_result(result),
                None => self.panel.transfer_warning = None,
            }
        }
    }

    fn show_transfer_result(&mut self, result: RecipeTransferResult) {
        if result == RecipeTransferResult::Moves {
            self.close_transferred_recipe();
        }
        self.panel.transfer_warning = match result {
            RecipeTransferResult::Moves => None,
            RecipeTransferResult::Missing => {
                Some(("horizon.recipe_transfer.missing", "Missing Items"))
            }
            RecipeTransferResult::NoRoom => {
                Some(("horizon.recipe_transfer.no_room", "Inventory full"))
            }
            RecipeTransferResult::NotIntoMenu => Some((
                "horizon.recipe_transfer.unavailable",
                "Recipe does not fit this menu",
            )),
        };
    }

    /// `RecipesClientHost::transfer_recipe_check`: the server's latest check of a viewer
    /// transfer of `display_id` into the open crafting grid. Asked for when
    /// there is no answer for what the client sees now (the window, its state
    /// id, the inventory and the grid), and again every `CHECK_REFRESH` while
    /// it is wanted, at most `CHECK_REFRESHES` times for that view: chests a
    /// station takes from change where the client can't see them. One request
    /// out at a time. None until the answer comes, outside a screen that takes
    /// recipes, and on a server that doesn't advertise `horizon:recipe_check`.
    pub(in crate::game) fn transfer_recipe_check(
        &mut self,
        display_id: i32,
    ) -> Option<RecipeTransferCheck> {
        // The game is its own server: a mod menu's part answers the dry run, as
        // `horizon:recipe_check` asks it on a server; with none the check is as before.
        if self.remote.is_none() && self.mod_menu_open() {
            self.ensure_book();
            let id = (self.recipe_ui.book.as_ref())
                .and_then(|book| book.entries.iter().find(|e| e.display_id == display_id))
                .map(|e| e.id.clone())?;
            let result = self.local_menu_transfer(&id, false, true)?;
            return Some(RecipeTransferCheck {
                result,
                missing_cells: Vec::new(),
            });
        }
        let eligible_menu = self.mod_recipe_screen()
            || (self.screen == Screen::Inventory && !self.spectator() && self.mod_menu_open());
        if !eligible_menu || !self.remote.as_ref().is_some_and(|r| r.recipe_viewer.check) {
            return None;
        }
        let now = Instant::now();
        let seen = (
            self.remote_menu_id(),
            self.remote.as_ref().map_or(0, |r| r.state_id),
            (self.inventory.iter().take(36))
                .chain(&self.craft_grid)
                .cloned()
                .collect::<Vec<_>>(),
        );
        let checks = &mut self.recipe_ui.checks;
        if checks.seen.as_ref() != Some(&seen) {
            *checks = TransferChecks {
                seen: Some(seen),
                ..Default::default()
            };
        }
        let answer = (checks.answers.iter()).find(|a| a.display_id == display_id);
        let ask = match answer {
            None => true,
            Some(a) => a.refreshes < CHECK_REFRESHES && now.duration_since(a.at) >= CHECK_REFRESH,
        };
        let answer = answer.map(|a| a.check.clone());
        let waiting = (checks.asked).is_some_and(|(_, _, at)| now.duration_since(at) < CHECK_WAIT);
        if ask && !waiting {
            self.ensure_book();
            let id = (self.recipe_ui.book.as_ref())
                .and_then(|book| book.entries.iter().find(|e| e.display_id == display_id))
                .map(|e| e.id.clone());
            if let Some(id) = id {
                let mut request = horizon_protocol::buf::Writer::new();
                request.varint(self.remote_menu_id());
                let Some(tag) = self.recipe_ui.checks.ask(display_id, now) else {
                    return answer;
                };
                request.varint(tag);
                request.string(&id);
                self.send(horizon_protocol::packets::play::Serverbound::CustomPayload(
                    horizon_protocol::packets::configuration::CustomPayload {
                        channel: horizon_client::lan::recipes::CHECK_CHANNEL.into(),
                        data: request.data,
                    },
                ));
            }
        }
        answer
    }

    /// The server's answer to `horizon:recipe_check`. Kept only as the answer
    /// to the request out, for the view of the menu it was asked for.
    fn remote_recipe_check(&mut self, data: &[u8]) {
        let mut reader = horizon_protocol::buf::Reader::new(data);
        let (Ok(container), Ok(tag), Ok(byte)) = (reader.varint(), reader.varint(), reader.u8())
        else {
            return;
        };
        let Some(result) = horizon_client::lan::recipes::byte_result(byte) else {
            return;
        };
        let mut missing_cells = Vec::new();
        if result == RecipeTransferResult::Missing {
            let Ok(n) = reader.varint() else {
                return;
            };
            for _ in 0..n.clamp(0, 256) {
                let Ok(cell) = reader.u8() else {
                    return;
                };
                missing_cells.push(u32::from(cell));
            }
        }
        let checks = &mut self.recipe_ui.checks;
        if reader.remaining() != 0
            || checks.asked.map(|(id, _, _)| id) != Some(tag)
            || checks.seen.as_ref().map(|s| s.0) != Some(container)
        {
            return;
        }
        let (_, display_id, _) = checks.asked.take().unwrap();
        let refreshes = match checks
            .answers
            .iter()
            .position(|a| a.display_id == display_id)
        {
            Some(i) => checks.answers.remove(i).refreshes + 1,
            None => 0,
        };
        if checks.answers.len() >= CHECK_KEPT {
            checks.answers.remove(0);
        }
        checks.answers.push(CheckAnswer {
            display_id,
            at: Instant::now(),
            refreshes,
            check: RecipeTransferCheck {
                result,
                missing_cells,
            },
        });
    }

    /// `ServerboundPlaceRecipePacket`: the server places the recipe
    /// (`ServerPlaceRecipe` on the menu engine) and answers with a ghost
    /// recipe when the items are missing.
    fn place_book_recipe(&mut self, book_type: BookType, entry_index: usize, use_max: bool) {
        let Some(entry) = self
            .recipe_ui
            .book
            .as_ref()
            .and_then(|b| b.entries.get(entry_index))
            .cloned()
        else {
            return;
        };
        if self.remote.is_some() {
            // The server places it and answers with the slots, or a ghost.
            let container = self.remote_menu_id();
            self.send(horizon_protocol::packets::play::Serverbound::PlaceRecipe(
                horizon_protocol::packets::recipes::PlaceRecipe {
                    container,
                    recipe: entry.display_id,
                    use_max,
                },
            ));
            return;
        }
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

    // ---- a server's recipe book ----

    /// ClientboundRecipeBookAddPacket (`ClientRecipeBook.add`): the
    /// displays join the book, highlighted when new, and the recipes whose
    /// entries ask for it show the toast.
    pub(super) fn remote_recipe_book_add(
        &mut self,
        p: horizon_protocol::packets::recipes::RecipeBookAdd,
    ) {
        let ui = &mut self.recipe_ui;
        // `display_index`, borrowed by field beside the book.
        let index = match &self.recipe_tables.synced_index {
            Some(index) => index,
            None => &world_stations!(self).recipe_index,
        };
        if p.replace {
            ui.client.clear();
            ui.server_entries.clear();
        }
        let mut toast = Vec::new();
        for e in &p.entries {
            ui.client.add(e.id);
            ui.server_entries.insert(e.id, e.clone());
            if e.flags & 2 != 0 {
                ui.client.highlight.insert(e.id);
            }
            if e.flags & 1 != 0
                && let Some(id) = index.recipe_of_display(e.id)
                && !toast.iter().any(|t| t == id)
            {
                toast.push(id.to_owned());
            }
        }
        // A local world opened later syncs its book from scratch.
        ui.client_known = None;
        ui.client_version += 1;
        // A replacing packet starts the book again from its own recipes; a
        // later one puts its entries over the book as it is.
        if p.replace {
            ui.book = None;
        } else if let Some(mut book) = ui.book.take() {
            self.server_entries_over(&mut book, p.entries.iter());
            self.recipe_ui.book = Some(book);
        }
        self.recipe_toast(&toast);
    }
    /// ClientboundRecipeBookRemovePacket.
    pub(super) fn remote_recipe_book_remove(
        &mut self,
        p: horizon_protocol::packets::recipes::RecipeBookRemove,
    ) {
        for id in p.ids {
            self.recipe_ui.client.remove(id);
            self.recipe_ui.server_entries.remove(&id);
            if let Some(book) = self.recipe_ui.book.as_mut() {
                book.unconfirm(id);
            }
        }
        self.recipe_ui.client_version += 1;
    }
    /// ClientboundRecipeBookSettingsPacket: each book's open and filter flags.
    pub(super) fn remote_recipe_book_settings(
        &mut self,
        p: horizon_protocol::packets::recipes::RecipeBookSettings,
    ) {
        for (i, (open, filtering)) in p.books.into_iter().enumerate() {
            self.level.recipe_book.open[i] = open;
            self.level.recipe_book.filtering[i] = filtering;
        }
        self.build_screen();
    }
    /// ClientboundPlaceGhostRecipePacket: the recipe last placed, faded into
    /// the open menu's grid.
    pub(super) fn remote_ghost_recipe(
        &mut self,
        p: horizon_protocol::packets::recipes::PlaceGhostRecipe,
    ) {
        let open = self.remote_menu_id();
        let Some(book_type) = self.active_book() else {
            return;
        };
        if p.container != open {
            return;
        }
        let entry = self.recipe_ui.last_placed.as_ref().and_then(|id| {
            let book = self.recipe_ui.book.as_ref()?;
            book.entry(id).cloned()
        });
        if let Some(entry) = entry {
            self.recipe_ui.ghost.clear();
            self.fill_ghost(book_type, &entry);
        }
        // The server is missing an item the client counted: nothing was placed.
        self.direct_craft_refused();
    }

    // ---- ghost recipe ----

    /// `fillGhostRecipe`: the recipe's ingredients faded into the slots.
    fn fill_ghost(&mut self, book_type: BookType, entry: &Entry) {
        let grid = self.craft_size;
        let fuel_empty = self.slot_mut(301).is_none();
        let Some(book) = self.recipe_ui.book.as_ref() else {
            return;
        };
        let ghost = book
            .ghost(entry, book_type, grid, fuel_empty, self.items())
            .into_iter()
            .map(|g| Ghost {
                slot: match (g.target, book_type) {
                    (GhostTarget::Result, BookType::Crafting) => 200,
                    (GhostTarget::Result, _) => 302,
                    (GhostTarget::Grid(i), _) => 100 + i,
                    (GhostTarget::Input, _) => 300,
                    (GhostTarget::Fuel, _) => 301,
                },
                result: g.target == GhostTarget::Result,
                items: g.items,
            })
            .collect();
        self.recipe_ui.ghost = ghost;
    }

    // ---- drawing ----

    /// The book button, ghost recipe and open book, returning its deferred tooltip.
    pub(super) fn draw_recipe_book(&mut self, batch: &mut HudBatch) -> Option<Vec<(Text, u32)>> {
        if self.native_screen_shown() {
            return self.draw_mod_book(batch);
        }
        let book_type = self.active_book()?;
        self.ensure_book();
        if !self.shift_down_or_ctrl() {
            self.recipe_ui.time += self.frame_ticks;
        }
        let (ox, _) = self.inventory_origin(batch);
        let cursor = self.cursor;
        let (bx, by) = self.book_button_pos(book_type, ox, batch);
        let hover = inside(cursor, bx, by, 20.0, 18.0);
        draw_book_button(batch, &self.gui.atlas, bx, by, hover);
        self.draw_ghost(batch, book_type);
        let tooltip = if self.book_open(book_type) {
            self.refresh_book_view(book_type);
            self.draw_book_panel(batch, book_type)
        } else {
            None
        };
        self.narrate_recipe_book(batch, book_type, [bx, by, 20.0, 18.0]);
        tooltip
    }
    /// `AbstractRecipeBookScreen`'s narratables: the book button (an
    /// `ImageButton` without a message) and the `RecipeBookComponent`, which
    /// narrates its recipe buttons, search box, filter and tabs while open.
    fn narrate_recipe_book(&self, batch: &HudBatch, book_type: BookType, button: [f32; 4]) {
        use horizon_client::widgets::{self, Narratable, key_of};
        if !widgets::narrating() {
            return;
        }
        // The button moves with the book: it keeps the focus a click gave it.
        widgets::narratable(|| {
            Narratable::button("", button, true).key(key_of(&"recipe_book/button"))
        });
        let open = self.book_open(book_type);
        let (xo, yo) = self.book_origin(batch);
        let group = key_of(&"recipe_book");
        widgets::narratable(|| Narratable::group(open, [xo, yo, WIDTH, HEIGHT]).key(group));
        if !open {
            return;
        }
        let in_book = |n: Narratable, i: usize| n.key(key_of(&(group, i))).in_group(group);
        let ui = &self.recipe_ui;
        let first = ui.page * 20;
        let shown = ui.view.buttons.iter().skip(first).take(20);
        if let Some(book) = ui.book.as_ref() {
            for (n, b) in shown.enumerate() {
                widgets::narratable(|| {
                    // RecipeButton: the result it shows now.
                    let results: Vec<Vec<ItemStack>> = b
                        .entries
                        .iter()
                        .map(|e| vec![book.entries[e.0].result.clone()])
                        .collect();
                    let stack = screen::display_stack(&results, self.swap_index());
                    let item = horizon_client::stations::hover_name(&self.pack.names, &stack);
                    let (x, y) = Self::button_pos(xo, yo, n);
                    in_book(
                        Narratable::recipe(item, b.entries.len() > 1, [x, y, 25.0, 25.0]),
                        n,
                    )
                });
            }
        }
        let tr = |key: &str, args: &[&str]| horizon_client::screen_narrator::translate(key, args);
        if let Some(search) = &ui.search {
            widgets::narratable(|| {
                Narratable::edit_box(
                    tr("itemGroup.search", &[]),
                    search.value.clone(),
                    [xo + 25.0, yo + 13.0, 81.0, 14.0],
                )
                .focused(search.focused)
                .key(key_of(&(group, "search")))
                .in_group(group)
            });
        }
        widgets::narratable(|| {
            // A CycleButton of no name that hides its label: ": <value>",
            // the value's tooltip.
            let label = |filtering: bool| {
                tr(
                    if !filtering {
                        "gui.recipebook.toggleRecipes.all"
                    } else {
                        match book_type {
                            BookType::Crafting => "gui.recipebook.toggleRecipes.craftable",
                            BookType::Furnace => "gui.recipebook.toggleRecipes.smeltable",
                            BookType::BlastFurnace => "gui.recipebook.toggleRecipes.blastable",
                            BookType::Smoker => "gui.recipebook.toggleRecipes.smokable",
                        }
                    },
                    &[],
                )
            };
            let filtering = self.book_filtering(book_type);
            let full = |filtering| tr("options.generic_value", &["", &label(filtering)]);
            Narratable::cycle(
                full(filtering),
                full(!filtering),
                [xo + 110.0, yo + 12.0, 26.0, 16.0],
                true,
            )
            .hint(label(filtering))
            .key(key_of(&(group, "filter")))
            .in_group(group)
        });
        for n in 0..ui.view.tabs.len() {
            let (x, y) = self.tab_pos(xo, yo, n);
            widgets::narratable(|| {
                Narratable::button("", [x, y, 35.0, 27.0], true)
                    .key(key_of(&(group, "tab", n)))
                    .in_group(group)
            });
        }
    }
    fn shift_down_or_ctrl(&self) -> bool {
        self.has_control_down()
    }
    /// `GhostSlots.extractRenderState`, among the container's slots.
    fn draw_ghost(&mut self, batch: &mut HudBatch, book_type: BookType) {
        if self.recipe_ui.ghost.is_empty() {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        let big_result = book_type != BookType::Crafting || self.craft_size == 3;
        let ghosts: Vec<GhostView> = self
            .recipe_ui
            .ghost
            .iter()
            .map(|g| {
                let (sx, sy) = self.slot_position(g.slot);
                GhostView {
                    x: sx as i32,
                    y: sy as i32,
                    items: g.items.clone(),
                    result: g.result,
                }
            })
            .collect();
        let calls = screen::ghost(self.swap_index(), ox as i32, oy as i32, big_result, &ghosts);
        self.paint_book(batch, &calls);
    }
    /// `GhostSlots.extractTooltip`: the stack a ghost shows on menu slot
    /// `slot` now, cycling like its draw; only while the open book is visible
    /// (`RecipeBookComponent.extractTooltip`).
    pub(super) fn ghost_stack(&self, slot: usize) -> Option<ItemStack> {
        if self.native_screen_shown() {
            if !self.mod_book_open() {
                return None;
            }
            self.native_screen_slot(slot.checked_sub(300)?)?;
        } else if !self.book_open(self.active_book()?) {
            return None;
        }
        let ghost = self
            .recipe_ui
            .ghost
            .iter()
            .find(|g| g.slot == slot && !g.items.is_empty())?;
        Some(ghost.items[self.swap_index() % ghost.items.len()].clone())
    }
    /// `RecipeBookComponent.extractRenderState` and `extractTooltip`, laid
    /// out by `horizon_client::recipe_book_screen`.
    fn draw_book_panel(
        &mut self,
        batch: &mut HudBatch,
        book_type: BookType,
    ) -> Option<Vec<(Text, u32)>> {
        let (xo, yo) = self.book_origin(batch);
        let cursor = self.cursor;
        let atlas = &self.gui.atlas;
        let search = match self.recipe_ui.search.as_mut() {
            Some(edit) => {
                // The box's place, for clicks and typing.
                edit.x = xo + 25.0;
                edit.y = yo + 13.0;
                let (display, blink_ms) = edit.view_state(atlas);
                Search {
                    value: edit.value.clone(),
                    cursor: edit.cursor,
                    highlight: edit.highlight,
                    display,
                    focused: edit.focused,
                    blink_ms,
                }
            }
            None => Search::default(),
        };
        let ui = &self.recipe_ui;
        let current = ui
            .tab
            .filter(|t| ui.view.tabs.contains(t))
            .unwrap_or(Tab::Search);
        let tabs: Vec<TabView> = ui
            .view
            .tabs
            .iter()
            .map(|tab| {
                let (a, b) = tab.icons();
                TabView {
                    icons: (a.to_owned(), b.map(str::to_owned)),
                    selected: *tab == current,
                    anim: ui
                        .tab_anim
                        .iter()
                        .find(|e| e.0 == *tab)
                        .map_or(0.0, |e| e.1),
                }
            })
            .collect();
        let pages = ui.view.buttons.len().div_ceil(20);
        let first = ui.page * 20;
        let (buttons, overlay) = match ui.book.as_ref() {
            Some(book) => {
                let buttons: Vec<ButtonView> = ui
                    .view
                    .buttons
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(20)
                    .map(|(n, b)| ButtonView {
                        has_craftable: b.craftable(),
                        entries: b
                            .entries
                            .iter()
                            .map(|e| vec![book.entries[e.0].result.clone()])
                            .collect(),
                        anim: ui
                            .button_anim
                            .iter()
                            .find(|e| e.0 == n)
                            .map_or(0.0, |e| e.1),
                    })
                    .collect();
                let overlay = ui.overlay.as_ref().map(|o| OverlayView {
                    x: o.x as i32,
                    y: o.y as i32,
                    recipes: o
                        .entries
                        .iter()
                        .map(|&(i, craftable)| OverlayRecipe {
                            craftable,
                            cells: screen::overlay_cells(book, &book.entries[i]),
                        })
                        .collect(),
                });
                (buttons, overlay)
            }
            None => (Vec::new(), None),
        };
        let translate = |key: &str| self.translate_with_fallback(key, key);
        let text_width = |s: &str| HudBatch::text_width(atlas, s) as i32;
        let layout = Layout {
            book: book_type,
            origin: (xo as i32, yo as i32),
            mouse: (cursor.0.floor() as i32, cursor.1.floor() as i32),
            index: self.swap_index(),
            filtering: self.book_filtering(book_type),
            search: &search,
            tabs: &tabs,
            buttons: &buttons,
            page: ui.page,
            pages,
            overlay: overlay.as_ref(),
            ghost: None,
            translate: &translate,
            text_width: &text_width,
        };
        let calls = screen::draw(&layout);
        let (panel_x, panel_y) = layout.origin;
        let tooltip = self.paint_book(batch, &calls);
        let covers_menu = batch.width < NARROW;
        self.native_recipe_book_render(
            batch,
            [panel_x as f32, panel_y as f32, WIDTH, HEIGHT],
            covers_menu,
        );
        // The squeeze animations run down with the frames.
        let frame = self.frame_ticks;
        for e in &mut self.recipe_ui.tab_anim {
            e.1 -= frame;
        }
        for e in &mut self.recipe_ui.button_anim {
            e.1 -= frame;
        }
        self.recipe_ui.tab_anim.retain(|e| e.1 > 0.0);
        self.recipe_ui.button_anim.retain(|e| e.1 > 0.0);
        tooltip
    }
    /// Paints recipe book calls; returns the first tooltip set (vanilla keeps
    /// the first one a frame asks for).
    fn paint_book(
        &mut self,
        batch: &mut HudBatch,
        calls: &[Drawn],
    ) -> Option<Vec<(horizon_client::text::Text, u32)>> {
        let mut tooltip: Option<Vec<(horizon_client::text::Text, u32)>> = None;
        for d in calls {
            let start = batch.vertices.len();
            let atlas = &self.gui.atlas;
            match &d.call {
                Call::Blit {
                    x, y, w, h, u, v, ..
                } => {
                    if let Some(r) = atlas.sprites.get("gui/recipe_book").copied() {
                        let (w, h) = (*w as f32, *h as f32);
                        batch.rect_uv(
                            *x as f32,
                            *y as f32,
                            w,
                            h,
                            sub_rect(r, [*u, *v, w, h]),
                            u32::MAX,
                        );
                    }
                }
                Call::Sprite { id, x, y, w, h } => {
                    let name = format!("gui/sprites/{id}");
                    let native = atlas
                        .sprites
                        .get(&name)
                        .is_some_and(|r| r.w as i32 == *w && r.h as i32 == *h);
                    if native {
                        batch.sprite(atlas, &name, *x as f32, *y as f32);
                    } else {
                        horizon_client::ui::nine_slice(
                            batch,
                            atlas,
                            &name,
                            *x as f32,
                            *y as f32,
                            *w as f32,
                            *h as f32,
                            u32::MAX,
                        );
                    }
                }
                Call::Fill {
                    x0,
                    y0,
                    x1,
                    y1,
                    color,
                } => batch.fill(
                    atlas,
                    *x0 as f32,
                    *y0 as f32,
                    (x1 - x0) as f32,
                    (y1 - y0) as f32,
                    argb(*color),
                ),
                Call::Highlight { x0, y0, x1, y1 } => {
                    let (a, b) = (*x0.min(x1), *x0.max(x1));
                    batch.text_highlight(
                        atlas,
                        (a as f32, *y0 as f32, (b - a) as f32, (y1 - y0) as f32),
                        true,
                    );
                }
                Call::Text {
                    runs,
                    x,
                    y,
                    color,
                    shadow,
                } => {
                    let mut x = *x as f32;
                    for r in runs {
                        let style = Style {
                            color: (r.color >= 0).then_some(r.color as u32),
                            italic: r.italic.then_some(true),
                            ..Style::default()
                        };
                        let t = Text::styled(r.text.clone(), style);
                        batch.text_styled(atlas, &t, x, *y as f32, argb(*color), *shadow);
                        x += atlas.font.width_text(&t);
                    }
                }
                Call::Item { call, stack, x, y } => match call {
                    ItemCall::Item | ItemCall::Fake => {
                        self.draw_item_model(batch, stack, *x as f32, *y as f32)
                    }
                    ItemCall::Decorations => {
                        self.draw_item_decorations(batch, stack, *x as f32, *y as f32, None)
                    }
                },
                Call::Tooltip(lines) => {
                    if tooltip.is_none() {
                        tooltip = Some(
                            lines
                                .iter()
                                .map(|l| {
                                    (
                                        horizon_client::text::Text::literal(
                                            l.iter().map(|r| r.text.as_str()).collect::<String>(),
                                        ),
                                        u32::MAX,
                                    )
                                })
                                .collect(),
                        );
                    }
                }
                Call::ItemTooltip { stack, more } => {
                    if tooltip.is_none() {
                        let mut lines = self.stack_tooltip(stack);
                        if *more {
                            lines.push((
                                horizon_client::text::Text::literal(self.translate_with_fallback(
                                    "gui.recipebook.moreRecipes",
                                    "Right Click for More",
                                )),
                                u32::MAX,
                            ));
                        }
                        tooltip = Some(lines);
                    }
                }
                Call::Stratum => {}
            }
            if d.pose != screen::IDENTITY {
                let m = d.pose;
                for v in &mut batch.vertices[start..] {
                    let [x, y] = v.pos;
                    v.pos = [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
                }
            }
        }
        tooltip
    }
}

/// An ARGB colour as a HUD vertex colour.
fn argb(c: i32) -> u32 {
    let c = c as u32;
    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8)
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

#[cfg(test)]
mod request_generation_tests {
    use super::*;
    #[test]
    fn delayed_answers_cannot_match_inventory_changes_or_retries() {
        let mut checks = TransferChecks::default();
        let first = checks.ask(42, Instant::now()).unwrap();
        checks = TransferChecks::default(); // Inventory change clears the cache.
        let second = checks.ask(42, Instant::now()).unwrap();
        assert_ne!(first, second);
        let retry = checks.ask(42, Instant::now()).unwrap();
        assert_ne!(second, retry);
        assert_eq!(
            checks.asked.map(|(tag, id, _)| (tag, id)),
            Some((retry, 42))
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn recipe_book_toggle_uses_the_widget_bounds_and_the_selected_sprite() {
        use super::*;
        use horizon_assets::resource_source::ResourceSource;

        let mut atlas = HudAtlas::fonts(&ResourceSource::default()).unwrap();
        for (name, u0) in [
            ("gui/sprites/recipe_book/button", 0.0),
            ("gui/sprites/recipe_book/button_highlighted", 0.25),
        ] {
            atlas.sprites.insert(
                name.into(),
                Rect {
                    u0,
                    v0: 0.0,
                    u1: u0 + 0.25,
                    v1: 0.25,
                    // An animated override of the vanilla button gets a
                    // square atlas rect rather than the widget's bounds.
                    w: 18,
                    h: 18,
                },
            );
        }
        for (hovered, u0) in [(false, 0.0), (true, 0.25)] {
            let mut batch = HudBatch::scratch([320, 240]);
            draw_book_button(&mut batch, &atlas, 5.0, 7.0, hovered);
            assert_eq!(batch.vertices.len(), 6);
            assert_eq!(batch.vertices[0].pos, [5.0, 7.0]);
            assert_eq!(batch.vertices[0].uv, [u0, 0.0]);
            assert_eq!(batch.vertices[2].pos, [25.0, 25.0]);
            assert_eq!(batch.vertices[2].uv, [u0 + 0.25, 0.25]);
        }
    }

    /// `RecipeBookComponent.keyPressed`: Escape closes a narrow window's open
    /// book (`!isOffsetNextToMainGUI`), not the container; a wide window's
    /// book leaves Escape to the container.
    #[test]
    fn escape_closes_only_a_narrow_books_menu() {
        assert!(super::escape_closes_book(super::NARROW - 1.0, true));
        assert!(!super::escape_closes_book(super::NARROW, true));
        assert!(!super::escape_closes_book(super::NARROW - 1.0, false));
    }

    /// `CraftingScreen` has the book; a server's menu without a grid has none.
    #[test]
    #[ignore = "daily"]
    fn a_servers_crafting_table_has_the_book() {
        use super::*;
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Crafter").unwrap();
            game.screen = Screen::Inventory;
            game.container = None;
            let open = |game: &mut Game<'_>, menu: &str| {
                game.remote
                    .as_mut()
                    .unwrap()
                    .menu
                    .set(Some((3, menu.into())));
                game.active_book()
            };
            assert_eq!(open(game, "minecraft:crafting"), Some(BookType::Crafting));
            assert_eq!(open(game, "minecraft:cartography_table"), None);
        });
    }
}

#[cfg(test)]
#[path = "recipe_book/check_mod_menu_tests.rs"]
mod check_mod_menu_tests;

#[cfg(test)]
mod screen_room_tests {
    use super::*;
    #[test]
    #[ignore = "daily"]
    fn shifted_book_painter_and_input_share_the_absolute_origin() {
        use mod_api::{ClientHost, Hooks, ModEntry, ScreenRoomEvent};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Panel(Arc<AtomicUsize>);
        #[mod_api::__subscriptions]
        impl Hooks for Panel {
            fn screen_room(&self, _: &dyn ClientHost, event: &mut ScreenRoomEvent<'_>) {
                event.right = event.right.max(self.0.load(Ordering::Relaxed) as f32);
            }
        }
        let room = Arc::new(AtomicUsize::new(100));
        let mods: mod_api::Mods =
            vec![ModEntry::new("panel", Box::new(Panel(room.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_client_game(|game| {
                game.screen = Screen::Inventory;
                game.container = None;
                game.craft_size = 2;
                game.settings.gui_scale = 1;
                game.level.recipe_book.open[BookType::Crafting.index()] = true;
                let batch = HudBatch::new([640, 240], game.settings.gui_scale_inputs());
                let (x, y) = game.book_origin(&batch);
                assert_eq!((x, y), (110.0, 37.0));
                assert_eq!(game.tab_pos(x, y, 0), (80.0, 40.0));
                assert_eq!(Game::button_pos(x, y, 0), (121.0, 68.0));
                assert_eq!(game.inventory_origin(&batch), (259.0, 37.0));
                let search = Search::default();
                let tabs = [screen::TabView {
                    icons: ("minecraft:stone".into(), None),
                    selected: true,
                    anim: 0.0,
                }];
                let layout = Layout {
                    book: BookType::Crafting,
                    origin: (x as i32, y as i32),
                    mouse: (221, 50),
                    index: 0,
                    filtering: false,
                    search: &search,
                    tabs: &tabs,
                    buttons: &[],
                    page: 0,
                    pages: 1,
                    overlay: None,
                    ghost: None,
                    translate: &str::to_owned,
                    text_width: &|s| s.len() as i32,
                };
                let calls = screen::draw(&layout);
                assert!(matches!(calls[0].call, Call::Blit { x: 110, y: 37, .. }));
                assert!(calls.iter().any(|c| matches!(&c.call, Call::Sprite { id, x: 78, y: 40, .. } if id == "recipe_book/tab_selected")));
                assert!(calls.iter().any(|c| matches!(&c.call, Call::Sprite { id, x: 220, y: 49, .. } if id == "recipe_book/filter_disabled_highlighted")));
                // A panel-only fit at 510 must be refused because the selected tab would clip.
                let batch = HudBatch::new([510, 240], game.settings.gui_scale_inputs());
                room.store(180, Ordering::Relaxed);
                game.frames += 1;
                assert_eq!(game.open_book_left(&batch), Some(63.0));
                assert_eq!(game.book_origin(&batch), (95.0, 37.0));
                assert_eq!(game.inventory_origin(&batch), (244.0, 37.0));
                room.store(100, Ordering::Relaxed);
                game.frames += 1;
                assert_eq!(game.book_origin(&batch), (45.0, 37.0));
                assert_eq!(game.inventory_origin(&batch), (194.0, 37.0));
            });
        });
    }
}
