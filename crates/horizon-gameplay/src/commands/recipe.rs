//! `/recipe`.
use super::entity::{EntityHost, holder, num};
use super::host::R;
use horizon_client::{
    command::{Arg, Context},
    scoreboard_cmd::Tracker,
    text::Text,
};

/// What `/recipe` needs from the game.
pub trait RecipeHost: EntityHost {
    /// Learnable recipes and registered hidden crafting kinds for `/recipe` resolution.
    fn learnable_recipes(&self) -> Vec<String>;
    /// `ServerRecipeBook.addRecipes/removeRecipes` on the target's own recipe book: how many
    /// recipe book displays changed (none when the target has no book).
    fn change_recipes_of(
        &mut self,
        entity: super::entity::EntityRef,
        give: bool,
        recipes: &[String],
    ) -> i32;
}

/// `/recipe give|take <targets> <recipe>|*`; `id` is `recipe.give` or `recipe.take`.
pub fn recipe(host: &mut dyn RecipeHost, id: &str, ctx: &Context) -> R<i32> {
    let targets = host.select_entities(ctx, "targets", true)?;
    let give = id == "recipe.give";
    let recipes: Vec<String> = match ctx.get("recipe") {
        Some(Arg::Resource(r)) => {
            if !host.learnable_recipes().contains(r) {
                return Err(host.error("recipe.notFound", &[r]));
            }
            vec![r.clone()]
        }
        _ => host.learnable_recipes(),
    };
    let mut tracker = Tracker::default();
    for &t in &targets {
        let h = holder(host.entity_name(t));
        // ServerPlayer.awardRecipes / resetRecipes on the target's own
        // ServerRecipeBook: a guest's is swapped in for the call, and the
        // packets go to its client.
        let n = host.change_recipes_of(t, give, &recipes);
        tracker.track(&h, n);
    }
    let verb = if give { "give" } else { "take" };
    let (message, total) = tracker.message(
        false,
        Some(host.error(&format!("commands.recipe.{verb}.failed"), &[])),
        |h, total| {
            Text::translate(
                &format!("commands.recipe.{verb}.success.single"),
                &[num(total), h.text.clone()],
            )
        },
        |count, total| {
            Text::translate(
                &format!("commands.recipe.{verb}.success.multiple"),
                &[num(total), num(count)],
            )
        },
    )?;
    host.feedback(message);
    Ok(total)
}
