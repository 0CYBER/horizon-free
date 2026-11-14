//! A mod's recipes shown in its item panel's place (`native_panel_recipes`,
//! or `browser.set-recipes` of a WebAssembly mod, docs/modding/src/how-to/content-mods.md), laid out by the game with its own slots, arrows and
//! buttons, paged to the panel's height, under a title and a back button.
//! The mod hears clicks on their slots, their action buttons, the back
//! button and page turns.
use super::{
    Game,
    native_hooks::{
        NativeRecipeLayout as BrowserLayout, NativeRecipeResult as BrowserResult,
        NativeRecipeView as BrowserRecipe, PanelEvent as BrowserEvent,
    },
    native_panel::{Layout, Owner, browser_stack, inside},
};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
};
use mod_api::Stack as BrowserStack;
use std::time::Instant;

const SLOT: f32 = 18.0;
/// The title row's height, and the gap under each recipe.
const TITLE: f32 = 18.0;
/// A tab's step: an 18 px slot and a gap.
const TAB: f32 = 20.0;
const GAP: f32 = 6.0;
/// A fluid well's step along a row (its slot, and room for "16000 mB" under
/// it) and the row's height under the slot.
const FLUID_WELL: f32 = 44.0;
const FLUID_TEXT: f32 = 10.0;
/// The room a line of chances takes under the results, and the step between
/// results when one has a chance and the panel has room: the widest chance
/// text ("99.5%") is 26 px.
const CHANCE_TEXT: f32 = 9.0;
const CHANCE_STEP: f32 = 26.0;

/// Recipes a page may show, slots in one, stacks a slot cycles, and tabs.
const RECIPES: usize = 256;
const RECIPE_SLOTS: usize = 27;
const RESULTS: usize = 4;
const FLUIDS: usize = 8;
const SLOT_STACKS: usize = 64;
const TABS: usize = 16;

/// What the panel shows instead of its items: a title, tabs (`tab` the
/// selected one) and the recipes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RecipePage {
    pub(super) title: String,
    pub(super) tabs: Vec<BrowserStack>,
    pub(super) tab: u32,
    pub(super) recipes: Vec<BrowserRecipe>,
}
impl RecipePage {
    /// `native_panel_recipes`' page, cut as `set-recipes` cuts one; none
    /// without recipes (the items again).
    pub(super) fn new(
        title: &str,
        tabs: Vec<BrowserStack>,
        tab: u32,
        recipes: Vec<BrowserRecipe>,
    ) -> Option<Self> {
        let cycle = |mut list: Vec<BrowserStack>| {
            list.truncate(SLOT_STACKS);
            list
        };
        let recipes: Vec<BrowserRecipe> = recipes
            .into_iter()
            .take(RECIPES)
            .map(|r| BrowserRecipe {
                width: r.width.clamp(1, 3),
                height: r.height.clamp(1, 3),
                slots: r.slots.into_iter().take(RECIPE_SLOTS).map(cycle).collect(),
                results: r
                    .results
                    .into_iter()
                    .take(RESULTS)
                    .map(|BrowserResult { stacks, chance }| BrowserResult {
                        stacks: cycle(stacks),
                        chance: chance
                            .filter(|c| c.is_finite())
                            .map(|c| c.clamp(0.0, 100.0)),
                    })
                    .collect(),
                fluid_inputs: r.fluid_inputs.into_iter().take(FLUIDS).collect(),
                fluid_outputs: r.fluid_outputs.into_iter().take(FLUIDS).collect(),
                ..r
            })
            .collect();
        (!recipes.is_empty()).then(|| Self {
            title: title.to_owned(),
            tabs: tabs.into_iter().take(TABS).collect(),
            tab,
            recipes,
        })
    }
}

/// One recipe as the game draws it.
pub(super) struct Shown {
    layout: BrowserLayout,
    width: usize,
    height: usize,
    /// Each slot's stacks (the results last), as the mod sent them and as
    /// stacks; a stack the game doesn't have is left out of the cycle.
    slots: Vec<Vec<(BrowserStack, ItemStack)>>,
    /// Each result's chance as it is written under it ("25%"), in order; its
    /// length is the number of result slots at the end of `slots`.
    chances: Vec<Option<String>>,
    fluid_inputs: Vec<Well>,
    fluid_outputs: Vec<Well>,
    note: String,
    action: Option<bool>,
}

/// The recipes a panel shows, and its page.
pub(super) struct Pages {
    title: String,
    /// The tabs' icons and the selected one.
    tabs: Vec<(BrowserStack, Option<ItemStack>)>,
    tab: usize,
    recipes: Vec<Shown>,
    page: usize,
    since: Instant,
}

