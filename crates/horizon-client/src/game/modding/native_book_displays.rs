//! Validate the complete mod recipe-book generation before publishing any tab.
use super::*;
use horizon_client::recipe_book::{DisplayResources, ModBookIndex};

pub(super) struct Resources<'a, 'w>(pub(super) &'a Game<'w>);
impl DisplayResources for Resources<'_, '_> {
    fn book(&self, key: &mod_api::item::recipe_book::RecipeBookTypeKey) -> bool {
        key.is_registered()
    }
    fn item(&self, id: &str) -> bool {
        self.0.items().get(id).is_some()
    }
    fn stack(&self, stack: &mod_api::ItemStack) -> bool {
        api_values::stack_value(stack).is_ok_and(|stack| self.item(&stack.item))
    }
    fn tag(&self, id: &str) -> bool {
        self.0.items().item_tags.contains_key(id)
    }
    fn component(&self, id: &str) -> bool {
        self.0
            .ui
            .registries
            .ids("minecraft:data_component_type")
            .iter()
            .any(|name| name == id)
            || horizon_protocol::item::custom::id(id).is_some()
    }
    fn trim_pattern(&self, id: &str) -> bool {
        self.0
            .ui
            .registries
            .ids("minecraft:trim_pattern")
            .iter()
            .any(|name| name == id)
    }
    fn menu(&self, id: &str) -> bool {
        mod_api::__private::menu_keys()
            .into_iter()
            .any(|key| mod_api::__private::menu_key_id(key).as_deref() == Some(id))
    }
    fn menu_layout(
        &self,
        id: &str,
    ) -> Option<horizon_client::recipe_book::menu_layout::MenuLayoutMetadata> {
        let menu = horizon_client::containers::mod_menu_type(id)?;
        Some(
            horizon_client::recipe_book::menu_layout::MenuLayoutMetadata {
                slots: u16::try_from(menu.slots).ok()?,
                crafting: menu
                    .crafting
                    .map(|grid| (grid.grid as u16, grid.size as u16, grid.result as u16)),
            },
        )
    }
    fn recipe_type(&self, kind: &str) -> bool {
        self.0.recipes().by_type.contains_key(kind)
    }
    fn recipe(&self, id: &str, kind: &str) -> bool {
        self.0
            .recipes()
            .by_type
            .get(kind)
            .is_some_and(|recipes| recipes.iter().any(|recipe| recipe.id == id))
    }
}

impl Game<'_> {
    /// The owning declaration generation, including its typed load refusal.
    /// Cache failures too: malformed input never invokes callbacks every frame.
    pub(super) fn ensure_mod_book_displays(&mut self) {
        if self.recipe_tables.books.is_some() {
            return;
        }
        let mut declarations = Vec::new();
        self.native_recipe_book_categories(&mut declarations);
        let books = ModBookIndex::build(declarations, &Resources(self));
        if let Err(error) = &books {
            log::error!("recipe book declarations refused: {error:?}");
        }
        self.recipe_tables.books = Some(books);
    }
}
