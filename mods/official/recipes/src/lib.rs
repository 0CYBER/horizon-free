//! Horizon's official recipe viewer. Every item sits in the item panel the
//! game draws beside the inventory and container screens (`browser`), with
//! a search box. R over an item, the panel's or any slot's, shows its
//! recipes, U its uses (`input.register-key`, so both can be rebound in
//! Controls); a click does the same, left for recipes and right for uses.
//! A recipe page has a tab per station, and + beside a crafting recipe the
//! player knows fills the grid as the recipe book does (`recipes.place`).
//! In creative mode, with "Click gives items" on, a click on a panel item
//! gives one, a right click a stack (`recipes.give`).
//!
//! On a server that is not Horizon the recipes are the game's own 26.3
//! ones; `known` and `place` follow the server's recipe book.
use horizon_mod_sdk::{
    Mod, Registration, browser, chat, game, input, recipes,
    recipes::Kind,
    storage,
    ui::{self, ButtonWidget, Widget},
};

const RECIPE_KEY: &str = "recipe";
const USES_KEY: &str = "uses";
const GIVE_BUTTON: u32 = 1;
/// What `set-recipes` takes at most.
const MAX_RECIPES: usize = 256;
const MAX_STACKS: usize = 64;
/// The stations, in the order of their tabs, with each one's icon.
const STATIONS: [&str; 8] = [
    "minecraft:crafting_table",
    "minecraft:furnace",
    "minecraft:blast_furnace",
    "minecraft:smoker",
    "minecraft:campfire",
    "minecraft:stonecutter",
    "minecraft:smithing_table",
    "minecraft:brewing_stand",
];

fn station(kind: Kind) -> usize {
    match kind {
        Kind::CraftingShaped | Kind::CraftingShapeless => 0,
        Kind::Smelting => 1,
        Kind::Blasting => 2,
        Kind::Smoking => 3,
        Kind::Campfire => 4,
        Kind::Stonecutting => 5,
        Kind::Smithing => 6,
        Kind::Brewing => 7,
    }
}

fn stack(item: &str) -> browser::Stack {
    browser::Stack {
        item: item.into(),
        count: 1,
        components: String::new(),
    }
}

/// A recipe page: an item's recipes or its uses, on one station's tab.
#[derive(Clone)]
struct View {
    item: browser::Stack,
    uses: bool,
    tab: usize,
}

#[derive(Default)]
struct Recipes {
    items: Vec<recipes::Item>,
    all: Vec<recipes::Recipe>,
    /// The panel's items now: indexes into `items`, after the search.
    listed: Vec<usize>,
    search: String,
    /// The page shown, then the ones Back returns to.
    history: Vec<View>,
    /// The book id behind each recipe the page shows (-1: none).
    shown: Vec<i32>,
    give: bool,
}

impl Recipes {
    fn id(&self, s: &recipes::Stack) -> &str {
        self.items
            .get(s.item as usize)
            .map_or("minecraft:air", |i| &i.id)
    }
    fn convert(&self, s: &recipes::Stack) -> browser::Stack {
        browser::Stack {
            item: self.id(s).into(),
            count: s.count,
            components: s.components.clone(),
        }
    }
    /// The same item, and the same components when `want` names some.
    fn matches(&self, s: &recipes::Stack, want: &browser::Stack) -> bool {
        self.id(s) == want.item && (want.components.is_empty() || s.components == want.components)
    }
    fn name<'a>(&'a self, item: &'a str) -> &'a str {
        self.items
            .iter()
            .find(|i| i.id == item)
            .map_or(item, |i| &i.name)
    }

    /// Loads the tables once a world is open, and drops them when it closes.
    fn load(&mut self) {
        let in_world = game::get_player().is_some();
        if in_world && self.items.is_empty() {
            self.items = recipes::items();
            self.all = recipes::all();
            self.list();
        } else if !in_world && !self.items.is_empty() {
            browser::set_recipes("", &[], 0, &[]);
            browser::set_items(&[]);
            *self = Recipes {
                give: self.give,
                ..Recipes::default()
            };
        }
    }

