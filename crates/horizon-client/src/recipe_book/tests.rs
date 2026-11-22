/// A book saved before #7186's fix holds bare names: reading fills the
/// namespace in, so those recipes show in the book too.
#[test]
fn a_book_saved_with_bare_names_reads_qualified() {
    use horizon_core::nbt::{Compound, Tag};
    let list = || Tag::List(vec![Tag::String("folder_recipe".into())]);
    let mut compound = Compound::new();
    compound.insert("recipes".into(), list());
    compound.insert("toBeDisplayed".into(), list());
    let tag = Tag::Compound(compound);
    let read = Settings::read_valid(Some(&tag), &|_| true);
    assert_eq!(
        read.recipes,
        ["minecraft:folder_recipe".to_owned()].into_iter().collect()
    );
    assert!(read.to_be_displayed.contains("minecraft:folder_recipe"));
}

use super::*;

fn content_recipe_root(test: &str, recipe: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-recipe-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
    std::fs::create_dir_all(root.join("data/example/recipe")).unwrap();
    std::fs::write(
        root.join(horizon_items::content_namespaces::MARKER),
        br#"["example"]"#,
    )
    .unwrap();
    std::fs::write(root.join("data/example/recipe/test.json"), recipe).unwrap();
    root
}

const CONTENT_RECIPE: &str = r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:stick"}}"#;

