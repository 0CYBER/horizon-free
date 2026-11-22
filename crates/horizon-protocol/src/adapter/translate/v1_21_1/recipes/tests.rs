use super::*;
use crate::adapter::ServerVersion;

// Independent 1.21.1 ClientboundUpdateRecipesPacket.STREAM_CODEC output. The fixture
// contains one recipe for every one of the 23 vanilla RecipeSerializer registrations.
const VANILLA_RECIPES: &str = "1713686f72697a6f6e5f776972653a736861706564000c686f72697a6f6e5f776972650302010101ab0600000101ab06000002e40200000016686f72697a6f6e5f776972653a73686170656c657373010c686f72697a6f6e5f776972650002010101000002011c0000012300000301000019686f72697a6f6e5f776972653a73746f6e6563757474696e67130c686f72697a6f6e5f77697265010101000002880200001f686f72697a6f6e5f776972653a736d697468696e675f7472616e73666f726d140101f50900000101e40600000101b006000001ec0600001a686f72697a6f6e5f776972653a736d697468696e675f7472696d150101f60900000101e40600000101af06000015686f72697a6f6e5f776972653a736d656c74696e670f0c686f72697a6f6e5f7769726501010140000001ab0600003f4000007815686f72697a6f6e5f776972653a626c617374696e67100c686f72697a6f6e5f7769726501010140000001ab0600003f4000007814686f72697a6f6e5f776972653a736d6f6b696e67110c686f72697a6f6e5f7769726501010140000001ab0600003f400000781d686f72697a6f6e5f776972653a63616d70666972655f636f6f6b696e67120c686f72697a6f6e5f7769726501010140000001ab0600003f4000007826686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f61726d6f72647965020329686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f626f6f6b636c6f6e696e67030328686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170636c6f6e696e6704032a686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170657874656e64696e6705032d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f726f636b657406032b686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f73746172070330686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f737461725f66616465080329686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7469707065646172726f7709032d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f62616e6e65726475706c69636174650a032e686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f736869656c646465636f726174696f6e0b0330686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7368756c6b6572626f78636f6c6f72696e670c032c686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f737573706963696f7573737465770d0328686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7265706169726974656d0e0323686f72697a6f6e5f776972653a6372616674696e675f6465636f72617465645f706f741603";

fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
fn setup() -> Session {
    let session = Session::new(ServerVersion::V1_21_1);
    session.trim_patterns.lock().unwrap().insert(1580, 7); // synced sentry pattern deliberately reordered
    clientbound(&session, cb::UPDATE_RECIPES, &bytes(VANILLA_RECIPES)).unwrap();
    session
}
fn old_book(action: i32, names: &[&str], highlights: &[&str]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(action);
    RecipeBookSettings::write(
        &RecipeBookSettings {
            books: [(true, false), (false, true), (false, false), (true, true)],
        },
        &mut w,
    );
    w.varint(names.len() as i32);
    for name in names {
        w.string(&format!("horizon_wire:{name}"));
    }
    if action == 0 {
        w.varint(highlights.len() as i32);
        for name in highlights {
            w.string(&format!("horizon_wire:{name}"));
        }
    }
    w.data
}
fn next_book(session: &Session) -> RecipeBookAdd {
    let (id, payload) = session.next_incoming().unwrap();
    assert_eq!(id, cb::RECIPE_BOOK_ADD);
    RecipeBookAdd::read(&mut Reader::new(&payload)).unwrap()
}

