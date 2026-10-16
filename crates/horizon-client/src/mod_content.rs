//! Content mods in the resource root (docs/modpacks.md, registries 0.2).
//!
//! Every loader reads one resource root, and most of its block and item
//! tables are exports from vanilla indexed by block-state id or keyed by
//! name. A content mod's block copies a vanilla *template*. So the mods'
//! root is an overlay, like the resource-pack one:
//! - the base root linked, with the per-state and per-name tables
//!   rewritten: each mod block gets its template's rows appended (states
//!   after vanilla's, mods and blocks in id order) with names, translation
//!   keys and in-table state ids moved over, and the declared overrides
//!   applied;
//! - each mod's own `assets/` and `data/` files;
//! - additions to tags merged into the tag files and bundles.
//!
//! The loaders need no mod code, vanilla state ids never move, and with no
//! content mod the root is the base root itself.
use crate::{Result, blocks::MAP_COLORS};
use horizon_mods::content::{Block, Content, Rarity};
use horizon_world::nbt::{self, JavaString, Limits, Tag};
use serde::{Deserialize, Serialize, de};
use std::{
    collections::BTreeMap,
    fmt, fs,
    ops::Range,
    path::{Path, PathBuf},
};

/// A content mod to put in the root: its id and content.
pub struct ContentMod {
    pub id: String,
    pub content: Content,
}

