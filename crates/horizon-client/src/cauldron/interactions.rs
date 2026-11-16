//! Resolved cauldron dispatch, shared by vanilla and mod entries.
use super::{Action, COLORS};
/// Prebind vanilla entries directly into a preparation candidate.
fn vanilla<H>() -> [std::collections::HashMap<String, Entry<H>>; 4] {
    use Action::*;
    let mut maps = std::array::from_fn::<_, 4, _>(|_| std::collections::HashMap::new());
    for map in &mut maps {
        for (item, action) in [
            ("lava_bucket", EmptyLava),
            ("water_bucket", EmptyWater),
            ("powder_snow_bucket", EmptySnow),
        ] {
            map.insert(format!("minecraft:{item}"), Entry::Vanilla(action));
        }
    }
    for (kind, item, action) in [
        (0, "potion", EmptyPotion),
        (1, "bucket", FillWater),
        (1, "glass_bottle", FillBottle),
        (1, "potion", TopUpPotion),
        (2, "bucket", FillLava),
        (3, "bucket", FillSnow),
    ] {
        maps[kind].insert(format!("minecraft:{item}"), Entry::Vanilla(action));
    }
    for color in COLORS {
        maps[1].insert(
            format!("minecraft:{color}_banner"),
            Entry::Vanilla(CleanBanner),
        );
        maps[1].insert(
            format!("minecraft:{color}_shulker_box"),
            Entry::Vanilla(CleanShulker),
        );
    }
    maps
}

use std::{collections::HashMap, sync::OnceLock};
/// One entry in a prepared cauldron dispatcher.
#[derive(Clone)]
pub enum Entry<H> {
    /// Prebound vanilla operation.
    Vanilla(super::Action),
    /// Engine-owned mod callback handle.
    Mod(H),
    /// Terminal vanilla dye-tag branch.
    Dye,
}
/// A single immutable lookup table after preparation.
#[derive(Clone)]
pub struct Dispatcher<H> {
    maps: [HashMap<String, Entry<H>>; 4],
    custom: HashMap<String, HashMap<String, Entry<H>>>,
    /// Item-tag entries by block, in registration order; asked after the exact entries.
    tagged: HashMap<String, Vec<(String, H)>>,
}
impl<H> Default for Dispatcher<H> {
    fn default() -> Self {
        Self {
            maps: vanilla(),
            custom: HashMap::new(),
            tagged: HashMap::new(),
        }
    }
}
fn index(block: &str) -> Option<usize> {
    match block {
        "minecraft:cauldron" => Some(0),
        "minecraft:water_cauldron" => Some(1),
        "minecraft:lava_cauldron" => Some(2),
        "minecraft:powder_snow_cauldron" => Some(3),
        _ => None,
    }
}
impl Dispatcher<()> {
    /// Vanilla-only snapshot for resource-independent callers.
    pub fn vanilla() -> &'static Self {
        static TABLE: OnceLock<Dispatcher<()>> = OnceLock::new();
        TABLE.get_or_init(Self::default)
    }
}
impl<H> Dispatcher<H> {
    /// The dye tag wins terminally, including when its handler passes; then the exact
    /// item entry, then the first item-tag entry whose tag `in_tag` says holds the item,
    /// read against the tags loaded now.
    pub fn select(
        &self,
        block: &str,
        item: &str,
        removes_dye: bool,
        in_tag: impl Fn(&str) -> bool,
    ) -> Option<Entry<&H>> {
        if block == "minecraft:water_cauldron" && removes_dye {
            return Some(Entry::Dye);
        }
        let exact = match index(block) {
            Some(i) => self.maps[i].get(item),
            None => self.custom.get(block).and_then(|map| map.get(item)),
        };
        if let Some(entry) = exact {
            return Some(match entry {
                Entry::Vanilla(action) => Entry::Vanilla(*action),
                Entry::Mod(handler) => Entry::Mod(handler),
                Entry::Dye => Entry::Dye,
            });
        }
        let (_, handler) = self
            .tagged
            .get(block)?
            .iter()
            .find(|(tag, _)| in_tag(tag))?;
        Some(Entry::Mod(handler))
    }
    /// Explicit membership rather than a block-name convention.
    pub fn contains_block(&self, block: &str) -> bool {
        index(block).is_some() || self.custom.contains_key(block)
    }
    /// Preparation-only insertion. Returns the occupied entry without replacing it.
    pub fn register(&mut self, block: String, item: String, handler: H) -> Result<(), &Entry<H>> {
        let map = match index(&block) {
            Some(i) => &mut self.maps[i],
            None => self.custom.entry(block).or_default(),
        };
        match map.entry(item) {
            std::collections::hash_map::Entry::Occupied(entry) => Err(entry.into_mut()),
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Entry::Mod(handler));
                Ok(())
            }
        }
    }
    /// Preparation-only insertion of an item-tag entry, which declares a custom block as
    /// `register` does. Returns the handler already holding the tag on that block.
    pub fn register_tag(&mut self, block: String, tag: String, handler: H) -> Result<(), &H> {
        if index(&block).is_none() {
            self.custom.entry(block.clone()).or_default();
        }
        let entries = self.tagged.entry(block).or_default();
        match entries.iter().position(|(held, _)| *held == tag) {
            Some(at) => Err(&entries[at].1),
            None => {
                entries.push((tag, handler));
                Ok(())
            }
        }
    }
    /// Complete prepared dispatcher set, including declared custom blocks.
    pub fn blocks(&self) -> Vec<String> {
        let mut blocks = [
            "minecraft:cauldron",
            "minecraft:water_cauldron",
            "minecraft:lava_cauldron",
            "minecraft:powder_snow_cauldron",
        ]
        .map(str::to_owned)
        .to_vec();
        blocks.extend(self.custom.keys().cloned());
        blocks.sort();
        blocks
    }
}
