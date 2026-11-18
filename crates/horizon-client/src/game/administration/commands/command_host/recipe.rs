use super::entity::target_of;
use crate::game::Game;
use horizon_gameplay::commands::{entity::EntityRef, recipe::RecipeHost};

impl RecipeHost for Game<'_> {
    fn learnable_recipes(&self) -> Vec<String> {
        Game::learnable_recipes(self)
    }
    fn change_recipes_of(&mut self, entity: EntityRef, give: bool, recipes: &[String]) -> i32 {
        self.for_player(target_of(entity), |g| g.change_recipes(give, recipes))
            .unwrap_or(0)
    }
}