/// JSON with its object keys in file order, so a rewritten table differs
/// from the original only by what was added.
#[derive(Clone, Debug, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}
impl<'de> Deserialize<'de> for J {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = J;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_unit<E>(self) -> std::result::Result<J, E> {
                Ok(J::Null)
            }
            fn visit_bool<E>(self, v: bool) -> std::result::Result<J, E> {
                Ok(J::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> std::result::Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_u64<E>(self, v: u64) -> std::result::Result<J, E> {
                Ok(J::Num(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<J, E> {
                serde_json::Number::from_f64(v)
                    .map(J::Num)
                    .ok_or_else(|| E::custom("not a JSON number"))
            }
            fn visit_str<E>(self, v: &str) -> std::result::Result<J, E> {
                Ok(J::Str(v.to_owned()))
            }
            fn visit_string<E>(self, v: String) -> std::result::Result<J, E> {
                Ok(J::Str(v))
            }
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<J, A::Error> {
                let mut out = Vec::new();
                while let Some(v) = a.next_element()? {
                    out.push(v);
                }
                Ok(J::Arr(out))
            }
            fn visit_map<A: de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<J, A::Error> {
                let mut out = Vec::new();
                while let Some(kv) = a.next_entry()? {
                    out.push(kv);
                }
                Ok(J::Obj(out))
            }
        }
        d.deserialize_any(V)
    }
}
impl Serialize for J {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            J::Null => s.serialize_unit(),
            J::Bool(b) => s.serialize_bool(*b),
            J::Num(n) => n.serialize(s),
            J::Str(t) => s.serialize_str(t),
            J::Arr(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            J::Obj(o) => {
                let mut map = s.serialize_map(Some(o.len()))?;
                for (k, v) in o {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}
impl J {
    fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut J> {
        match self {
            J::Obj(o) => o.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Sets `key`, in place if present, else at the end.
    fn set(&mut self, key: &str, value: J) {
        if let Some(v) = self.get_mut(key) {
            *v = value;
        } else if let J::Obj(o) = self {
            o.push((key.to_owned(), value));
        }
    }
    /// The value at a `/`-separated path ("" is the document).
    fn at(&mut self, path: &str) -> Option<&mut J> {
        path.split('/')
            .filter(|p| !p.is_empty())
            .try_fold(self, |j, key| j.get_mut(key))
    }
    fn f64(v: f64) -> J {
        serde_json::Number::from_f64(v).map_or(J::Null, J::Num)
    }
    fn as_u32(&self) -> Option<u32> {
        match self {
            J::Num(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
            _ => None,
        }
    }
    /// Renames strings and, with `ids`, moves state ids, everywhere inside.
    fn rename(&mut self, r: &Rename, ids: bool) {
        match self {
            J::Str(s) => {
                if let Some(new) = r.string(s) {
                    *s = new;
                }
            }
            J::Num(n) if ids => {
                if let Some(new) = n
                    .as_u64()
                    .and_then(|v| u32::try_from(v).ok())
                    .and_then(|id| r.id(id))
                {
                    *n = new.into();
                }
            }
            J::Arr(a) => a.iter_mut().for_each(|v| v.rename(r, ids)),
            J::Obj(o) => o.iter_mut().for_each(|(_, v)| v.rename(r, ids)),
            _ => {}
        }
    }
}

/// One id moving to another: `minecraft:iron_block` → `ruby:ruby_block`,
/// with its state range for blocks.
#[derive(Clone)]
struct Rename {
    from: String,
    to: String,
    states: Range<u32>,
    first: u32,
}
impl Rename {
    fn new(from: &str, to: &str) -> Self {
        Rename {
            from: from.to_owned(),
            to: to.to_owned(),
            states: 0..0,
            first: 0,
        }
    }
    /// The id itself, `Block{id}`, and `block.<ns>.<path>` / `item.<ns>.<path>` keys.
    fn string(&self, s: &str) -> Option<String> {
        if s == self.from {
            return Some(self.to.clone());
        }
        if s.strip_prefix("Block{").and_then(|x| x.strip_suffix('}')) == Some(&self.from) {
            return Some(format!("Block{{{}}}", self.to));
        }
        let key = |id: &str| id.replacen(':', ".", 1).replace('/', ".");
        for kind in ["block", "item"] {
            if s.strip_prefix(kind).and_then(|x| x.strip_prefix('.')) == Some(&key(&self.from)) {
                return Some(format!("{kind}.{}", key(&self.to)));
            }
        }
        None
    }
    fn id(&self, id: u32) -> Option<u32> {
        self.states
            .contains(&id)
            .then(|| self.first + (id - self.states.start))
    }
}

/// A block being added: its declaration and where its states go.
struct NewBlock<'a> {
    decl: &'a Block,
    rename: Rename,
    /// The template's item, if it has one.
    item: Option<String>,
}
/// An item being added (declared, or a block's).
struct NewItem {
    rename: Rename,
    max_stack_size: Option<u8>,
    rarity: Option<Rarity>,
}

struct Plan<'a> {
    blocks: Vec<NewBlock<'a>>,
    items: Vec<NewItem>,
    /// Vanilla's state count (the tables' length before).
    states: usize,
    /// Each vanilla block's default state.
    defaults: BTreeMap<String, u32>,
    /// The gameplay rows' sound objects, by block, for `sounds` overrides.
    sounds: BTreeMap<String, J>,
}

fn read_json(path: &Path) -> Result<J> {
    Ok(serde_json::from_slice(&fs::read(path)?).map_err(|e| format!("{}: {e}", path.display()))?)
}

impl<'a> Plan<'a> {
    fn new(base: &Path, mods: &'a [ContentMod]) -> Result<Self> {
        let mut facts = read_json(&base.join("facts.json"))?;
        let names: Vec<String> = match facts.at("states") {
            Some(J::Arr(states)) => states
                .iter()
                .map(|s| match s {
                    J::Arr(row) => match row.first() {
                        Some(J::Str(name)) => Ok(name.clone()),
                        _ => Err("facts.json: a state without a name"),
                    },
                    _ => Err("facts.json: a state that is not a row"),
                })
                .collect::<std::result::Result<_, _>>()?,
            _ => return Err("facts.json: no states".into()),
        };
        let defaults: BTreeMap<String, u32> = match facts.at("defaults") {
            Some(J::Obj(o)) => o
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u32()?)))
                .collect(),
            _ => return Err("facts.json: no defaults".into()),
        };
        let range = |name: &str| -> Option<Range<u32>> {
            let start = names.iter().position(|n| n == name)?;
            let len = names[start..].iter().take_while(|n| *n == name).count();
            // A block's states are one run in registry order.
            (names[start + len..].iter().all(|n| n != name))
                .then_some(start as u32..(start + len) as u32)
        };
        let mut blocks = Vec::new();
        let mut next = names.len() as u32;
        let mut sorted: Vec<&ContentMod> = mods.iter().collect();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        let mut wanted_sounds = Vec::new();
        for m in &sorted {
            let mut declared: Vec<&Block> = m.content.blocks.iter().collect();
            declared.sort_by(|a, b| a.id.cmp(&b.id));
            for decl in declared {
                if defaults.contains_key(&decl.id) {
                    return Err(format!("{}: {} is already a block", m.id, decl.id).into());
                }
                let states = range(&decl.template).ok_or_else(|| {
                    format!("{}: template {} is not a block", decl.id, decl.template)
                })?;
                let mut rename = Rename::new(&decl.template, &decl.id);
                rename.first = next;
                next += states.len() as u32;
                rename.states = states;
                if let Some(s) = &decl.properties.sounds {
                    if !defaults.contains_key(s) {
                        return Err(format!("{}: sounds {s} is not a block", decl.id).into());
                    }
                    wanted_sounds.push(s.clone());
                }
                blocks.push(NewBlock {
                    decl,
                    rename,
                    item: None,
                });
            }
        }
        // Block items copy the template block's item.
        let gameplay = read_json(&base.join("gameplay-facts.json"))?;
        let rows = match gameplay.get("states") {
            Some(J::Arr(rows)) if rows.len() == names.len() => rows,
            _ => return Err("gameplay-facts.json does not match facts.json".into()),
        };
        let mut sounds = BTreeMap::new();
        for s in wanted_sounds {
            let row = &rows[defaults[&s] as usize];
            sounds.insert(s, row.get("sound").cloned().unwrap_or(J::Null));
        }
        let mut items = Vec::new();
        for b in &mut blocks {
            let row = &rows[b.rename.states.start as usize];
            b.item = match row.get("item") {
                Some(J::Str(i)) if i != "minecraft:air" => Some(i.clone()),
                _ => None,
            };
            if b.decl.item {
                let item = b.item.as_deref().ok_or_else(|| {
                    format!("{}: template {} has no item", b.decl.id, b.decl.template)
                })?;
                items.push(NewItem {
                    rename: Rename::new(item, &b.decl.id),
                    max_stack_size: None,
                    rarity: None,
                });
            }
        }
        for m in &sorted {
            for i in &m.content.items {
                items.push(NewItem {
                    rename: Rename::new(&i.template, &i.id),
                    max_stack_size: i.max_stack_size,
                    rarity: i.rarity,
                });
            }
        }
        Ok(Plan {
            blocks,
            items,
            states: names.len(),
            defaults,
            sounds,
        })
    }

    /// What moves in a new block's copies of its template's rows: the
    /// block, and the template's item when it has another name. A block
    /// without an item gets `minecraft:air` where rows name one.
    fn renames(&self, b: &NewBlock) -> Vec<Rename> {
        let mut out = vec![b.rename.clone()];
        if let Some(item) = b.item.as_deref().filter(|i| *i != b.decl.template)
            && b.decl.item
        {
            out.push(Rename::new(item, &b.decl.id));
        }
        out
    }

    /// Appends each new block's copies of the template's states to `list`;
    /// `gameplay` rows get their index and the declared overrides.
    fn states(&self, list: &mut Vec<J>, ids: bool, gameplay: bool) -> Result<()> {
        if list.len() != self.states {
            return Err(format!("{} states, not {}", list.len(), self.states).into());
        }
        for b in &self.blocks {
            let renames = self.renames(b);
            for (i, id) in b.rename.states.clone().enumerate() {
                let mut row = list[id as usize].clone();
                for r in &renames {
                    row.rename(r, ids);
                }
                if gameplay {
                    row.set("id", J::Num((b.rename.first + i as u32).into()));
                    if !b.decl.item {
                        row.set("item", J::Str("minecraft:air".into()));
                    }
                    self.override_gameplay(b, &mut row);
                }
                list.push(row);
            }
        }
        Ok(())
    }

    /// The declared properties, on a gameplay row.
    fn override_gameplay(&self, b: &NewBlock, row: &mut J) {
        let p = &b.decl.properties;
        if let Some(v) = p.hardness {
            row.set("hardness", J::f64(v as f64));
        }
        if let Some(v) = p.explosion_resistance {
            row.set("explosion_resistance", J::f64(v as f64));
        }
        if let Some(v) = p.light {
            row.set("emission", J::Num(v.into()));
        }
        if let Some(v) = p.requires_tool {
            row.set("requires_tool", J::Bool(v));
        }
        if let Some(v) = p.friction {
            row.set("friction", J::f64(v as f64));
        }
        if let Some(v) = p.map_color {
            let col = if v == 0 { 0 } else { MAP_COLORS[v as usize] };
            row.set("map_color", J::Num(col.into()));
        }
        if let Some(s) = &p.sounds {
            row.set("sound", self.sounds[s].clone());
        }
    }

    /// Adds each new block's copy of the template's entry to a map by name.
    fn names(&self, map: &mut J, ids: bool) {
        for b in &self.blocks {
            if let Some(v) = map.get(&b.decl.template) {
                let mut v = v.clone();
                for r in &self.renames(b) {
                    v.rename(r, ids);
                }
                map.set(&b.decl.id, v);
            }
        }
    }

    /// Adds each new item's copy of its template's entry to a map by name;
    /// `components` applies the overrides (items.json).
    fn items(&self, map: &mut J, components: bool) {
        for i in &self.items {
            let Some(v) = map.get(&i.rename.from) else {
                continue;
            };
            let mut v = v.clone();
            v.rename(&i.rename, false);
            if components {
                if let Some(n) = i.max_stack_size {
                    v.set("minecraft:max_stack_size", J::Num(n.into()));
                }
                if let Some(r) = i.rarity {
                    let r = match r {
                        Rarity::Common => "common",
                        Rarity::Uncommon => "uncommon",
                        Rarity::Rare => "rare",
                        Rarity::Epic => "epic",
                    };
                    v.set("minecraft:rarity", J::Str(r.into()));
                }
            }
            map.set(&i.rename.to, v);
        }
    }

    /// One JSON table, rewritten.
    fn json(&self, name: &str, doc: &mut J) -> Result<()> {
        let states = |doc: &mut J, path: &str, ids: bool, gameplay: bool| -> Result<()> {
            match doc.at(path) {
                Some(J::Arr(a)) => self.states(a, ids, gameplay),
                _ => Err(format!("no list at /{path}").into()),
            }
        };
        match name {
            "facts.json" => {
                states(doc, "states", false, false)?;
                let defaults = doc.at("defaults").ok_or("no defaults")?;
                for b in &self.blocks {
                    let default = self.defaults[&b.decl.template];
                    let id = b.rename.id(default).ok_or("a default outside its block")?;
                    defaults.set(&b.decl.id, J::Num(id.into()));
                }
            }
            "gameplay-facts.json" => states(doc, "states", false, true)?,
            "block-behaviours.json" => {
                states(doc, "states", false, false)?;
                let Some(J::Arr(blocks)) = doc.at("blocks") else {
                    return Err("no blocks".into());
                };
                for b in &self.blocks {
                    let template = blocks.iter().find(
                        |x| matches!(x.get("name"), Some(J::Str(n)) if *n == b.decl.template),
                    );
                    if let Some(mut v) = template.cloned() {
                        for r in &self.renames(b) {
                            v.rename(r, false);
                        }
                        if !b.decl.item {
                            v.set("item", J::Str("minecraft:air".into()));
                        }
                        blocks.push(v);
                    }
                }
                self.items(doc.at("items").ok_or("no items")?, false);
                for odds in ["fire/burnOdds", "fire/igniteOdds"] {
                    if let Some(map) = doc.at(odds) {
                        self.names(map, false);
                    }
                }
            }
            "model-states.json"
            | "landing-definitions.json"
            | "worldgen/blocks/survival-rules.json" => self.names(doc, false),
            "render-definitions.json" => self.names(doc, true),
            "worldgen/blocks/survival-extra-definitions.json" => {
                if let Some(map) = doc.at("ignition_odds") {
                    self.names(map, false);
                }
            }
            "items.json" => self.items(doc, true),
            "command-registries.json" => {
                self.names(doc.at("blocks").ok_or("no blocks")?, false);
                let Some(J::Arr(items)) = doc.at("items") else {
                    return Err("no items".into());
                };
                items.extend(self.items.iter().map(|i| J::Str(i.rename.to.clone())));
            }
            "mob-data.json" => states(doc, "states", false, false)?,
            "worldgen/block_flags.json" => states(doc, "", false, false)?,
            "worldgen/blocks/structure-blocks.json" => {
                for key in ["rotate", "mirror", "full_block"] {
                    states(doc, key, true, false)?;
                }
                let added: usize = self.blocks.iter().map(|b| b.rename.states.len()).sum();
                doc.set("states", J::Num(((self.states + added) as u64).into()));
            }
            _ => return Err("no rule for this table".into()),
        }
        Ok(())
    }

    /// One NBT table: `states` lists, `cached_shapes` arrays and `blocks`
    /// compounds, wherever they are.
    fn nbt(&self, tag: &mut Tag) {
        let Tag::Compound(c) = tag else {
            return;
        };
        for (key, value) in c.iter_mut() {
            match (key.to_string_lossy().as_str(), value) {
                ("states", Tag::List(list)) if list.len() == self.states => {
                    for b in &self.blocks {
                        for id in b.rename.states.clone() {
                            let mut state = list[id as usize].clone();
                            if let Some(slot) = state.get_mut("id") {
                                *slot = Tag::String(b.decl.id.as_str().into());
                            }
                            list.push(state);
                        }
                    }
                }
                ("cached_shapes", Tag::IntArray(a)) if a.len() == self.states => {
                    for b in &self.blocks {
                        let r = &b.rename.states;
                        let copy = a[r.start as usize..r.end as usize].to_vec();
                        a.extend(copy);
                    }
                }
                ("blocks", Tag::Compound(map)) => {
                    for b in &self.blocks {
                        let Some(mut v) = map
                            .get(&JavaString::from(b.decl.template.as_str()))
                            .cloned()
                        else {
                            continue;
                        };
                        if let (Some(f), Some(slot)) =
                            (b.decl.properties.friction, v.get_mut("friction"))
                        {
                            *slot = Tag::Float(f.to_bits());
                        }
                        map.insert(b.decl.id.as_str().into(), v);
                    }
                }
                (_, inner @ Tag::Compound(_)) => self.nbt(inner),
                _ => {}
            }
        }
    }
}

/// The JSON and NBT tables rewritten for new blocks and items.
const JSON_TABLES: [&str; 13] = [
    "facts.json",
    "gameplay-facts.json",
    "block-behaviours.json",
    "model-states.json",
    "render-definitions.json",
    "landing-definitions.json",
    "items.json",
    "command-registries.json",
    "mob-data.json",
    "worldgen/block_flags.json",
    "worldgen/blocks/structure-blocks.json",
    "worldgen/blocks/survival-rules.json",
    "worldgen/blocks/survival-extra-definitions.json",
];
const NBT_TABLES: [&str; 6] = [
    "occlusion.nbt",
    "cached-collision.nbt",
    "movement-properties.nbt",
    "worldgen/blocks/block-properties.nbt",
    "worldgen/blocks/cached-support.nbt",
    "worldgen/blocks/cached-collision.nbt",
];
/// Bundles of block tag files (tag id → the tag file's text).
const TAG_BUNDLES: [&str; 2] = ["block-tags.json", "worldgen/block_tags.json"];

/// The overlay being written: `out` mirrors `base` through links, and a
/// directory becomes real only when something inside it is written.
struct Overlay<'a> {
    base: &'a Path,
    out: &'a Path,
}
impl Overlay<'_> {
    fn new<'a>(base: &'a Path, out: &'a Path) -> Result<Overlay<'a>> {
        if fs::symlink_metadata(out).is_ok() {
            fs::remove_dir_all(out)?;
        }
        fs::create_dir_all(out)?;
        let overlay = Overlay { base, out };
        overlay.link_children(Path::new(""))?;
        Ok(overlay)
    }
    fn link_children(&self, rel: &Path) -> Result<()> {
        let Ok(read) = fs::read_dir(self.base.join(rel)) else {
            return Ok(());
        };
        for entry in read.flatten() {
            let rel = rel.join(entry.file_name());
            link(&self.base.join(&rel), &self.out.join(&rel))?;
        }
        Ok(())
    }
    /// Makes `rel`'s directories real, each holding links to the base's.
    fn dir(&self, rel: &Path) -> Result<()> {
        let mut at = PathBuf::new();
        for part in rel.iter() {
            at.push(part);
            let path = self.out.join(&at);
            let meta = fs::symlink_metadata(&path);
            if meta
                .as_ref()
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            {
                continue;
            }
            if meta.is_ok() {
                fs::remove_file(&path)?;
            }
            fs::create_dir(&path)?;
            self.link_children(&at)?;
        }
        Ok(())
    }
    fn write(&self, rel: &str, bytes: &[u8]) -> Result<()> {
        let rel = Path::new(rel);
        if let Some(parent) = rel.parent() {
            self.dir(parent)?;
        }
        let path = self.out.join(rel);
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_file(&path)?;
        }
        fs::write(path, bytes)?;
        Ok(())
    }
}
#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}
#[cfg(not(unix))]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        fs::create_dir_all(at)?;
        for entry in fs::read_dir(target)?.flatten() {
            link(&entry.path(), &at.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(target, at).map(|_| ())
    }
}

/// A tag file's JSON text with `more` appended to its values (no repeats).
fn merge_tag(text: Option<&str>, more: &[J], what: &str) -> Result<String> {
    let mut doc = match text {
        Some(t) => serde_json::from_str(t).map_err(|e| format!("{what}: {e}"))?,
        None => J::Obj(vec![("values".into(), J::Arr(Vec::new()))]),
    };
    let Some(J::Arr(values)) = doc.get_mut("values") else {
        return Err(format!("{what}: no values").into());
    };
    for v in more {
        if !values.contains(v) {
            values.push(v.clone());
        }
    }
    Ok(serde_json::to_string_pretty(&doc)?)
}

/// Builds the content root for `mods` over `base` in `out`.
pub fn build(base: &Path, mods: &[ContentMod], out: &Path) -> Result<()> {
    let plan = Plan::new(base, mods)?;
    let overlay = Overlay::new(base, out)?;
    for name in JSON_TABLES {
        let path = base.join(name);
        if !path.exists() {
            continue;
        }
        let mut doc = read_json(&path)?;
        plan.json(name, &mut doc)
            .map_err(|e| format!("{name}: {e}"))?;
        overlay.write(name, &serde_json::to_vec(&doc)?)?;
    }
    for name in NBT_TABLES {
        let path = base.join(name);
        if !path.exists() {
            continue;
        }
        let (mut named, _) = nbt::read_named(&fs::read(&path)?, Limits::default())
            .map_err(|e| format!("{name}: {e}"))?;
        plan.nbt(&mut named.tag);
        let bytes =
            nbt::write_named(&named, Limits::default()).map_err(|e| format!("{name}: {e}"))?;
        overlay.write(name, &bytes)?;
    }
    // Mod files, and every tag addition gathered per tag file.
    let mut tags: BTreeMap<String, Vec<J>> = BTreeMap::new();
    for m in mods {
        let declarations = format!("data/{}/horizon/", m.id);
        for file in &m.content.files {
            let bytes = m.content.file(file)?;
            let is_tag = file.starts_with("data/") && file.contains("/tags/");
            if is_tag && file.ends_with(".json") {
                let doc: J = serde_json::from_slice(&bytes).map_err(|e| format!("{file}: {e}"))?;
                if doc.get("replace") == Some(&J::Bool(true)) {
                    let e = format!(
                        "{}: {file}: a mod adds to tags, it cannot replace them",
                        m.id
                    );
                    return Err(e.into());
                }
                let Some(J::Arr(values)) = doc.get("values") else {
                    return Err(format!("{}: {file}: no values", m.id).into());
                };
                tags.entry(file.clone())
                    .or_default()
                    .extend(values.iter().cloned());
            } else if !file.starts_with(&declarations) {
                overlay.write(file, &bytes)?;
            }
        }
    }
    let mut bundles: Vec<(&str, J)> = Vec::new();
    for name in TAG_BUNDLES {
        if base.join(name).exists() {
            bundles.push((name, read_json(&base.join(name))?));
        }
    }
    for (file, values) in &tags {
        let vanilla = fs::read_to_string(base.join(file)).ok();
        overlay.write(
            file,
            merge_tag(vanilla.as_deref(), values, file)?.as_bytes(),
        )?;
        // data/<ns>/tags/block/<path>.json is the block tag <ns>:<path>.
        let Some((ns, path)) = file
            .strip_prefix("data/")
            .and_then(|r| r.split_once("/tags/block/"))
            .and_then(|(ns, p)| Some((ns, p.strip_suffix(".json")?)))
        else {
            continue;
        };
        let id = format!("{ns}:{path}");
        for (name, doc) in &mut bundles {
            let text = match doc.get(&id) {
                Some(J::Str(t)) => Some(t.clone()),
                _ => None,
            };
            let merged = merge_tag(text.as_deref(), values, name)?;
            doc.set(&id, J::Str(merged));
        }
    }
    for (name, doc) in &bundles {
        overlay.write(name, &serde_json::to_vec(doc)?)?;
    }
    Ok(())
}

/// The root to load: `base` itself with no content, else the mods' overlay
/// over it, built in `cache/mod-content`.
pub fn resource_root(base: &Path, mods: &[ContentMod], cache: &Path) -> Result<PathBuf> {
    if mods.iter().all(|m| m.content.is_empty()) {
        return Ok(base.to_owned());
    }
    let out = cache.join("mod-content");
    build(base, mods, &out)?;
    Ok(out)
}

#[cfg(test)]
#[path = "mod_content_tests.rs"]
mod tests;