impl Pages {
    pub(super) fn new(
        items: &horizon_client::items::Items,
        shown: &RecipePage,
        old: Option<Pages>,
    ) -> Self {
        let (title, list, tab) = (&shown.title, &shown.recipes, shown.tab as usize);
        let kept = old.filter(|o| o.title == *title && o.tab == tab);
        let cycle = |stacks: &[BrowserStack]| -> Vec<(BrowserStack, ItemStack)> {
            stacks
                .iter()
                .filter_map(|s| Some((s.clone(), browser_stack(items, s)?)))
                .collect()
        };
        let recipes = list
            .iter()
            .map(|r| {
                let mut slots: Vec<_> = r.slots.iter().map(|s| cycle(s)).collect();
                slots.extend(r.results.iter().map(|res| cycle(&res.stacks)));
                Shown {
                    layout: r.layout,
                    width: r.width as usize,
                    height: r.height as usize,
                    slots,
                    chances: r
                        .results
                        .iter()
                        .map(|res| res.chance.map(chance_text))
                        .collect(),
                    fluid_inputs: r.fluid_inputs.iter().map(Well::new).collect(),
                    fluid_outputs: r.fluid_outputs.iter().map(Well::new).collect(),
                    note: r.note.clone(),
                    action: r.action,
                }
            })
            .collect();
        Self {
            title: title.clone(),
            tabs: shown
                .tabs
                .iter()
                .map(|t| (t.clone(), browser_stack(items, t)))
                .collect(),
            tab,
            recipes,
            page: kept.as_ref().map_or(0, |o| o.page),
            since: kept.map_or_else(Instant::now, |o| o.since),
        }
    }
}

impl Shown {
    /// The inputs' block: its size and each input slot's place in it.
    fn inputs(&self) -> ((f32, f32), Vec<(f32, f32)>) {
        let row = |n: usize| (0..n).map(|i| (i as f32 * SLOT, 0.0)).collect();
        match self.layout {
            BrowserLayout::Grid => {
                // Slots past the grid continue in rows under it.
                let w = self.width.max(1);
                let h = self
                    .height
                    .max(1)
                    .max((self.slots.len() - self.chances.len()).div_ceil(w));
                let at = (0..w * h)
                    .map(|i| ((i % w) as f32 * SLOT, (i / w) as f32 * SLOT))
                    .collect();
                ((w as f32 * SLOT, h as f32 * SLOT), at)
            }
            // The input over the flame.
            BrowserLayout::Cooking => ((SLOT, 2.0 * SLOT), vec![(0.0, 0.0)]),
            BrowserLayout::Single => ((SLOT, SLOT), row(1)),
            BrowserLayout::Smithing => ((3.0 * SLOT, SLOT), row(3)),
            BrowserLayout::Brewing => ((2.0 * SLOT, SLOT), row(2)),
        }
    }
    /// The height of its slots, arrow and button, `room` wide; a chance is
    /// written under its result, so the results' row has room for it.
    fn block(&self, room: f32) -> f32 {
        let h = self.inputs().0.1;
        let (_, _, action) = self.places(room);
        let lines = self.result_step(room).1;
        let h = h.max(((h - SLOT) / 2.0).floor() + SLOT + lines * CHANCE_TEXT);
        match self.action {
            Some(_) => h.max(action.1 + 12.0),
            None => h,
        }
    }
    /// The step between results and the lines of chances under them, `room`
    /// wide: with a chance they step wide enough for its text when that fits,
    /// else slot to slot with every other chance a line lower, so two never
    /// touch.
    fn result_step(&self, room: f32) -> (f32, f32) {
        let n = self.chances.len();
        if !self.chances.iter().any(Option::is_some) {
            return (SLOT, 0.0);
        }
        let first = self.inputs().0.0 + 32.0;
        if first + n.saturating_sub(1) as f32 * CHANCE_STEP + SLOT <= room {
            (CHANCE_STEP, 1.0)
        } else {
            (SLOT, if n > 1 { 2.0 } else { 1.0 })
        }
    }
    /// The fluid wells' places from the recipe's corner, `room` wide: the
    /// inputs' rows under the slots, then the outputs' rows; and the height they take.
    fn fluid_places(&self, room: f32) -> (Vec<(f32, f32)>, Vec<(f32, f32)>, f32) {
        let per_row = ((room / FLUID_WELL).floor() as usize).max(1);
        let mut top = self.block(room) + 2.0;
        let mut rows = |n: usize| {
            let at: Vec<(f32, f32)> = (0..n)
                .map(|i| {
                    (
                        (i % per_row) as f32 * FLUID_WELL,
                        top + (i / per_row) as f32 * (SLOT + FLUID_TEXT),
                    )
                })
                .collect();
            top += n.div_ceil(per_row) as f32 * (SLOT + FLUID_TEXT);
            at
        };
        let inputs = rows(self.fluid_inputs.len());
        let outputs = rows(self.fluid_outputs.len());
        let used = if inputs.is_empty() && outputs.is_empty() {
            0.0
        } else {
            top - self.block(room)
        };
        (inputs, outputs, used)
    }
    fn height(&self, room: f32) -> f32 {
        let note = if self.note.is_empty() { 0.0 } else { 11.0 };
        self.block(room) + self.fluid_places(room).2 + note + GAP
    }
    /// Every slot's place from the recipe's corner (the results last, in a
    /// row), the arrow's and the action button's: beside the last result, or
    /// under it when the panel is `room` wide and that doesn't fit.
    fn places(&self, room: f32) -> (Vec<(f32, f32)>, (f32, f32), (f32, f32)) {
        let ((w, h), mut at) = self.inputs();
        // The mod's slots beyond the layout's are not shown.
        let inputs = self.slots.len() - self.chances.len();
        at.truncate(inputs);
        let arrow = (w + 4.0, ((h - 16.0) / 2.0).floor());
        let first = (w + 32.0, ((h - SLOT) / 2.0).floor());
        let (step, lines) = self.result_step(room);
        let result = (
            first.0 + self.chances.len().saturating_sub(1) as f32 * step,
            first.1,
        );
        let action = if result.0 + SLOT + 16.0 <= room {
            (result.0 + SLOT + 4.0, ((h - 12.0) / 2.0).floor())
        } else {
            // Under the result, past the lines its chance is written on.
            (result.0 + 3.0, result.1 + SLOT + lines * CHANCE_TEXT + 2.0)
        };
        while at.len() < inputs {
            at.push((f32::NAN, f32::NAN));
        }
        at.extend((0..self.chances.len()).map(|j| (first.0 + j as f32 * step, first.1)));
        (at, arrow, action)
    }
}

