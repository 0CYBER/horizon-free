//! The item panel beside container screens: the items a recipe viewer mod
//! lists (a native mod's `panel_items`, or WIT `interface browser`
//! of a WebAssembly mod, docs/modding/src/how-to/content-mods.md), drawn as the game's own item slots
//! in pages right of the screen's window, with a search box under them. The
//! mod says what the panel holds; the game pages it and tells the mod what
//! the player clicked, typed, and which keys went down over an item
//! (`native_panel_event`). With no mod showing a panel, each hook here
//! returns after one check.
use super::{
    Game, Screen, native_hooks::PanelEvent as BrowserEvent, native_panel_recipes::RecipePage,
};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
    ui::EditBox,
};
use mod_api::{PanelStyle, Stack as BrowserStack};

/// One slot of the panel, in GUI pixels.
const SLOT: f32 = 18.0;
/// The most columns the panel takes, and the fewest it shows with.
const COLUMNS: (usize, usize) = (12, 2);
/// The gap between the window and the panel, and the screen's edges.
const MARGIN: f32 = 8.0;
const RECIPES_SETTINGS_WIDTH: f32 = 20.0;
/// The classic look's gap to the window and the screen's edges, and its
/// square buttons and search box (`PanelStyle::Classic`, JEI 1.12's
/// `IngredientListOverlay`).
const CLASSIC_PAD: f32 = 2.0;
const CLASSIC_BUTTON: f32 = 20.0;

/// Items a panel may hold.
const ITEMS: usize = 8192;

/// The browser's overlay selected while painting its contents.
pub(super) enum BrowserTooltip {
    Text(String),
    Stack(ItemStack),
    Fluid(mod_api::FluidStack),
}

/// Whose panel it is: a native mod's id.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Owner {
    Native(String),
}
impl Owner {
    fn native_id(&self) -> &str {
        let Owner::Native(id) = self;
        id
    }
}

/// What a mod shows on the panel: its items, and the recipes shown in
/// their place.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Shown {
    pub(super) owner: Owner,
    /// Which change the panel is from, so the game converts it once.
    pub(super) version: u64,
    pub(super) items: Vec<BrowserStack>,
    pub(super) recipes: Option<RecipePage>,
}

/// The panel's state in the game.
#[derive(Default)]
pub(super) struct Panel {
    /// A native mod's panel: the last one to show something.
    native: Option<Shown>,
    /// The last native change.
    versions: u64,
    /// The items' owner and version they were made from.
    of: Option<(Owner, u64)>,
    /// The mod's items as stacks; `None` for an item the game doesn't have.
    stacks: Vec<Option<ItemStack>>,
    page: usize,
    pub(super) search: Option<EditBox>,
    /// The search text the mod last heard.
    sent: String,
    /// Only while dispatching Search: an empty reply still needs an editable field.
    searching: bool,
    pub(super) recipes_settings_return: Option<Screen>,
    /// The recipes shown instead of the items.
    pub(super) recipes: Option<super::native_panel_recipes::Pages>,
    pub(super) transfer_warning: Option<(&'static str, &'static str)>,
    /// The mods that asked for another look than the recipe book's.
    styles: std::collections::BTreeMap<String, PanelStyle>,
}

impl Panel {
    /// `panel_style`.
    pub(super) fn set_style(&mut self, mod_id: &str, style: PanelStyle) {
        match style {
            PanelStyle::RecipeBook => self.styles.remove(mod_id),
            _ => self.styles.insert(mod_id.to_owned(), style),
        };
    }
    /// How `owner`'s panel looks: Horizon's own Recipes panel is the viewer's.
    fn style(&self, owner: &Owner) -> PanelStyle {
        match owner.native_id() {
            "recipes" => PanelStyle::Viewer,
            id => self.styles.get(id).copied().unwrap_or_default(),
        }
    }
    /// Whether `owner`'s panel draws square page buttons and the viewer's
    /// search box: the viewer's look or the classic one.
    fn viewer_style(&self, owner: &Owner) -> bool {
        !matches!(self.style(owner), PanelStyle::RecipeBook)
    }
    /// The classic look's settings (`PanelStyle::Classic`), if `owner`
    /// asked for it.
    fn classic(&self, owner: &Owner) -> Option<(bool, u16)> {
        match self.style(owner) {
            PanelStyle::Classic {
                centred_search,
                window_width,
            } => Some((centred_search, window_width)),
            _ => None,
        }
    }
    /// What the panel shows: a native mod's, one `on`.
    pub(super) fn shown(&self, on: impl Fn(&str) -> bool) -> Option<&Shown> {
        self.native.as_ref().filter(|s| on(s.owner.native_id()))
    }
    /// `panel_items`.
    pub(super) fn show_items(&mut self, mod_id: &str, mut items: Vec<BrowserStack>) {
        items.truncate(ITEMS);
        if let Some(shown) = self.native_of(mod_id, !items.is_empty()) {
            shown.items = items;
            // An empty search result must leave a field to edit the query.
            self.native_changed(self.searching && !self.sent.is_empty());
        }
    }
    /// `native_panel_recipes`.
    pub(super) fn show_recipes(&mut self, mod_id: &str, page: Option<RecipePage>) {
        let filtering = mod_id == "recipes" && !self.sent.is_empty();
        if let Some(shown) = self.native_of(mod_id, page.is_some()) {
            let returning_to_search = filtering && shown.recipes.is_some() && page.is_none();
            shown.recipes = page;
            self.native_changed(returning_to_search);
        }
    }
    /// Mod `mod_id`'s panel, made when it shows something on another's;
    /// none when it clears a panel that isn't its own.
    fn native_of(&mut self, mod_id: &str, showing: bool) -> Option<&mut Shown> {
        let own = self
            .native
            .as_ref()
            .is_some_and(|s| s.owner.native_id() == mod_id);
        if !own {
            if !showing {
                return None;
            }
            self.native = Some(Shown {
                owner: Owner::Native(mod_id.to_owned()),
                version: 0,
                items: Vec::new(),
                recipes: None,
            });
        }
        self.native.as_mut()
    }
    /// A native panel changed: a new version, and no panel once it shows
    /// nothing, except while filtering the item list with a non-empty query.
    fn native_changed(&mut self, keep_search: bool) {
        self.versions += 1;
        self.transfer_warning = None;
        if let Some(shown) = self.native.as_mut() {
            shown.version = self.versions;
            if shown.items.is_empty() && shown.recipes.is_none() && !keep_search {
                self.native = None;
            }
        }
    }
}

