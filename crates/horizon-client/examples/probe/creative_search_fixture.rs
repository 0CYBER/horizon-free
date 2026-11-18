//! The creative inventory, for tools/test_creative_parity.py: loads an
//! exported `creative-inventory.json` into `horizon_client::creative::Creative`
//! and prints the tabs it shows (canonical stacks) and the search tab's
//! results for every query of the vanilla oracle's output; with `permissions`,
//! as an operator with the Operator Items Tab option on. It also middle-clicks
//! display stacks from the oracle's input through `creative::click_display`.
//!
//! usage: creative_search_fixture RESOURCES CREATIVE_JSON VANILLA_JSON OUTPUT [permissions]
use horizon_client::{
    creative::{Creative, Names, click_display, creative_stack_limit},
    enchant::Enchantments,
    item_data::{canon_stack, parse_stack},
    items::Items,
    lang::{self, Language},
    menu::Input,
};
use horizon_world::loot::PackOverlay;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage =
        "usage: creative_search_fixture RESOURCES CREATIVE_JSON VANILLA_JSON OUTPUT [permissions]";
    let resources = PathBuf::from(args.get(1).ok_or(usage)?);
    let source = horizon_assets::resource_source::ResourceSource::directory(resources.clone());
    let names = Names::load(&source);
    let mut creative = Creative::from_json(&serde_json::from_slice(&fs::read(
        args.get(2).ok_or(usage)?,
    )?)?)?;
    let defaults = horizon_world::item_defaults::read(&resources)?;
    creative.set_enchantments(Arc::new(Enchantments::load(&resources, &defaults)?));
    let items = Items::catalog(&resources, &PackOverlay::NONE, &defaults)?;
    let permissions = args.get(5).is_some_and(|a| a == "permissions");
    if creative.tabs_to_build(permissions).is_some() {
        creative.build(permissions, Vec::new(), &names);
    }
    let vanilla: Value = serde_json::from_slice(&fs::read(args.get(3).ok_or(usage)?)?)?;
    let tabs: Vec<Value> = creative
        .tabs
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "title": t.title,
                "icon": t.icon,
                "row": if t.top { "TOP" } else { "BOTTOM" },
                "column": t.column,
                "items": t.items.iter().map(|s| canon_stack(Some(s))).collect::<Vec<_>>(),
            })
        })
        .collect();
    let searches = query_results(&creative, &names, &vanilla["searches"])?;
    let french = Language::load(&[&source], "fr_fr");
    let (french_searches, _) = lang::scoped(french, || {
        query_results(&creative, &names, &vanilla["frenchSearches"])
    });
    let search: Vec<String> = creative
        .search_items()
        .iter()
        .map(|s| canon_stack(Some(s)))
        .collect();
    let out = json!({
        "tabs": tabs,
        "search": search,
        "searches": searches,
        "frenchSearches": french_searches?,
        "displayCopies": display_copies(&items, &vanilla["displayCopies"])?,
    });
    fs::write(args.get(4).ok_or(usage)?, out.to_string())?;
    Ok(())
}

fn display_copies(items: &Items, cases: &Value) -> horizon_client::Result<Vec<Value>> {
    cases
        .as_array()
        .ok_or("displayCopies")?
        .iter()
        .map(|case| {
            let stack = case["stack"].as_str().ok_or("display stack")?;
            let shown = parse_stack(stack)?.ok_or("empty display stack")?;
            let item_max = items
                .get(&shown.item)
                .ok_or("unknown display item")?
                .max_stack;
            let mut carried = None;
            click_display(
                &mut carried,
                Some(&shown),
                |stack| creative_stack_limit(stack, item_max),
                2,
                Input::Clone,
            );
            let copy = carried.as_ref().ok_or("creative copy was empty")?;
            Ok(json!({"stack": stack, "copy": canon_stack(Some(copy)), "count": copy.count}))
        })
        .collect()
}

fn query_results(
    creative: &Creative,
    names: &Names,
    queries: &Value,
) -> horizon_client::Result<Vec<Value>> {
    let results = queries
        .as_array()
        .ok_or("searches")?
        .iter()
        .map(|s| {
            let query = s["query"].as_str().unwrap_or_default();
            json!({"query": query, "results": creative.search_results(query, names)})
        })
        .collect();
    Ok(results)
}
