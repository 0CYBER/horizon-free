use super::TagLayers;
use std::{borrow::Cow, collections::HashMap};

impl TagLayers {
    /// Resolve layered tags once in dependency order. Vanilla 26.3's
    /// `DependencySorter.orderByDependencies` uses `isCyclic` to avoid
    /// cycle-closing edges. `TagLoader.build` publishes only successful
    /// `tryBuildTag` results, so a missing required target fails its tag.
    /// As in `loot/tags.rs`, a cycle or dependency on one builds an empty
    /// list here. Unknown tag names contribute no entries.
    ///
    /// Graph traversal is O(V + E), plus the work of applying members in
    /// entry order for successfully built tags.
    pub fn resolve_all(raw: HashMap<String, Self>) -> HashMap<String, Vec<String>> {
        let (names, layers): (Vec<_>, Vec<_>) = raw.into_iter().unzip();
        let index: HashMap<_, _> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();
        let dependencies: Vec<Vec<_>> = layers
            .iter()
            .map(|layer| {
                layer
                    .tags()
                    .filter_map(|name| index.get(name).copied())
                    .collect()
            })
            .collect();
        let mut resolved = vec![Vec::<String>::new(); names.len()];
        let mut failed = vec![false; names.len()];

        // Components come out of Kosaraju's second pass in dependents-first order.
        for tags in components(&dependencies).into_iter().rev() {
            let &[tag] = tags.as_slice() else {
                for tag in tags {
                    failed[tag] = true;
                }
                continue;
            };
            if dependencies[tag]
                .iter()
                .any(|&child| child == tag || failed[child])
            {
                failed[tag] = true;
                continue;
            }
            resolved[tag] = layers[tag].build_ordered(|name| {
                let members = index
                    .get(name)
                    .map_or(&[][..], |&id| resolved[id].as_slice());
                members.iter().map(|id| Cow::Borrowed(id.as_str()))
            });
        }

        names.into_iter().zip(resolved).collect()
    }
}

fn components(dependencies: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut dependents = vec![Vec::new(); dependencies.len()];
    for (tag, children) in dependencies.iter().enumerate() {
        for &child in children {
            dependents[child].push(tag);
        }
    }

    let mut seen = vec![false; dependencies.len()];
    let order = postorder(0..dependencies.len(), dependencies, |tag| {
        !std::mem::replace(&mut seen[tag], true)
    });
    seen.fill(false);
    let mut components = Vec::new();
    for root in order.into_iter().rev() {
        if !seen[root] {
            components.push(postorder([root], &dependents, |tag| {
                !std::mem::replace(&mut seen[tag], true)
            }));
        }
    }
    components
}

fn postorder(
    roots: impl IntoIterator<Item = usize>,
    edges: &[Vec<usize>],
    mut enter: impl FnMut(usize) -> bool,
) -> Vec<usize> {
    enum Step {
        Enter(usize),
        Exit(usize),
    }

    let mut order = Vec::new();
    let mut pending = Vec::new();
    for root in roots {
        pending.push(Step::Enter(root));
        while let Some(step) = pending.pop() {
            #[cfg(test)]
            super::work_budget::check_step();
            match step {
                Step::Enter(tag) if enter(tag) => {
                    pending.push(Step::Exit(tag));
                    pending.extend(edges[tag].iter().rev().copied().map(Step::Enter));
                }
                Step::Exit(tag) => order.push(tag),
                Step::Enter(_) => {}
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_layers::work_budget;
    use serde_json::{Value, json};

    fn layers(values: Value) -> TagLayers {
        TagLayers::from_file(&json!({"values": values}))
    }

    #[test]
    fn cycle_rejects_its_tags_and_dependents_but_keeps_unrelated_tags() {
        let resolved = TagLayers::resolve_all(HashMap::from([
            (
                "minecraft:a".into(),
                layers(json!(["minecraft:stick", "#b"])),
            ),
            (
                "minecraft:b".into(),
                layers(json!(["minecraft:dirt", "#a"])),
            ),
            (
                "minecraft:c".into(),
                layers(json!(["minecraft:apple", "#a"])),
            ),
            ("minecraft:d".into(), layers(json!(["minecraft:stone"]))),
            (
                "minecraft:e".into(),
                layers(json!(["minecraft:sand", "#c"])),
            ),
        ]));
        for id in ["minecraft:a", "minecraft:b", "minecraft:c", "minecraft:e"] {
            assert!(resolved[id].is_empty(), "{id}");
        }
        assert_eq!(resolved["minecraft:d"], ["minecraft:stone"]);
    }

    #[test]
    fn self_reference_rejects_its_tag() {
        let resolved = TagLayers::resolve_all(HashMap::from([(
            "minecraft:a".into(),
            layers(json!(["minecraft:stick", "#a"])),
        )]));
        assert!(resolved["minecraft:a"].is_empty());
    }

    #[test]
    fn missing_and_empty_references_contribute_no_members() {
        let resolved = TagLayers::resolve_all(HashMap::from([
            ("minecraft:empty".into(), layers(json!([]))),
            (
                "minecraft:a".into(),
                layers(json!(["stick", "#unknown", "#empty", "apple"])),
            ),
        ]));
        assert_eq!(
            resolved["minecraft:a"],
            ["minecraft:stick", "minecraft:apple"]
        );
    }

    #[test]
    fn dense_cycle_has_linear_traversal_work() {
        const TAGS: usize = 128;
        let tags = (0..TAGS)
            .map(|tag| {
                let references: Vec<_> = (0..TAGS)
                    .filter(|&other| other != tag)
                    .map(|other| format!("#test:cycle/{other}"))
                    .chain(["stick".into()])
                    .collect();
                (format!("test:cycle/{tag}"), layers(json!(references)))
            })
            .collect();

        let resolved = work_budget::with_limit(4 * TAGS * TAGS, || TagLayers::resolve_all(tags));
        assert_eq!(resolved.len(), TAGS);
        assert!(resolved.values().all(Vec::is_empty));
    }

    #[test]
    fn deep_diamond_dag_has_linear_traversal_work() {
        const DEPTH: usize = 40;
        let mut tags = HashMap::new();
        for depth in 0..DEPTH {
            for side in ["left", "right"] {
                tags.insert(
                    format!("test:{side}/{depth}"),
                    layers(json!([format!("#test:join/{}", depth + 1)])),
                );
            }
            tags.insert(
                format!("test:join/{depth}"),
                layers(json!([
                    format!("#test:left/{depth}"),
                    format!("#test:right/{depth}")
                ])),
            );
        }
        tags.insert(
            format!("test:join/{DEPTH}"),
            layers(json!(["stick", "dirt"])),
        );

        let resolved = work_budget::with_limit(4 * (tags.len() + 4 * DEPTH), || {
            TagLayers::resolve_all(tags)
        });
        assert_eq!(
            resolved["test:join/0"],
            ["minecraft:stick", "minecraft:dirt"]
        );
    }
}