/// Where the panel's parts are this frame.
pub(super) struct Layout {
    /// The panel's whole area, for clicks that hit none of its parts.
    pub(super) area: [f32; 4],
    /// The grid's top left corner, its columns and rows.
    pub(super) grid: (f32, f32),
    pub(super) columns: usize,
    pub(super) rows: usize,
    /// The page arrows' row.
    pub(super) arrows_y: f32,
    pub(super) search: [f32; 4],
    settings: Option<[f32; 4]>,
    /// The classic look: its page buttons always show and wrap around.
    classic: bool,
    /// The page row's left edge and width: the grid's, or JEI's least 80
    /// pixels flush right when the grid is narrower (the classic look).
    nav: (f32, f32),
}
impl Layout {
    fn browser_arrow_rects(&self, square: bool) -> [[f32; 4]; 2] {
        if square {
            let (x, w) = self.nav;
            return [
                [x, self.arrows_y, 20.0, 20.0],
                [x + w - 20.0, self.arrows_y, 20.0, 20.0],
            ];
        }
        self.arrows().map(|(x, y)| [x, y, 12.0, 17.0])
    }

    fn per_page(&self) -> usize {
        self.columns * self.rows
    }
    pub(super) fn width(&self) -> f32 {
        self.columns as f32 * SLOT
    }
    fn slot(&self, n: usize) -> (f32, f32) {
        (
            self.grid.0 + (n % self.columns) as f32 * SLOT,
            self.grid.1 + (n / self.columns) as f32 * SLOT,
        )
    }
    /// The page arrows: backward, then forward.
    pub(super) fn arrows(&self) -> [(f32, f32); 2] {
        [
            (self.grid.0, self.arrows_y),
            (self.grid.0 + self.width() - 12.0, self.arrows_y),
        ]
    }
}

/// The classic look's parts (`PanelStyle::Classic`, JEI 1.12's
/// `IngredientListOverlay`): the items from 2 pixels right of `window` (or of
/// a window `window_width` wide centred on the screen, when that is wider) to
/// 2 from the screen's edge, flush right and centred in their height, under a
/// 20 pixel row of page buttons; the search box 20 tall at the foot of the
/// screen, under the window when `centred` and there is room below it, else
/// under the items, with the 20 pixel settings button on its right. None
/// without room for two columns or a row.
fn classic_layout(
    screen: [f32; 2],
    window: [f32; 4],
    window_width: f32,
    centred: bool,
) -> Option<Layout> {
    let [width, height] = screen;
    let window = if window_width > window[2] {
        let x = ((width - window_width) / 2.0).floor();
        [x, window[1], window_width, window[3]]
    } else {
        window
    };
    let left = window[0] + window[2] + CLASSIC_PAD;
    let right = width - CLASSIC_PAD;
    let columns = (((right - left) / SLOT).floor().max(0.0) as usize).min(COLUMNS.0);
    if columns < COLUMNS.1 {
        return None;
    }
    let grid_width = columns as f32 * SLOT;
    // JEI keeps the list 80 pixels wide at the least, the grid centred in it.
    let list_width = grid_width.max(4.0 * CLASSIC_BUTTON);
    let list_x = right - list_width;
    let x = list_x + ((list_width - grid_width) / 2.0).floor();
    let centred = centred && window[1] + window[3] + CLASSIC_BUTTON < height;
    let foot = height
        - CLASSIC_BUTTON
        - if centred {
            CLASSIC_PAD
        } else {
            2.0 * CLASSIC_PAD
        };
    let (search_x, search_width) = if centred {
        (window[0], window[2])
    } else {
        (list_x, list_width)
    };
    let search = [
        search_x,
        foot,
        search_width - CLASSIC_BUTTON + 1.0,
        CLASSIC_BUTTON,
    ];
    let settings = [
        search[0] + search[2] - 1.0,
        foot,
        CLASSIC_BUTTON,
        CLASSIC_BUTTON,
    ];
    let top = CLASSIC_PAD + CLASSIC_BUTTON;
    let bottom = if centred {
        height - CLASSIC_PAD
    } else {
        foot - CLASSIC_PAD
    };
    let rows = ((bottom - top) / SLOT).floor().max(0.0) as usize;
    let y = top + ((bottom - top - rows as f32 * SLOT) / 2.0).floor();
    (rows > 0).then_some(Layout {
        // The grid can start left of `left` when it is centred in the
        // minimum-width page row, so the area reaches back to cover it.
        area: [left.min(x), 0.0, right - left.min(x), height],
        grid: (x, y),
        columns,
        rows,
        arrows_y: CLASSIC_PAD,
        search,
        settings: Some(settings),
        classic: true,
        nav: (list_x, list_width),
    })
}

