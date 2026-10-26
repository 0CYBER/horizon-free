//! `block.properties` (spec 4.1): `block.<id>=<block> <block> ...`, each block
//! `[namespace:]name` with optional state filters `:<property>=<v1>,<v2>`. Values in one filter
//! are alternatives; separate filters must all match. Names the game does not know are simply
//! never asked for. Block tags (`%tag`, tier E) are not read yet.

use crate::Properties;

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    id: i32,
    /// `minecraft:stone`.
    name: String,
    filters: Vec<(String, Vec<String>)>,
}

/// The ids `block.properties` gives block states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockIds {
    rules: Vec<Rule>,
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
                if word.starts_with('%') {
                    out.notes.push(format!(
                        "line {}: block tags are not read yet: {word}",
                        e.line
                    ));
                    continue;
                }
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
                out.rules.push(Rule { id, name, filters });
            }
        }
        out
    }

    /// The id of block `name` (`minecraft:stone`) in the state with `properties`. The rule with
    /// the most filters that all match wins, then the first in the file (one name belongs to
    /// one id; a filtered entry is the more specific one).
    pub fn id<'a>(&self, name: &str, properties: impl Fn(&str) -> Option<&'a str>) -> Option<i32> {
        self.rules
            .iter()
            .filter(|r| r.name == name)
            .filter(|r| {
                r.filters
                    .iter()
                    .all(|(k, vs)| properties(k).is_some_and(|v| vs.iter().any(|x| x == v)))
            })
            .min_by_key(|r| std::cmp::Reverse(r.filters.len()))
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
        self.rules.is_empty()
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
        assert_eq!(ids.notes.len(), 1, "{:?}", ids.notes);
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
