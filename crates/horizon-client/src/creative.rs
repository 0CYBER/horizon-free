//! Creative inventory (CreativeModeInventoryScreen): vanilla's tabs and their
//! contents as built by the game's own display generators
//! (`creative-tabs.json`), a 9x5 scrolling grid and the search tab.
use crate::Result;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

#[derive(Clone, Debug)]
pub struct Tab {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub items: Vec<String>,
    pub top: bool,
    pub column: u32,
    pub search: bool,
    pub inventory: bool,
}
pub struct Creative {
    pub tabs: Vec<Tab>,
    pub selected: usize,
    /// Scroll position 0..1.
    pub scroll: f32,
    pub search: String,
    all_items: Vec<String>,
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
    pub fn load(root: &Path) -> Result<Self> {
        let raw: Value = serde_json::from_slice(&fs::read(root.join("creative-tabs.json"))?)?;
        let mut tabs = Vec::new();
        let mut all_items: Vec<String> = Vec::new();
        for t in raw.as_array().into_iter().flatten() {
            let items: Vec<String> = t["items"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
            for i in &items {
                if !all_items.contains(i) {
                    all_items.push(i.clone());
                }
            }
            let kind = t["type"].as_str().unwrap_or("");
            if t["id"] == "minecraft:hotbar" || t["id"] == "minecraft:op_blocks" {
                continue;
            }
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
        Ok(Self { tabs, selected: 0, scroll: 0.0, search: String::new(), all_items })
    }
    pub fn tab(&self) -> &Tab {
        &self.tabs[self.selected.min(self.tabs.len() - 1)]
    }
    /// Items shown by the selected tab (the search tab filters by name).
    pub fn items(&self, names: &Names) -> Vec<String> {
        let tab = self.tab();
        if tab.search {
            let query = self.search.to_lowercase();
            return self.all_items.iter().filter(|i| query.is_empty() || names.item(i).to_lowercase().contains(&query) || i.contains(&query)).cloned().collect();
        }
        tab.items.clone()
    }
    pub fn rows(&self, names: &Names) -> usize {
        self.items(names).len().div_ceil(9)
    }
    /// First visible grid row for the scroll position.
    pub fn first_row(&self, names: &Names) -> usize {
        let hidden = self.rows(names).saturating_sub(5);
        (self.scroll * hidden as f32 + 0.5) as usize
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
