//! Authoritative placement data crosses one checked load boundary, never a draw path.
#[cfg(test)]
#[path = "mod_placement_tests.rs"]
mod tests;
use super::{DisplayLoadError, DisplayResources, RecipeBookCategory};
use crate::{
    crafting::{Ingredient, Placement},
    menu::place::custom_plan::{MenuPlacement, PlacementRefusal},
};
use mod_api::{PlacementInfo, RecipeBookDisplayEntry};

// A rejected native declaration may contain a tree too deep even to drop recursively.
// Drain every new placement value before its category batch is dropped on refusal.
struct BatchGuard<'a> {
    values: &'a mut [RecipeBookCategory],
    accepted: bool,
}
impl Drop for BatchGuard<'_> {
    fn drop(&mut self) {
        if self.accepted {
            return;
        }
        for category in &mut *self.values {
            for entry in &mut category.displays {
                if let Some(info) = entry.placement.take() {
                    let mut pending: Vec<_> = info.cells.into_iter().flatten().collect();
                    while let Some(mut ingredient) = pending.pop() {
                        if let Some((_, children)) = ingredient.children.take() {
                            pending.extend(children);
                        }
                        if let Some((_, data)) = ingredient.custom.take() {
                            for (_, tag) in data {
                                horizon_core::nbt::discard_tag(tag);
                            }
                        }
                    }
                }
            }
        }
    }
}
fn invalid(reason: impl Into<String>) -> DisplayLoadError {
    DisplayLoadError::InvalidPlacement(reason.into())
}

pub(super) fn validate_batch(
    values: &mut [RecipeBookCategory],
    resources: &dyn DisplayResources,
) -> Result<(), DisplayLoadError> {
    let mut guard = BatchGuard {
        values,
        accepted: false,
    };
    for category in &*guard.values {
        for entry in &category.displays {
            let Some(info) = &entry.placement else {
                continue;
            };
            validate(info, resources).map_err(|reason| {
                invalid(format!("recipe {} placement: {reason}", entry.recipe))
            })?;
        }
    }
    guard.accepted = true;
    Ok(())
}
fn validate(info: &PlacementInfo, resources: &dyn DisplayResources) -> Result<(), String> {
    if let Some([w, h]) = info.shape
        && (w == 0
            || h == 0
            || usize::from(w).checked_mul(usize::from(h)) != Some(info.cells.len()))
    {
        return Err("shape dimensions disagree with cells".into());
    }
    // This is only a preflight for values provably unable to cross Wire. The
    // derived codec below, not a second decoder or local depth budget, decides
    // acceptance identically for native and wasm declarations. A record and
    // each recursive ingredient record contributes one reader level;
    // category/entry/info/ingredient start at four (lists/options/tuples
    // do not add reader depth).
    // Reject that proven lower bound before invoking the recursive encoder.
    let mut pending: Vec<_> = info
        .cells
        .iter()
        .flatten()
        .map(|value| (value, 4usize))
        .collect();
    while let Some((ingredient, depth)) = pending.pop() {
        if depth > usize::from(mod_api::wire::MAX_DEPTH) {
            return Err("Wire refused placement: ingredient exceeds depth".into());
        }
        if ingredient.items.iter().any(|id| !resources.item(id)) {
            return Err("unknown ingredient item".into());
        }
        let ordered: std::collections::HashSet<_> = ingredient.order.iter().cloned().collect();
        if !ordered.is_subset(&ingredient.items)
            || (simple_ingredient(ingredient) && ordered != ingredient.items)
            || ordered.len() != ingredient.order.len()
        {
            return Err("ingredient holder order disagrees with its items".into());
        }
        if let Some((kind, _)) = &ingredient.custom
            && (ingredient.children.is_some()
                || !super::super::custom_books::valid_book_id(kind)
                || kind.starts_with("minecraft:"))
        {
            return Err("custom ingredient kind is invalid or mixed with a child shape".into());
        }
        // Tag owns its independent, iterative vanilla depth budget.
        // The shared codec below validates it; ingredient depth is not NBT depth.
        if let Some((shape, children)) = &ingredient.children {
            if children.is_empty()
                || matches!(shape, mod_api::item::crafting::Shape::Difference)
                    && children.len() != 2
            {
                return Err("ingredient shape has invalid children".into());
            }
            pending.extend(children.iter().map(|value| (value, depth + 1)));
        }
    }
    let mut output = mod_api::wire::Writer::new();
    output.put(info);
    if let Some(error) = output.error() {
        return Err(format!("Wire refused placement: {error}"));
    }
    // Preserve the actual hook field's enclosing category and entry record
    // levels. Lists, options and tuples add no reader level.
    // This uses the shared reader, with no parallel placement codec.
    let mut input = mod_api::wire::Reader::new(output.bytes());
    let _: PlacementInfo = input
        .nested(|input| input.nested(|input| input.take()))
        .map_err(|error| format!("Wire refused placement: {error}"))?;
    Ok(())
}

pub(super) fn compile(
    owner: &str,
    menu: &str,
    entry: &RecipeBookDisplayEntry,
    layout: &super::super::slot_layout::CheckedSlotLayout,
) -> Result<Option<MenuPlacement>, DisplayLoadError> {
    let info = entry.placement.as_ref().ok_or_else(|| {
        invalid(format!(
            "mod {owner} recipe {}: menu-bound recipe type must supply PlacementInfo",
            entry.recipe
        ))
    })?;
    let cells = info
        .cells
        .iter()
        .map(|cell| cell.as_ref().map(convert))
        .collect();
    let placement = Placement::of(
        cells,
        info.shape.map(|[w, h]| (usize::from(w), usize::from(h))),
    );
    match MenuPlacement::new(menu.into(), placement, layout) {
        Ok(plan) => Ok(Some(plan)),
        Err(PlacementRefusal::NotPlaceable) => Ok(None),
        Err(error) => Err(invalid(format!(
            "mod {owner} recipe {} placement: {error:?}",
            entry.recipe
        ))),
    }
}
// Only called after validate_batch accepted the shared Wire form.
fn convert(value: &mod_api::Ingredient) -> Ingredient {
    Ingredient {
        items: value.items.clone(),
        order: value.order.clone(),
        custom: value.custom.clone(),
        rule: None,
        compiled: None,
        display: value.display.as_deref().cloned(),
        component_defaults: Default::default(),
        shown: None,
        children: value
            .children
            .as_ref()
            .map(|(shape, children)| (shape.clone(), children.iter().map(convert).collect())),
    }
}

// Bound the walk before consulting an untrusted native provider's recursive tree.
fn simple_ingredient(ingredient: &mod_api::Ingredient) -> bool {
    let mut pending = vec![(ingredient, 0usize)];
    while let Some((ingredient, depth)) = pending.pop() {
        if depth > usize::from(mod_api::wire::MAX_DEPTH)
            || ingredient.rule.is_some()
            || ingredient.custom.is_some()
        {
            return false;
        }
        if let Some((_, children)) = &ingredient.children {
            pending.extend(children.iter().map(|child| (child, depth + 1)));
        }
    }
    true
}
