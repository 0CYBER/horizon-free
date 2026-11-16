# Recipe book categories and displays

Use `Hooks::recipe_book_categories` to supply categories and displays for your custom recipe
types. Append `RecipeBookCategory` values to `event.categories`. Register the book identity with
`recipe_book_type` during content registration and retain its `RecipeBookTypeKey`; declaring a
category does not register a book, change its settings or unlock recipes.

This example assumes `book` is your registered key and the named menu, items and recipe exist:

```rust,ignore
use mod_api::{RecipeBookCategory, RecipeBookDisplay, RecipeBookDisplayEntry, RecipeSlotDisplay};

event.categories.push(RecipeBookCategory {
    id: "mymod:crushing".into(),
    recipe_type: "mymod:crusher".into(),
    icon: "mymod:crusher".into(),
    target_menu: "mymod:crusher".into(),
    book: book.clone(),
    search_group: "mymod:crusher_search".into(),
    displays: vec![RecipeBookDisplayEntry {
        recipe: "mymod:crushed_stone".into(),
        displays: vec![RecipeBookDisplay::Shapeless {
            ingredients: vec![RecipeSlotDisplay::Item("minecraft:stone".into())],
            result: RecipeSlotDisplay::Item("minecraft:gravel".into()),
            station: RecipeSlotDisplay::Item("mymod:crusher".into()),
        }],
        placement: None,
    }],
});
```

## Declaration rules

| Field | Requirement |
| --- | --- |
| `id` | Unique namespaced category identifier outside `minecraft:`. |
| `recipe_type` | Namespaced identifier outside `minecraft:` present in the recipe table. Multiple categories can use one type. |
| `icon` | Registered item identifier other than `minecraft:air`. |
| `target_menu` | Registered menu identifier. |
| `book` | Registered `RecipeBookTypeKey`. All categories in one book name the same menu and search group. |
| `search_group` | Namespaced identifier outside `minecraft:` owned by exactly one book; it cannot also name a category. Search includes exactly that book's categories. |
| `displays` | Recipe entries belonging to this type. Each recipe identifier occurs only once across the declaration batch. |

Build displays from your type's recipe JSON through `Host::recipes_of_type`. The hook runs lazily
once per recipe table, not per frame. A client reads its own recipe table unless
`datapack_sync` supplies the server's custom type.

`host.recipe_book_category(recipe_type)` returns a validated declaration or `None` for an
undeclared or invalid type, before resources load, or outside a world. When a type has several
categories, it returns the first in category-id order. Implement
`Hooks::recipe_book_categories` to supply your declarations.

## Display and placement rules

`RecipeBookDisplay` provides `Shaped`, `Shapeless`, `Furnace`, `Stonecutter` and `Smithing` layouts.
Each recipe entry can supply several displays. `RecipeSlotDisplay` supports items, owned
`ItemStack` values, tags, fuel, empty slots, alternatives through `Composite`, potion and
component filters, dyeing, smithing trims and remainders.

- Resource references must resolve, and owned stacks must pass stack validation.
- A shaped display has positive width and height, each at most `i32::MAX`, and exactly
  `width * height` ingredient cells.
- Furnace duration is at most `i32::MAX`; experience is finite and nonnegative.
- Nested slot displays have a maximum depth of 32, with the outer slot at depth 0.
- When your menu has a registered `RecipeBookLayout`, its book and recipe type must match
  the category. Displays must fit the layout.
- For a recipe type bound to a menu placement layout, supply authoritative ingredient cells in
  `RecipeBookDisplayEntry::placement` using `PlacementInfo`. Display alternatives do not imply
  placement ingredients. The example's `None` does not declare a placement plan.

## Validation failures

The game validates the complete declaration generation before publishing any category. Invalid
identifiers, missing resources, duplicate categories or recipes, conflicting book ownership,
invalid layouts and invalid placement refuse the entire generation rather than skipping an entry.
The diagnostic begins `recipe book declarations refused:` and includes the typed error, such as
`InvalidId`, `UnknownResource`, `DuplicateCategory`, `DuplicateRecipe`, `ConflictingBook`,
`InvalidLayout` or `InvalidPlacement`.

Both successful and refused generations are cached until the recipe table changes. A recipe-table
replacement, new session or server `/reload` rebuilds declarations. Categories are ordered by
category identifier; a book's search group includes only its own categories, not categories from
other books sharing a menu.