/// JEI's tint of a button's icon (`GuiIconButton`): light grey, darker on a
/// button that is off, pale yellow under the pointer.
fn icon_tint(on: bool, hovered: bool) -> u32 {
    match (on, hovered) {
        (false, _) => rgba(0xA0, 0xA0, 0xA0, 0xFF),
        (true, true) => rgba(0xFF, 0xFF, 0xA0, 0xFF),
        (true, false) => rgba(0xE0, 0xE0, 0xE0, 0xFF),
    }
}

pub(super) fn inside(p: (f32, f32), [x, y, w, h]: [f32; 4]) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}

impl Game<'_> {
    /// How mod `mod_id`'s panel draws its page buttons and search box from
    /// now on (`PanelStyle`; the recipe book's until it says). Kept for
    /// the session, whether or not its panel is showing.
    pub(in crate::game) fn panel_style(&mut self, mod_id: &str, style: PanelStyle) {
        self.panel.set_style(mod_id, style);
    }
    fn browser_batch(&self) -> Option<HudBatch> {
        let r = self.renderer.as_ref()?;
        Some(HudBatch::new(
            [r.config.width, r.config.height],
            self.settings.gui_scale_inputs(),
        ))
    }
    /// The mod showing a panel, with its items made into stacks once per
    /// change; none while no mod shows one or no container screen is up.
    fn browser_sync(&mut self) -> Option<Owner> {
        if !matches!(self.screen, Screen::Inventory | Screen::Creative) || self.title.is_some() {
            return None;
        }
        let disabled = &self.settings.mods.disabled;
        let Some(shown) = self.panel.shown(|id| !disabled.contains(id)) else {
            self.panel.of = None;
            self.panel.stacks.clear();
            return None;
        };
        let of = (shown.owner.clone(), shown.version);
        if self.screen == Screen::Creative && of.0.native_id() != "recipes" {
            return None;
        }
        if self.panel.of.as_ref() != Some(&of) {
            let items = world_items!(self);
            let stacks = shown
                .items
                .iter()
                .map(|s| browser_stack(items, s))
                .collect();
            let shown = shown.recipes.clone();
            self.panel.stacks = stacks;
            // The same title again (the mod updating its buttons) keeps the page.
            let old = self.panel.recipes.take();
            self.panel.recipes = shown
                .as_ref()
                .map(|shown| super::native_panel_recipes::Pages::new(items, shown, old));
            self.panel.of = Some(of.clone());
        }
        Some(of.0)
    }
    /// What the panel shows now.
    fn panel_shown(&self) -> Option<&Shown> {
        let disabled = &self.settings.mods.disabled;
        self.panel.shown(|id| !disabled.contains(id))
    }
    /// The player did `event` on `owner`'s panel.
    pub(super) fn panel_event(&mut self, owner: &Owner, event: BrowserEvent) {
        let Owner::Native(id) = owner;
        self.native_panel_event(id, &event);
    }
    /// The panel's parts: right of the window, from the top of the screen to
    /// the search box at its foot. None when the screen has no room for it.
    fn browser_layout(&self, batch: &HudBatch) -> Option<Layout> {
        let image = self.container_image(batch);
        let recipes = self
            .panel
            .of
            .as_ref()
            .is_some_and(|(owner, _)| owner.native_id() == "recipes");
        let window_right = if self.screen == Screen::Creative {
            Self::creative_origin(batch).0 + 198.0
        } else {
            (image.left + image.width) as f32
        };
        let classic = self
            .panel
            .of
            .as_ref()
            .and_then(|(owner, _)| self.panel.classic(owner));
        if let Some((centred, window_width)) = classic.filter(|_| !recipes) {
            let (left, top) = (image.left as f32, image.top as f32);
            let window = [left, top, window_right - left, image.height as f32];
            let screen = [batch.width, batch.height];
            return classic_layout(screen, window, f32::from(window_width), centred);
        }
        let left = window_right + MARGIN;
        let (width, height) = self.renderer.as_ref().filter(|_| recipes).map_or(
            (batch.width, batch.height),
            |renderer| {
                (
                    (renderer.config.width as f32 / batch.scale).floor(),
                    (renderer.config.height as f32 / batch.scale).floor(),
                )
            },
        );
        let right = width - if recipes { 6.0 } else { MARGIN };
        let columns = (((right - left) / SLOT).floor().max(0.0) as usize).min(if recipes {
            9
        } else {
            COLUMNS.0
        });
        if columns < COLUMNS.1 {
            return None;
        }
        let grid_width = columns as f32 * SLOT;
        let x = if recipes {
            right - grid_width
        } else {
            (left + (right - left - grid_width) / 2.0).floor()
        };
        let arrows_y = if recipes { 6.0 } else { MARGIN };
        let top = arrows_y + if recipes { 22.0 } else { 21.0 };
        let mut search = if recipes {
            [x, height - 26.0, grid_width, 20.0]
        } else {
            [x, batch.height - MARGIN - 16.0, grid_width, 16.0]
        };
        let settings = recipes.then(|| {
            search[2] -= RECIPES_SETTINGS_WIDTH;
            [
                search[0] + search[2],
                search[1],
                RECIPES_SETTINGS_WIDTH,
                search[3],
            ]
        });
        let rows = ((search[1] - 4.0 - top) / SLOT).floor().max(0.0) as usize;
        (rows > 0).then_some(Layout {
            area: if recipes {
                [x, 0.0, right - x, batch.height]
            } else {
                [left, 0.0, right - left, batch.height]
            },
            grid: (x, top),
            columns,
            rows: if recipes { rows.min(16) } else { rows },
            arrows_y,
            search,
            settings,
            classic: false,
            nav: (x, grid_width),
        })
    }
    fn browser_pages(&self, layout: &Layout) -> usize {
        self.panel.stacks.len().div_ceil(layout.per_page()).max(1)
    }
    /// The stack under the pointer on the panel, an item's or a recipe's,
    /// as the mod sent it and as a stack.
    fn browser_pointed_stack(&self, layout: &Layout) -> Option<(BrowserStack, ItemStack)> {
        if self.panel.recipes.is_some() {
            if let Some((_, sent, stack)) = self.recipe_tab_hovered(layout) {
                return Some((sent, stack));
            }
            let (_, _, sent, stack) = self.recipe_slot_hovered(layout)?;
            return Some((sent, stack));
        }
        let (index, stack) = self.browser_hovered(layout)?;
        let items = &self.panel_shown()?.items;
        Some((items.get(index)?.clone(), stack))
    }
    /// The panel's item under the pointer: its index and stack.
    fn browser_hovered(&self, layout: &Layout) -> Option<(usize, ItemStack)> {
        if self.panel.recipes.is_some() {
            return None;
        }
        let first = self.panel.page * layout.per_page();
        (0..layout.per_page()).find_map(|n| {
            let (x, y) = layout.slot(n);
            if !inside(self.cursor, [x, y, SLOT, SLOT]) {
                return None;
            }
            let stack = self.panel.stacks.get(first + n)?.clone()?;
            Some((first + n, stack))
        })
    }
    fn browser_search(&mut self, layout: &Layout) -> &mut EditBox {
        let search = self.panel.search.get_or_insert_with(|| {
            let mut edit = EditBox::new(0.0, 0.0, 0.0, 0.0);
            edit.max_length = 50;
            edit.focused = false;
            edit
        });
        [search.x, search.y, search.width, search.height] = layout.search;
        search
    }

    fn draw_item_browser_arrows(
        &self,
        batch: &mut HudBatch,
        rects: [[f32; 4]; 2],
        labels: &[String; 2],
        enabled: [bool; 2],
        square: bool,
        classic: bool,
    ) {
        let atlas = &self.gui.atlas;
        for (i, (rect @ [x, y, w, h], on)) in rects.into_iter().zip(enabled).enumerate() {
            let hovered = inside(self.cursor, rect);
            if square {
                let background = match (on, hovered) {
                    (false, _) => "horizon/jei/button_disabled_v2",
                    (true, false) => "horizon/jei/button_enabled_v2",
                    (true, true) => "horizon/jei/button_highlight_v2",
                };
                horizon_client::ui::nine_slice(batch, atlas, background, x, y, w, h, u32::MAX);
                let arrow = [
                    "horizon/jei/icons/arrow_previous_v2",
                    "horizon/jei/icons/arrow_next_v2",
                ][i];
                // The classic look tints its icons as JEI: grey, darker when
                // off, yellow under the pointer.
                let tint = if classic {
                    icon_tint(on, hovered)
                } else {
                    u32::MAX
                };
                batch.sprite_scaled(atlas, arrow, x + 5.0, y + 6.0, 9.0, 7.0, tint);
            } else if on {
                let name = ["page_backward", "page_forward"][i];
                let suffix = if hovered { "_highlighted" } else { "" };
                batch.sprite(
                    atlas,
                    &format!("gui/sprites/recipe_book/{name}{suffix}"),
                    x,
                    y,
                );
            } else {
                continue;
            }
            horizon_client::widgets::record(&labels[i], rect, on);
        }
    }

    /// The items' page arrows and grid.
    fn draw_browser_items(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let classic = layout.classic;
        let pages = self.browser_pages(layout);
        self.panel.page = self.panel.page.min(pages - 1);
        let page = self.panel.page;
        let cursor = self.cursor;
        let square = self
            .panel
            .of
            .as_ref()
            .is_some_and(|(owner, _)| self.panel.viewer_style(owner));
        if pages > 1 || (square && !self.panel.stacks.is_empty()) || classic {
            let labels = [
                self.translate_with_fallback("gui.recipebook.previous_page", "Previous Page"),
                self.translate_with_fallback("gui.recipebook.next_page", "Next Page"),
            ];
            // The classic buttons wrap around, so both work with two pages.
            let shown = if classic {
                [pages > 1; 2]
            } else {
                [page > 0, page + 1 < pages]
            };
            let atlas = &self.gui.atlas;
            if square && !classic {
                batch.fill(
                    atlas,
                    layout.grid.0,
                    layout.arrows_y,
                    layout.width(),
                    20.0,
                    rgba(10, 14, 20, 42),
                );
            }
            self.draw_item_browser_arrows(
                batch,
                layout.browser_arrow_rects(square),
                &labels,
                shown,
                square,
                classic,
            );
            let text = format!("{}/{pages}", page + 1);
            let w = HudBatch::text_width(atlas, &text);
            let x = (layout.nav.0 + (layout.nav.1 - w) / 2.0).floor();
            batch.text(
                atlas,
                &text,
                x,
                layout.arrows_y + if square { 6.0 } else { 5.0 },
                u32::MAX,
                true,
            );
        }
        let first = page * layout.per_page();
        for n in 0..layout.per_page() {
            let Some(stack) = self.panel.stacks.get(first + n) else {
                break;
            };
            let Some(stack) = stack.clone() else {
                continue;
            };
            let (x, y) = layout.slot(n);
            self.draw_item(batch, &stack, x + 1.0, y + 1.0);
            let rect = [x + 1.0, y + 1.0, 16.0, 16.0];
            if inside(cursor, [x, y, SLOT, SLOT]) {
                // The classic look lights the whole slot, as JEI 1.12.
                let lit = if classic { [x, y, SLOT, SLOT] } else { rect };
                batch.fill(
                    &self.gui.atlas,
                    lit[0],
                    lit[1],
                    lit[2],
                    lit[3],
                    rgba(255, 255, 255, 128),
                );
            }
            let name = self.pack.names.display_item(&stack.item);
            horizon_client::widgets::record_as(
                &name,
                rect,
                true,
                horizon_client::widgets::Kind::Button,
            );
        }
    }
    /// Draw the browser's contents once, leaving its tooltip for the screen's
    /// overlay phase. The carried stack belongs to the enclosing screen.
    pub(super) fn draw_mod_browser(&mut self, batch: &mut HudBatch) -> Option<BrowserTooltip> {
        self.browser_sync()?;
        let layout = self.browser_layout(batch)?;
        let cursor = self.cursor;
        if self.panel.recipes.is_some() {
            self.draw_browser_recipes(batch, &layout);
        } else {
            self.draw_browser_items(batch, &layout);
        }
        let hint = self.translate_with_fallback("gui.recipebook.search_hint", "Search...");
        let skin = self
            .panel
            .of
            .as_ref()
            .is_some_and(|(owner, _)| self.panel.viewer_style(owner))
            .then_some("horizon/jei/search_background_v2");
        let nothing_found = self.panel.stacks.is_empty();
        let search = self.browser_search(&layout);
        // The hint shows in an empty box without focus, as the recipe book's.
        search.suggestion = (search.value.is_empty() && !search.focused).then_some(hint);
        // Reset every frame, including when leaving the classic style.
        search.text_color = if layout.classic {
            if nothing_found && !search.value.is_empty() {
                rgba(255, 0, 0, 255)
            } else {
                rgba(255, 255, 255, 255)
            }
        } else {
            rgba(224, 224, 224, 255)
        };
        let mut search = search.clone();
        search.draw_with_skin(batch, &self.gui.atlas, None, skin);
        self.panel.search = Some(search);
        if let Some(rect @ [x, y, width, height]) = layout.settings {
            let hovered = inside(cursor, rect);
            let atlas = &self.gui.atlas;
            if layout.classic {
                // JEI's config button: its wrench on its own button.
                let sprite = if hovered {
                    "horizon/jei/button_highlight_v2"
                } else {
                    "horizon/jei/button_enabled_v2"
                };
                horizon_client::ui::nine_slice(batch, atlas, sprite, x, y, width, height, u32::MAX);
                let icon = "horizon/jei/icons/config_button";
                batch.sprite_scaled(atlas, icon, x + 2.0, y + 2.0, 16.0, 16.0, u32::MAX);
            } else {
                let sprite = if hovered {
                    "gui/sprites/widget/button_highlighted"
                } else {
                    "gui/sprites/widget/button"
                };
                horizon_client::ui::nine_slice(batch, atlas, sprite, x, y, width, height, u32::MAX);
                batch.sprite_scaled(
                    atlas,
                    "horizon/recipes/settings",
                    x + 4.0,
                    y + 2.0,
                    12.0,
                    12.0,
                    u32::MAX,
                );
            }
            let label = horizon_client::lang::get("options.title");
            horizon_client::widgets::record(&label, rect, true);
            if hovered && self.carried.is_none() {
                return Some(BrowserTooltip::Text(label));
            }
        }
        if self.carried.is_some() {
            None
        } else if let Some(stack) = self.browser_pointed_stack(&layout) {
            Some(BrowserTooltip::Stack(stack.1))
        } else {
            self.recipe_fluid_hovered(&layout)
                .map(BrowserTooltip::Fluid)
        }
    }

    pub(super) fn draw_mod_browser_tooltip(
        &mut self,
        batch: &mut HudBatch,
        tooltip: BrowserTooltip,
    ) {
        match tooltip {
            BrowserTooltip::Text(text) => {
                self.draw_tooltip_lines(batch, &[(text, u32::MAX)], self.cursor.0, self.cursor.1);
            }
            BrowserTooltip::Stack(stack) => {
                self.draw_stack_tooltip(batch, &stack, self.cursor.0, self.cursor.1);
            }
            BrowserTooltip::Fluid(fluid) => {
                self.draw_fluid_tooltip(batch, &fluid, self.cursor.0, self.cursor.1);
            }
        }
    }

    /// A mouse press on a container screen: true when the panel took it
    /// (its search box, page arrows, items or empty space; a click there
    /// never throws the carried stack).
    pub(super) fn mod_browser_press(&mut self, right: bool) -> bool {
        let Some(owner) = self.browser_sync() else {
            return false;
        };
        let Some(batch) = self.browser_batch() else {
            return false;
        };
        let Some(layout) = self.browser_layout(&batch) else {
            return false;
        };
        let cursor = self.cursor;
        if layout.settings.is_some_and(|rect| inside(cursor, rect)) {
            if !right {
                self.list_native_mods();
                if let Some(index) = self
                    .settings
                    .mods
                    .installed
                    .iter()
                    .position(|entry| entry.id == owner.native_id() && entry.native.is_some())
                {
                    self.mod_browser_closed();
                    self.panel.recipes_settings_return = Some(self.screen);
                    self.open_options_at(false, horizon_client::options_ui::Page::NativeMod(index));
                }
            }
            return true;
        }
        let shift = self.has_shift_down();
        let atlas = &self.gui.atlas;
        let search = self.panel.search.get_or_insert_with(|| {
            let mut edit = EditBox::new(0.0, 0.0, 0.0, 0.0);
            edit.max_length = 50;
            edit
        });
        [search.x, search.y, search.width, search.height] = layout.search;
        if inside(cursor, layout.search) {
            search.focused = true;
            search.click(atlas, cursor.0, shift, false);
            search.reset_blink();
            return true;
        }
        search.focused = false;
        if self.panel.recipes.is_some() {
            return self.browser_recipes_press(&layout, &owner, right)
                || inside(cursor, layout.area);
        }
        let pages = self.browser_pages(&layout);
        let page = self.panel.page;
        let [back, forward] = layout.browser_arrow_rects(self.panel.viewer_style(&owner));
        let step = if pages > 1 && page > 0 && inside(cursor, back) {
            Some(page - 1)
        } else if pages > 1 && page + 1 < pages && inside(cursor, forward) {
            Some(page + 1)
        } else if layout.classic && pages > 1 && inside(cursor, back) {
            // The classic buttons wrap around, as JEI's.
            Some(pages - 1)
        } else if layout.classic && pages > 1 && inside(cursor, forward) {
            Some(0)
        } else {
            None
        };
        if let Some(to) = step {
            self.panel.page = to;
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            return true;
        }
        if let Some((index, _)) = self.browser_hovered(&layout) {
            let event = BrowserEvent::Clicked {
                index: index as u32,
                right,
            };
            self.panel_event(&owner, event);
            return true;
        }
        inside(cursor, layout.area)
    }

    /// Whether keys go to the panel's search box.
    pub(super) fn mod_browser_typing(&self) -> bool {
        matches!(self.screen, Screen::Inventory | Screen::Creative)
            && self.panel.of.is_some()
            && self.panel.search.as_ref().is_some_and(|s| s.focused)
    }
    /// A key press while the search box has focus.
    pub(super) fn mod_browser_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::PhysicalKey;
        let shortcut = self.edit_shortcut(event);
        let ctrl = self.has_control_down();
        let shift = self.has_shift_down();
        let types_text = self.types_text();
        let Some(search) = self.panel.search.as_mut() else {
            return;
        };
        if let Some(shortcut) = shortcut {
            search.shortcut(shortcut, &mut self.clipboard);
        } else {
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
        }
        search.reset_blink();
        self.browser_searched();
    }
    /// Text typed (or one backspace) into the search box (`horizon-ctl type`).
    pub(super) fn mod_browser_text(&mut self, text: Option<&str>, backspace: bool) {
        let Some(search) = self.panel.search.as_mut() else {
            return;
        };
        if backspace {
            search.delete_chars(-1);
        } else if let Some(text) = text {
            search.insert_text(text);
        }
        self.browser_searched();
    }
    /// The mod hears the search text when it changed.
    fn browser_searched(&mut self) {
        let (Some((owner, _)), Some(search)) = (&self.panel.of, &self.panel.search) else {
            return;
        };
        if search.value != self.panel.sent {
            let (owner, value) = (owner.clone(), search.value.clone());
            self.panel.sent.clone_from(&value);
            self.panel.searching = true;
            self.panel.page = 0;
            self.panel_event(&owner, BrowserEvent::Search(value));
            self.panel.searching = false;
        }
    }
    /// The container screen closed: the search box lets go of the keys.
    pub(super) fn mod_browser_closed(&mut self) {
        self.panel.transfer_warning = None;
        if let Some(search) = self.panel.search.as_mut() {
            search.focused = false;
        }
    }
    pub(super) fn mod_browser_back(&mut self) -> bool {
        let Some(owner) = self.browser_sync() else {
            return false;
        };
        if owner.native_id() != "recipes" || self.panel.recipes.is_none() {
            return false;
        }
        self.mod_browser_closed();
        self.panel_event(&owner, BrowserEvent::Back);
        true
    }
    pub(super) fn close_transferred_recipe(&mut self) {
        if let Some(owner) = self.browser_sync()
            && owner.native_id() == "recipes"
            && self.panel.recipes.is_some()
        {
            self.mod_browser_closed();
            self.panel_event(&owner, BrowserEvent::Back);
        }
    }
    /// The item under the pointer on a container screen, for the mods' keys
    /// (the `Key` event): the panel's, else a slot's.
    pub(super) fn mod_browser_pointed(&mut self) -> Option<BrowserStack> {
        let batch = self.browser_batch()?;
        let stack = match self
            .browser_sync()
            .and_then(|_| self.browser_layout(&batch))
        {
            // The panel's stacks as the mod sent them.
            Some(layout) if inside(self.cursor, layout.area) => {
                return Some(self.browser_pointed_stack(&layout)?.0);
            }
            _ => {
                let slot = self.slot_at(&batch, self.cursor.0, self.cursor.1)?;
                self.slot_view(slot)?
            }
        };
        Some(BrowserStack {
            item: mod_api::__private::item_key(&stack.item)?,
            count: stack.count,
            components: super::native_recipes::components(&stack),
        })
    }
    /// A key `key` (`key.keyboard.r`) went down: over an item on a container
    /// screen, a native mod's panel hears it as a `Key` event.
    pub(super) fn native_panel_key(&mut self, key: &str) {
        if !matches!(self.screen, Screen::Inventory | Screen::Creative)
            || self.title.is_some()
            || self.mod_browser_typing()
        {
            return;
        }
        let Some(owner) = self.browser_sync() else {
            return;
        };
        if let Some(stack) = self.mod_browser_pointed() {
            let key = key.to_owned();
            self.panel_event(&owner, BrowserEvent::Key { key, stack });
        }
    }
}

