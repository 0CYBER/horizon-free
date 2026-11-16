//! Engine recipe ingredients and stack matching.
#[path = "ingredient/components.rs"]
mod components;
#[path = "ingredient/display.rs"]
mod display;
#[path = "ingredient/matching.rs"]
mod matching;
use super::native_hooks::native_ingredient;
use super::{ItemStack, Items};
use crate::native_stack_hooks::ingredient_test as native_ingredient_test;
use display::display_stacks;
use horizon_world::native_load_context::LoadContext;
use mod_api::__private::{WalkedIngredient, dedup_keep_order, resolved_items};
pub use mod_api::item::crafting::{IngredientRule, Shape};
use serde_json::Value;
use std::collections::HashSet;
/// Where `Ingredient::resolve` leaves the hook's answer inside the occurrence
/// it resolved. Written and read only here; the loads that own the sources
/// never send or save it.
const RESOLVED_ITEMS: &str = "horizon:resolved_items";

/// The kind an ingredient object names: `neoforge:ingredient_type` in
/// current NeoForge, `type` in older packs.
fn ingredient_kind(o: &serde_json::Map<String, Value>) -> Option<&str> {
    o.get("neoforge:ingredient_type")
        .or_else(|| o.get("type"))?
        .as_str()
}

/// A mod's own kind: a namespaced type that is not minecraft's.
fn is_custom(kind: &str) -> bool {
    kind.contains(':') && !kind.starts_with("minecraft:")
}

/// A set of accepted items (expanded from an item, list or tag), and the
/// items in its `HolderSet` order, with built-in component and display rules.
#[derive(Clone, Debug)]
// exhaustive: the game builds it with a literal; mods only read it
pub struct Ingredient {
    /// Full candidate set; a non-simple predicate tests each concrete stack.
    pub items: HashSet<String>,
    /// Default-stack display/simple-match order; may be empty for non-simple kinds.
    pub order: Vec<String>,
    /// A mod's own kind, for matching at craft time: the kind and its
    /// NBT compound, as the load-time hook saw them (`native_ingredient_test`).
    pub custom: Option<(String, horizon_core::nbt::Compound)>,
    /// NeoForge's shapes, with their children parsed: matching a concrete
    /// stack asks each child its own `test` (a mod's kind among them),
    /// as on NeoForge — the flat list above only names items.
    pub children: Option<(Shape, Vec<Self>)>,
    /// Built-in predicate evaluated against the concrete stack.
    pub rule: Option<IngredientRule>,
    /// Validated decoded component predicate, shared by all candidate checks.
    pub compiled: Option<std::sync::Arc<components::ComponentPredicate>>,
    /// Slot display override; membership continues to use `items`.
    pub display: Option<horizon_core::nbt::Compound>,
    /// Resolved slot display stacks; never used for placement or matching.
    pub shown: Option<Vec<ItemStack>>,
    /// Shared prototypes of the load that parsed this ingredient.
    pub component_defaults:
        std::sync::Arc<std::collections::HashMap<String, horizon_core::nbt::Compound>>,
}
impl Ingredient {
    /// Reject malformed component patches before any part of a recipe is published.
    pub fn validate_recipe(value: &Value, items: &Items, recipe: &str) -> bool {
        components::validate_recipe(value, items, recipe)
    }
    /// Decode built-in rules and materialize display stacks at a load boundary.
    pub fn prepare_components(&mut self, items: &Items) -> Option<()> {
        if let Some(rule) = &self.rule {
            let compiled = std::sync::Arc::new(
                components::ComponentPredicate::compile(rule, &self.items, items).ok()?,
            );
            if self.display.is_none() {
                self.shown = Some(compiled.shown(&self.order));
            }
            self.compiled = Some(compiled);
        }
        Some(())
    }
    fn default_matches(&self, stack: &ItemStack) -> bool {
        match &self.children {
            Some((Shape::Compound, kids)) => kids.iter().any(|kid| kid.default_matches(stack)),
            Some((Shape::Intersection, kids)) => kids.iter().all(|kid| kid.default_matches(stack)),
            Some((Shape::Difference, kids)) => {
                kids[0].default_matches(stack) && !kids[1].default_matches(stack)
            }
            Some((Shape::CustomDisplay, kids)) => kids[0].default_matches(stack),
            None => {
                self.items.contains(stack.item.as_ref())
                    && self
                        .compiled
                        .as_ref()
                        .is_none_or(|rule| rule.matches(stack))
            }
        }
    }