    /// Fills the panel with the items the search keeps: every word must be
    /// in the name or the id; `@minecraft` keeps a namespace, `#logs` a tag.
    fn list(&mut self) {
        let words: Vec<String> = self
            .search
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();
        let keeps = |item: &recipes::Item| {
            words.iter().all(|w| {
                if let Some(ns) = w.strip_prefix('@') {
                    item.id.split(':').next().is_some_and(|n| n.starts_with(ns))
                } else if let Some(tag) = w.strip_prefix('#') {
                    item.tags.iter().any(|t| t.contains(tag))
                } else {
                    item.name.to_lowercase().contains(w.as_str()) || item.id.contains(w.as_str())
                }
            })
        };
        self.listed = (self.items.iter().enumerate())
            .filter(|(_, i)| i.id != "minecraft:air" && keeps(i))
            .map(|(n, _)| n)
            .collect();
        let stacks: Vec<_> = self
            .listed
            .iter()
            .map(|&n| stack(&self.items[n].id))
            .collect();
        browser::set_items(&stacks);
    }

    /// The recipes a view lists, all stations together.
    fn found(&self, view: &View) -> Vec<&recipes::Recipe> {
        (self.all.iter())
            .filter(|r| {
                if view.uses {
                    r.slots
                        .iter()
                        .flatten()
                        .any(|s| self.matches(s, &view.item))
                } else {
                    r.results.iter().any(|s| self.matches(s, &view.item))
                }
            })
            .collect()
    }

    /// Opens a page, pushing it on the history; nothing when it is empty.
    fn open(&mut self, item: browser::Stack, uses: bool) {
        let view = View { item, uses, tab: 0 };
        if self.found(&view).is_empty() {
            return;
        }
        self.history.push(view);
        self.show();
    }

    /// Sends the page on top of the history, or the items when there is none.
    fn show(&mut self) {
        let Some(mut view) = self.history.last().cloned() else {
            self.shown.clear();
            browser::set_recipes("", &[], 0, &[]);
            return;
        };
        let found = self.found(&view);
        let stations: Vec<usize> = (0..STATIONS.len())
            .filter(|&s| found.iter().any(|r| station(r.kind) == s))
            .collect();
        view.tab = view.tab.min(stations.len().saturating_sub(1));
        let on: Vec<&recipes::Recipe> = (found.iter().copied())
            .filter(|r| stations.get(view.tab) == Some(&station(r.kind)))
            .take(MAX_RECIPES)
            .collect();
        let known = recipes::known();
        let views: Vec<browser::RecipeView> = on.iter().map(|r| self.view(r, &known)).collect();
        let mut title = format!(
            "{}: {}",
            if view.uses { "Uses" } else { "Recipes" },
            self.name(&view.item.item)
        );
        let total = found
            .iter()
            .filter(|r| stations.get(view.tab) == Some(&station(r.kind)))
            .count();
        if total > on.len() {
            title.push_str(&format!(" ({} of {total})", on.len()));
        }
        let tabs: Vec<_> = stations.iter().map(|&s| stack(STATIONS[s])).collect();
        browser::set_recipes(&title, &tabs, view.tab as u32, &views);
        self.shown = on.iter().map(|r| r.book_id).collect();
        if let Some(top) = self.history.last_mut() {
            top.tab = view.tab;
        }
    }