#[test]
fn a_trim_recipe_whose_previews_would_balloon_is_refused() {
    // One smithing_trim recipe (serializer 21) with the fixture's template and addition, and
    // a base ingredient of `stacks` netherite chestplates.
    let packet = |stacks: usize| {
        let mut hex = String::from("011a686f72697a6f6e5f776972653a736d697468696e675f7472696d15");
        hex += "0101f6090000";
        let mut w = Writer::new();
        w.varint(stacks as i32);
        hex += &w
            .data
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        hex += &"01e4060000".repeat(stacks);
        hex += "0101af060000";
        bytes(&hex)
    };
    let session = Session::new(ServerVersion::V1_21_1);
    session.trim_patterns.lock().unwrap().insert(1580, 7);
    assert!(clientbound(&session, cb::UPDATE_RECIPES, &packet(40)).is_ok());
    let err = clientbound(&session, cb::UPDATE_RECIPES, &packet(5000)).unwrap_err();
    assert!(err.to_string().contains("trim preview too large"), "{err}");
}

#[test]
fn vanilla_recipe_serializers_become_pinned_displays_with_canonical_item_ids() {
    let session = setup();
    let state = session.legacy_recipes.lock().unwrap();
    assert_eq!(state.recipes.len(), 23);
    assert_eq!(
        state
            .recipes
            .values()
            .filter(|r| r.display.is_some())
            .count(),
        9
    );
    let shaped = &state.recipes["horizon_wire:shaped"];
    let Some(RecipeDisplay::Shaped {
        width,
        height,
        ingredients,
        result,
        station,
    }) = &shaped.display
    else {
        panic!("shaped")
    };
    assert_eq!((*width, *height), (2, 1));
    assert_eq!(ingredients.len(), 2);
    assert_eq!(*station, SlotDisplay::Item(405));
    let SlotDisplay::ItemStack(stack) = result else {
        panic!("result")
    };
    assert_eq!((stack.item, stack.count), (473, 2));
    assert_eq!(
        shaped.requirements,
        [HolderSet::Items(vec![1018]), HolderSet::Items(vec![1018])]
    );
    assert!(!shaped.notify);
    let Some(RecipeDisplay::Smithing { result, .. }) =
        &state.recipes["horizon_wire:smithing_trim"].display
    else {
        panic!("smithing")
    };
    let SlotDisplay::Composite(previews) = result else {
        panic!("trim previews")
    };
    let SlotDisplay::SmithingTrim { pattern, .. } = &previews[0] else {
        panic!("trim")
    };
    assert_eq!(*pattern, TrimPattern::Id(7));
    let Some(RecipeDisplay::Furnace {
        duration,
        experience,
        station,
        ..
    }) = &state.recipes["horizon_wire:blasting"].display
    else {
        panic!("blast")
    };
    assert_eq!(
        (*duration, *experience, station.clone()),
        (120, 0.75, SlotDisplay::Item(1508))
    );
    let properties = state.properties();
    let mut r = Reader::new(&properties);
    assert_eq!(r.varint().unwrap(), 9);
    let mut sets = HashMap::new();
    for _ in 0..9 {
        let name = r.string(32767).unwrap();
        let n = r.varint().unwrap();
        let list = (0..n).map(|_| r.varint().unwrap()).collect::<Vec<_>>();
        sets.insert(name, list);
    }
    assert_eq!(sets["minecraft:furnace_input"], [95]);
    assert_eq!(r.varint().unwrap(), 1);
    assert_eq!(HolderSet::read(&mut r).unwrap(), HolderSet::Items(vec![1]));
    let SlotDisplay::ItemStack(stack) = SlotDisplay::read(&mut r).unwrap() else {
        panic!("stonecutting")
    };
    assert_eq!((stack.item, stack.count), (355, 2));
    assert_eq!(r.remaining(), 0);
}

