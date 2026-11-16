//! Frozen mod-book membership and recipe-display validation, built on table reload.
#[path = "mod_panel.rs"]
mod panel;
#[path = "mod_placement.rs"]
mod placement;
#[cfg(test)]
#[path = "mod_index_tests.rs"]
mod tests;
use super::RecipeBookCategory;
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};
pub use panel::ModBookPanel;
use std::collections::{BTreeMap, BTreeSet};

/// Resource tables consulted at the load boundary, never while drawing tabs.
pub trait DisplayResources {
    fn book(&self, key: &mod_api::item::recipe_book::RecipeBookTypeKey) -> bool;
    fn item(&self, id: &str) -> bool;
    fn stack(&self, stack: &mod_api::ItemStack) -> bool;
    fn tag(&self, id: &str) -> bool;
    fn component(&self, id: &str) -> bool;
    fn trim_pattern(&self, id: &str) -> bool;
    fn menu(&self, id: &str) -> bool;
    fn menu_layout(&self, _id: &str) -> Option<super::menu_layout::MenuLayoutMetadata> {
        None
    }
    fn recipe_type(&self, kind: &str) -> bool;
    fn recipe(&self, id: &str, kind: &str) -> bool;
}

/// A refused declaration. Construction is atomic: no partial book is published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DisplayLoadError {
    InvalidId(String),
    InvalidStack,
    UnknownResource { kind: &'static str, id: String },
    DuplicateCategory(String),
    DuplicateRecipe(String),
    ConflictingBook(String),
    InvalidLayout(String),
    InvalidPlacement(String),
}

