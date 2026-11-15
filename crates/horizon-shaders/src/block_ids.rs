//! `block.properties` (spec 4.1): `block.<id>=<block> <block> ...`, each block
//! `[namespace:]name` with optional state filters `:<property>=<v1>,<v2>`. Values in one filter
//! are alternatives; separate filters must all match. Names the game does not know are simply
//! never asked for. Block tags (`%tag`, tier E) resolve against the current world's tags.

use crate::Properties;

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    id: i32,
    /// `minecraft:stone`.
    name: String,
    filters: Vec<(String, Vec<String>)>,
    from_tag: bool,
}

/// The ids `block.properties` gives block states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockIds {
    rules: Vec<Rule>,
    /// Unexpanded declarations, retained so a reload can replace all tag members.
    tags: Vec<Rule>,
    /// Lines skipped and why.
    pub notes: Vec<String>,
}

impl BlockIds {
    pub fn parse(properties: &Properties) -> Self {
        Self::parse_kind(properties, "block")
    }

    /// The same for `<kind>.<id>=` lines: `entity` in `entity.properties`, `item` in
    /// `item.properties` (4.1); names there carry no state filters.
    pub fn parse_kind(properties: &Properties, kind: &str) -> Self {
        let mut out = Self::default();
        let prefix = format!("{kind}.");
        for e in &properties.entries {
            let Some(id) = e.key.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let Ok(id) = id.parse::<i32>() else {
                // `layer.*` (4.2) and other keys are not ids.
                continue;
            };
            for word in e.value.split_whitespace() {
                let (word, from_tag) = match word.strip_prefix('%') {
                    Some(tag) if kind == "block" => (tag, true),
                    Some(_) => {
                        out.notes.push(format!(
                            "line {}: tags are only supported for blocks: {word}",
                            e.line
                        ));
                        continue;
                    }
                    None => (word, false),
                };
                let (names, filters): (Vec<&str>, Vec<&str>) =
                    word.split(':').partition(|p| !p.contains('='));
                let name = match names[..] {
                    [name] => format!("minecraft:{name}"),
                    [namespace, name] => format!("{namespace}:{name}"),
                    _ => {
                        out.notes
                            .push(format!("line {}: not a block name: {word}", e.line));
                        continue;
                    }
                };
                let filters = filters
                    .iter()
                    .filter_map(|f| f.split_once('='))
                    .map(|(k, v)| (k.to_owned(), v.split(',').map(str::to_owned).collect()))
                    .collect();
                let rule = Rule {
                    id,
                    name,
                    filters,
                    from_tag,
                };
                if from_tag {
                    out.tags.push(rule);
                } else {
                    out.rules.push(rule);
                }
            }
        }
        out
    }

    /// Resolve `%tag` declarations against a world's current block tags. Re-resolving
    /// replaces earlier members; missing tags contribute no assignments.
    pub fn with_block_tags(
        &self,
        tags: &std::collections::HashMap<String, std::collections::HashSet<String>>,
    ) -> Self {
        let mut out = self.clone();
        out.rules.retain(|r| !r.from_tag);
        for rule in &self.tags {
            for member in tags.get(&rule.name).into_iter().flatten() {
                let mut expanded = rule.clone();
                expanded.name.clone_from(member);
                out.rules.push(expanded);
            }
        }
        out
    }

    pub fn has_block_tags(&self) -> bool {
        !self.tags.is_empty()
    }