#[test]
fn init_add_remove_and_reload_preserve_keys_highlights_and_settings() {
    let session = setup();
    let names = [
        "shaped",
        "shapeless",
        "stonecutting",
        "smithing_transform",
        "smithing_trim",
        "smelting",
        "blasting",
        "smoking",
        "campfire_cooking",
        "crafting_special_armordye",
    ];
    let (id, payload) = recipe_book(
        &session,
        &old_book(0, &names, &["shaped", "crafting_special_armordye"]),
    )
    .unwrap();
    assert_eq!(id, cb::RECIPE_BOOK_SETTINGS);
    assert_eq!(
        RecipeBookSettings::read(&mut Reader::new(&payload))
            .unwrap()
            .books[0],
        (true, false)
    );
    let initial = next_book(&session);
    assert!(initial.replace);
    assert_eq!(initial.entries.len(), 9);
    assert_eq!(initial.entries[0].flags, 2);
    assert!(initial.entries[1..].iter().all(|r| r.flags == 0));
    let shaped_id = initial.entries[0].id;
    recipe_book(&session, &old_book(1, &["shaped", "shapeless"], &[])).unwrap();
    let added = next_book(&session);
    assert!(!added.replace);
    assert_eq!(added.entries[0].id, shaped_id);
    assert_eq!(added.entries[0].flags, 2);
    assert_eq!(added.entries[1].flags, 3);
    recipe_book(&session, &old_book(2, &["shaped"], &[])).unwrap();
    let (id, payload) = session.next_incoming().unwrap();
    assert_eq!(id, cb::RECIPE_BOOK_REMOVE);
    assert_eq!(
        RecipeBookRemove::read(&mut Reader::new(&payload))
            .unwrap()
            .ids,
        [shaped_id]
    );
    clientbound(&session, cb::UPDATE_RECIPES, &bytes(VANILLA_RECIPES)).unwrap();
    let reloaded = next_book(&session);
    assert!(reloaded.replace);
    assert_eq!(reloaded.entries.len(), 8);
    assert_eq!(
        session.legacy_recipes.lock().unwrap().recipes["horizon_wire:shaped"].id,
        shaped_id
    );
    let mut bad = Writer::new();
    bad.varint(1);
    bad.string("horizon_wire:bad");
    bad.varint(99);
    assert!(clientbound(&session, cb::UPDATE_RECIPES, &bad.data).is_err());
    assert_eq!(session.legacy_recipes.lock().unwrap().recipes.len(), 23);
}

