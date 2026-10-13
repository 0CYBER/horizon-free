//! A player's advancements on a server (PlayerAdvancements): on the
//! player's first tick the `minecraft:tick` criteria are met, the
//! advancements they complete grant their recipes (ServerRecipeBook.addRecipes:
//! ClientboundRecipeBookAddPacket) and the advancements the player may see go
//! out in ClientboundUpdateAdvancementsPacket (PlayerAdvancements.flushDirty).
use super::data::{SyncData, json_to_nbt};
use crate::{advancements::Tree, recipe_book::RecipeIndex};
use horizon_protocol::buf::Writer;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

pub struct ServerAdvancements {
    tree: Tree,
    kids: Vec<Vec<usize>>,
    layout: Vec<Option<(i32, f32)>>,
    /// Recipe files by id.
    recipes: BTreeMap<String, Value>,
    displays: RecipeIndex,
    /// RecipeManager.unpackRecipeInfo's group numbers.
    groups: HashMap<String, i32>,
}

/// What the first tick sends: ClientboundRecipeBookAddPacket's payload (when
/// recipes were unlocked) and ClientboundUpdateAdvancementsPacket's.
pub struct FirstTick {
    pub recipe_book_add: Option<Vec<u8>>,
    pub update_advancements: Vec<u8>,
}

impl ServerAdvancements {
    pub fn load(resources: &Path) -> Self {
        let tree = Tree::load(&resources.join("data"));
        let kids = tree.children();
        let layout = tree.layout();
        let mut recipes = BTreeMap::new();
        super::recipes::recipes(&resources.join("data/minecraft/recipe"), "", &mut recipes);
        let mut groups = HashMap::new();
        for r in recipes.values() {
            if let Some(g) = r["group"].as_str().filter(|g| !g.is_empty()) {
                let next = groups.len() as i32;
                groups.entry(g.to_owned()).or_insert(next);
            }
        }
        Self {
            tree,
            kids,
            layout,
            recipes,
            displays: RecipeIndex::load(resources),
            groups,
        }
    }

    /// The first tick of a player without advancements.
    pub fn first_tick(&self, sync: &SyncData, now_millis: i64) -> FirstTick {
        // CriteriaTriggers.TICK: the criteria it meets, and what they complete.
        let mut obtained: Vec<Vec<&str>> = vec![Vec::new(); self.tree.list.len()];
        for (i, a) in self.tree.list.iter().enumerate() {
            for c in &a.criteria {
                if c.trigger == "minecraft:tick" && c.conditions.is_null() {
                    obtained[i].push(&c.name);
                }
            }
        }
        let done: Vec<bool> = self
            .tree
            .list
            .iter()
            .zip(&obtained)
            .map(|(a, got)| {
                !a.requirements.is_empty()
                    && a.requirements
                        .iter()
                        .all(|group| group.iter().any(|c| got.contains(&c.as_str())))
            })
            .collect();
        // AdvancementRewards.grant: the recipes, in the advancements' order.
        let mut entries = Vec::new();
        for (i, a) in self.tree.list.iter().enumerate() {
            if !done[i] {
                continue;
            }
            for id in &a.rewards.recipes {
                entries.extend(self.book_entries(id, sync));
            }
        }
        let recipe_book_add = (!entries.is_empty()).then(|| {
            let mut w = Writer::new();
            w.varint(entries.len() as i32);
            for e in entries {
                w.data.extend(e);
            }
            w.bool(false); // not a replacement
            w.data
        });
        // flushDirty: every advancement the player may see, and its progress.
        let visible = self.tree.visible(&self.kids, |i| done[i]);
        let shown: Vec<usize> = (0..self.tree.list.len()).filter(|i| visible[*i]).collect();
        let mut w = Writer::new();
        w.bool(true); // the first packet resets the client's advancements
        w.varint(shown.len() as i32);
        for &i in &shown {
            let a = &self.tree.list[i];
            w.string(&a.id);
            w.bool(a.parent.is_some());
            if let Some(p) = &a.parent {
                w.string(p);
            }
            w.bool(a.display.is_some());
            if let Some(d) = &a.display {
                w.nbt(&json_to_nbt(&d.title));
                w.nbt(&json_to_nbt(&d.description));
                w.varint(sync.id("minecraft:item", &d.icon).unwrap_or(0));
                w.varint(1);
                w.varint(0);
                w.varint(0);
                w.varint(match d.frame {
                    crate::advancements::Frame::Task => 0,
                    crate::advancements::Frame::Challenge => 1,
                    crate::advancements::Frame::Goal => 2,
                });
                w.i32(
                    i32::from(d.background.is_some())
                        | i32::from(d.show_toast) << 1
                        | i32::from(d.hidden) << 2,
                );
                if let Some(b) = &d.background {
                    w.string(b);
                }
            }
            w.varint(a.requirements.len() as i32);
            for group in &a.requirements {
                w.varint(group.len() as i32);
                for c in group {
                    w.string(c);
                }
            }
            w.bool(false); // sends_telemetry_event
            let (x, y) = self.layout[i].unwrap_or((0, 0.0));
            w.f32(x as f32);
            w.f32(y);
        }
        w.varint(0); // nothing removed
        w.varint(shown.len() as i32);
        for &i in &shown {
            let a = &self.tree.list[i];
            w.string(&a.id);
            w.varint(a.criteria.len() as i32);
            for c in &a.criteria {
                w.string(&c.name);
                let got = obtained[i].contains(&c.name.as_str());
                w.bool(got);
                if got {
                    w.i64(now_millis);
                }
            }
        }
        w.bool(true); // show the advancements
        FirstTick {
            recipe_book_add,
            update_advancements: w.data,
        }
    }

