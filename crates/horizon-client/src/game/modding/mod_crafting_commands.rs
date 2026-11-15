//! Hidden crafting recipe knowledge for /recipe, without recipe-book displays.
use crate::game::Game;
impl Game<'_> {
    pub(in crate::game) fn change_mod_crafting_recipes(
        &mut self,
        give: bool,
        ids: &[String],
    ) -> (Vec<String>, i32) {
        let (custom, ordinary): (Vec<_>, Vec<_>) = ids.iter().cloned().partition(|id| {
            matches!(
                self.recipes().recipe(id),
                Some(horizon_client::crafting::Recipe::Mod(_))
            )
        });
        let mut count = 0;
        for id in custom {
            let changed = if give {
                self.level.recipe_book.recipes.insert(id)
            } else {
                self.level.recipe_book.recipes.remove(&id)
            };
            count += i32::from(changed);
        }
        (ordinary, count)
    }
}