    /// The id of block `name` (`minecraft:stone`) in the state with `properties`. The rule with
    /// the most filters that all match wins, then the first in the file. Matching individual
    /// assignments take priority over tag assignments, regardless of filter count.
    pub fn id<'a>(&self, name: &str, properties: impl Fn(&str) -> Option<&'a str>) -> Option<i32> {
        self.rules
            .iter()
            .filter(|r| r.name == name)
            .filter(|r| {
                r.filters
                    .iter()
                    .all(|(k, vs)| properties(k).is_some_and(|v| vs.iter().any(|x| x == v)))
            })
            .min_by_key(|r| (r.from_tag, std::cmp::Reverse(r.filters.len())))
            .map(|r| r.id)
    }

    /// Every name without state filters and its id (the first line naming it wins): what
    /// `entity.properties` and `item.properties` hold.
    pub fn plain(&self) -> std::collections::HashMap<String, i32> {
        let mut out = std::collections::HashMap::new();
        for r in self.rules.iter().filter(|r| r.filters.is_empty()) {
            out.entry(r.name.clone()).or_insert(r.id);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.tags.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    #[test]
    fn names_states_and_precedence() {
        let text = "block.10=stone minecraft:dirt mod:thing \\\n grass_block:snowy=false\n\
                    block.11=grass_block\nblock.12=oak_leaves:persistent=true:distance=1,2 %leaves\n\
                    layer.translucent=glass\n";
        let ids = BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let none = |_: &str| None;
        assert_eq!(ids.id("minecraft:stone", none), Some(10));
        assert_eq!(ids.id("mod:thing", none), Some(10));
        assert_eq!(ids.id("minecraft:glass", none), None);
        let snowy = |s: &'static str| move |k: &str| (k == "snowy").then_some(s);
        // The filtered entry is the more specific one.
        assert_eq!(ids.id("minecraft:grass_block", snowy("false")), Some(10));
        assert_eq!(ids.id("minecraft:grass_block", snowy("true")), Some(11));
        let leaves = |p: &'static str, d: &'static str| {
            move |k: &str| match k {
                "persistent" => Some(p),
                "distance" => Some(d),
                _ => None,
            }
        };
        assert_eq!(
            ids.id("minecraft:oak_leaves", leaves("true", "2")),
            Some(12)
        );
        assert_eq!(ids.id("minecraft:oak_leaves", leaves("true", "3")), None);
        assert_eq!(ids.id("minecraft:oak_leaves", leaves("false", "1")), None);
        assert!(ids.notes.is_empty(), "{:?}", ids.notes);
    }

    #[test]
    fn block_tags_resolve_current_members_with_individual_priority() {
        use std::collections::{HashMap, HashSet};

        let parse = |text| BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        // A tag-only file is still an ID mapping file, even before its members are known.
        assert!(!parse("block.20=%grove").is_empty());
        let declared = parse(
            "block.30=oak_log\n\
            block.20=%grove:axis=y,z:waterlogged=false\n\
            block.31=birch_log:axis=x\n\
            block.21=%demo:grove\nblock.90=%missing\n",
        );
        assert!(declared.notes.is_empty(), "{:?}", declared.notes);
        let mut tags: HashMap<String, HashSet<String>> = HashMap::from([
            (
                "minecraft:grove".into(),
                HashSet::from(["minecraft:oak_log".into(), "minecraft:birch_log".into()]),
            ),
            ("demo:grove".into(), HashSet::from(["demo:stem".into()])),
        ]);
        let ids = declared.with_block_tags(&tags);
        let state = |axis| {
            move |key: &str| match key {
                "axis" => Some(axis),
                "waterlogged" => Some("false"),
                _ => None,
            }
        };
        // Individuals before and after tag declarations win when they match.
        // The unfiltered individual also beats a tag with two matching filters.
        assert_eq!(ids.id("minecraft:oak_log", state("y")), Some(30));
        assert_eq!(ids.id("minecraft:birch_log", state("x")), Some(31));
        assert_eq!(ids.id("minecraft:birch_log", state("z")), Some(20));
        assert_eq!(ids.id("minecraft:birch_log", |_| None), None);
        assert_eq!(
            ids.id("minecraft:birch_log", |k| match k {
                "axis" => Some("y"),
                "waterlogged" => Some("true"),
                _ => None,
            }),
            None
        );
        assert_eq!(ids.id("demo:stem", |_| None), Some(21));
        assert_eq!(ids.id("minecraft:missing", |_| None), None);
        assert_eq!(ids.id("minecraft:stone", |_| None), None);

        // A datapack replacement removes old members as well as adding new ones.
        tags.insert(
            "minecraft:grove".into(),
            HashSet::from(["minecraft:spruce_log".into()]),
        );
        tags.remove("demo:grove");
        let reloaded = ids.with_block_tags(&tags);
        assert_eq!(reloaded.id("minecraft:birch_log", state("y")), None);
        assert_eq!(reloaded.id("minecraft:spruce_log", state("y")), Some(20));
        assert_eq!(reloaded.id("demo:stem", |_| None), None);
        assert_eq!(reloaded.id("minecraft:oak_log", state("y")), Some(30));
        assert_eq!(reloaded.id("minecraft:birch_log", state("x")), Some(31));
        assert_eq!(declared.id("minecraft:spruce_log", state("y")), None);
    }

    #[test]
    fn entity_lines_by_name() {
        let text = "entity.10=zombie minecraft:husk\nentity.11=zombie mod:thing\nblock.5=stone\n";
        let ids = BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), "entity");
        let plain = ids.plain();
        assert_eq!(
            plain["minecraft:zombie"], 10,
            "the first line naming it wins"
        );
        assert_eq!(plain["minecraft:husk"], 10);
        assert_eq!(plain["mod:thing"], 11);
        assert!(!plain.contains_key("minecraft:stone"));
    }
}