    /// Resolve a stored slot display at the owning resource boundary.
    pub fn resolve_display(
        display: &horizon_core::nbt::Compound,
        items: &Items,
    ) -> Option<Vec<ItemStack>> {
        display_stacks(display, items)
    }

    /// Whether item membership fully determines shapeless matching.
    pub fn is_simple(&self) -> bool {
        self.rule.is_none()
            && self.custom.is_none()
            && self
                .children
                .as_ref()
                .is_none_or(|(_, kids)| kids.iter().all(Self::is_simple))
    }

    /// The items of a list, in order.
    pub fn of(items: Vec<String>) -> Self {
        Self {
            items: items.iter().cloned().collect(),
            order: items,
            custom: None,
            children: None,
            rule: None,
            compiled: None,
            display: None,
            shown: None,
            component_defaults: Default::default(),
        }
    }
    /// Asks the `ingredient` hook once for each mod kind in `v`, nested ones
    /// included, and leaves each answer with its occurrence for `parse`: the
    /// one place the hook is called, so every table that reads the recipes
    /// shares the same answers. A file's own answer is dropped first, so data
    /// cannot forge one. `true`: `v` changed.
    // hidden: the load's half of the ingredient hook, not part of the mod surface
    #[doc(hidden)]
    pub fn resolve(v: &mut Value, context: &LoadContext) -> bool {
        match v {
            Value::Array(entries) => entries
                .iter_mut()
                .fold(false, |changed, e| Self::resolve(e, context) | changed),
            Value::Object(o) => {
                let mut changed = o.remove(RESOLVED_ITEMS).is_some();
                let kind = ingredient_kind(o).map(str::to_owned);
                match kind.as_deref() {
                    Some("neoforge:compound" | "neoforge:intersection") => {
                        for key in ["children", "ingredients"] {
                            if let Some(children) = o.get_mut(key) {
                                changed |= Self::resolve(children, context);
                            }
                        }
                    }
                    Some("neoforge:difference") => {
                        for key in ["base", "subtracted"] {
                            if let Some(part) = o.get_mut(key) {
                                changed |= Self::resolve(part, context);
                            }
                        }
                    }
                    Some("neoforge:custom_display") => {
                        if let Some(base) = o.get_mut("base") {
                            changed |= Self::resolve(base, context);
                        }
                    }
                    Some("neoforge:components" | "neoforge:block_tag") => {}
                    Some(k) if is_custom(k) => {
                        let mut list = Vec::new();
                        if mod_api::__private::listened(mod_api::HookId::Ingredient)
                            && let Ok(data) = crate::json_nbt::json_to_compound(o)
                        {
                            native_ingredient(context, k, &data, &mut list);
                        }
                        dedup_keep_order(&mut list);
                        o.insert(RESOLVED_ITEMS.to_owned(), list.into());
                        changed = true;
                    }
                    Some(_) => {}
                    // A brewing slot's item predicate wraps its ingredient.
                    None => {
                        if let Some(item) = o.get_mut("item") {
                            changed |= Self::resolve(item, context);
                        }
                    }
                }
                changed
            }
            _ => false,
        }
    }
    /// Every mod kind named in `v`, nested ones included, with whether
    /// `resolve` already left an empty answer for it. Walks the same shapes
    /// `resolve` does.
    pub fn custom_kinds(v: &Value) -> Vec<(String, bool)> {
        let mut found = Vec::new();
        Self::collect_custom_kinds(v, &mut found);
        found
    }
    fn collect_custom_kinds(v: &Value, found: &mut Vec<(String, bool)>) {
        match v {
            Value::Array(entries) => entries
                .iter()
                .for_each(|e| Self::collect_custom_kinds(e, found)),
            Value::Object(o) => {
                let parts: &[&str] = match ingredient_kind(o) {
                    Some("neoforge:compound" | "neoforge:intersection") => {
                        &["children", "ingredients"]
                    }
                    Some("neoforge:difference") => &["base", "subtracted"],
                    Some("neoforge:custom_display") => &["base"],
                    Some("neoforge:components" | "neoforge:block_tag") => &[],
                    Some(k) if is_custom(k) => {
                        let empty = o
                            .get(RESOLVED_ITEMS)
                            .and_then(Value::as_array)
                            .is_some_and(Vec::is_empty);
                        found.push((k.to_owned(), empty));
                        &[]
                    }
                    Some(_) => &[],
                    None => &["item"],
                };
                for key in parts {
                    if let Some(part) = o.get(*key) {
                        Self::collect_custom_kinds(part, found);
                    }
                }
            }
            _ => {}
        }
    }
    /// Drops every answer `resolve` left anywhere in `v`: what a load sends
    /// or shows is the files' own JSON. `true`: `v` changed.
    // hidden: the load's half of the ingredient hook, not part of the mod surface
    #[doc(hidden)]
    pub fn forget_answers(v: &mut Value) -> bool {
        match v {
            Value::Array(entries) => entries
                .iter_mut()
                .fold(false, |changed, e| Self::forget_answers(e) | changed),
            Value::Object(o) => {
                let mut changed = o.remove(RESOLVED_ITEMS).is_some();
                for part in o.values_mut() {
                    changed |= Self::forget_answers(part);
                }
                changed
            }
            _ => false,
        }
    }
    pub fn parse(v: &Value, items: &Items) -> Option<Self> {
        // A mod's own kind: `resolve` asked the hook once at load and left
        // the item list it matches with the occurrence; one no load
        // resolved fails the file.
        let walked = WalkedIngredient::walk(
            v,
            &items.item_tag_order,
            &|tag| {
                Some(
                    items
                        .block_tag_order
                        .get(tag)?
                        .iter()
                        .filter_map(|block| {
                            let item = items
                                .block_items
                                .get(block)
                                .map(String::as_str)
                                .unwrap_or(block);
                            (item != "minecraft:air" && items.get(item).is_some())
                                .then(|| item.to_owned())
                        })
                        .collect(),
                )
            },
            &resolved_items,
            &components::json_patch,
        );
        let Some(walked) = walked else {
            log::error!("recipes: invalid ingredient (unresolved tag or malformed input): {v}");
            return None;
        };
        // A mod kind's empty answer is warned about at load, with its recipe file.
        if walked.order.is_empty() && walked.custom.is_none() {
            log::warn!(
                "recipes: ingredient has no placement candidates; recipe cannot be placed: {v}"
            );
        }
        Self::of_walked(walked, items)
    }
    /// The walk of an item ingredient's answer: NeoForge's standard shapes
    /// keep their children, and a mod's own kind keeps its kind and NBT metadata,
    /// since the load-time list only names items and matching a concrete
    /// stack (its components) is the match-time hook's.
    fn of_walked(walked: WalkedIngredient, items: &Items) -> Option<Self> {
        // The shapes' children walk the same way; a child no wrapper
        // answers fails the file.
        let children = match walked.children {
            Some((shape, kids)) => Some((
                shape,
                kids.into_iter()
                    .map(|kid| Self::of_walked(kid, items))
                    .collect::<Option<_>>()?,
            )),
            None => None,
        };
        let shown = match &walked.display {
            Some(display) => Some(display_stacks(display, items)?),
            None => None,
        };
        let mut ingredient = Self {
            items: walked.set,
            order: walked.order,
            custom: walked.custom,
            rule: walked.rule,
            compiled: None,
            shown,
            display: walked.display,
            component_defaults: items.component_defaults.clone(),
            children,
        };
        ingredient.prepare_components(items)?;
        let order: Vec<String> = match &ingredient.children {
            Some((Shape::Difference, kids)) => ingredient
                .order
                .iter()
                .filter(|id| !kids[1].default_matches(&ItemStack::new(id, 1)))
                .cloned()
                .collect(),
            Some((Shape::Intersection, _)) => ingredient
                .order
                .iter()
                .filter(|id| ingredient.default_matches(&ItemStack::new(id, 1)))
                .cloned()
                .collect(),
            _ => return Some(ingredient),
        };
        if ingredient.is_simple() {
            ingredient.items = order.iter().cloned().collect();
        }
        ingredient.order = order;
        Some(ingredient)
    }
}

#[cfg(test)]
#[path = "custom_data_tests.rs"]
mod custom_data_tests;

#[cfg(test)]
#[path = "ingredient/builtin_kinds_tests/mod.rs"]
mod builtin_kinds_tests;

#[cfg(test)]
#[path = "ingredient/review_tests.rs"]
mod review_tests;

#[cfg(test)]
#[path = "ingredient/second_review_tests.rs"]
mod second_review_tests;