#[test]
fn content_recipe_is_indexed() {
    let root = content_recipe_root("index", CONTENT_RECIPE);
    let index = RecipeIndex::load(&root, &PackOverlay::NONE);
    assert!(index.contains("example:test"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn content_recipe_unlocks_and_appears_in_book() {
    let root = content_recipe_root("unlock", CONTENT_RECIPE);
    let items = crate::items::Items::for_tests(HashMap::new());
    let recipes = Recipes::load(
        &root,
        &items,
        &PackOverlay::NONE,
        &crate::test_resources::server_load(),
    )
    .unwrap();
    let index = RecipeIndex::load(&root, &PackOverlay::NONE);
    let mut settings = Settings::default();
    assert_eq!(
        settings.add_recipes(&index, &["example:test".into()]),
        (1, vec!["example:test".into()])
    );
    let mut book = Book::build(&recipes, &items);
    book.set_display_ids(&index);
    let mut client = ClientBook::default();
    for id in index.display_ids("example:test") {
        client.add(id);
    }
    let collections = client.collections(&book, BookType::Crafting, Tab::Search);
    assert!(
        collections
            .iter()
            .flat_map(|c| &c.entries)
            .any(|&i| book.entries[i].id == "example:test")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn content_recipe_condition_gates_crafting_and_index() {
    let recipe = r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:stick"},"neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:true"}}]}"#;
    let root = content_recipe_root("condition", recipe);
    let items = crate::items::Items::for_tests(HashMap::new());
    let recipes = Recipes::load(
        &root,
        &items,
        &PackOverlay::NONE,
        &crate::test_resources::server_load(),
    )
    .unwrap();
    assert!(
        recipes
            .find(
                crate::test_resources::server_loop_context(),
                &[Some(ItemStack::new("minecraft:flint", 1)), None, None, None],
                2
            )
            .is_none()
    );
    assert!(!RecipeIndex::load(&root, &PackOverlay::NONE).contains("example:test"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn overlay_replaces_content_recipe_in_crafting_and_index() {
    let root = content_recipe_root("overlay", CONTENT_RECIPE);
    let replacement = br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:torch"}}"#;
    let files = [("recipe/example:test".to_owned(), vec![replacement.to_vec()])];
    let overlay = PackOverlay::new(&files);
    let items = crate::items::Items::for_tests(HashMap::new());
    let recipes = Recipes::load(
        &root,
        &items,
        &overlay,
        &crate::test_resources::server_load(),
    )
    .unwrap();
    let (_, result) = recipes
        .find(
            crate::test_resources::server_loop_context(),
            &[Some(ItemStack::new("minecraft:flint", 1)), None, None, None],
            2,
        )
        .unwrap();
    assert_eq!(&*result.item, "minecraft:torch");
    assert_eq!(
        RecipeIndex::load(&root, &overlay).result("example:test"),
        Some("minecraft:torch")
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn ing(items: &[&str]) -> Ingredient {
    Ingredient::of(items.iter().map(|s| (*s).to_owned()).collect())
}
#[test]
fn the_index_knows_each_recipes_toast_station_and_result() {
    let root = std::env::temp_dir().join(format!("horizon-toast-index-{}", std::process::id()));
    let dir = root.join("data/minecraft/recipe");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, json) in [
        (
            "stick",
            r#"{"type":"minecraft:crafting_shaped","result":{"id":"minecraft:stick"}}"#,
        ),
        (
            "crafting_table",
            r#"{"type":"minecraft:crafting_shaped","show_notification":false,"result":{"id":"minecraft:crafting_table"}}"#,
        ),
        (
            "cooked_beef_from_campfire_cooking",
            r#"{"type":"minecraft:campfire_cooking","result":{"id":"minecraft:cooked_beef"}}"#,
        ),
        (
            "stone_slab_from_stonecutting",
            r#"{"type":"minecraft:stonecutting","result":{"id":"minecraft:stone_slab"}}"#,
        ),
        (
            "coast_trim",
            r#"{"type":"minecraft:smithing_trim","pattern":"minecraft:coast"}"#,
        ),
    ] {
        std::fs::write(dir.join(format!("{name}.json")), json).unwrap();
    }
    let index = RecipeIndex::load(&root, &PackOverlay::NONE);
    std::fs::remove_dir_all(&root).unwrap();
    let toast = |id: &str| (index.toast_station(id), index.result(id));
    assert_eq!(
        toast("minecraft:stick"),
        (Some("minecraft:crafting_table"), Some("minecraft:stick"))
    );
    // show_notification: false unlocks silently.
    assert_eq!(toast("minecraft:crafting_table").0, None);
    assert_eq!(
        toast("minecraft:cooked_beef_from_campfire_cooking"),
        (Some("minecraft:campfire"), Some("minecraft:cooked_beef"))
    );
    assert_eq!(
        toast("minecraft:stone_slab_from_stonecutting").0,
        Some("minecraft:stonecutter")
    );
    assert_eq!(
        toast("minecraft:coast_trim"),
        (Some("minecraft:smithing_table"), None)
    );
}
#[test]
fn content_namespace_recipes_have_display_ids() {
    let root = std::env::temp_dir().join(format!(
        "horizon-mod-recipe-index-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let vanilla = root.join("data/minecraft/recipe");
    let content = root.join("data/recipe_pilot/recipe");
    std::fs::create_dir_all(&vanilla).unwrap();
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(
        root.join(horizon_items::content_namespaces::MARKER),
        br#"["recipe_pilot"]"#,
    )
    .unwrap();
    std::fs::write(
        vanilla.join("control.json"),
        br#"{"type":"minecraft:crafting_shapeless","result":{"id":"minecraft:stick"}}"#,
    )
    .unwrap();
    std::fs::write(
        content.join("flint_to_echo_shard.json"),
        br#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:flint"],"result":{"id":"minecraft:echo_shard","count":2}}"#,
    )
    .unwrap();
    std::fs::write(
        content.join("gated.json"),
        br#"{"type":"minecraft:crafting_shapeless","neoforge:conditions":[{"type":"neoforge:mod_loaded","modid":"absent"}],"result":{"id":"minecraft:diamond"}}"#,
    )
    .unwrap();
    let index = RecipeIndex::load(&root, &PackOverlay::NONE);
    std::fs::remove_dir_all(&root).unwrap();
    let id = "recipe_pilot:flint_to_echo_shard";
    let display = index
        .first_display(id)
        .expect("content recipe has a display ID");
    assert!(display >= 0);
    assert_eq!(index.recipe_of_display(display), Some(id));
    assert_eq!(index.result(id), Some("minecraft:echo_shard"));
    assert!(index.first_display("minecraft:control").is_some());
    assert!(!index.contains("recipe_pilot:gated"));
}

#[test]
fn small_recipes_are_centred_in_the_grid() {
    // One ingredient in a 3x3 grid goes to the middle cell.
    assert_eq!(place_cells(3, 3, 1, 1, &['a']), vec![('a', 4)]);
    // A 2x2 recipe fills the top-left corner.
    let cells: Vec<usize> = place_cells(3, 3, 2, 2, &['a', 'b', 'c', 'd'])
        .into_iter()
        .map(|(_, i)| i)
        .collect();
    assert_eq!(cells, vec![0, 1, 3, 4]);
    // A one-row recipe sits on the middle row.
    let row: Vec<usize> = place_cells(3, 3, 3, 1, &['a', 'b', 'c'])
        .into_iter()
        .map(|(_, i)| i)
        .collect();
    assert_eq!(row, vec![3, 4, 5]);
    // The 2x2 grid is never centred.
    let small: Vec<usize> = place_cells(2, 2, 1, 1, &['a'])
        .into_iter()
        .map(|(_, i)| i)
        .collect();
    assert_eq!(small, vec![0]);
}
#[test]
fn collections_group_recipes_of_a_category() {
    let entry = |id: &str, display_id, group: Option<&str>, category| Entry {
        id: id.into(),
        display_id,
        category,
        group: group.map(str::to_owned),
        display: Display::Shapeless(vec![ing(&["minecraft:stick"])]),
        result: ItemStack::new("minecraft:stick", 1),
        variants: Vec::new(),
        stacks: Vec::new(),
    };
    let book = Book {
        entries: vec![
            entry("minecraft:a", 0, Some("planks"), Category::CraftingBuilding),
            entry("minecraft:b", 1, Some("planks"), Category::CraftingBuilding),
            entry("minecraft:c", 2, None, Category::CraftingBuilding),
            entry("minecraft:d", 3, Some("planks"), Category::CraftingMisc),
        ],
        placements: HashMap::new(),
        order: HashMap::new(),
        server_requirements: HashMap::new(),
        server_groups: HashMap::new(),
        ..Default::default()
    };
    let mut client = ClientBook::default();
    for d in 0..4 {
        client.add(d);
    }
    let tab = Tab::Category(Category::CraftingBuilding);
    let c = client.collections(&book, BookType::Crafting, tab);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].entries, vec![0, 1]);
    // The search tab lists every category; the same group in another category is separate.
    let all = client.collections(&book, BookType::Crafting, Tab::Search);
    assert_eq!(all.len(), 3);
    // Displays the client was not sent are not listed.
    client.clear();
    client.add(2);
    assert_eq!(client.collections(&book, BookType::Crafting, tab).len(), 1);
}
#[test]
fn display_ids_collide_in_a_small_table() {
    // A 16-bucket table: 17 and 1 share a bucket and keep arrival order.
    let mut client = ClientBook::default();
    for d in [17, 3, 1] {
        client.add(d);
    }
    assert_eq!(client.displays(), vec![17, 1, 3]);
}
#[test]
fn search_reads_tooltip_lines_and_ids() {
    let book = Book {
        entries: vec![Entry {
            id: "minecraft:stick".into(),
            display_id: 0,
            category: Category::CraftingMisc,
            group: None,
            display: Display::Shapeless(Vec::new()),
            result: ItemStack::new("minecraft:stick", 4),
            variants: Vec::new(),
            stacks: Vec::new(),
        }],
        placements: HashMap::new(),
        order: HashMap::new(),
        server_requirements: HashMap::new(),
        server_groups: HashMap::new(),
        ..Default::default()
    };
    let lines = |s: &ItemStack| vec![format!("Item {}", s.item), "Stick".to_owned()];
    assert!(book.collection_matches(&[0], "STI", &lines));
    // Plain text searches tooltips only, not the registry path.
    assert!(!book.collection_matches(&[0], "stick:", &lines));
    assert!(book.collection_matches(&[0], "minecraft:stic", &lines));
    assert!(!book.collection_matches(&[0], "other:stick", &lines));
}
#[test]
fn settings_round_trip() {
    let mut s = Settings::default();
    s.open[BookType::Furnace.index()] = true;
    s.filtering[BookType::Crafting.index()] = true;
    s.recipes = ["minecraft:stick".to_owned()].into();
    let back = Settings::read(Some(&s.write()));
    assert_eq!(back.open, s.open);
    assert_eq!(back.filtering, s.filtering);
    assert_eq!(back.recipes, s.recipes);
}
#[test]
fn a_knowledge_book_validates_recipes_and_is_used_up() {
    let root = std::env::temp_dir().join(format!("horizon-knowledge-book-{}", std::process::id()));
    let dir = root.join("data/minecraft/recipe");
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["stick", "crafting_table", "torch"] {
        std::fs::write(
            dir.join(format!("{name}.json")),
            format!(
                r#"{{"type":"minecraft:crafting_shaped","result":{{"id":"minecraft:{name}"}}}}"#
            ),
        )
        .unwrap();
    }
    let index = RecipeIndex::load(&root, &PackOverlay::NONE);
    std::fs::remove_dir_all(&root).unwrap();
    let book = |recipes: &[&str], count: u32| {
        let ids = recipes.iter().map(|id| Tag::String((*id).into())).collect();
        let mut stack = ItemStack::new("minecraft:knowledge_book", count);
        stack
            .data_mut()
            .set_other("minecraft:recipes", Some(Tag::List(ids)));
        stack
    };

    // A player who knows no recipe, with a stack of two books naming two.
    let mut hand = Some(book(&["minecraft:stick", "minecraft:crafting_table"], 2));
    let unlocked = Settings::knowledge_book_recipes(&index, &mut hand, false).unwrap();
    assert_eq!(unlocked, ["minecraft:stick", "minecraft:crafting_table"]);
    // One book used up, the second still in the hand.
    assert_eq!(hand.as_ref().map(|s| s.count), Some(1));

    // A creative player has infinite materials: the book stays, and a
    // recipe already known is still the success `awardRecipes` reports.
    let mut hand = Some(book(&["minecraft:stick"], 1));
    assert_eq!(
        Settings::knowledge_book_recipes(&index, &mut hand, true),
        Ok(vec!["minecraft:stick".to_owned()])
    );
    assert_eq!(hand.as_ref().map(|s| s.count), Some(1));

    // One id this server has no recipe for (vanilla's "Invalid recipe: {}"):
    // the recipe beside it is not learned either, and the book is gone all
    // the same — vanilla consumes it before it looks the ids up.
    let mut hand = Some(book(&["minecraft:torch", "minecraft:no_such_recipe"], 1));
    assert_eq!(
        Settings::knowledge_book_recipes(&index, &mut hand, false),
        Err(Some("minecraft:no_such_recipe".to_owned()))
    );
    assert_eq!(hand, None);

    // A book that names no recipe at all (a bare `/give`): nothing to
    // learn, and vanilla fails after using it up like any other.
    let mut hand = Some(ItemStack::new("minecraft:knowledge_book", 1));
    assert_eq!(
        Settings::knowledge_book_recipes(&index, &mut hand, false),
        Err(None)
    );
    assert_eq!(hand, None);
}

/// `RecipeManager.unpackRecipeInfo` numbers the displays in registration
/// order: the file's path (with `.json`) first, then its namespace.
#[test]
fn display_ids_follow_registration_order() {
    let root = std::env::temp_dir().join(format!("horizon-display-order-{}", std::process::id()));
    let dir = root.join("data/minecraft/recipe");
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = r#"{"type":"minecraft:crafting_shapeless","result":{"id":"minecraft:stick"}}"#;
    for name in ["z_pick", "prefix", "prefix-early"] {
        std::fs::write(dir.join(format!("{name}.json")), recipe).unwrap();
    }
    let packs = [(
        "recipe/parity:a_pick".to_owned(),
        vec![recipe.as_bytes().to_vec()],
    )];
    let index = RecipeIndex::load(&root, &PackOverlay::new(&packs));
    std::fs::remove_dir_all(&root).unwrap();
    let ids = [
        "parity:a_pick",
        "minecraft:prefix-early",
        "minecraft:prefix",
        "minecraft:z_pick",
    ];
    assert_eq!(
        ids.map(|id| index.first_display(id)),
        [Some(0), Some(1), Some(2), Some(3)]
    );
    assert_eq!(index.recipe_of_display(0), Some("parity:a_pick"));
}

/// A client sent a server's recipes (`horizon:recipe_content`) numbers
/// them over the same set the server does: a world's data pack recipe
/// moves every display after it, here `copper_axe` and `fishing_rod`,
/// and the client's own files alone would number them one lower, so the
/// server's book entries landed on other recipes (Braid's Copper-Reel
/// Rod vanished from the book and from recipe viewers).
#[test]
fn synced_recipes_number_their_displays_as_the_server_does() {
    let root = std::env::temp_dir().join(format!("horizon-synced-order-{}", std::process::id()));
    let dir = root.join("data/minecraft/recipe");
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stick"],"result":{"id":"minecraft:stick"}}"#;
    for name in ["acacia_boat", "copper_axe", "fishing_rod"] {
        std::fs::write(dir.join(format!("{name}.json")), recipe).unwrap();
    }
    let pack: serde_json::Value = serde_json::from_str(recipe).unwrap();
    let packs = [(
        "recipe/braid:cauldron_from_copper".to_owned(),
        vec![recipe.as_bytes().to_vec()],
    )];
    let server = RecipeIndex::load(&root, &PackOverlay::new(&packs));
    let own = horizon_items::recipe_sources::sources(&root, &PackOverlay::NONE).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let client = RecipeIndex::load_from_sources(&own);
    let mut synced = own;
    synced.push(("braid:cauldron_from_copper".to_owned(), pack));
    let synced = RecipeIndex::load_from_sources(&synced);
    let ids = [
        "minecraft:acacia_boat",
        "braid:cauldron_from_copper",
        "minecraft:copper_axe",
        "minecraft:fishing_rod",
    ];
    assert_eq!(
        ids.map(|id| server.first_display(id)),
        [Some(0), Some(1), Some(2), Some(3)]
    );
    assert_eq!(
        ids.map(|id| synced.first_display(id)),
        ids.map(|id| server.first_display(id))
    );
    // The client's own files alone: the pack's recipe is not there, and
    // the server's 2 is the client's `fishing_rod`.
    assert_eq!(client.first_display("minecraft:copper_axe"), Some(1));
    assert_eq!(client.recipe_of_display(2), Some("minecraft:fishing_rod"));
}

// ---- a server whose display ids are not this client's (#11705) ----

use horizon_protocol::packets::recipes::RecipeBookAdd;

/// The ids of the items the packet below names.
const ITEMS: [&str; 6] = [
    "minecraft:tnt",
    "minecraft:piston",
    "minecraft:stick",
    "minecraft:diamond",
    "minecraft:gunpowder",
    "minecraft:blaze_rod",
];
fn item_name(id: i32) -> Option<String> {
    ITEMS
        .get(usize::try_from(id).ok()?)
        .map(|n| (*n).to_owned())
}
/// The server's category ids: 0 building, 1 redstone, 2 equipment, 3 misc,
/// 4 stonecutter (a tab this book has none for); any other is unknown.
fn category_name(id: i32) -> Option<String> {
    if id == 4 {
        return Some("minecraft:stonecutter".to_owned());
    }
    ["building_blocks", "redstone", "equipment", "misc"]
        .get(usize::try_from(id).ok()?)
        .map(|n| format!("minecraft:crafting_{n}"))
}
/// The book of this client: it numbers `tnt`, `piston`, `stick` 10, 11, 12.
fn local_book() -> Book {
    let entry = |id: &str, display_id, category| Entry {
        id: id.into(),
        display_id,
        category,
        group: None,
        display: Display::Shapeless(vec![ing(&["minecraft:gunpowder"])]),
        result: ItemStack::new(id, 1),
        variants: Vec::new(),
        stacks: Vec::new(),
    };
    Book {
        entries: vec![
            entry("minecraft:tnt", 10, Category::CraftingRedstone),
            entry("minecraft:piston", 11, Category::CraftingRedstone),
            entry("minecraft:stick", 12, Category::CraftingMisc),
        ],
        placements: HashMap::new(),
        order: HashMap::new(),
        server_requirements: HashMap::new(),
        server_groups: HashMap::new(),
        ..Book::default()
    }
}
/// A server entry: a shapeless recipe of one gunpowder for `item`.
fn packet_entry(display: i32, item: i32, category: i32) -> RecipeBookEntry {
    RecipeBookEntry {
        id: display,
        display: RecipeDisplay::Shapeless {
            ingredients: vec![SlotDisplay::Item(4)],
            result: SlotDisplay::Item(item),
            station: SlotDisplay::Item(0),
        },
        group: 0,
        category,
        requirements: Some(vec![HolderSet::Items(vec![4])]),
        flags: 0,
    }
}
/// A server numbering by its own recipes: a plugin recipe at 5, then the
/// vanilla ones far from where this client numbers them. Its category
/// ids: 1 redstone, 3 misc.
fn paper_packet() -> RecipeBookAdd {
    RecipeBookAdd {
        entries: vec![
            packet_entry(5, 3, 3),
            packet_entry(110, 0, 1),
            packet_entry(111, 1, 1),
            packet_entry(112, 2, 3),
        ],
        replace: true,
    }
}
/// A server whose TNT has the number this client gives the stick.
fn colliding_packet() -> RecipeBookAdd {
    RecipeBookAdd {
        entries: vec![packet_entry(12, 0, 1)],
        replace: true,
    }
}
/// What a tab lists once the client has `packet`.
fn tab_after(packet: &RecipeBookAdd, category: Category) -> Vec<String> {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    book.use_server_entries(
        &packet.entries,
        &PacketNames {
            item: &item_name,
            category: &category_name,
            tag: &tags,
        },
    );
    let mut client = ClientBook::default();
    for e in &packet.entries {
        client.add(e.id);
    }
    let mut names: Vec<String> = client
        .collections(&book, BookType::Crafting, Tab::Category(category))
        .iter()
        .flat_map(|c| {
            c.entries
                .iter()
                .map(|&i| book.entries[i].result.item.to_string())
        })
        .collect();
    names.sort();
    names
}
#[test]
fn the_book_lists_the_recipes_a_servers_packet_names() {
    let packet = paper_packet();
    assert_eq!(
        tab_after(&packet, Category::CraftingRedstone),
        ["minecraft:piston", "minecraft:tnt"]
    );
    assert_eq!(
        tab_after(&packet, Category::CraftingMisc),
        ["minecraft:diamond", "minecraft:stick"]
    );
}
#[test]
fn a_number_this_client_gives_another_recipe_shows_the_servers() {
    let packet = colliding_packet();
    assert_eq!(
        tab_after(&packet, Category::CraftingRedstone),
        ["minecraft:tnt"]
    );
    assert!(tab_after(&packet, Category::CraftingMisc).is_empty());
}

fn names<'a>(tags: &'a dyn Fn(&str) -> Vec<String>) -> PacketNames<'a> {
    PacketNames {
        item: &item_name,
        category: &category_name,
        tag: tags,
    }
}
#[test]
fn a_server_of_this_clients_recipes_keeps_the_books_own_entries() {
    // The same number, result, category and ingredient count: the entry
    // the client made stands, so its order and ghost stay as they were.
    let mut book = local_book();
    let before: Vec<String> = book.entries.iter().map(|e| e.id.clone()).collect();
    let tags = |_: &str| Vec::new();
    let packet = RecipeBookAdd {
        entries: vec![packet_entry(10, 0, 1), packet_entry(11, 1, 1)],
        replace: true,
    };
    book.use_server_entries(&packet.entries, &names(&tags));
    let after: Vec<String> = book.entries.iter().map(|e| e.id.clone()).collect();
    assert_eq!(after, before);
    assert!(book.server_requirements.is_empty());
}
fn ids(book: &Book, recipe: &str) -> Vec<i32> {
    book.confirmed_ids(recipe).to_vec()
}
#[test]
fn a_recipe_is_tied_to_the_number_a_server_sent_for_the_same_recipe() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    // The server numbers the tnt and the piston as this book does; it sent
    // no stick.
    book.use_server_entries(
        &[packet_entry(10, 0, 1), packet_entry(11, 1, 1)],
        &names(&tags),
    );
    assert_eq!(ids(&book, "minecraft:tnt"), [10]);
    assert_eq!(ids(&book, "minecraft:piston"), [11]);
    assert_eq!(ids(&book, "minecraft:stick"), Vec::<i32>::new());
    // The book's entry at that number is what a transfer by number finds.
    let at = book.entry_of_display(10).unwrap();
    assert_eq!(book.entries[at].id, "minecraft:tnt");
}
#[test]
fn a_number_the_server_gives_another_recipe_ties_no_name_to_it() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    // This book's stick is number 12; the server's 12 is a tnt.
    book.use_server_entries(&colliding_packet().entries, &names(&tags));
    for recipe in ["minecraft:stick", "minecraft:tnt", "server:12"] {
        assert_eq!(ids(&book, recipe), Vec::<i32>::new(), "{recipe}");
    }
    // A server of other numbers throughout.
    let mut book = local_book();
    book.use_server_entries(&paper_packet().entries, &names(&tags));
    for recipe in ["minecraft:tnt", "minecraft:piston", "minecraft:stick"] {
        assert_eq!(ids(&book, recipe), Vec::<i32>::new(), "{recipe}");
    }
}
#[test]
fn a_colliding_packet_applied_twice_confirms_no_server_entry() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    // The first packet replaces this book's stick at 12 with a `server:` entry, the second
    // matches that entry: neither ties a name to 12.
    book.use_server_entries(&colliding_packet().entries, &names(&tags));
    book.use_server_entries(&colliding_packet().entries, &names(&tags));
    let at = book.entry_of_display(12).unwrap();
    assert!(book.entries[at].id.starts_with("server:"));
    assert_eq!(ids(&book, &book.entries[at].id), Vec::<i32>::new());
    assert_eq!(ids(&book, "minecraft:stick"), Vec::<i32>::new());
}
#[test]
fn an_entry_of_an_unknown_category_withdraws_that_numbers_confirmation() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    book.use_server_entries(&[packet_entry(10, 0, 1)], &names(&tags));
    assert_eq!(ids(&book, "minecraft:tnt"), [10]);
    // The server's new entry at 10 is of a category this client doesn't know: unverified.
    book.use_server_entries(&[packet_entry(10, 0, 9999)], &names(&tags));
    assert_eq!(ids(&book, "minecraft:tnt"), Vec::<i32>::new());
}
#[test]
fn renumbering_the_book_drops_its_confirmations() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    book.use_server_entries(&[packet_entry(10, 0, 1)], &names(&tags));
    assert_eq!(ids(&book, "minecraft:tnt"), [10]);
    book.set_display_ids(&RecipeIndex::default());
    assert_eq!(ids(&book, "minecraft:tnt"), Vec::<i32>::new());
}
#[test]
fn unknown_empty_and_very_long_names_have_no_ids() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    book.use_server_entries(&[packet_entry(10, 0, 1)], &names(&tags));
    for recipe in ["", "minecraft:nope", &"x".repeat(100_000)] {
        assert_eq!(ids(&book, recipe), Vec::<i32>::new());
    }
    assert_eq!(ids(&Book::default(), "minecraft:tnt"), Vec::<i32>::new());
}
#[test]
fn the_ids_follow_added_removed_and_replaced_entries() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    book.use_server_entries(&[packet_entry(10, 0, 1)], &names(&tags));
    assert_eq!(ids(&book, "minecraft:piston"), Vec::<i32>::new());
    // Added later: the answer asked for before is not kept.
    book.use_server_entries(&[packet_entry(11, 1, 1)], &names(&tags));
    assert_eq!(ids(&book, "minecraft:piston"), [11]);
    assert_eq!(ids(&book, "minecraft:tnt"), [10]);
    // Removed: the server no longer vouches for 11.
    book.unconfirm(11);
    assert_eq!(ids(&book, "minecraft:piston"), Vec::<i32>::new());
    assert_eq!(ids(&book, "minecraft:tnt"), [10]);
    // Replaced by entries of other numbers, as after a reload.
    let mut book = local_book();
    book.use_server_entries(&[packet_entry(10, 0, 1)], &names(&tags));
    assert_eq!(ids(&book, "minecraft:tnt"), [10]);
    let mut reloaded = local_book();
    reloaded.use_server_entries(&colliding_packet().entries, &names(&tags));
    assert_eq!(ids(&reloaded, "minecraft:tnt"), Vec::<i32>::new());
}
#[test]
fn an_entry_this_book_cannot_show_hides_the_recipe_that_has_its_number() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    let mut stonecutter = packet_entry(11, 1, 1);
    stonecutter.display = RecipeDisplay::Stonecutter {
        input: SlotDisplay::Item(4),
        result: SlotDisplay::Item(1),
        station: SlotDisplay::Item(0),
    };
    book.use_server_entries(&[stonecutter], &names(&tags));
    // Number 11 was the piston here: not on this server.
    assert_eq!(book.entry_of_display(11), None);
    assert_eq!(book.entry_of_display(10), Some(0));
}
#[test]
fn the_craftable_filter_asks_the_servers_ingredients() {
    use crate::menu::place::Stacked;
    let mut book = local_book();
    let tags = |tag: &str| {
        if tag == "minecraft:planks" {
            vec!["minecraft:stick".to_owned()]
        } else {
            Vec::new()
        }
    };
    let mut asks_planks = packet_entry(110, 0, 1);
    asks_planks.requirements = Some(vec![HolderSet::Tag("minecraft:planks".into())]);
    let mut asks_nothing_known = packet_entry(111, 1, 1);
    asks_nothing_known.requirements = None;
    let mut asks_none = packet_entry(112, 2, 3);
    asks_none.requirements = Some(Vec::new());
    book.use_server_entries(&[asks_planks, asks_nothing_known, asks_none], &names(&tags));
    let stock = Stacked::default();
    let craft = |display| {
        let i = book.entry_of_display(display).unwrap();
        book.can_craft(&book.entries[i], &stock)
    };
    // Nothing in hand: the tag's item is missing, no list is never
    // craftable (`RecipeDisplayEntry.canCraft`), an empty list always is.
    assert!(!craft(110));
    assert!(!craft(111));
    assert!(craft(112));
}
#[test]
fn a_servers_tag_ingredient_shows_the_items_the_client_knows_for_it() {
    let mut book = local_book();
    let tags = |tag: &str| {
        assert_eq!(tag, "minecraft:planks");
        vec!["minecraft:stick".to_owned(), "minecraft:diamond".to_owned()]
    };
    let mut e = packet_entry(110, 0, 1);
    e.display = RecipeDisplay::Shapeless {
        ingredients: vec![SlotDisplay::Tag(HolderSet::Tag("minecraft:planks".into()))],
        result: SlotDisplay::Item(0),
        station: SlotDisplay::Item(0),
    };
    book.use_server_entries(&[e], &names(&tags));
    let entry = &book.entries[book.entry_of_display(110).unwrap()];
    let items: Vec<&str> = entry.requirements()[0]
        .order
        .iter()
        .map(String::as_str)
        .collect();
    assert_eq!(items, ["minecraft:stick", "minecraft:diamond"]);
}

