//! `ServerGamePacketListenerImpl.signBook`: signing a book and quill keeps
//! every component but its content (`transmuteCopy(Items.WRITTEN_BOOK)`).
use crate::game::game_tests::tests::server_game;
use horizon_client::item_data::parse_stack;

#[test]
#[ignore = "daily"]
fn signing_a_book_keeps_its_other_components() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let book = parse_stack(
        r#"{id:"minecraft:writable_book",count:1,components:{
            "minecraft:custom_name":"Diary",
            "minecraft:lore":[{text:"Kept",color:"gray"}],
            "minecraft:custom_data":{signed_by:"player"},
            "minecraft:repair_cost":3,
            "minecraft:tooltip_display":{hide_tooltip:false},
            "minecraft:writable_book_content":{pages:[{raw:"a page"}]}
        }}"#,
    )
    .unwrap()
    .unwrap();
    game.inventory[0] = Some(book);

    game.write_book(0, vec!["a page".into()], Some("Notes".into()));

    let stack = game.inventory[0].as_ref().expect("the signed book");
    assert_eq!(&*stack.item, "minecraft:written_book");
    let data = stack.data().expect("the kept components");
    assert_eq!(
        data.custom_name_plain().as_deref(),
        Some("Diary"),
        "transmuteCopy keeps custom_name"
    );
    assert!(data.other("minecraft:lore").is_some(), "lore kept");
    assert!(
        data.other("minecraft:custom_data").is_some(),
        "custom_data kept"
    );
    assert_eq!(data.repair_cost, 3, "repair_cost kept");
    assert!(
        data.other("minecraft:tooltip_display").is_some(),
        "tooltip_display kept"
    );
    // Only the content components changed.
    assert!(data.writable_pages.is_empty(), "writable content removed");
    let written = data.written.as_ref().expect("written content");
    assert_eq!(written.title, "Notes");
    assert_eq!(written.pages, ["a page"]);
    assert!(
        written.resolved,
        "signBook's WrittenBookContent(..., true) is resolved"
    );
}