/// Immutable index. Search membership is derived from book ownership, not an
/// unrestricted union of categories sharing a user-supplied search name.
#[derive(Debug, Default)]
pub struct ModBookIndex {
    generation: u64,
    categories: BTreeMap<String, RecipeBookCategory>,
    layouts: BTreeMap<String, super::slot_layout::CheckedSlotLayout>,
    placements: BTreeMap<String, Option<crate::menu::place::custom_plan::MenuPlacement>>,
    books: BTreeMap<String, (String, String, Vec<String>)>,
}
impl ModBookIndex {
    pub fn build(
        mut declarations: Vec<RecipeBookCategory>,
        resources: &dyn DisplayResources,
    ) -> Result<Self, DisplayLoadError> {
        placement::validate_batch(&mut declarations, resources)?;
        let mut index = Self::default();
        for menu in mod_api::RecipeBookLayout::registered_menus() {
            let Some((_, recipe_type, layout)) = mod_api::RecipeBookLayout::registered(&menu)
            else {
                continue;
            };
            if !resources.recipe_type(&recipe_type) {
                return Err(unknown("recipe type", &recipe_type));
            }
            let metadata = resources
                .menu_layout(&menu)
                .ok_or_else(|| unknown("menu layout", &menu))?;
            let compiled = super::menu_layout::compile(layout, metadata).map_err(|error| {
                DisplayLoadError::InvalidLayout(format!("menu {menu}: {error:?}"))
            })?;
            index.layouts.insert(menu, compiled);
        }
        let mut search_owners = BTreeMap::new();
        let mut recipe_ids = BTreeSet::new();
        for category in declarations {
            for id in [&category.id, &category.recipe_type, &category.search_group] {
                if !super::custom_books::valid_book_id(id) || id.starts_with("minecraft:") {
                    return Err(DisplayLoadError::InvalidId(id.clone()));
                }
            }
            if !resources.recipe_type(&category.recipe_type) {
                return Err(unknown("recipe type", &category.recipe_type));
            }
            if !resources.book(&category.book) {
                return Err(unknown("book", category.book.as_str()));
            }
            if !super::custom_books::valid_book_id(&category.target_menu) {
                return Err(DisplayLoadError::InvalidId(category.target_menu));
            }
            if !resources.menu(&category.target_menu) {
                return Err(unknown("menu", &category.target_menu));
            }
            if let Some((book, recipe_type, _)) =
                mod_api::RecipeBookLayout::registered(&category.target_menu)
                && (book != category.book || recipe_type != category.recipe_type)
            {
                return Err(DisplayLoadError::ConflictingBook(category.target_menu));
            }
            if !resources.item(&category.icon) || category.icon == "minecraft:air" {
                return Err(unknown("item", &category.icon));
            }
            if index.categories.contains_key(&category.id) {
                return Err(DisplayLoadError::DuplicateCategory(category.id));
            }
            if let Some(owner) = search_owners.insert(
                category.search_group.clone(),
                category.book.as_str().to_owned(),
            ) && owner != category.book.as_str()
            {
                return Err(DisplayLoadError::ConflictingBook(
                    category.book.as_str().to_owned(),
                ));
            }
            let book = index
                .books
                .entry(category.book.as_str().to_owned())
                .or_insert_with(|| {
                    (
                        category.target_menu.clone(),
                        category.search_group.clone(),
                        Vec::new(),
                    )
                });
            if book.0 != category.target_menu || book.1 != category.search_group {
                return Err(DisplayLoadError::ConflictingBook(
                    category.book.as_str().to_owned(),
                ));
            }
            for recipe in &category.displays {
                if u32::try_from(recipe.displays.len()).is_err() {
                    return Err(DisplayLoadError::InvalidPlacement(format!(
                        "recipe {} has more RecipeBookDisplay entries than Wire can index",
                        recipe.recipe
                    )));
                }
                if let Some(layout) = index.layouts.get(&category.target_menu) {
                    let owner = mod_api::RecipeBookLayout::declaring_mod(&category.target_menu)
                        .ok_or_else(|| {
                            DisplayLoadError::InvalidPlacement(format!(
                                "menu {} has no declaring mod",
                                category.target_menu
                            ))
                        })?;
                    index.placements.insert(
                        recipe.recipe.clone(),
                        placement::compile(owner, &category.target_menu, recipe, layout)?,
                    );
                }
                if !resources.recipe(&recipe.recipe, &category.recipe_type) {
                    return Err(unknown("recipe", &recipe.recipe));
                }
                if !recipe_ids.insert(recipe.recipe.clone()) {
                    return Err(DisplayLoadError::DuplicateRecipe(recipe.recipe.clone()));
                }
                for display in &recipe.displays {
                    validate_display(display, &recipe.recipe, resources)?;
                    if let Some(layout) = index.layouts.get(&category.target_menu) {
                        layout.map(display, true).map_err(|error| {
                            DisplayLoadError::InvalidLayout(format!(
                                "menu {} recipe {}: {error:?}",
                                category.target_menu, recipe.recipe
                            ))
                        })?;
                    }
                }
            }
            book.2.push(category.id.clone());
            index.categories.insert(category.id.clone(), category);
        }
        if let Some(id) = search_owners
            .keys()
            .find(|id| index.categories.contains_key(*id))
        {
            return Err(DisplayLoadError::DuplicateCategory(id.clone()));
        }
        for (_, _, categories) in index.books.values_mut() {
            categories.sort();
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        index.generation = NEXT
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |value| value.checked_add(1),
            )
            .map_err(|_| {
                DisplayLoadError::InvalidPlacement("display generation exhausted".into())
            })?;
        Ok(index)
    }
    /// Opaque successful declaration generation; zero is never a published generation.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Authoritative, validated plan; explicit NOT_PLACEABLE differs from an unknown recipe.
    pub fn placement(
        &self,
        recipe: &str,
    ) -> Result<
        &crate::menu::place::custom_plan::MenuPlacement,
        crate::menu::place::custom_plan::PlacementRefusal,
    > {
        use crate::menu::place::custom_plan::PlacementRefusal as Refused;
        self.placements
            .get(recipe)
            .ok_or(Refused::Unavailable)?
            .as_ref()
            .ok_or(Refused::NotPlaceable)
    }
    /// Frozen, validated menu-order destination map, if explicitly declared.
    pub fn layout(&self, menu: &str) -> Option<&super::slot_layout::CheckedSlotLayout> {
        self.layouts.get(menu)
    }
    /// Search tab's exact membership. Unknown (including vanilla) books are empty.
    pub fn search_categories(&self, book: &str) -> &[String] {
        self.books
            .get(book)
            .map_or(&[], |(_, _, categories)| categories.as_slice())
    }
    pub fn category(&self, id: &str) -> Option<&RecipeBookCategory> {
        self.categories.get(id)
    }
    /// Validated declarations in stable category-id order.
    pub fn categories(&self) -> impl Iterator<Item = &RecipeBookCategory> {
        self.categories.values()
    }
}
fn unknown(kind: &'static str, id: &str) -> DisplayLoadError {
    DisplayLoadError::UnknownResource {
        kind,
        id: id.to_owned(),
    }
}
fn validate_display(
    display: &Display,
    recipe: &str,
    resources: &dyn DisplayResources,
) -> Result<(), DisplayLoadError> {
    let invalid = || DisplayLoadError::InvalidLayout(recipe.to_owned());
    let slots: Vec<&Slot> = match display {
        Display::Shaped {
            width,
            height,
            ingredients,
            result,
            station,
        } => {
            if *width == 0
                || *height == 0
                || *width > i32::MAX as u32
                || *height > i32::MAX as u32
                || width.checked_mul(*height).map(u64::from) != Some(ingredients.len() as u64)
            {
                return Err(invalid());
            }
            ingredients.iter().chain([result, station]).collect()
        }
        Display::Shapeless {
            ingredients,
            result,
            station,
        } => ingredients.iter().chain([result, station]).collect(),
        Display::Furnace {
            ingredient,
            fuel,
            result,
            station,
            duration,
            experience,
        } => {
            if *duration > i32::MAX as u32 || !experience.is_finite() || *experience < 0.0 {
                return Err(invalid());
            }
            vec![ingredient, fuel, result, station]
        }
        Display::Stonecutter {
            input,
            result,
            station,
        } => vec![input, result, station],
        Display::Smithing {
            template,
            base,
            addition,
            result,
            station,
        } => vec![template, base, addition, result, station],
    };
    let mut pending: Vec<_> = slots.into_iter().map(|slot| (slot, 0)).collect();
    while let Some((slot, depth)) = pending.pop() {
        if depth > 32 {
            return Err(invalid());
        }
        let mut push = |slot| pending.push((slot, depth + 1));
        match slot {
            Slot::Empty | Slot::AnyFuel => {}
            Slot::Item(id) => {
                if !resources.item(id) {
                    return Err(unknown("item", id));
                }
            }
            Slot::ItemStack(stack) => {
                if !resources.stack(stack) {
                    return Err(DisplayLoadError::InvalidStack);
                }
            }
            Slot::Tag(id) => {
                if !resources.tag(id) {
                    return Err(unknown("tag", id));
                }
            }
            Slot::WithAnyPotion(source) => push(source),
            Slot::OnlyWithComponent { source, component } => {
                if !resources.component(component) {
                    return Err(unknown("component", component));
                }
                push(source);
            }
            Slot::Dyed { dye, target } => {
                push(dye);
                push(target);
            }
            Slot::SmithingTrim {
                base,
                material,
                pattern,
            } => {
                if !resources.trim_pattern(pattern) {
                    return Err(unknown("trim pattern", pattern));
                }
                push(base);
                push(material);
            }
            Slot::WithRemainder { input, remainder } => {
                push(input);
                push(remainder);
            }
            Slot::Composite(slots) => {
                for slot in slots {
                    push(slot);
                }
            }
        }
    }
    Ok(())
}