/// A fluid well as it is drawn: its look and its texts, made once when the
/// page is set.
#[derive(Clone, Debug)]
struct Well {
    /// The fluid the well shows, for its tooltip.
    stack: mod_api::FluidStack,
    /// The fluid's atlas sprite; none draws a flat square of the tint.
    sprite: Option<String>,
    tint: u32,
    /// "250 mB", under the well.
    amount: String,
    /// The fluid's name and amount, for the widget list.
    label: String,
}

impl Well {
    fn new(fluid: &mod_api::FluidStack) -> Self {
        let id = mod_api::__private::fluid_identity_name(fluid.fluid);
        let look = horizon_client::mod_content::fluid_look(&id);
        let tint = fluid_tint(&id, look.as_ref().and_then(|l| l.tint));
        let amount = format!("{} mB", fluid.amount);
        let name = if fluid.key().is_some() {
            horizon_client::fluid_tooltip::fluid_name(&id)
        } else {
            horizon_client::lang::get("horizon.fluid.unknown")
        };
        Self {
            stack: fluid.clone(),
            sprite: look.and_then(|l| l.sprite),
            tint,
            label: format!("{name} {amount}"),
            amount,
        }
    }
}

/// A fluid's colour: its look's tint, else vanilla water's and lava's, else grey.
pub(super) fn fluid_tint(fluid: &str, look: Option<u32>) -> u32 {
    look.unwrap_or(match fluid {
        "minecraft:water" => 0xFF3F76E4,
        "minecraft:lava" => 0xFFFF6A00,
        _ => 0xFF9A9A9A,
    })
}

/// A chance as the panel writes it: a whole percent as "25%", else "12.5%".
fn chance_text(chance: f32) -> String {
    if (chance - chance.round()).abs() < 0.05 {
        format!("{}%", chance.round() as u32)
    } else {
        format!("{chance:.1}%")
    }
}

/// The tabs' places, in rows under the title row.
fn tab_places(pages: &Pages, layout: &Layout) -> Vec<(f32, f32)> {
    let per_row = ((layout.width() / TAB).floor() as usize).max(1);
    (0..pages.tabs.len())
        .map(|i| {
            (
                layout.grid.0 + (i % per_row) as f32 * TAB,
                layout.grid.1 + TITLE + 3.0 + (i / per_row) as f32 * TAB,
            )
        })
        .collect()
}
/// Where the recipes start: under the title row and the tabs.
fn recipes_top(pages: &Pages, layout: &Layout) -> f32 {
    let tabs = tab_places(pages, layout);
    tabs.last()
        .map_or(layout.grid.1 + TITLE, |&(_, y)| y + TAB + 3.0)
}