/// A mod's item as a stack: an item the game has, its components read as
/// `/give` reads them (ones it can't read are left out).
pub(super) fn browser_stack(
    items: &horizon_client::items::Items,
    s: &BrowserStack,
) -> Option<ItemStack> {
    let item = mod_api::__private::item_key_id(s.item)?;
    items.get(item)?;
    let (damage, extra) = if s.components.is_empty() {
        (0, None)
    } else {
        horizon_client::item_data::command_components(&s.components).map_or((0, None), |tag| {
            horizon_client::item_data::decode_components(Some(&tag))
        })
    };
    Some(ItemStack {
        item: item.into(),
        count: s.count.clamp(1, 99),
        damage,
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::{BrowserStack, Owner, Panel, RecipePage};
    use crate::game::native_hooks::{NativeRecipeLayout, NativeRecipeResult, NativeRecipeView};
    use mod_api::FluidStack;

    fn keys() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        mod_api::__private::resolve_items([
            "minecraft:oak_log",
            "minecraft:oak_planks",
            "minecraft:stone",
            "minecraft:dirt",
            "minecraft:crafting_table",
            "recipe_pilot:pressed_fragment",
        ])
        .unwrap();
        mod_api::__private::resolve_fluids(
            &["minecraft:water", "minecraft:lava"],
            &horizon_world::fluid::FluidKind::ALL.map(|kind| kind.name()),
        )
        .unwrap();
    }
    fn stack(item: &str) -> BrowserStack {
        BrowserStack {
            item: mod_api::__private::item_key(item).unwrap(),
            count: 1,
            components: String::new(),
        }
    }
    fn fluid(id: &str, amount: u32) -> FluidStack {
        FluidStack::new(mod_api::__private::fluid_key(id).unwrap(), amount)
    }
    fn view() -> NativeRecipeView {
        NativeRecipeView {
            layout: NativeRecipeLayout::Grid,
            width: 5,
            height: 1,
            slots: vec![vec![stack("minecraft:oak_log")]; 30],
            results: (0..6)
                .map(|i| NativeRecipeResult {
                    stacks: vec![stack("minecraft:oak_planks")],
                    chance: (i == 1).then_some(250.0),
                })
                .collect(),
            fluid_inputs: vec![fluid("minecraft:water", 1000); 10],
            fluid_outputs: vec![fluid("minecraft:lava", 250)],
            note: String::new(),
            action: Some(true),
        }
    }

    #[test]
    fn a_mod_chooses_the_viewers_page_buttons_and_keeps_them() {
        keys();
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        assert!(
            !panel.viewer_style(&jei),
            "the recipe book's arrows by default"
        );
        assert!(panel.viewer_style(&Owner::Native("recipes".into())));
        panel.set_style("jei", super::PanelStyle::Viewer);
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        assert!(panel.viewer_style(&jei));
        // Another mod's choice is its own.
        assert!(!panel.viewer_style(&Owner::Native("other".into())));
        // It outlasts the panel going away and coming back.
        panel.show_items("jei", Vec::new());
        panel.show_items("jei", vec![stack("minecraft:stone")]);
        assert!(panel.viewer_style(&jei));
        panel.set_style("jei", super::PanelStyle::RecipeBook);
        assert!(!panel.viewer_style(&jei));
    }

    /// JEI 1.12's overlay: the items flush right under a 20 pixel page row,
    /// the search box at the foot of the screen with the settings button on
    /// its right, under the window when asked and there is room.
    #[test]
    fn the_classic_look_lays_the_items_out_as_jei_did() {
        let mut panel = Panel::default();
        let jei = Owner::Native("jei".into());
        let style = super::PanelStyle::Classic {
            centred_search: true,
            window_width: 198,
        };
        panel.set_style("jei", style);
        assert!(
            panel.viewer_style(&jei),
            "square buttons and the search skin"
        );
        assert_eq!(panel.classic(&jei), Some((true, 198)));
        assert_eq!(panel.classic(&Owner::Native("recipes".into())), None);

        // A 570x473 GUI with the inventory's 176x166 window at (197, 153).
        let screen = [570.0, 473.0];
        let window = [197.0, 153.0, 176.0, 166.0];
        let centred = super::classic_layout(screen, window, 0.0, true).unwrap();
        // 375 to 568 holds ten columns, flush with the right edge.
        assert_eq!((centred.columns, centred.grid.0), (10, 388.0));
        // 449 pixels under the page row hold 24 rows, centred.
        assert_eq!(
            (centred.rows, centred.grid.1, centred.arrows_y),
            (24, 30.0, 2.0)
        );
        assert_eq!(centred.search, [197.0, 451.0, 157.0, 20.0]);
        assert_eq!(centred.settings, Some([353.0, 451.0, 20.0, 20.0]));
        assert_eq!(
            centred.browser_arrow_rects(true)[1],
            [548.0, 2.0, 20.0, 20.0]
        );

        // A recipe screen 198 wide over the window: the items step right of
        // it, and the search box goes under it.
        let wide = super::classic_layout(screen, window, 198.0, true).unwrap();
        assert_eq!(wide.area[0], 386.0);
        assert_eq!(wide.search, [186.0, 451.0, 179.0, 20.0]);

        // Under the items, with the rows above it.
        let beside = super::classic_layout(screen, window, 0.0, false).unwrap();
        assert_eq!(beside.search, [388.0, 449.0, 161.0, 20.0]);
        assert_eq!(beside.settings, Some([548.0, 449.0, 20.0, 20.0]));
        assert_eq!((beside.rows, beside.grid.1), (23, 27.0));

        // A window reaching the foot keeps the search box under the items.
        let tall = super::classic_layout(screen, [197.0, 153.0, 176.0, 310.0], 0.0, true).unwrap();
        assert_eq!(tall.search[0], 388.0);
        // Three columns: the page row keeps JEI's 80 pixels, the grid centred.
        let few = super::classic_layout([300.0, 473.0], [62.0, 153.0, 176.0, 166.0], 0.0, false);
        let few = few.unwrap();
        assert_eq!(
            (few.columns, few.grid.0, few.nav),
            (3, 231.0, (218.0, 80.0))
        );
        assert_eq!(few.browser_arrow_rects(true)[1], [278.0, 2.0, 20.0, 20.0]);
        // The area covers the first column's left pixels (x 231 to 239).
        assert!(few.area[0] <= few.grid.0);
        assert!(super::inside(
            (few.grid.0 + 1.0, few.grid.1 + 1.0),
            few.area
        ));
        assert_eq!(few.search, [218.0, 449.0, 61.0, 20.0]);
        // No room for two columns: no panel.
        let narrow = [37.0, 17.0, 176.0, 166.0];
        assert!(super::classic_layout([250.0, 200.0], narrow, 0.0, true).is_none());
    }

    /// A native mod's items and recipe page are what the panel holds, and
    /// its owner (who hears the panel's events) is that mod.
    #[test]
    fn native_owner_holds_the_panel() {
        keys();
        let mut panel = Panel::default();
        let items = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        panel.show_items("recipes", items.clone());
        let shown = panel.shown(|_| true).expect("a panel");
        assert_eq!(shown.owner, Owner::Native("recipes".into()));
        assert_eq!(shown.items, items);
        assert_eq!(shown.recipes, None);
        let first = shown.version;

        let tabs = vec![stack("minecraft:crafting_table")];
        let page = RecipePage::new("Oak Planks", tabs.clone(), 0, vec![view()]);
        panel.show_recipes("recipes", page);
        let shown = panel.shown(|_| true).expect("a panel");
        assert_ne!(shown.version, first, "the game converts each change");
        assert_eq!(shown.items, items);
        let page = shown.recipes.as_ref().expect("a recipe page");
        assert_eq!(
            (page.title.as_str(), &page.tabs, page.tab),
            ("Oak Planks", &tabs, 0)
        );
        // Cut as `set-recipes` cuts it: a 3 x 3 grid at most, 27 slots.
        let r = &page.recipes[0];
        assert_eq!((r.width, r.height, r.slots.len()), (3, 1, 27));
        // At most four results, a chance kept to 0..=100, at most eight fluids.
        let results = r.results.iter().map(|x| (x.stacks.len(), x.chance));
        assert_eq!(
            results.collect::<Vec<_>>(),
            [(1, None), (1, Some(100.0)), (1, None), (1, None)]
        );
        assert_eq!((r.fluid_inputs.len(), r.fluid_outputs.len()), (8, 1));

        // Another mod clearing its own (empty) panel leaves this one.
        panel.show_items("other", Vec::new());
        let owner = panel.shown(|_| true).map(|s| s.owner.clone());
        assert_eq!(owner, Some(Owner::Native("recipes".into())));
        // A mod the player turned off shows nothing.
        assert!(panel.shown(|id| id != "recipes").is_none());

        // No recipes: the items again; then no items: no panel.
        panel.show_recipes("recipes", RecipePage::new("", Vec::new(), 0, Vec::new()));
        let shown = panel.shown(|_| true).expect("a panel");
        assert_eq!((shown.items.len(), shown.recipes.is_none()), (2, true));
        panel.show_items("recipes", Vec::new());
        assert!(panel.shown(|_| true).is_none());
    }

    #[test]
    fn empty_search_can_be_edited_and_an_explicit_clear_still_hides_the_panel() {
        keys();
        let mut panel = Panel::default();
        let items = vec![stack("recipe_pilot:pressed_fragment")];
        panel.show_items("recipes", items.clone());
        panel.sent = "no_such_item".into();
        panel.searching = true;
        panel.show_items("recipes", Vec::new());
        panel.searching = false;
        let empty = panel
            .shown(|_| true)
            .expect("keep the search field's owner");
        assert_eq!(empty.owner, Owner::Native("recipes".into()));
        assert!(empty.items.is_empty());
        assert!(panel.shown(|id| id != "recipes").is_none());

        panel.show_recipes(
            "recipes",
            RecipePage::new("Oak Planks", Vec::new(), 0, vec![view()]),
        );
        panel.show_recipes("recipes", None);
        let empty = panel
            .shown(|_| true)
            .expect("return to the empty search after a recipe");
        assert!(empty.items.is_empty() && empty.recipes.is_none());
        assert_eq!(panel.sent, "no_such_item");

        panel.show_items("other", Vec::new());
        panel.sent = "pressed_fragment".into();
        panel.show_items("recipes", items.clone());
        assert_eq!(panel.shown(|_| true).unwrap().items, items);

        // Outside Search, clearing the page then items still closes the viewer,
        // even though the last query was non-empty.
        panel.show_recipes("recipes", None);
        panel.show_items("recipes", Vec::new());
        assert!(panel.shown(|_| true).is_none());
        panel.show_items("other", Vec::new());
        assert!(
            panel.shown(|_| true).is_none(),
            "an empty list cannot take ownership"
        );
    }
}