#[test]
fn an_entry_that_asks_for_other_ingredients_is_not_the_books_recipe() {
    // The server's TNT takes diamonds where this client's takes gunpowder
    // (the stick of planks and the stick of bamboo, the iron ingot of any
    // ore): same result, same category, same number of ingredients.
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    let mut tnt = packet_entry(10, 0, 1);
    tnt.display = RecipeDisplay::Shapeless {
        ingredients: vec![SlotDisplay::Item(3)],
        result: SlotDisplay::Item(0),
        station: SlotDisplay::Item(0),
    };
    book.use_server_entries(&[tnt], &names(&tags));
    let entry = &book.entries[book.entry_of_display(10).unwrap()];
    assert_eq!(entry.id, "server:10");
    assert_eq!(entry.requirements()[0].order, ["minecraft:diamond"]);
}
#[test]
fn the_entries_taken_join_the_group_of_the_ones_kept() {
    let mut book = local_book();
    book.entries[2].group = Some("planks".into());
    let tags = |_: &str| Vec::new();
    // The server's group 7: the stick this book has, and a recipe it lacks.
    let mut kept = packet_entry(12, 2, 3);
    kept.group = 7;
    let mut taken = packet_entry(50, 3, 3);
    taken.group = 7;
    // The taken one comes first in the packet.
    let packet = [taken, kept];
    book.use_server_entries(&packet, &names(&tags));
    let mut client = ClientBook::default();
    for e in &packet {
        client.add(e.id);
    }
    let misc = client.collections(
        &book,
        BookType::Crafting,
        Tab::Category(Category::CraftingMisc),
    );
    assert_eq!(misc.len(), 1);
    assert_eq!(misc[0].entries.len(), 2);
}
#[test]
fn an_entry_of_a_category_this_client_lacks_changes_nothing() {
    // A registry the client does not have (or a server newer than it):
    // every entry is of an unknown category, and the book stays its own.
    let mut book = local_book();
    let no_category = |_: i32| None;
    let tags = |_: &str| Vec::new();
    let packet = [packet_entry(10, 0, 1), packet_entry(11, 1, 1)];
    book.use_server_entries(
        &packet,
        &PacketNames {
            item: &item_name,
            category: &no_category,
            tag: &tags,
        },
    );
    assert_eq!(book.entries.len(), 3);
    assert!(book.entries.iter().all(|e| !e.id.starts_with("server:")));
    assert_eq!(book.entry_of_display(11), Some(1));
    // A category id the registry names but a tab does not exist for is
    // another matter: that recipe is not this book's.
    let mut stonecutter = packet_entry(11, 1, 4);
    stonecutter.display = packet[1].display.clone();
    book.use_server_entries(&[stonecutter], &names(&tags));
    assert_eq!(book.entry_of_display(11), None);
}