impl Game<'_> {
    /// The first recipe of each page, filling the panel from the tabs to
    /// the search box.
    fn recipe_pages(pages: &Pages, layout: &Layout) -> Vec<usize> {
        let room = layout.search[1] - 4.0 - recipes_top(pages, layout);
        let mut starts = vec![0];
        let mut used = 0.0;
        for (i, r) in pages.recipes.iter().enumerate() {
            let h = r.height(layout.width());
            if used > 0.0 && used + h > room {
                starts.push(i);
                used = 0.0;
            }
            used += h;
        }
        starts
    }
    /// The recipes on the open page, each with its corner.
    fn recipes_shown(&self, layout: &Layout) -> Vec<(usize, (f32, f32))> {
        let Some(pages) = &self.panel.recipes else {
            return Vec::new();
        };
        let starts = Self::recipe_pages(pages, layout);
        let page = pages.page.min(starts.len() - 1);
        let end = starts.get(page + 1).copied().unwrap_or(pages.recipes.len());
        let mut y = recipes_top(pages, layout);
        (starts[page]..end)
            .map(|i| {
                let at = (layout.grid.0, y);
                y += pages.recipes[i].height(layout.width());
                (i, at)
            })
            .collect()
    }
    fn back_button(&self, layout: &Layout) -> ([f32; 4], String) {
        let label = self.translate_with_fallback("gui.back", "Back");
        let w = HudBatch::text_width(&self.gui.atlas, &label) + 8.0;
        ([layout.grid.0, layout.grid.1, w, 14.0], label)
    }
    /// What a slot of a shown recipe holds now: its stacks cycle once a second.
    fn recipe_slot_now(&self, recipe: usize, slot: usize) -> Option<&(BrowserStack, ItemStack)> {
        let pages = self.panel.recipes.as_ref()?;
        let stacks = pages.recipes.get(recipe)?.slots.get(slot)?;
        let n = pages.since.elapsed().as_secs() as usize;
        stacks.get(n % stacks.len().max(1))
    }
    /// The tab under the pointer: its index and icon.
    pub(super) fn recipe_tab_hovered(
        &self,
        layout: &Layout,
    ) -> Option<(usize, BrowserStack, ItemStack)> {
        let pages = self.panel.recipes.as_ref()?;
        let places = tab_places(pages, layout);
        places.iter().enumerate().find_map(|(i, &(x, y))| {
            if !inside(self.cursor, [x, y, SLOT, SLOT]) {
                return None;
            }
            let (sent, stack) = pages.tabs.get(i)?;
            Some((i, sent.clone(), stack.clone()?))
        })
    }
    /// The fluid in the well under the pointer.
    pub(super) fn recipe_fluid_hovered(&self, layout: &Layout) -> Option<mod_api::FluidStack> {
        let pages = self.panel.recipes.as_ref()?;
        self.recipes_shown(layout)
            .into_iter()
            .find_map(|(i, (x, y))| {
                let r = pages.recipes.get(i)?;
                let (wells_in, wells_out, _) = r.fluid_places(layout.width());
                [(wells_in, &r.fluid_inputs), (wells_out, &r.fluid_outputs)]
                    .into_iter()
                    .flat_map(|(places, wells)| places.into_iter().zip(wells))
                    .find(|&((sx, sy), _)| inside(self.cursor, [x + sx, y + sy, SLOT, SLOT]))
                    .map(|(_, well)| well.stack.clone())
            })
    }
    /// The recipe slot under the pointer: recipe, slot, and its stack now.
    pub(super) fn recipe_slot_hovered(
        &self,
        layout: &Layout,
    ) -> Option<(usize, usize, BrowserStack, ItemStack)> {
        let pages = self.panel.recipes.as_ref()?;
        self.recipes_shown(layout)
            .into_iter()
            .find_map(|(i, (x, y))| {
                let (at, _, _) = pages.recipes[i].places(layout.width());
                at.iter().enumerate().find_map(|(k, &(sx, sy))| {
                    if !inside(self.cursor, [x + sx, y + sy, SLOT, SLOT]) {
                        return None;
                    }
                    let (mod_stack, stack) = self.recipe_slot_now(i, k)?.clone();
                    Some((i, k, mod_stack, stack))
                })
            })
    }

    /// Draws the title row, the page's recipes and the page arrows.
    pub(super) fn draw_browser_recipes(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let Some(pages) = &self.panel.recipes else {
            return;
        };
        let cursor = self.cursor;
        let starts = Self::recipe_pages(pages, layout);
        let page = pages.page.min(starts.len() - 1);
        let (title, pages_n) = (pages.title.clone(), starts.len());
        let (back, label) = self.back_button(layout);
        let atlas = &self.gui.atlas;
        let hover = inside(cursor, back);
        horizon_client::widgets::declare_action(horizon_client::widgets::Action::Back, back, true);
        let sprite = if hover {
            "gui/sprites/widget/button_highlighted"
        } else {
            "gui/sprites/widget/button"
        };
        horizon_client::ui::nine_slice(
            batch,
            atlas,
            sprite,
            back[0],
            back[1],
            back[2],
            back[3],
            u32::MAX,
        );
        batch.text(atlas, &label, back[0] + 4.0, back[1] + 3.0, u32::MAX, true);
        horizon_client::widgets::record(&label, back, true);
        let room = layout.width() - back[2] - 4.0;
        // Cut to the panel with vanilla's ellipsis (CommonComponents.ELLIPSIS).
        let shown = if atlas.font.fit(&title, room) >= title.chars().count() {
            title
        } else {
            let fit = atlas.font.fit(&title, room - atlas.font.width("..."));
            let kept: String = title.chars().take(fit).collect();
            format!("{}...", kept.trim_end())
        };
        batch.text(
            atlas,
            &shown,
            back[0] + back[2] + 4.0,
            back[1] + 3.0,
            u32::MAX,
            true,
        );
        self.draw_recipe_tabs(batch, layout);
        if pages_n > 1 {
            let labels = [
                self.translate_with_fallback("gui.recipebook.previous_page", "Previous Page"),
                self.translate_with_fallback("gui.recipebook.next_page", "Next Page"),
            ];
            let visible = [page > 0, page + 1 < pages_n];
            let atlas = &self.gui.atlas;
            for (((x, y), name), (label, visible)) in layout
                .arrows()
                .into_iter()
                .zip(["page_backward", "page_forward"])
                .zip(labels.iter().zip(visible))
            {
                if !visible {
                    continue;
                }
                let suffix = if inside(cursor, [x, y, 12.0, 17.0]) {
                    "_highlighted"
                } else {
                    ""
                };
                batch.sprite(
                    atlas,
                    &format!("gui/sprites/recipe_book/{name}{suffix}"),
                    x,
                    y,
                );
                horizon_client::widgets::record(label, [x, y, 12.0, 17.0], true);
            }
            let text = format!("{}/{pages_n}", page + 1);
            let w = HudBatch::text_width(atlas, &text);
            let x = (layout.grid.0 + (layout.width() - w) / 2.0).floor();
            batch.text(atlas, &text, x, layout.arrows_y + 5.0, u32::MAX, true);
        }
        for (i, (x, y)) in self.recipes_shown(layout) {
            let Some(r) = self.panel.recipes.as_ref().map(|p| &p.recipes[i]) else {
                break;
            };
            let (at, arrow, action) = r.places(layout.width());
            let h = r.block(layout.width());
            let (cooking, button) = (r.layout == BrowserLayout::Cooking, r.action);
            let inputs = r.slots.len() - r.chances.len();
            let staggered = r.result_step(layout.width()).1 > 1.0;
            let (wells_in, wells_out, wells_h) = r.fluid_places(layout.width());
            let atlas = &self.gui.atlas;
            for &(sx, sy) in at.iter().filter(|p| p.0.is_finite()) {
                batch.sprite(atlas, "gui/sprites/container/slot", x + sx, y + sy);
            }
            if cooking {
                batch.sprite(
                    atlas,
                    "gui/sprites/container/furnace/lit_progress",
                    x + 2.0,
                    y + SLOT + 2.0,
                );
            }
            batch.sprite(
                atlas,
                "gui/sprites/container/furnace/burn_progress",
                x + arrow.0,
                y + arrow.1,
            );
            if self.screen != super::Screen::Creative
                && let Some(enabled) = button
            {
                let rect = [x + action.0, y + action.1, 12.0, 12.0];
                let sprite = match (enabled, inside(cursor, rect)) {
                    (false, _) => "gui/sprites/widget/button_disabled",
                    (true, true) => "gui/sprites/widget/button_highlighted",
                    (true, false) => "gui/sprites/widget/button",
                };
                horizon_client::ui::nine_slice(
                    batch,
                    atlas,
                    sprite,
                    rect[0],
                    rect[1],
                    12.0,
                    12.0,
                    u32::MAX,
                );
                let color = if enabled {
                    u32::MAX
                } else {
                    rgba(160, 160, 160, 255)
                };
                batch.text(atlas, "+", rect[0] + 3.0, rect[1] + 2.0, color, true);
                horizon_client::widgets::record(&format!("+ {i}"), rect, enabled);
            }
            if !r.note.is_empty() {
                batch.text(
                    atlas,
                    &r.note,
                    x,
                    y + h + wells_h + 2.0,
                    rgba(224, 224, 224, 255),
                    true,
                );
            }
            for (k, &(sx, sy)) in at.iter().enumerate().filter(|(_, p)| p.0.is_finite()) {
                let Some((_, stack)) = self.recipe_slot_now(i, k).cloned() else {
                    continue;
                };
                self.draw_item(batch, &stack, x + sx + 1.0, y + sy + 1.0);
                let chance = k.checked_sub(inputs).and_then(|j| r.chances[j].as_deref());
                if let Some(text) = chance {
                    let atlas = &self.gui.atlas;
                    let w = HudBatch::text_width(atlas, text);
                    let line = if staggered && (k - inputs) % 2 == 1 {
                        CHANCE_TEXT
                    } else {
                        0.0
                    };
                    let at = (
                        x + sx + ((SLOT - w) / 2.0).floor(),
                        y + sy + SLOT + 1.0 + line,
                    );
                    batch.text(atlas, text, at.0, at.1, rgba(255, 255, 85, 255), true);
                }
                if inside(cursor, [x + sx, y + sy, SLOT, SLOT]) {
                    batch.fill(
                        &self.gui.atlas,
                        x + sx + 1.0,
                        y + sy + 1.0,
                        16.0,
                        16.0,
                        rgba(255, 255, 255, 128),
                    );
                }
                let name = self.pack.names.display_item(&stack.item);
                horizon_client::widgets::record_as(
                    &format!("{name} {i}:{k}"),
                    [x + sx + 1.0, y + sy + 1.0, 16.0, 16.0],
                    true,
                    horizon_client::widgets::Kind::Button,
                );
            }
            for (places, wells) in [(wells_in, &r.fluid_inputs), (wells_out, &r.fluid_outputs)] {
                for (&(sx, sy), well) in places.iter().zip(wells) {
                    self.draw_fluid_well(batch, well, x + sx, y + sy);
                }
            }
        }
        if let Some((key, fallback)) = self.panel.transfer_warning {
            let warning = horizon_client::lang::get_or(key, fallback);
            let lines = [(
                horizon_client::text::Text::literal(warning.clone()).resolve_legacy(),
                rgba(255, 100, 100, 255),
            )];
            let (x, y, w, h) = Self::tooltip_geometry(
                &self.gui.atlas,
                &lines,
                cursor,
                (batch.width, batch.height),
                [0.0; 2],
            );
            horizon_client::widgets::record(&warning, [x, y, w, h], false);
            self.draw_tooltip_lines(batch, &lines, cursor.0, cursor.1);
        }
    }

    /// A fluid's well, no clicks: its slot, the sprite or colour in it and
    /// its amount under it.
    fn draw_fluid_well(&self, batch: &mut HudBatch, well: &Well, x: f32, y: f32) {
        let atlas = &self.gui.atlas;
        batch.sprite(atlas, "gui/sprites/container/slot", x, y);
        let dst = [x + 1.0, y + 1.0, 16.0, 16.0];
        let sprite = well
            .sprite
            .as_deref()
            .and_then(|s| self.gui_sprite_pixels(s).map(|size| (s, size)));
        match sprite {
            Some((name, size)) => {
                self.native_sprite(
                    batch,
                    name,
                    [0.0, 0.0, size[0], size[1]],
                    dst,
                    Some(well.tint),
                );
            }
            None => {
                let [a, r, g, b] = well.tint.to_be_bytes();
                batch.fill(atlas, dst[0], dst[1], dst[2], dst[3], rgba(r, g, b, a));
            }
        }
        batch.text(
            atlas,
            &well.amount,
            x,
            y + SLOT + 1.0,
            rgba(224, 224, 224, 255),
            true,
        );
        horizon_client::widgets::record_as(
            &well.label,
            [x, y, FLUID_WELL, SLOT + FLUID_TEXT],
            false,
            horizon_client::widgets::Kind::Button,
        );
    }

    /// The tabs, on slots; the selected one framed.
    fn draw_recipe_tabs(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let Some(pages) = &self.panel.recipes else {
            return;
        };
        let places = tab_places(pages, layout);
        let tabs: Vec<Option<ItemStack>> = pages.tabs.iter().map(|t| t.1.clone()).collect();
        let selected = pages.tab;
        for (i, (&(x, y), stack)) in places.iter().zip(tabs).enumerate() {
            let atlas = &self.gui.atlas;
            batch.sprite(atlas, "gui/sprites/container/slot", x, y);
            let Some(stack) = stack else {
                continue;
            };
            self.draw_item(batch, &stack, x + 1.0, y + 1.0);
            // The selected tab: a white frame around its slot.
            if i == selected {
                let white = rgba(255, 255, 255, 255);
                let atlas = &self.gui.atlas;
                batch.fill(atlas, x - 1.0, y - 1.0, SLOT + 2.0, 1.0, white);
                batch.fill(atlas, x - 1.0, y + SLOT, SLOT + 2.0, 1.0, white);
                batch.fill(atlas, x - 1.0, y, 1.0, SLOT, white);
                batch.fill(atlas, x + SLOT, y, 1.0, SLOT, white);
            }
            if inside(self.cursor, [x, y, SLOT, SLOT]) {
                batch.fill(
                    &self.gui.atlas,
                    x + 1.0,
                    y + 1.0,
                    16.0,
                    16.0,
                    rgba(255, 255, 255, 128),
                );
            }
            let name = self.pack.names.display_item(&stack.item);
            let rect = [x + 1.0, y + 1.0, 16.0, 16.0];
            let kind = horizon_client::widgets::Kind::Tab(i == selected);
            horizon_client::widgets::record_as(&format!("{name} tab"), rect, true, kind);
            // The kind reaches the bumpers only as a target (`key_focus::drawn_targets`).
            horizon_client::widgets::target_as(rect, kind);
        }
    }

    /// A press on the recipes: true when it hit one of their parts.
    pub(super) fn browser_recipes_press(
        &mut self,
        layout: &Layout,
        owner: &Owner,
        right: bool,
    ) -> bool {
        let Some(pages) = &self.panel.recipes else {
            return false;
        };
        let cursor = self.cursor;
        let starts = Self::recipe_pages(pages, layout);
        let page = pages.page.min(starts.len() - 1);
        let [back_arrow, forward] = layout.arrows();
        let turn = if page > 0 && inside(cursor, [back_arrow.0, back_arrow.1, 12.0, 17.0]) {
            Some(page - 1)
        } else if page + 1 < starts.len() && inside(cursor, [forward.0, forward.1, 12.0, 17.0]) {
            Some(page + 1)
        } else {
            None
        };
        if let Some(to) = turn {
            if let Some(p) = self.panel.recipes.as_mut() {
                p.page = to;
            }
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.panel_event(owner, BrowserEvent::Page(starts[to] as u32));
            return true;
        }
        if let Some((tab, _, _)) = self.recipe_tab_hovered(layout) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.panel_event(owner, BrowserEvent::Tab(tab as u32));
            return true;
        }
        if inside(cursor, self.back_button(layout).0) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.panel_event(owner, BrowserEvent::Back);
            return true;
        }
        if let Some((recipe, slot, stack, _)) = self.recipe_slot_hovered(layout) {
            let event = BrowserEvent::SlotClicked {
                recipe: recipe as u32,
                slot: slot as u32,
                stack,
                right,
            };
            self.panel_event(owner, event);
            return true;
        }
        let action = self.recipes_shown(layout).into_iter().find(|&(i, (x, y))| {
            let r = &pages.recipes[i];
            let (_, _, (ax, ay)) = r.places(layout.width());
            self.screen != super::Screen::Creative
                && r.action.is_some()
                && inside(cursor, [x + ax, y + ay, 12.0, 12.0])
        });
        if let Some((i, _)) = action {
            if pages.recipes[i].action == Some(true) {
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                self.panel_event(owner, BrowserEvent::Action(i as u32));
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Water and lava keep vanilla's colours, a look's tint wins, anything else is grey.
    #[test]
    fn a_fluid_has_a_colour() {
        assert_eq!(fluid_tint("minecraft:water", None), 0xFF3F76E4);
        assert_eq!(fluid_tint("minecraft:water", Some(0xFF112233)), 0xFF112233);
        assert_eq!(fluid_tint("mymod:oil", None), 0xFF9A9A9A);
    }

    /// The results sit in a row after the arrow, numbered after the inputs.
    #[test]
    fn results_form_a_row_after_the_inputs() {
        let shown = Shown {
            layout: BrowserLayout::Single,
            width: 1,
            height: 1,
            slots: vec![Vec::new(); 4],
            chances: vec![None, Some("25%".into()), None],
            fluid_inputs: Vec::new(),
            fluid_outputs: Vec::new(),
            note: String::new(),
            action: None,
        };
        let (at, _, action) = shown.places(200.0);
        assert_eq!(at.len(), 4);
        assert_eq!(at[0], (0.0, 0.0));
        // One has a chance, so they step wide enough for its text.
        assert_eq!(
            (at[2].0 - at[1].0, at[3].0 - at[2].0),
            (CHANCE_STEP, CHANCE_STEP)
        );
        assert_eq!(at[1].1, at[3].1);
        assert!(action.0 > at[3].0);
    }

    fn fluid(amount: u32) -> Well {
        mod_platform::__private::part_tests::game_thread();
        Well::new(&mod_api::FluidStack::new(
            mod_api::__private::fluid_key("minecraft:water").unwrap(),
            amount,
        ))
    }

    /// Inputs past the grid continue in rows under it, and the block grows with them.
    #[test]
    fn inputs_grow_the_grid_downwards() {
        let shown = Shown {
            layout: BrowserLayout::Grid,
            width: 3,
            height: 3,
            slots: vec![Vec::new(); 16],
            chances: vec![None],
            fluid_inputs: Vec::new(),
            fluid_outputs: Vec::new(),
            note: String::new(),
            action: None,
        };
        let (at, _, _) = shown.places(200.0);
        assert_eq!(at[9], (0.0, 3.0 * SLOT));
        assert_eq!(at[14], (2.0 * SLOT, 4.0 * SLOT));
        assert!(shown.block(200.0) >= 5.0 * SLOT);
    }

    /// Wells sit under the slots, inputs before outputs, wrapping to the room; a chance
    /// is written under its slot, which the block has room for.
    #[test]
    fn fluid_wells_sit_under_the_slots() {
        let mut shown = Shown {
            layout: BrowserLayout::Single,
            width: 1,
            height: 1,
            slots: vec![Vec::new(); 2],
            chances: vec![Some("10%".into())],
            fluid_inputs: vec![fluid(1000); 3],
            fluid_outputs: vec![fluid(250)],
            note: String::new(),
            action: None,
        };
        let block = shown.block(200.0);
        assert!(block >= SLOT + CHANCE_TEXT);
        let (inputs, outputs, used) = shown.fluid_places(200.0);
        assert_eq!((inputs.len(), outputs.len()), (3, 1));
        assert!(inputs.iter().all(|p| p.1 >= block));
        assert!(outputs[0].1 > inputs[0].1);
        assert_eq!(inputs[1].0 - inputs[0].0, FLUID_WELL);
        assert_eq!(shown.height(200.0), block + used + GAP);
        // Too narrow for three: they wrap.
        let (wrapped, _, _) = shown.fluid_places(2.0 * FLUID_WELL);
        assert_eq!(wrapped[2], (0.0, wrapped[0].1 + SLOT + FLUID_TEXT));
        shown.fluid_inputs.clear();
        shown.fluid_outputs.clear();
        assert_eq!(shown.fluid_places(200.0).2, 0.0);
    }

    /// In a panel too narrow for the action beside the results, it goes under
    /// them, past their chance line; the block has room for it.
    #[test]
    fn a_fallback_action_sits_below_the_chance_line() {
        let shown = Shown {
            layout: BrowserLayout::Single,
            width: 1,
            height: 1,
            slots: vec![Vec::new(); 2],
            chances: vec![Some("25%".into())],
            fluid_inputs: Vec::new(),
            fluid_outputs: Vec::new(),
            note: String::new(),
            action: Some(true),
        };
        let (at, _, action) = shown.places(72.0);
        let result = at[1];
        // The chance is written from one pixel under the slot, CHANCE_TEXT tall.
        assert!(
            action.1 >= result.1 + SLOT + CHANCE_TEXT,
            "{action:?} under {result:?}"
        );
        assert!(shown.block(72.0) >= action.1 + 12.0);
    }

    #[test]
    fn an_unresolved_fluid_uses_the_unknown_label() {
        mod_platform::__private::initialize().unwrap();
        let handle = mod_api::__private::intern_unresolved_fluid("missing:panel_fluid").unwrap();
        let well = Well::new(&mod_api::FluidStack {
            fluid: mod_api::FluidIdentity::Unresolved(handle),
            amount: 250,
            components: None,
        });
        assert!(well.label.contains("250 mB"));
        assert!(!well.label.contains("Panel Fluid"));
    }

    /// A well's texts are made once: its amount and its name, which falls back
    /// to the prettified path.
    #[test]
    fn a_well_names_its_fluid_and_amount() {
        mod_platform::__private::part_tests::game_thread();
        let well = Well::new(&mod_api::FluidStack::new(
            mod_api::__private::fluid_key("test:potion").unwrap(),
            250,
        ));
        assert_eq!(well.amount, "250 mB");
        assert_eq!(well.label, "Potion 250 mB");
        assert_eq!(well.tint, 0xFF9A9A9A);
    }

    /// Too narrow for the wide step, results with chances sit slot to slot and
    /// write every other chance a line lower; the block and a fallback action
    /// make room for both lines.
    #[test]
    fn narrow_results_stagger_their_chances() {
        let shown = Shown {
            layout: BrowserLayout::Single,
            width: 1,
            height: 1,
            slots: vec![Vec::new(); 4],
            chances: vec![None, Some("25%".into()), Some("12.5%".into())],
            fluid_inputs: Vec::new(),
            fluid_outputs: Vec::new(),
            note: String::new(),
            action: Some(true),
        };
        assert_eq!(shown.result_step(100.0), (SLOT, 2.0));
        let (at, _, action) = shown.places(100.0);
        assert_eq!(at[2].0 - at[1].0, SLOT);
        assert!(action.1 >= at[3].1 + SLOT + 2.0 * CHANCE_TEXT);
        assert!(shown.block(100.0) >= at[3].1 + SLOT + 2.0 * CHANCE_TEXT);
        assert_eq!(shown.result_step(200.0), (CHANCE_STEP, 1.0));
    }

    /// Without a chance the results sit slot to slot, as a crafting result does.
    #[test]
    fn results_without_chances_step_by_a_slot() {
        let shown = Shown {
            layout: BrowserLayout::Single,
            width: 1,
            height: 1,
            slots: vec![Vec::new(); 3],
            chances: vec![None, None],
            fluid_inputs: Vec::new(),
            fluid_outputs: Vec::new(),
            note: String::new(),
            action: None,
        };
        let (at, _, _) = shown.places(200.0);
        assert_eq!(at[2].0 - at[1].0, SLOT);
    }

    #[test]
    fn chance_and_fluid_names_read_well() {
        assert_eq!(chance_text(25.0), "25%");
        assert_eq!(chance_text(12.5), "12.5%");
    }
}
