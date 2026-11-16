//! Engine-owned tag reads passed only while evaluating custom loot kinds.

/// Authoritative tag reads without a game or registry borrow.
/// Registry names are vanilla directory names (`item`, `block`, `fluid`).
pub trait LootTags {
    /// Members of one tag in its resolved order. Unknown tags are empty.
    fn members(&self, registry: &str, tag: &str) -> Vec<String>;
    /// Whether one registry value belongs to a tag.
    fn contains(&self, registry: &str, tag: &str, id: &str) -> bool {
        self.members(registry, tag)
            .iter()
            .any(|member| member == id)
    }
}

impl LootTags for super::LootData {
    fn members(&self, registry: &str, tag: &str) -> Vec<String> {
        self.tag(registry, tag)
    }
}

#[cfg(test)]
thread_local! { static STEP_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }

/// Build a requested tag iteratively, each tag it names (adding or removing)
/// built once before it, without a depth limit. A cycle builds nothing.
pub(super) fn members(
    tags: &std::collections::HashMap<(String, String), crate::tag_layers::TagLayers>,
    registry: &str,
    name: &str,
) -> Vec<String> {
    enum Step<'a> {
        Enter(&'a str),
        Exit(&'a str),
    }
    let layers = |name: &str| tags.get(&(registry.to_owned(), name.to_owned()));
    let mut pending = vec![Step::Enter(name)];
    let mut active = std::collections::HashSet::new();
    let mut built: std::collections::HashMap<&str, Vec<String>> = Default::default();
    while let Some(step) = pending.pop() {
        #[cfg(test)]
        STEP_BUDGET.with(|budget| {
            if let Some(remaining) = budget.get() {
                budget.set(Some(
                    remaining
                        .checked_sub(1)
                        .expect("tag traversal repeated a completed subgraph"),
                ));
            }
        });
        match step {
            Step::Enter(name) => {
                if built.contains_key(name) {
                    continue;
                }
                if !active.insert(name) {
                    return Vec::new();
                }
                pending.push(Step::Exit(name));
                if let Some(layers) = layers(name) {
                    pending.extend(layers.tags().map(Step::Enter));
                }
            }
            Step::Exit(name) => {
                active.remove(name);
                let members = layers(name).map_or_else(Vec::new, |layers| {
                    layers.build(|tag| built.get(tag).cloned().unwrap_or_default())
                });
                built.insert(name, members);
            }
        }
    }
    built.remove(name).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_layers::TagLayers;
    use serde_json::json;

    fn layers(values: Vec<serde_json::Value>) -> TagLayers {
        TagLayers::from_file(&json!({ "values": values }))
    }

    #[test]
    fn a_removal_takes_out_an_entry_or_a_nested_tags_members() {
        let mut tags = std::collections::HashMap::new();
        let mut things = layers(vec![json!("minecraft:stone"), json!("#test:other")]);
        things.push(&json!({"values": [], "remove": ["minecraft:stone"]}));
        tags.insert(("item".into(), "test:things".into()), things);
        tags.insert(
            ("item".into(), "test:other".into()),
            layers(vec![json!("minecraft:dirt"), json!("minecraft:sand")]),
        );
        let mut gone = layers(vec![json!("#test:things")]);
        gone.push(&json!({"values": [], "remove": ["#test:other"]}));
        tags.insert(("item".into(), "test:gone".into()), gone);
        assert_eq!(
            members(&tags, "item", "test:things"),
            ["minecraft:dirt", "minecraft:sand"]
        );
        assert!(members(&tags, "item", "test:gone").is_empty());
    }
    #[test]
    fn repeated_shared_subtags_are_visited_once_without_exponential_work() {
        let mut tags = std::collections::HashMap::new();
        for index in 0..128 {
            let next = json!(format!("#test:{}", index + 1));
            tags.insert(
                ("item".into(), format!("test:{index}")),
                layers(vec![next.clone(), next]),
            );
        }
        tags.insert(
            ("item".into(), "test:128".into()),
            layers(vec![json!("minecraft:stick")]),
        );
        // A deterministic test-only work budget kills repeated traversal without a
        // timing assertion or allowing an exponential mutant to hang the suite.
        STEP_BUDGET.with(|budget| budget.set(Some(4096)));
        let result = members(&tags, "item", "test:0");
        STEP_BUDGET.with(|budget| budget.set(None));
        assert_eq!(result, ["minecraft:stick"]);
    }

    #[test]
    fn nested_members_preserve_order_deduplicate_and_do_not_truncate_at_sixteen() {
        let mut tags = std::collections::HashMap::new();
        for index in 0..64 {
            tags.insert(
                ("item".into(), format!("test:{index}")),
                layers(vec![json!(format!("#test:{}", index + 1))]),
            );
        }
        tags.insert(
            ("item".into(), "test:64".into()),
            layers(vec![
                json!("minecraft:stick"),
                json!({"id":"minecraft:potion"}),
                json!("minecraft:stick"),
            ]),
        );
        assert_eq!(
            members(&tags, "item", "test:0"),
            ["minecraft:stick", "minecraft:potion"]
        );
        assert!(members(&tags, "item", "test:missing").is_empty());
        tags.insert(
            ("item".into(), "test:64".into()),
            layers(vec![json!("#test:0")]),
        );
        assert!(members(&tags, "item", "test:0").is_empty());
    }
}