    fn view(&self, r: &recipes::Recipe, known: &[i32]) -> browser::RecipeView {
        let mut slots: Vec<Vec<browser::Stack>> = (r.slots.iter())
            .map(|s| s.iter().take(MAX_STACKS).map(|s| self.convert(s)).collect())
            .collect();
        let results = r
            .results
            .iter()
            .take(MAX_STACKS)
            .map(|s| self.convert(s))
            .collect();
        let (layout, width, height, note) = match r.kind {
            Kind::CraftingShaped => (browser::Layout::Grid, r.width, r.height, String::new()),
            Kind::CraftingShapeless => {
                let n = slots.len() as u32;
                let width = match n {
                    0 | 1 => 1,
                    2..=4 => 2,
                    _ => 3,
                };
                let height = n.div_ceil(width).max(1);
                slots.resize((width * height) as usize, Vec::new());
                (browser::Layout::Grid, width, height, "Shapeless".into())
            }
            Kind::Smelting | Kind::Blasting | Kind::Smoking | Kind::Campfire => {
                let seconds = r.cooking_time as f32 / 20.0;
                let note = format!("{seconds} s, {} XP", (r.experience * 100.0).round() / 100.0);
                (browser::Layout::Cooking, 1, 1, note)
            }
            Kind::Stonecutting => (browser::Layout::Single, 1, 1, String::new()),
            Kind::Smithing => (browser::Layout::Smithing, 3, 1, String::new()),
            Kind::Brewing => (browser::Layout::Brewing, 2, 1, String::new()),
        };
        // + only for what the recipe book shows; greyed until the book has it.
        let action = (r.book_id >= 0).then(|| known.contains(&r.book_id));
        browser::RecipeView {
            layout,
            width,
            height,
            slots,
            results,
            note,
            action,
        }
    }

    fn handle(&mut self, event: browser::Event) {
        match event {
            browser::Event::Search(text) => {
                self.search = text;
                self.list();
            }
            browser::Event::Clicked(c) => {
                let Some(&n) = self.listed.get(c.index as usize) else {
                    return;
                };
                let item = self.items[n].id.clone();
                if self.give {
                    let count = if c.right { 99 } else { 1 };
                    let gift = recipes::Stack {
                        item: n as u32,
                        count,
                        components: String::new(),
                    };
                    if recipes::give(&gift).is_ok() {
                        return;
                    }
                }
                self.open(stack(&item), c.right);
            }
            browser::Event::Key(k) if k.key == RECIPE_KEY || k.key == USES_KEY => {
                let item = browser::Stack {
                    count: 1,
                    ..k.stack
                };
                self.open(item, k.key == USES_KEY);
            }
            browser::Event::SlotClicked(c) => {
                let item = browser::Stack {
                    count: 1,
                    ..c.stack
                };
                self.open(item, c.right);
            }
            browser::Event::Tab(i) => {
                if let Some(top) = self.history.last_mut() {
                    top.tab = i as usize;
                }
                self.show();
            }
            browser::Event::Action(i) => {
                let book = self.shown.get(i as usize).copied();
                if let Some(Err(e)) = book.map(|b| recipes::place(b, false)) {
                    chat::show(&e);
                }
            }
            browser::Event::Back => {
                self.history.pop();
                self.show();
            }
            _ => {}
        }
    }

    fn page(&self) {
        ui::set_settings(&[Widget::Button(ButtonWidget {
            id: GIVE_BUTTON,
            label: format!(
                "Click gives items (creative): {}",
                if self.give { "ON" } else { "OFF" }
            ),
        })]);
    }
}

impl Mod for Recipes {
    fn init() -> Result<(Self, Registration), String> {
        input::register_key(RECIPE_KEY, "Show recipes", "key.keyboard.r")?;
        input::register_key(USES_KEY, "Show uses", "key.keyboard.u")?;
        let give = storage::get("give").is_some_and(|v| v == b"true");
        let me = Recipes {
            give,
            ..Recipes::default()
        };
        me.page();
        Ok((me, Registration::new().on_frame()))
    }
    fn frame(&mut self, _seconds: f32) {
        self.load();
        // The keys matter only over an item, which `browser` reports.
        let _ = input::poll();
        for event in browser::poll() {
            self.handle(event);
        }
    }
    fn ui_select(&mut self, id: u32, _row: u32) {
        if id == GIVE_BUTTON {
            self.give = !self.give;
            let _ = storage::set("give", if self.give { b"true" } else { b"false" });
            self.page();
        }
    }
    fn ui_settings_opened(&mut self) {
        self.page();
    }
}

horizon_mod_sdk::export_mod!(Recipes);