    /// ClientboundRecipeBookAddPacket.Entry for each display of a recipe:
    /// highlighted, and with a notification unless the recipe turns it off.
    fn book_entries(&self, id: &str, sync: &SyncData) -> Vec<Vec<u8>> {
        let Some(r) = self.recipes.get(id) else {
            return Vec::new();
        };
        let ids = self.displays.display_ids(id);
        if ids.len() != 1 {
            // Special recipes have no display; transmutations aren't unlocked on joining.
            return Vec::new();
        }
        let kind = r["type"].as_str().unwrap_or("");
        let item = |n: &str| sync.id("minecraft:item", n).unwrap_or(0);
        let slot = |n: &str| sync.id("minecraft:slot_display", n).unwrap_or(0);
        let mut w = Writer::new();
        w.varint(ids.start);
        let (ingredients, category): (Vec<Option<&Value>>, &str) = match kind {
            "minecraft:crafting_shaped" => {
                let pattern = shrink(r["pattern"].as_array().map_or(&[][..], |p| p));
                let (width, height) = (pattern.first().map_or(0, Vec::len), pattern.len());
                w.varint(sync.id("minecraft:recipe_display", kind).unwrap_or(0));
                w.varint(width as i32);
                w.varint(height as i32);
                let cells: Vec<Option<&Value>> = pattern
                    .iter()
                    .flatten()
                    .map(|c| (*c != ' ').then(|| &r["key"][c.to_string()]))
                    .collect();
                (cells, "misc")
            }
            "minecraft:crafting_shapeless" => {
                w.varint(sync.id("minecraft:recipe_display", kind).unwrap_or(0));
                let cells = r["ingredients"]
                    .as_array()
                    .map_or(Vec::new(), |a| a.iter().map(Some).collect());
                (cells, "misc")
            }
            _ => return Vec::new(),
        };
        // ShapedCraftingRecipeDisplay / ShapelessCraftingRecipeDisplay:
        // the ingredients' TagSlotDisplays, the result, the crafting table.
        w.varint(ingredients.len() as i32);
        for cell in &ingredients {
            match cell {
                Some(v) => {
                    w.varint(slot("minecraft:tag"));
                    holder_set(&mut w, v, &item);
                }
                None => w.varint(slot("minecraft:empty")),
            }
        }
        w.varint(slot("minecraft:item_stack"));
        w.varint(item(r["result"]["id"].as_str().unwrap_or("minecraft:air")));
        w.varint(r["result"]["count"].as_i64().unwrap_or(1) as i32);
        w.varint(0);
        w.varint(0);
        w.varint(slot("minecraft:item"));
        w.varint(item("minecraft:crafting_table"));
        // The group, the recipe book category and the placement's ingredients.
        match r["group"].as_str().and_then(|g| self.groups.get(g)) {
            Some(g) => w.varint(g + 1),
            None => w.varint(0),
        }
        let category = format!(
            "minecraft:crafting_{}",
            match r["category"].as_str().unwrap_or(category) {
                "building" => "building_blocks",
                c => c,
            }
        );
        w.varint(
            sync.id("minecraft:recipe_book_category", &category)
                .unwrap_or(0),
        );
        let placed: Vec<&Value> = ingredients.iter().flatten().copied().collect();
        w.bool(true);
        w.varint(placed.len() as i32);
        for v in placed {
            holder_set(&mut w, v, &item);
        }
        let notify = r["show_notification"].as_bool().unwrap_or(true);
        w.i8(i8::from(notify) | 2);
        vec![w.data]
    }
}

/// ShapedRecipePattern.shrink: the pattern without blank border rows and columns.
fn shrink(rows: &[Value]) -> Vec<Vec<char>> {
    let rows: Vec<Vec<char>> = rows
        .iter()
        .map(|r| r.as_str().unwrap_or("").chars().collect())
        .collect();
    let used = |r: &Vec<char>| r.iter().any(|c| *c != ' ');
    let (Some(top), Some(bottom)) = (rows.iter().position(used), rows.iter().rposition(used))
    else {
        return Vec::new();
    };
    let rows = &rows[top..=bottom];
    let left = rows
        .iter()
        .filter_map(|r| r.iter().position(|c| *c != ' '))
        .min()
        .unwrap_or(0);
    let right = rows
        .iter()
        .filter_map(|r| r.iter().rposition(|c| *c != ' '))
        .max()
        .unwrap_or(0);
    rows.iter()
        .map(|r| {
            (left..=right)
                .map(|x| r.get(x).copied().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// ByteBufCodecs.holderSet for an ingredient: a tag by name, or the items.
fn holder_set(w: &mut Writer, v: &Value, item: &dyn Fn(&str) -> i32) {
    match v {
        Value::String(s) if s.starts_with('#') => {
            w.varint(0);
            w.string(&s[1..]);
        }
        Value::String(s) => {
            w.varint(2);
            w.varint(item(s));
        }
        Value::Array(a) => {
            w.varint(a.len() as i32 + 1);
            for s in a {
                w.varint(item(s.as_str().unwrap_or("")));
            }
        }
        _ => w.varint(1),
    }
}
