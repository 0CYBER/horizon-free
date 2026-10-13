//! Creative inventory (CreativeModeInventoryScreen): vanilla's tabs and their
//! contents as built by the game's own display generators
//! (`creative-inventory.json`), a 9x5 scrolling grid and the search tab.
use crate::{Result, items::ItemStack};
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<ItemStack>,
    pub top: bool,
    pub column: u32,
    pub search: bool,
    pub inventory: bool,
}
/// What the client's creative search trees (SessionSearchTrees) index for one
/// stack of the search tab: its creative tooltip lines and its item tags.
#[derive(Clone, Debug, Default)]
struct SearchEntry {
    /// The item id's namespace and path, lower-cased.
    namespace: String,
    path: String,
    /// Tooltip lines, lower-cased; empty when the resources predate them.
    lines: Vec<String>,
    /// Item tags as (namespace, path), lower-cased.
    tags: Vec<(String, String)>,
}
pub struct Creative {
    pub tabs: Vec<Tab>,
    pub selected: usize,
    /// Scroll position 0..1 (scrollOffs).
    pub scroll: f32,
    /// The scrollbar is held (scrolling): dragging moves it.
    pub scrolling: bool,
    pub search: String,
    /// The search tab's stacks, in its display order.
    search_items: Vec<ItemStack>,
    search_index: Vec<SearchEntry>,
}
/// Display names from the pinned en_us language file.
pub struct Names {
    map: HashMap<String, String>,
}
impl Names {
    pub fn load(root: &Path) -> Self {
        let map = fs::read(root.join("assets/minecraft/lang/en_us.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok())
            .unwrap_or_default();
        Self { map }
    }
    pub fn item(&self, id: &str) -> String {
        let path = id.trim_start_matches("minecraft:");
        self.map
            .get(&format!("item.minecraft.{path}"))
            .or_else(|| self.map.get(&format!("block.minecraft.{path}")))
            .cloned()
            .unwrap_or_else(|| path.replace('_', " "))
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}
impl Creative {
    pub const WIDTH: f32 = 195.0;
    pub const HEIGHT: f32 = 136.0;
    pub const TRASH_POSITION: (f32, f32) = (173.0, 112.0);

    /// Visible player slots in the creative inventory tab (game slot ids).
    pub fn inventory_slots() -> impl Iterator<Item = usize> {
        (0..36).chain(500..505)
    }

    /// CreativeModeInventoryScreen.selectTab positions its InventoryMenu wrappers.
    pub fn inventory_slot_position(slot: usize) -> Option<(f32, f32)> {
        match slot {
            0..9 => Some((9.0 + slot as f32 * 18.0, 112.0)),
            9..36 => Some((
                9.0 + ((slot - 9) % 9) as f32 * 18.0,
                54.0 + ((slot - 9) / 9) as f32 * 18.0,
            )),
            500..504 => {
                let position = slot - 500;
                Some((
                    54.0 + (position / 2) as f32 * 54.0,
                    6.0 + (position % 2) as f32 * 27.0,
                ))
            }
            504 => Some((35.0, 20.0)),
            _ => None,
        }
    }
    /// The scrollbar track (insideScrollbar), relative to the screen origin.
    pub const SCROLL_X: f32 = 175.0;
    pub const SCROLL_Y: f32 = 18.0;
    pub const SCROLL_WIDTH: f32 = 14.0;
    pub const SCROLL_HEIGHT: f32 = 112.0;
    /// `creative-inventory.json` (VanillaCreativeTabsExporter); resources
    /// prepared before it fall back to `creative-tabs.json`'s item ids.
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("creative-inventory.json");
        if path.exists() {
            Self::from_json(&serde_json::from_slice(&fs::read(path)?)?)
        } else {
            Self::from_legacy(&serde_json::from_slice(&fs::read(
                root.join("creative-tabs.json"),
            )?)?)
        }
    }
    pub fn from_json(raw: &Value) -> Result<Self> {
        let mut tabs = Vec::new();
        let mut search_items = Vec::new();
        let mut search_index = Vec::new();
        for t in raw["tabs"].as_array().into_iter().flatten() {
            let mut items = Vec::new();
            for v in t["items"].as_array().into_iter().flatten() {
                let snbt = v.as_str().unwrap_or("-");
                if let Some(stack) = crate::item_data::parse_stack(snbt)? {
                    items.push(stack);
                }
            }
            let kind = t["type"].as_str().unwrap_or("");
            if kind == "SEARCH" {
                let strings = |v: &Value| -> Vec<String> {
                    v.as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|s| s.as_str().map(java_lower))
                        .collect()
                };
                for (i, stack) in items.iter().enumerate() {
                    let (namespace, path) = split_id(&java_lower(&stack.item));
                    search_index.push(SearchEntry {
                        namespace,
                        path,
                        lines: strings(&t["lines"][i]),
                        tags: strings(&t["tags"][i]).iter().map(|t| split_id(t)).collect(),
                    });
                }
                search_items = items.clone();
            }
            if kind == "HOTBAR" || t["visible"] == false {
                continue;
            }
            let icon = t["icon"]
                .as_str()
                .and_then(|s| crate::item_data::parse_stack(s).ok().flatten());
            tabs.push(Tab {
                id: t["id"].as_str().unwrap_or("").to_owned(),
                title: t["title"].as_str().unwrap_or("").to_owned(),
                icon: icon.map_or_else(|| "minecraft:stone".to_owned(), |s| s.item.to_string()),
                items,
                top: t["row"] == "TOP",
                column: t["column"].as_u64().unwrap_or(0) as u32,
                search: kind == "SEARCH",
                inventory: kind == "INVENTORY",
            });
        }
        Ok(Self::new(tabs, search_items, search_index))
    }
    /// The old export: item ids per tab, no components, tooltips or tags.
    fn from_legacy(raw: &Value) -> Result<Self> {
        let mut tabs = Vec::new();
        let mut search_items: Vec<ItemStack> = Vec::new();
        for t in raw.as_array().into_iter().flatten() {
            let items: Vec<ItemStack> = t["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(|id| ItemStack::new(id, 1)))
                .collect();
            if t["id"] == "minecraft:hotbar" || t["id"] == "minecraft:op_blocks" {
                continue;
            }
            for i in &items {
                if !search_items.iter().any(|s| s.item == i.item) {
                    search_items.push(i.clone());
                }
            }
            let kind = t["type"].as_str().unwrap_or("");
            tabs.push(Tab {
                id: t["id"].as_str().unwrap_or("").to_owned(),
                title: t["title"].as_str().unwrap_or("").to_owned(),
                icon: t["icon"].as_str().unwrap_or("minecraft:stone").to_owned(),
                items,
                top: t["row"] == "TOP",
                column: t["column"].as_u64().unwrap_or(0) as u32,
                search: kind == "SEARCH",
                inventory: kind == "INVENTORY",
            });
        }
        let search_index = search_items
            .iter()
            .map(|s| {
                let (namespace, path) = split_id(&java_lower(&s.item));
                SearchEntry {
                    namespace,
                    path,
                    ..Default::default()
                }
            })
            .collect();
        Ok(Self::new(tabs, search_items, search_index))
    }
    fn new(tabs: Vec<Tab>, search_items: Vec<ItemStack>, search_index: Vec<SearchEntry>) -> Self {
        Self {
            tabs,
            selected: 0,
            scroll: 0.0,
            scrolling: false,
            search: String::new(),
            search_items,
            search_index,
        }
    }
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.selected.min(self.tabs.len() - 1)]
    }
    /// selectTab: a different tab empties the search box; the scrollbar goes to the top.
    pub fn select(&mut self, tab: usize) {
        if tab != self.selected {
            self.search.clear();
        }
        self.selected = tab;
        self.scroll = 0.0;
    }
    /// The search tab's stacks.
    pub fn search_items(&self) -> &[ItemStack] {
        &self.search_items
    }
    /// Indices into `search_items` that CreativeModeInventoryScreen.refreshSearchResults
    /// shows for `term`: the whole tab when empty, the tag tree after `#`, else
    /// the name tree, both over vanilla's suffix arrays (substring matches,
    /// results in the tab's order).
    pub fn search_results(&self, term: &str, names: &Names) -> Vec<usize> {
        let all = 0..self.search_index.len();
        if term.is_empty() {
            return all.collect();
        }
        if let Some(tag) = term.strip_prefix('#') {
            // IdSearchTree over the stacks' tags.
            let text = java_lower(tag);
            return all
                .filter(|&i| {
                    let tags = &self.search_index[i].tags;
                    match text.split_once(':') {
                        None => any(tags.iter().map(|t| t.1.as_str()), &text),
                        Some((ns, path)) => {
                            let (ns, path) = (java_trim(ns), java_trim(path));
                            any(tags.iter().map(|t| t.0.as_str()), ns)
                                && any(tags.iter().map(|t| t.1.as_str()), path)
                        }
                    }
                })
                .collect();
        }
        // FullTextSearchTree: tooltip lines, and the item id after a colon.
        let text = java_lower(term);
        all.filter(|&i| {
            let e = &self.search_index[i];
            let fallback;
            let lines: &[String] = if e.lines.is_empty() {
                fallback = [java_lower(&names.item(&self.search_items[i].item))];
                &fallback
            } else {
                &e.lines
            };
            match text.split_once(':') {
                None => any(lines.iter().map(String::as_str), &text),
                Some((ns, path)) => {
                    let (ns, path) = (java_trim(ns), java_trim(path));
                    e.namespace.contains(ns)
                        && (e.path.contains(path) || any(lines.iter().map(String::as_str), path))
                }
            }
        })
        .collect()
    }
    /// Items shown by the selected tab (the search tab filters by `search`).
    pub fn items(&self, names: &Names) -> Vec<ItemStack> {
        let tab = self.tab();
        if tab.search {
            return self
                .search_results(&self.search, names)
                .into_iter()
                .map(|i| self.search_items[i].clone())
                .collect();
        }
        tab.items.clone()
    }
    pub fn rows(&self, names: &Names) -> usize {
        self.items(names).len().div_ceil(9)
    }
    /// ItemPickerMenu.calculateRowCount: rows beyond the five shown.
    fn hidden_rows(&self, names: &Names) -> i32 {
        self.rows(names) as i32 - 5
    }
    /// First visible grid row for the scroll position (getRowIndexForScroll).
    pub fn first_row(&self, names: &Names) -> usize {
        ((self.scroll * self.hidden_rows(names) as f32 + 0.5) as i32).max(0) as usize
    }
    /// CreativeModeInventoryScreen.canScroll: more than five rows, on a tab with a scrollbar.
    pub fn can_scroll(&self, names: &Names) -> bool {
        !self.tab().inventory && self.items(names).len() > 45
    }
    /// insideScrollbar, for a point relative to the screen origin.
    pub fn inside_scrollbar(&self, x: f32, y: f32) -> bool {
        x >= Self::SCROLL_X
            && y >= Self::SCROLL_Y
            && x < Self::SCROLL_X + Self::SCROLL_WIDTH
            && y < Self::SCROLL_Y + Self::SCROLL_HEIGHT
            && !self.tab().inventory
    }
    /// mouseClicked on the scrollbar: it is held only if the tab can scroll
    /// (the thumb doesn't move until the mouse does).
    pub fn press_scrollbar(&mut self, x: f32, y: f32, names: &Names) -> bool {
        if !self.inside_scrollbar(x, y) {
            return false;
        }
        self.scrolling = self.can_scroll(names);
        true
    }
    /// mouseDragged while the scrollbar is held, at a cursor height relative to
    /// the screen origin, wherever the cursor is (even off the track or the window).
    pub fn drag_scrollbar(&mut self, y: f32) -> bool {
        if !self.scrolling {
            return false;
        }
        let (top, bottom) = (Self::SCROLL_Y, Self::SCROLL_Y + Self::SCROLL_HEIGHT);
        self.scroll = ((y - top - 7.5) / (bottom - top - 15.0)).clamp(0.0, 1.0);
        true
    }
    /// mouseReleased.
    pub fn release(&mut self) {
        self.scrolling = false;
    }
    /// mouseScrolled: ItemPickerMenu.subtractInputFromScroll.
    pub fn wheel(&mut self, amount: f64, names: &Names) {
        if !self.can_scroll(names) {
            return;
        }
        let rows = self.hidden_rows(names) as f64;
        self.scroll = (self.scroll - (amount / rows) as f32).clamp(0.0, 1.0);
    }
    /// The scroller sprite's offset from the top of the track.
    pub fn scroller_offset(&self) -> f32 {
        ((Self::SCROLL_HEIGHT - 17.0) * self.scroll) as i32 as f32
    }
    /// Tab button position relative to the screen origin (CreativeModeInventoryScreen.getTabX/Y).
    pub fn tab_position(tab: &Tab) -> (f32, f32) {
        let mut x = tab.column as f32 * 27.0;
        if tab.column >= 5 {
            // Right-aligned tabs (search, inventory).
            x = Self::WIDTH - 27.0 * (7 - tab.column) as f32 + 1.0;
        }
        let y = if tab.top { -28.0 } else { Self::HEIGHT - 4.0 };
        (x, y)
    }
}
/// Whether a suffix array of `strings` has a suffix starting with `text`:
/// some string contains it (none for no strings, even when `text` is empty).
fn any<'a>(strings: impl IntoIterator<Item = &'a str>, text: &str) -> bool {
    strings.into_iter().any(|s| s.contains(text))
}
/// String.toLowerCase(Locale.ROOT).
fn java_lower(s: &str) -> String {
    s.to_lowercase()
}
/// String.trim: strips code points up to U+0020.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}
/// Identifier.parse's namespace and path (default namespace `minecraft`).
fn split_id(id: &str) -> (String, String) {
    match id.split_once(':') {
        Some((ns, path)) => (ns.to_owned(), path.to_owned()),
        None => ("minecraft".to_owned(), id.to_owned()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn creative(items: usize) -> Creative {
        let mut stacks: Vec<String> = ["anvil", "crafting_table", "wooden_hoe"]
            .iter()
            .map(|i| format!("{{count:1,id:\"minecraft:{i}\"}}"))
            .collect();
        stacks.extend((0..items).map(|i| format!("{{count:1,id:\"minecraft:item_{i}\"}}")));
        let mut lines = vec![
            vec!["Anvil"],
            vec!["Crafting Table"],
            vec!["Wooden Hoe", "When in Main Hand:", "-3 Attack Speed"],
        ];
        lines.extend(vec![vec!["Thing"]; items]);
        let mut tags = vec![
            vec!["minecraft:anvil"],
            vec![],
            vec!["minecraft:hoes", "minecraft:enchantable/mining"],
        ];
        tags.extend(vec![vec![]; items]);
        let raw = serde_json::json!({"schema": 2, "tabs": [
            {"id": "minecraft:functional_blocks", "type": "CATEGORY", "row": "TOP", "column": 3, "title": "Functional Blocks",
             "icon": "{count:1,id:\"minecraft:oak_sign\"}", "visible": true,
             "items": ["{count:1,id:\"minecraft:anvil\"}", "{count:1,id:\"minecraft:crafting_table\"}"]},
            {"id": "minecraft:op_blocks", "type": "CATEGORY", "row": "BOTTOM", "column": 5, "title": "Operator Utilities",
             "icon": "{count:1,id:\"minecraft:command_block\"}", "visible": false, "items": []},
            {"id": "minecraft:search", "type": "SEARCH", "row": "TOP", "column": 6, "title": "Search Items",
             "icon": "{count:1,id:\"minecraft:compass\"}", "visible": true,
             "items": stacks, "lines": lines, "tags": tags},
            {"id": "minecraft:inventory", "type": "INVENTORY", "row": "BOTTOM", "column": 6, "title": "Survival Inventory",
             "icon": "{count:1,id:\"minecraft:chest\"}", "visible": true, "items": []}
        ]});
        Creative::from_json(&raw).unwrap()
    }

    #[test]
    fn inventory_tab_slot_layout_matches_vanilla() {
        assert_eq!(Creative::inventory_slots().count(), 41);
        for (slot, expected) in [
            (0, (9.0, 112.0)),
            (8, (153.0, 112.0)),
            (9, (9.0, 54.0)),
            (35, (153.0, 90.0)),
            (500, (54.0, 6.0)),
            (501, (54.0, 33.0)),
            (502, (108.0, 6.0)),
            (503, (108.0, 33.0)),
            (504, (35.0, 20.0)),
        ] {
            assert_eq!(Creative::inventory_slot_position(slot), Some(expected));
        }
        assert_eq!(Creative::inventory_slot_position(100), None);
        assert_eq!(Creative::TRASH_POSITION, (173.0, 112.0));
        let mut c = creative(90);
        c.select(2);
        assert!(c.tab().inventory);
        assert!(!c.can_scroll(&Names {
            map: HashMap::new()
        }));
    }

    #[test]
    fn hidden_tabs_are_skipped() {
        let c = creative(0);
        let ids: Vec<&str> = c.tabs.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "minecraft:functional_blocks",
                "minecraft:search",
                "minecraft:inventory"
            ]
        );
        assert_eq!(c.tabs[0].items[0].item.as_ref(), "minecraft:anvil");
        let mut c = c;
        c.select(1);
        c.search = "anvil".into();
        c.scroll = 0.5;
        c.select(1);
        assert_eq!((c.search.as_str(), c.scroll), ("anvil", 0.0));
        c.select(0);
        assert_eq!(c.search, "");
    }

    #[test]
    fn search_matches_vanilla_trees() {
        let c = creative(0);
        let names = Names {
            map: HashMap::new(),
        };
        let q = |s: &str| c.search_results(s, &names);
        assert_eq!(q(""), [0, 1, 2]);
        assert_eq!(q("ANVIL"), [0]);
        assert_eq!(q("table"), [1]);
        assert_eq!(q("a"), [0, 1, 2]);
        // Tooltip lines, not just names; no trimming without a colon.
        assert_eq!(q("attack speed"), [2]);
        assert_eq!(q(" anvil"), Vec::<usize>::new());
        // A colon searches the id: namespace, then the path or a tooltip line.
        assert_eq!(q("minecraft:crafting_t"), [1]);
        assert_eq!(q(" minecraft : hoe "), [2]);
        assert_eq!(q("mine:anvil"), [0]);
        assert_eq!(q("other:anvil"), Vec::<usize>::new());
        assert_eq!(q(":"), [0, 1, 2]);
        // Tags after "#": only stacks with tags.
        assert_eq!(q("#"), [0, 2]);
        assert_eq!(q("#hoe"), [2]);
        assert_eq!(q("#minecraft:enchantable/"), [2]);
        assert_eq!(q("#craft"), Vec::<usize>::new());
    }

    #[test]
    fn scrollbar_drag_follows_the_mouse_while_held() {
        let mut c = creative(90);
        let names = Names {
            map: HashMap::new(),
        };
        c.selected = 1;
        // Pressing the track holds the scrollbar without moving it.
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(c.scrolling);
        assert_eq!(c.scroll, 0.0);
        // Dragging maps the cursor height onto the track (minus half the thumb).
        assert!(c.drag_scrollbar(18.0 + 7.5 + 48.5));
        assert!((c.scroll - 0.5).abs() < 1e-6);
        // The cursor may leave the track, the panel or the window: it keeps scrolling.
        c.drag_scrollbar(400.0);
        assert_eq!(c.scroll, 1.0);
        c.drag_scrollbar(-50.0);
        assert_eq!(c.scroll, 0.0);
        c.drag_scrollbar(18.0 + 7.5 + 97.0 * 0.25);
        assert!((c.scroll - 0.25).abs() < 1e-6);
        let rows = (93usize.div_ceil(9) - 5) as f32;
        assert_eq!(c.first_row(&names), (0.25 * rows + 0.5) as usize);
        // Releasing stops it.
        c.release();
        assert!(!c.drag_scrollbar(400.0));
        assert!((c.scroll - 0.25).abs() < 1e-6);
        // Off the track, or with too few items, it isn't held.
        assert!(!c.press_scrollbar(170.0, 60.0, &names));
        assert!(!c.press_scrollbar(180.0, 130.0, &names));
        c.selected = 0;
        assert!(c.press_scrollbar(180.0, 60.0, &names));
        assert!(!c.scrolling);
    }

    #[test]
    fn wheel_moves_one_row_per_notch() {
        let mut c = creative(90);
        let names = Names {
            map: HashMap::new(),
        };
        c.selected = 1;
        let rows = (93usize.div_ceil(9) - 5) as f32;
        c.wheel(-1.0, &names);
        assert!((c.scroll - 1.0 / rows).abs() < 1e-6);
        assert_eq!(c.first_row(&names), 1);
        c.wheel(5.0, &names);
        assert_eq!(c.scroll, 0.0);
        assert_eq!(c.scroller_offset(), 0.0);
        c.scroll = 1.0;
        assert_eq!(c.scroller_offset(), 95.0);
    }
}