#[test]
fn outbound_selection_and_ghosts_follow_the_servers_recipe_key() {
    let session = setup();
    let id = session.legacy_recipes.lock().unwrap().recipes["horizon_wire:shaped"].id;
    let payload = encode(&PlaceRecipe {
        container: 11,
        recipe: id,
        use_max: true,
    });
    let old = serverbound(&session, sb::PLACE_RECIPE, &payload)
        .unwrap()
        .unwrap();
    let mut r = Reader::new(&old);
    assert_eq!(r.i8().unwrap(), 11);
    assert_eq!(r.string(32767).unwrap(), "horizon_wire:shaped");
    assert!(r.bool().unwrap());
    assert_eq!(r.remaining(), 0);
    let old = serverbound(
        &session,
        sb::RECIPE_BOOK_SEEN_RECIPE,
        &encode(&RecipeBookSeenRecipe { recipe: id }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        Reader::new(&old).string(32767).unwrap(),
        "horizon_wire:shaped"
    );
    assert!(
        serverbound(
            &session,
            sb::RECIPE_BOOK_SEEN_RECIPE,
            &encode(&RecipeBookSeenRecipe { recipe: 65000 })
        )
        .unwrap()
        .is_none()
    );
    let mut ghost = Writer::new();
    ghost.i8(11);
    ghost.string("horizon_wire:shaped");
    let payload = clientbound(&session, cb::PLACE_GHOST_RECIPE, &ghost.data)
        .unwrap()
        .unwrap();
    let ghost = PlaceGhostRecipe::read(&mut Reader::new(&payload)).unwrap();
    assert_eq!(ghost.container, 11);
    assert!(matches!(
        ghost.display,
        RecipeDisplay::Shaped {
            width: 2,
            height: 1,
            ..
        }
    ));
    let mut empty = Writer::new();
    empty.varint(0);
    clientbound(&session, cb::UPDATE_RECIPES, &empty.data).unwrap();
    assert!(
        serverbound(
            &session,
            sb::RECIPE_BOOK_SEEN_RECIPE,
            &encode(&RecipeBookSeenRecipe { recipe: id })
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn a_book_before_declarations_is_filled_when_its_recipe_arrives() {
    let session = Session::new(ServerVersion::V1_21_1);
    recipe_book(&session, &old_book(0, &["shaped"], &["shaped"])).unwrap();
    assert!(next_book(&session).entries.is_empty());
    clientbound(&session, cb::UPDATE_RECIPES, &bytes(VANILLA_RECIPES)).unwrap();
    let book = next_book(&session);
    assert!(book.replace);
    assert_eq!(book.entries.len(), 1);
    assert_eq!(book.entries[0].flags, 2);
}

#[test]
fn vanilla_1_20_5_and_1_20_6_recipe_codecs_reuse_displays_and_request_keys() {
    // Official 1.20.6 UpdateRecipes codec output, independently byte-identical on 1.20.5.
    // Includes all 23 serializers; registry ids differ from the 1.21.1 fixture above.
    let old = bytes(
        "1713686f72697a6f6e5f776972653a736861706564000c686f72697a6f6e5f776972650302010101aa0600000101aa06000002e40200000016686f72697a6f6e5f776972653a73686170656c657373010c686f72697a6f6e5f776972650002010101000002011c0000012300000301000019686f72697a6f6e5f776972653a73746f6e6563757474696e67130c686f72697a6f6e5f77697265010101000002880200001f686f72697a6f6e5f776972653a736d697468696e675f7472616e73666f726d140101f20900000101e40600000101af06000001ec0600001a686f72697a6f6e5f776972653a736d697468696e675f7472696d150101f30900000101e40600000101ae06000015686f72697a6f6e5f776972653a736d656c74696e670f0c686f72697a6f6e5f7769726501010140000001aa0600003f4000007815686f72697a6f6e5f776972653a626c617374696e67100c686f72697a6f6e5f7769726501010140000001aa0600003f4000007814686f72697a6f6e5f776972653a736d6f6b696e67110c686f72697a6f6e5f7769726501010140000001aa0600003f400000781d686f72697a6f6e5f776972653a63616d70666972655f636f6f6b696e67120c686f72697a6f6e5f7769726501010140000001aa0600003f4000007826686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f61726d6f72647965020329686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f626f6f6b636c6f6e696e67030328686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170636c6f6e696e6704032a686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170657874656e64696e6705032d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f726f636b657406032b686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f73746172070330686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f737461725f66616465080329686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7469707065646172726f7709032d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f62616e6e65726475706c69636174650a032e686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f736869656c646465636f726174696f6e0b0330686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7368756c6b6572626f78636f6c6f72696e670c032c686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f737573706963696f7573737465770d0328686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7265706169726974656d0e0323686f72697a6f6e5f776972653a6372616674696e675f6465636f72617465645f706f741603",
    );
    let session = Session::new(ServerVersion::V1_20_6);
    session.trim_patterns.lock().unwrap().insert(1580, 7);
    let properties = clientbound(&session, cb::UPDATE_RECIPES, &old)
        .unwrap()
        .unwrap();
    assert!(!properties.is_empty());
    {
        let state = session.legacy_recipes.lock().unwrap();
        assert_eq!(state.recipes.len(), 23);
        assert_eq!(
            state
                .recipes
                .values()
                .filter(|r| r.display.is_some())
                .count(),
            9
        );
        let Some(RecipeDisplay::Shaped { result, .. }) =
            &state.recipes["horizon_wire:shaped"].display
        else {
            panic!("shaped")
        };
        let SlotDisplay::ItemStack(stack) = result else {
            panic!("result")
        };
        assert_eq!((stack.item, stack.count), (473, 2));
        assert_eq!(
            state.recipes["horizon_wire:shaped"].requirements,
            [HolderSet::Items(vec![1018]), HolderSet::Items(vec![1018])]
        );
        let Some(RecipeDisplay::Smithing { result, .. }) =
            &state.recipes["horizon_wire:smithing_trim"].display
        else {
            panic!("smithing")
        };
        let SlotDisplay::Composite(previews) = result else {
            panic!("previews")
        };
        let SlotDisplay::SmithingTrim { pattern, .. } = &previews[0] else {
            panic!("trim")
        };
        assert_eq!(*pattern, TrimPattern::Id(7));
    }
    recipe_book(
        &session,
        &old_book(
            0,
            &["shaped", "smelting", "crafting_special_armordye"],
            &["shaped"],
        ),
    )
    .unwrap();
    let book = next_book(&session);
    assert!(book.replace);
    assert_eq!(book.entries.len(), 2);
    assert_eq!(book.entries[0].flags, 2);
    let id = book.entries[0].id;
    let selected = serverbound(
        &session,
        sb::PLACE_RECIPE,
        &encode(&PlaceRecipe {
            container: 3,
            recipe: id,
            use_max: true,
        }),
    )
    .unwrap()
    .unwrap();
    let mut r = Reader::new(&selected);
    assert_eq!(r.i8().unwrap(), 3);
    assert_eq!(r.string(32767).unwrap(), "horizon_wire:shaped");
    assert!(r.bool().unwrap());
    assert_eq!(r.remaining(), 0);
    let seen = serverbound(
        &session,
        sb::RECIPE_BOOK_SEEN_RECIPE,
        &encode(&RecipeBookSeenRecipe { recipe: id }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        Reader::new(&seen).string(32767).unwrap(),
        "horizon_wire:shaped"
    );
}
#[test]
fn vanilla_1_20_3_and_1_20_4_recipe_codecs_reuse_displays_and_request_keys() {
    // Official 1.20.4 UpdateRecipes codec output, independently byte-identical on 1.20.3.
    // All 23 serializer identifiers and NBT slots pass through the full recipe bridge.
    let old = bytes(
        "17196d696e6563726166743a6372616674696e675f73686170656413686f72697a6f6e5f776972653a7368617065640c686f72697a6f6e5f776972650302010101a70601000101a706010001e3020200001c6d696e6563726166743a6372616674696e675f73686170656c65737316686f72697a6f6e5f776972653a73686170656c6573730c686f72697a6f6e5f776972650002010101010002011c01000123010001010300166d696e6563726166743a73746f6e6563757474696e6719686f72697a6f6e5f776972653a73746f6e6563757474696e670c686f72697a6f6e5f77697265010101010001870202001c6d696e6563726166743a736d697468696e675f7472616e73666f726d1f686f72697a6f6e5f776972653a736d697468696e675f7472616e73666f726d0101e90901000101e106010a03000644616d61676500000000000101ac06010001e906010a03000644616d6167650000000000176d696e6563726166743a736d697468696e675f7472696d1a686f72697a6f6e5f776972653a736d697468696e675f7472696d0101ea0901000101e106010a03000644616d61676500000000000101ab060100126d696e6563726166743a736d656c74696e6715686f72697a6f6e5f776972653a736d656c74696e670c686f72697a6f6e5f7769726501010140010001a70601003f40000078126d696e6563726166743a626c617374696e6715686f72697a6f6e5f776972653a626c617374696e670c686f72697a6f6e5f7769726501010140010001a70601003f40000078116d696e6563726166743a736d6f6b696e6714686f72697a6f6e5f776972653a736d6f6b696e670c686f72697a6f6e5f7769726501010140010001a70601003f400000781a6d696e6563726166743a63616d70666972655f636f6f6b696e671d686f72697a6f6e5f776972653a63616d70666972655f636f6f6b696e670c686f72697a6f6e5f7769726501010140010001a70601003f40000078236d696e6563726166743a6372616674696e675f7370656369616c5f61726d6f7264796526686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f61726d6f7264796503266d696e6563726166743a6372616674696e675f7370656369616c5f626f6f6b636c6f6e696e6729686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f626f6f6b636c6f6e696e6703256d696e6563726166743a6372616674696e675f7370656369616c5f6d6170636c6f6e696e6728686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170636c6f6e696e6703276d696e6563726166743a6372616674696e675f7370656369616c5f6d6170657874656e64696e672a686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170657874656e64696e67032a6d696e6563726166743a6372616674696e675f7370656369616c5f66697265776f726b5f726f636b65742d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f726f636b657403286d696e6563726166743a6372616674696e675f7370656369616c5f66697265776f726b5f737461722b686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f73746172032d6d696e6563726166743a6372616674696e675f7370656369616c5f66697265776f726b5f737461725f6661646530686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f737461725f6661646503266d696e6563726166743a6372616674696e675f7370656369616c5f7469707065646172726f7729686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7469707065646172726f77032a6d696e6563726166743a6372616674696e675f7370656369616c5f62616e6e65726475706c69636174652d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f62616e6e65726475706c6963617465032b6d696e6563726166743a6372616674696e675f7370656369616c5f736869656c646465636f726174696f6e2e686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f736869656c646465636f726174696f6e032d6d696e6563726166743a6372616674696e675f7370656369616c5f7368756c6b6572626f78636f6c6f72696e6730686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7368756c6b6572626f78636f6c6f72696e6703296d696e6563726166743a6372616674696e675f7370656369616c5f737573706963696f7573737465772c686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f737573706963696f75737374657703256d696e6563726166743a6372616674696e675f7370656369616c5f7265706169726974656d28686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7265706169726974656d03206d696e6563726166743a6372616674696e675f6465636f72617465645f706f7423686f72697a6f6e5f776972653a6372616674696e675f6465636f72617465645f706f7403",
    );
    let session = Session::new(ServerVersion::V1_20_3);
    let _names = ids::legacy_registries(session.legacy_registries.lock().unwrap().clone());
    let _items = ids::legacy_items(session.legacy_items.clone());
    session.trim_patterns.lock().unwrap().insert(1580, 7);
    let properties = clientbound(&session, cb::UPDATE_RECIPES, &old)
        .unwrap()
        .unwrap();
    assert!(!properties.is_empty());
    {
        let state = session.legacy_recipes.lock().unwrap();
        assert_eq!(state.recipes.len(), 23);
        assert_eq!(
            state
                .recipes
                .values()
                .filter(|r| r.display.is_some())
                .count(),
            9
        );
        let Some(RecipeDisplay::Shaped { result, .. }) =
            &state.recipes["horizon_wire:shaped"].display
        else {
            panic!("shaped")
        };
        let SlotDisplay::ItemStack(stack) = result else {
            panic!("result")
        };
        assert_eq!((stack.item, stack.count), (473, 2));
        assert_eq!(
            state.recipes["horizon_wire:shaped"].requirements,
            [HolderSet::Items(vec![1018]), HolderSet::Items(vec![1018])]
        );
        let Some(RecipeDisplay::Smithing { result, .. }) =
            &state.recipes["horizon_wire:smithing_trim"].display
        else {
            panic!("smithing")
        };
        let SlotDisplay::Composite(previews) = result else {
            panic!("previews")
        };
        let SlotDisplay::SmithingTrim { pattern, .. } = &previews[0] else {
            panic!("trim")
        };
        assert_eq!(*pattern, TrimPattern::Id(7));
    }
    recipe_book(
        &session,
        &old_book(
            0,
            &["shaped", "smelting", "crafting_special_armordye"],
            &["shaped"],
        ),
    )
    .unwrap();
    let book = next_book(&session);
    assert!(book.replace);
    assert_eq!(book.entries.len(), 2);
    assert_eq!(book.entries[0].flags, 2);
    let id = book.entries[0].id;
    let selected = serverbound(
        &session,
        sb::PLACE_RECIPE,
        &encode(&PlaceRecipe {
            container: 3,
            recipe: id,
            use_max: true,
        }),
    )
    .unwrap()
    .unwrap();
    let mut r = Reader::new(&selected);
    assert_eq!(r.i8().unwrap(), 3);
    assert_eq!(r.string(32767).unwrap(), "horizon_wire:shaped");
    assert!(r.bool().unwrap());
    assert_eq!(r.remaining(), 0);
    let seen = serverbound(
        &session,
        sb::RECIPE_BOOK_SEEN_RECIPE,
        &encode(&RecipeBookSeenRecipe { recipe: id }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        Reader::new(&seen).string(32767).unwrap(),
        "horizon_wire:shaped"
    );
}

#[test]
fn vanilla_1_20_2_recipe_codecs_keep_width_first_and_reuse_request_keys() {
    // Independent official 1.20.2 UpdateRecipes output; shaped dimensions precede group/category.
    // All 23 serializer identifiers and NBT slots pass through the full recipe bridge.
    let old = bytes(
        "17196d696e6563726166743a6372616674696e675f73686170656413686f72697a6f6e5f776972653a73686170656402010c686f72697a6f6e5f776972650301018206010001018206010001ce020200001c6d696e6563726166743a6372616674696e675f73686170656c65737316686f72697a6f6e5f776972653a73686170656c6573730c686f72697a6f6e5f776972650002010101010002010f01000116010001010300166d696e6563726166743a73746f6e6563757474696e6719686f72697a6f6e5f776972653a73746f6e6563757474696e670c686f72697a6f6e5f77697265010101010001f20102001c6d696e6563726166743a736d697468696e675f7472616e73666f726d1f686f72697a6f6e5f776972653a736d697468696e675f7472616e73666f726d0101c20901000101bc06010a03000644616d616765000000000001018706010001c406010a03000644616d6167650000000000176d696e6563726166743a736d697468696e675f7472696d1a686f72697a6f6e5f776972653a736d697468696e675f7472696d0101c30901000101bc06010a03000644616d6167650000000000010186060100126d696e6563726166743a736d656c74696e6715686f72697a6f6e5f776972653a736d656c74696e670c686f72697a6f6e5f7769726501010133010001820601003f40000078126d696e6563726166743a626c617374696e6715686f72697a6f6e5f776972653a626c617374696e670c686f72697a6f6e5f7769726501010133010001820601003f40000078116d696e6563726166743a736d6f6b696e6714686f72697a6f6e5f776972653a736d6f6b696e670c686f72697a6f6e5f7769726501010133010001820601003f400000781a6d696e6563726166743a63616d70666972655f636f6f6b696e671d686f72697a6f6e5f776972653a63616d70666972655f636f6f6b696e670c686f72697a6f6e5f7769726501010133010001820601003f40000078236d696e6563726166743a6372616674696e675f7370656369616c5f61726d6f7264796526686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f61726d6f7264796503266d696e6563726166743a6372616674696e675f7370656369616c5f626f6f6b636c6f6e696e6729686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f626f6f6b636c6f6e696e6703256d696e6563726166743a6372616674696e675f7370656369616c5f6d6170636c6f6e696e6728686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170636c6f6e696e6703276d696e6563726166743a6372616674696e675f7370656369616c5f6d6170657874656e64696e672a686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f6d6170657874656e64696e67032a6d696e6563726166743a6372616674696e675f7370656369616c5f66697265776f726b5f726f636b65742d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f726f636b657403286d696e6563726166743a6372616674696e675f7370656369616c5f66697265776f726b5f737461722b686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f73746172032d6d696e6563726166743a6372616674696e675f7370656369616c5f66697265776f726b5f737461725f6661646530686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f66697265776f726b5f737461725f6661646503266d696e6563726166743a6372616674696e675f7370656369616c5f7469707065646172726f7729686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7469707065646172726f77032a6d696e6563726166743a6372616674696e675f7370656369616c5f62616e6e65726475706c69636174652d686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f62616e6e65726475706c6963617465032b6d696e6563726166743a6372616674696e675f7370656369616c5f736869656c646465636f726174696f6e2e686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f736869656c646465636f726174696f6e032d6d696e6563726166743a6372616674696e675f7370656369616c5f7368756c6b6572626f78636f6c6f72696e6730686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7368756c6b6572626f78636f6c6f72696e6703296d696e6563726166743a6372616674696e675f7370656369616c5f737573706963696f7573737465772c686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f737573706963696f75737374657703256d696e6563726166743a6372616674696e675f7370656369616c5f7265706169726974656d28686f72697a6f6e5f776972653a6372616674696e675f7370656369616c5f7265706169726974656d03206d696e6563726166743a6372616674696e675f6465636f72617465645f706f7423686f72697a6f6e5f776972653a6372616674696e675f6465636f72617465645f706f7403",
    );
    let session = Session::new(ServerVersion::V1_20_2);
    let _names = ids::legacy_registries(session.legacy_registries.lock().unwrap().clone());
    let _items = ids::legacy_items(session.legacy_items.clone());
    session.trim_patterns.lock().unwrap().insert(1580, 7);
    let properties = clientbound(&session, cb::UPDATE_RECIPES, &old)
        .unwrap()
        .unwrap();
    assert!(!properties.is_empty());
    {
        let state = session.legacy_recipes.lock().unwrap();
        assert_eq!(state.recipes.len(), 23);
        assert_eq!(
            state
                .recipes
                .values()
                .filter(|r| r.display.is_some())
                .count(),
            9
        );
        let Some(RecipeDisplay::Shaped {
            width,
            height,
            result,
            ..
        }) = &state.recipes["horizon_wire:shaped"].display
        else {
            panic!("shaped")
        };
        assert_eq!((*width, *height), (2, 1));
        let SlotDisplay::ItemStack(stack) = result else {
            panic!("result")
        };
        assert_eq!((stack.item, stack.count), (473, 2));
        assert_eq!(
            state.recipes["horizon_wire:shaped"].requirements,
            [HolderSet::Items(vec![1018]), HolderSet::Items(vec![1018])]
        );
        let Some(RecipeDisplay::Smithing { result, .. }) =
            &state.recipes["horizon_wire:smithing_trim"].display
        else {
            panic!("smithing")
        };
        let SlotDisplay::Composite(previews) = result else {
            panic!("previews")
        };
        let SlotDisplay::SmithingTrim { pattern, .. } = &previews[0] else {
            panic!("trim")
        };
        assert_eq!(*pattern, TrimPattern::Id(7));
    }
    recipe_book(
        &session,
        &old_book(
            0,
            &["shaped", "smelting", "crafting_special_armordye"],
            &["shaped"],
        ),
    )
    .unwrap();
    let book = next_book(&session);
    assert!(book.replace);
    assert_eq!(book.entries.len(), 2);
    assert_eq!(book.entries[0].flags, 2);
    let id = book.entries[0].id;
    let selected = serverbound(
        &session,
        sb::PLACE_RECIPE,
        &encode(&PlaceRecipe {
            container: 3,
            recipe: id,
            use_max: true,
        }),
    )
    .unwrap()
    .unwrap();
    let mut r = Reader::new(&selected);
    assert_eq!(r.i8().unwrap(), 3);
    assert_eq!(r.string(32767).unwrap(), "horizon_wire:shaped");
    assert!(r.bool().unwrap());
    assert_eq!(r.remaining(), 0);
    let seen = serverbound(
        &session,
        sb::RECIPE_BOOK_SEEN_RECIPE,
        &encode(&RecipeBookSeenRecipe { recipe: id }),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        Reader::new(&seen).string(32767).unwrap(),
        "horizon_wire:shaped"
    );
}
