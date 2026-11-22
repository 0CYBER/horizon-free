//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `recipes` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait RecipesHost {
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The recipes of registered recipe type `kind` (`minecraft:crafting`, `mymod:crusher`), vanilla's
    /// types and mods' own alike, from `data/<namespace>/recipe/*.json` of vanilla,
    /// the content mods (`.hmod`) and the world's data packs, in file order: each
    /// one typed (`Recipe`: its ingredients, results, group, cooking time and
    /// experience, or its stonecutting, smithing or special crafting rule),
    /// with its file as JSON for a mod's own type. The file's type field names its
    /// serializer; this can differ from the reported recipe kind. Dynamic rules are not fixed
    /// outputs; use `LevelRef::recipe_for` for engine-matchable recipe types.
    /// Registered crafting kinds match through LevelRef::recipe_for. Cache other mod-owned
    /// types once per load/reload and match those in mod memory; those return
    /// RecipeLookupError::NotMatchable. These are the
    /// recipes in force (`RecipeManager.getAllRecipesFor`): data packs merged and
    /// load conditions held, so a pack's override or removal shows. A data pack
    /// changes them on `/reload`, so look them up again when you need them. Empty
    /// for a type nobody uses and outside a world. These are the server's table on
    /// the server thread, a dedicated server or a LAN host; a client, the
    /// single-player window included, sees vanilla's and the content mods' recipes,
    /// not the world's data packs', unless a mod had the server send it the type
    /// (`native_datapack_sync`).
    #[access(PublishedResources)]
    fn recipes_of_type(&self, kind: &str) -> Arc<[Recipe]>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The recipe types `recipes_of_type` answers, sorted: each type with a recipe in force and
    /// each type a registered serializer produces, vanilla's and mods' alike
    /// (`register_raw_recipe_serializer`). A recipe viewer passes them to `datapack_sync` on
    /// the server to be sent every mod's recipes, its machines' included. Recipe types, never
    /// serializer names (`minecraft:crafting`, not `minecraft:crafting_shaped`). Empty outside a
    /// world.
    #[access(PublishedResources)]
    fn recipe_types(&self) -> Arc<[String]>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// An ingredient from recipe-file JSON text: an item, a `#tag`, or a list
    /// of them, a bare id as `minecraft:` (as vanilla reads it); `test` says
    /// whether a stack fits. Items the game doesn't know are left out; None
    /// for missing tags, invalid ingredients or no known named items, and
    /// outside a world. An existing-empty tag returns an empty ingredient.
    /// Invalid JSON is refused with `RecipeJsonError::InvalidJson`.
    #[access(PublishedResources)]
    fn recipe_ingredient(&self, json: &str) -> Result<Option<Ingredient>, RecipeJsonError>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// A stack from recipe-file `result` JSON text: `id` (a bare one as
    /// `minecraft:`), `count` (1 by default, up to 99) and `components`. None
    /// without an `id`, for an item the game doesn't know or a count outside
    /// that, and outside a world.
    /// Invalid JSON is refused with `RecipeJsonError::InvalidJson`.
    #[access(PublishedResources)]
    fn recipe_result(&self, json: &str) -> Result<Option<ItemStack>, RecipeJsonError>;
    /// Side: both.
    /// The `book_id`s of the recipes the player's recipe book holds now
    /// (the server's book on a server); empty outside a world.
    #[access(WorldOnly)]
    fn known_recipes(&mut self) -> Arc<[i32]>;
    /// Side: both.
    /// Published asset/data read: available in Setup, Reloading and World.
    /// The validated category and icon of a custom recipe type, or None for
    /// an undeclared/invalid type, before resources load, or outside a world.
    /// Returns the first category in category-id order when a type has several.
    /// Any invalid declaration refuses the complete generation.
    /// Rebuilt when the recipe table changes. Does not unlock any recipes.
    #[access(PublishedResources)]
    fn recipe_book_category(&mut self, kind: &str) -> Option<RecipeBookCategory>;
}
/// The client host's methods of the `recipes` area, implemented by the game and reached through [`crate::ClientHost`].
#[horizon_mod_api_macros::host_area]
pub trait RecipesClientHost {
    /// Side: client.
    /// Shows the panel beside container screens with these items (at most
    /// 8192), as the game's own slots in pages, with a search box; the
    /// player's clicks, searches and keys over items come back as
    /// `panel_event`. The panel is the last mod's to show one; empty
    /// items and no recipe page take yours away.
    #[access(WorldOnly)]
    fn panel_items(&mut self, mod_id: &str, items: Vec<Stack>);
    /// Side: client.
    /// How mod `mod_id`'s item panel (`panel_items`) looks from now on:
    /// `PanelStyle::RecipeBook` (the default: the recipe book's page arrows
    /// and a plain search box), `PanelStyle::Viewer` (square page buttons on
    /// a bar over the items and the viewer's search box, as Horizon's Recipes
    /// panel draws them) or `PanelStyle::Classic` (JEI's list on Minecraft
    /// 1.12: flush right under page buttons that always show and wrap around,
    /// the search box at the foot of the screen with a settings button that
    /// opens the mod's page in Options > Mods). The viewer's look changes
    /// only the drawing; the classic one also lays the panel out. Kept for
    /// the session, whether or not the panel is showing.
    #[access(WorldOnly)]
    fn panel_style(&mut self, mod_id: &str, style: PanelStyle);
    /// Side: client.
    /// Places a recipe in the open screen's grid as a click in the recipe
    /// book does (`ServerboundPlaceRecipePacket`): the inventory's grid or a
    /// crafting table's. `all` places as many as the items allow, as with
    /// Shift. Refused while no such screen is open and for a recipe the
    /// player's book doesn't hold; one missing items shows as a ghost.
    #[access(WorldOnly)]
    fn place_recipe(&mut self, book_id: i32, all: bool) -> Result<(), String>;
    /// Side: client.
    /// Move available ingredients for a viewer recipe into the open crafting grid,
    /// without requiring the recipe to be learned. The screen shows any refusal.
    #[access(WorldOnly)]
    fn transfer_recipe(&mut self, book_id: i32);
    /// Side: client.
    /// A viewer's "+" for a recipe by its id, into the open menu: a machine's mod menu as
    /// well as a crafting grid. The server asks the `menu_transfer` part of the open mod menu's
    /// type first, then makes the game's own grid transfer; the screen shows the result as for
    /// `transfer_recipe`. `max` moves as many sets as the items allow (Shift). Shown as
    /// `NotIntoMenu` with no container screen open, in spectator mode, and on a server that
    /// doesn't take viewer transfers. In a solo world or on a LAN host the game is the server:
    /// its own player's transfer takes the same path, part first, then the grid.
    #[access(WorldOnly)]
    fn transfer_recipe_id(&mut self, recipe: &str, max: bool);
    /// Side: client.
    /// What `transfer_recipe(book_id)` would do now, worked out by the server
    /// without moving anything: the player's inventory, the grid and every
    /// container a mod lends the grid's recipes (a crafting station's chests,
    /// which the client can't see) counted. For a viewer's "+", to mark the
    /// inputs it lacks. The first call for a recipe asks and returns None;
    /// the answer comes a moment later and is kept for that recipe while the
    /// menu, the inventory and the grid stay as they were; while it is still
    /// wanted it is asked again every two seconds, at most fifteen times for
    /// one state of the menu. None outside a screen that
    /// takes `transfer_recipe` and on a server that doesn't answer it.
    #[access(WorldOnly)]
    fn transfer_recipe_check(&mut self, book_id: i32) -> Option<RecipeTransferCheck>;
    /// Side: client.
    /// The `book_id`s of the recipe `recipe_id` (`minecraft:stick`), the ids
    /// `transfer_recipe`, `transfer_recipe_check`, `place_recipe` and
    /// `known_recipes` use: one for each way the recipe book shows it, in the
    /// book's order. For a recipe viewer, which has the recipe from
    /// `recipes_of_type` and needs its id for "+". A server numbers its
    /// recipes itself and names none of them, so an id is given only when the
    /// server sent the recipe book entry with that number and this client's
    /// own numbering of the recipe agrees with it: the entry is the same
    /// recipe (the same result, category and ingredients), so two different
    /// recipes with identical displays cannot be told apart. Empty otherwise: a
    /// recipe the server sent no entry for (the player's book does not hold
    /// it, or it was removed), one the server numbers differently, one the
    /// book doesn't show (campfire, stonecutting, smithing, a mod's own
    /// type), an unknown id, and outside a world. The answer changes when
    /// the server adds, replaces or removes entries and when recipes reload
    /// or arrive, and no hook announces a recipe book change: query it when
    /// you use it, rather than keeping an answer from `recipes_received`.
    #[access(WorldOnly)]
    fn recipe_book_ids(&mut self, recipe_id: &str) -> Arc<[i32]>;
    /// Side: client.
    /// Shows these recipes in mod `mod_id`'s panel (`panel_items`) instead of
    /// its items, as many as fit a page, under `title` and a back button,
    /// with a row of `tabs` between them (each an icon; `tab` is the
    /// selected one). The game lays each one out (`RecipeView`) with its own
    /// slots, arrow, flame, fluid wells and action button, and pages them to
    /// the panel's height. At most 256 recipes and 16 tabs; no recipes shows
    /// the items again. The same title and tab again keep the page the player
    /// turned to. The player's clicks on a slot, an action button, the back
    /// button, a tab and the page arrows come back as `panel_event`
    /// (`SlotClicked`, `Action`, `Back`, `Tab`, `Page`).
    #[access(WorldOnly)]
    fn panel_recipes(
        &mut self,
        mod_id: &str,
        title: &str,
        tabs: Vec<Stack>,
        tab: u32,
        views: Vec<RecipeView>,
    );
}
/// The handles' reads of the `recipes` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait RecipesBackend {
    /// Side: both. Exact load-time membership for component-independent ingredients.
    /// Custom/component predicates anywhere in the tree are NotEnumerable.
    #[access(PublishedResources)]
    fn recipe_ingredient_item_keys(
        &self,
        ingredient: &Ingredient,
    ) -> Result<Arc<[ItemKey]>, IngredientKeysError>;
    /// Side: both.
    /// Whether `stack` fits `ingredient` (`Ingredient.test`), matched with this
    /// game's loop context.
    #[access(WorldOnly)]
    fn recipe_ingredient_test(&self, ingredient: &Ingredient, stack: &ItemStack) -> bool;
}
