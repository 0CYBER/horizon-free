//! World-local generated class membership, shared by attachment and typed reads.
use horizon_client::blocks::BlockFacts;
use horizon_world::level::classes::Class;
use std::collections::{HashMap, HashSet};

pub(super) fn prepare(
    datapack: &HashMap<String, HashSet<String>>,
    facts: &BlockFacts,
) -> HashMap<String, HashSet<String>> {
    merge(
        datapack,
        facts.states.iter().map(|state| {
            (
                state.name.as_ref(),
                state.classes.iter().map(|name| name.as_ref()),
            )
        }),
    )
}

pub(super) fn merge<'a>(
    datapack: &HashMap<String, HashSet<String>>,
    states: impl IntoIterator<Item = (&'a str, impl IntoIterator<Item = &'a str>)>,
) -> HashMap<String, HashSet<String>> {
    let mut tags = datapack.clone();
    tags.retain(|tag, _| {
        if tag.starts_with("horizon:class/") {
            log::warn!("Ignoring reserved generated block tag {tag}");
            false
        } else {
            true
        }
    });
    for class in Class::ALL {
        tags.insert(class.tag().to_owned(), HashSet::new());
    }
    let mut seen = HashSet::new();
    for (block, chain) in states {
        if !seen.insert(block) {
            continue;
        }
        for name in chain {
            if let Some(class) = Class::from_java(name) {
                tags.get_mut(class.tag()).unwrap().insert(block.to_owned());
            }
        }
    }
    tags
}

/// Install generated membership into both views of the same tag table.
pub(super) fn install(items: &mut horizon_items::items::Items, facts: &BlockFacts) {
    let generated = prepare(&items.block_tags, facts);
    items
        .block_tag_order
        .retain(|tag, _| !tag.starts_with("horizon:class/"));
    for (tag, members) in &generated {
        if tag.starts_with("horizon:class/") {
            let mut members: Vec<_> = members.iter().cloned().collect();
            members.sort_unstable();
            items.block_tag_order.insert(tag.clone(), members);
        }
    }
    items.block_tags = generated;
}