/// A shaped server entry of `ingredients` (item ids, -1 an empty cell).
fn shaped_entry(display: i32, width: i32, height: i32, cells: &[i32]) -> RecipeBookEntry {
    let mut e = packet_entry(display, 0, 1);
    e.display = RecipeDisplay::Shaped {
        width,
        height,
        ingredients: cells
            .iter()
            .map(|&c| {
                if c < 0 {
                    SlotDisplay::Empty
                } else {
                    SlotDisplay::Item(c)
                }
            })
            .collect(),
        result: SlotDisplay::Item(0),
        station: SlotDisplay::Item(0),
    };
    e
}
fn shaped_local(width: usize, height: usize, cells: &[Option<&str>]) -> Book {
    let mut book = local_book();
    book.entries[0].display = Display::Shaped {
        width,
        height,
        cells: cells.iter().map(|c| c.map(|i| ing(&[i]))).collect(),
    };
    book
}
#[test]
fn a_shaped_recipe_must_have_the_same_layout_to_be_the_books_own() {
    let tags = |_: &str| Vec::new();
    let gp = Some("minecraft:gunpowder");
    let sand = Some("minecraft:diamond");
    // This client's TNT: 3 wide, 2 high.
    let cells = [gp, sand, gp, sand, gp, sand];
    let kept = |packet: RecipeBookEntry| {
        let mut book = shaped_local(3, 2, &cells);
        book.use_server_entries(&[packet], &names(&tags));
        book.entries[0].id == "minecraft:tnt"
    };
    // The same layout: the book's entry stands.
    assert!(kept(shaped_entry(10, 3, 2, &[4, 3, 4, 3, 4, 3])));
    // 2 wide and 3 high, the same ingredients in the same order.
    assert!(!kept(shaped_entry(10, 2, 3, &[4, 3, 4, 3, 4, 3])));
    // The same size and ingredients with a cell moved (an empty one).
    assert!(!kept(shaped_entry(10, 3, 2, &[4, 3, 4, 3, -1, 4])));
    // The empty cell counts: a hole where this client has an ingredient.
    let mut holed = shaped_local(2, 1, &[gp, None]);
    holed.use_server_entries(&[shaped_entry(10, 2, 1, &[4, 3])], &names(&tags));
    assert_eq!(holed.entries[0].id, "server:10");
}
/// What the book lists in every tab, as a comparable whole.
fn listing(book: &Book, client: &ClientBook) -> Vec<String> {
    let mut out = Vec::new();
    for category in [
        Category::CraftingBuilding,
        Category::CraftingRedstone,
        Category::CraftingEquipment,
        Category::CraftingMisc,
    ] {
        for c in client.collections(book, BookType::Crafting, Tab::Category(category)) {
            let mut one: Vec<String> = c
                .entries
                .iter()
                .map(|&i| {
                    let e = &book.entries[i];
                    format!("{}:{}:{:?}", e.display_id, e.result.item, e.group)
                })
                .collect();
            one.sort();
            out.push(format!("{category:?} {one:?}"));
        }
    }
    out.sort();
    out
}
#[test]
fn packets_one_after_another_end_as_one_rebuild_would() {
    let tags = |_: &str| Vec::new();
    let group = |mut e: RecipeBookEntry, g: i32| {
        e.group = g;
        e
    };
    // The server's group 7 holds the book's own stick (kept, "planks"),
    // and two recipes the book lacks, one in each packet, the kept one last.
    let first = vec![group(packet_entry(50, 3, 3), 7), packet_entry(110, 0, 1)];
    let second = vec![
        group(packet_entry(51, 4, 3), 7),
        group(packet_entry(12, 2, 3), 7),
    ];
    let make = || {
        let mut book = local_book();
        book.entries[2].group = Some("planks".into());
        book
    };
    let mut live = make();
    live.use_server_entries(&first, &names(&tags));
    live.use_server_entries(&second, &names(&tags));
    let mut rebuilt = make();
    let mut all: Vec<&RecipeBookEntry> = first.iter().chain(&second).collect();
    all.sort_by_key(|e| e.id);
    rebuilt.use_server_entries(all, &names(&tags));
    let mut client = ClientBook::default();
    for e in first.iter().chain(&second) {
        client.add(e.id);
    }
    assert_eq!(listing(&live, &client), listing(&rebuilt, &client));
    // And the misc tab holds the one group: three entries in one button.
    let misc = client.collections(
        &live,
        BookType::Crafting,
        Tab::Category(Category::CraftingMisc),
    );
    assert_eq!(misc.len(), 1);
    assert_eq!(misc[0].entries.len(), 3);
}
#[test]
fn a_kept_entry_leaves_no_server_requirements_behind() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    // Taken once, with its own ingredients.
    let mut taken = packet_entry(10, 0, 1);
    taken.requirements = Some(vec![HolderSet::Items(vec![3])]);
    taken.display = RecipeDisplay::Shapeless {
        ingredients: vec![SlotDisplay::Item(3)],
        result: SlotDisplay::Item(0),
        station: SlotDisplay::Item(0),
    };
    book.use_server_entries(&[taken], &names(&tags));
    assert!(book.server_requirements.contains_key(&10));
    // Hidden by an entry this book cannot show: nothing left of it.
    let mut stonecutter = packet_entry(10, 0, 4);
    stonecutter.display = RecipeDisplay::Stonecutter {
        input: SlotDisplay::Item(4),
        result: SlotDisplay::Item(0),
        station: SlotDisplay::Item(0),
    };
    book.use_server_entries(&[stonecutter], &names(&tags));
    assert!(!book.server_requirements.contains_key(&10));
}
#[test]
fn custom_server_crafting_recipe_is_shown_and_searchable() {
    let mut book = local_book();
    let tags = |_: &str| Vec::new();
    // A custom 1x3 shaped recipe for blaze rod (KiwiMC #14608).
    let mut custom = shaped_entry(99, 1, 3, &[3, 2, 4]);
    custom.display = RecipeDisplay::Shaped {
        width: 1,
        height: 3,
        ingredients: vec![
            SlotDisplay::Item(3),
            SlotDisplay::Item(2),
            SlotDisplay::Item(4),
        ],
        result: SlotDisplay::Item(5),
        station: SlotDisplay::Item(0),
    };
    custom.category = 3;
    book.use_server_entries(&[custom], &names(&tags));
    let mut client = ClientBook::default();
    client.add(99);
    let misc = client.collections(
        &book,
        BookType::Crafting,
        Tab::Category(Category::CraftingMisc),
    );
    assert_eq!(misc.len(), 1);
    assert_eq!(misc[0].entries.len(), 1);
    let custom_index = misc[0].entries[0];
    assert_eq!(
        &*book.entries[custom_index].result.item,
        "minecraft:blaze_rod"
    );
    // `FullTextSearchTree`: plain text searches the tooltip lines only
    // (#15321); `namespace:path` also searches the result's id.
    let lines = |_: &ItemStack| vec!["Custom Recipe".to_owned()];
    assert!(book.collection_matches(&[custom_index], "custom", &lines));
    assert!(!book.collection_matches(&[custom_index], "blaze", &lines));
    assert!(!book.collection_matches(&[custom_index], "rod", &lines));
    assert!(book.collection_matches(&[custom_index], "minecraft:blaze", &lines));
    assert!(!book.collection_matches(&[custom_index], "iron", &lines));
}
