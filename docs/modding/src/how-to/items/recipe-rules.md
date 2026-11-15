# Reading recipe rules

Use `Host::recipes_of_type` to read loaded recipe definitions and
`LevelRef::recipe_for` to compute the output and remainders for actual inputs.

| Recipe kind | Definition |
|---|---|
| Shaped, shapeless and cooking | Resolved ingredients and fixed results |
| Stonecutting | `RecipeRule::Stonecutting`, with its resolved input and complete fixed output |
| Smithing and special crafting | A typed `RecipeRule` with resolved ingredient roles; the result depends on the inputs |
| Mod-defined type | Original recipe data in `Recipe::json`; match it in your mod |

For smithing and special crafting, do not treat a `RecipeTemplate` as a fixed
stack. It retains the count and component patch used to assemble the result.
In `RecipeRule::SmithingTransform`, absent template or addition ingredients
require empty slots. Trim patterns are registry references or inline definitions.

`recipes_of_type` returns recipes after pack overrides, conditions and filters,
in file order. Read them again after a data reload. On the server, you read the
server's table. A client reads vanilla and content-mod recipes unless the server
sends the recipe type through `datapack_sync`.

`LevelRef::recipe_for` returns `Ok(None)` when no recipe matches. Mod-defined
recipe types return `RecipeLookupError::NotMatchable`; use their JSON to build
your own matching logic. See [Cache reload-stable recipe definitions](reload-stable-recipes.md)
for a custom-type cache.

## Finding a recipe's recipe book ids

A recipe viewer's "+" and its `transfer_recipe_check` and `place_recipe` take a
recipe book id, which `Recipe` does not carry. `RecipesClientHost::recipe_book_ids`
(`recipe_id`) returns the ids of a recipe's entries in the book, in the book's
order.

A server numbers its recipe book entries itself and names none of them, so the
ids are given only where the server sent the entry with that number and this
client's own numbering of the recipe agrees with it (the entry has the same
result, category and ingredients). Two different recipes with identical
displays cannot be told apart this way. The answer is empty for a recipe the server
sent no entry for (the player's book does not hold it, or it was removed), for
one the server numbers differently, for one the book does not show (campfire,
stonecutting, smithing, a mod's own type), for an unknown id, and outside a
world.

The answer changes when the server adds, replaces or removes recipe book
entries and when recipes reload or arrive, and no hook announces a recipe book
change: query it when you use it, rather than keeping an answer from
`recipes_received`. For a viewer's "+" by recipe id alone, use
`transfer_recipe_id`.
