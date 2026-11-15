//! `/data modify ... compute` (DataCommands.decorateModification's compute
//! source, over LootContextSources).
use super::*;
use crate::game::game_tests::tests::server_game;

/// A compute source stores the provider's value, in the loot context the
/// `default` / `block` / `entity` decoration builds; a provider the argument
/// cannot decode fails the command and writes nothing.
#[test]
#[ignore = "daily"]
fn compute_sources_store_the_providers_value() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    let store = "h_compute:values";
    let stored = |game: &Game<'_>, key: &str| game.score.storage.get(store).get(key).cloned();

    assert!(game.run_command(&format!("data merge storage {store} {{x:1}}")));
    assert!(game.run_command(&format!(
        r#"data modify storage {store} x set compute default integer {{type:"minecraft:constant",value:5}}"#
    )));
    assert_eq!(stored(&game, "x"), Some(Tag::Int(5)));
    assert!(game.run_command(&format!(
        r#"data modify storage {store} x set compute default float {{type:"minecraft:constant",value:2.5}}"#
    )));
    assert_eq!(stored(&game, "x"), Some(Tag::Float(2.5f32.to_bits())));

    // ContextFloatProvider.getFloat and ContextIntProvider.getInt read a
    // failure (a non-finite value, an ArithmeticException) as 0.
    assert!(game.run_command(&format!(
        r#"data modify storage {store} x set compute default float {{type:"minecraft:div",left:1,right:0}}"#
    )));
    assert_eq!(stored(&game, "x"), Some(Tag::Float(0.0f32.to_bits())));
    assert!(game.run_command(&format!(
        r#"data modify storage {store} x set compute default integer {{type:"minecraft:pow",base:0,exponent:0}}"#
    )));
    assert_eq!(stored(&game, "x"), Some(Tag::Int(0)));

    // A provider the argument cannot decode fails the command and writes nothing.
    assert!(game.run_command(&format!(
        r#"data modify storage {store} x set compute default integer {{type:"minecraft:constant",value:9}}"#
    )));
    assert!(!game.run_command(&format!(
        "data modify storage {store} x set compute default integer minecraft:no_such_provider"
    )));
    assert_eq!(stored(&game, "x"), Some(Tag::Int(9)));

    // `compute block <pos>`: the context's block state reaches the provider.
    assert!(game.run_command("forceload add 0 0 15 15"));
    assert!(game.run_command("setblock 8 229 8 minecraft:smoker"));
    let on_block = format!(
        r#"data modify storage {store} y set compute block 8 229 8 integer {{type:"minecraft:conditional",condition:{{type:"minecraft:match_block",blocks:"minecraft:smoker"}},on_true:7,on_false:3}}"#
    );
    assert!(game.run_command(&on_block));
    assert_eq!(stored(&game, "y"), Some(Tag::Int(7)));
    assert!(game.run_command("setblock 8 229 8 minecraft:air"));
    assert!(game.run_command(&on_block));
    assert_eq!(stored(&game, "y"), Some(Tag::Int(3)));

    // `compute entity <target>`: the entity context is resolved, and the
    // value is stored.
    assert!(game.run_command("summon pig 4.5 200 4.5 {UUID:[I;7,151,3,1],NoAI:1b}"));
    assert!(game.run_command(&format!(
        r#"data modify storage {store} z set compute entity @e[type=minecraft:pig,limit=1] integer {{type:"minecraft:add",inputs:[1,1]}}"#
    )));
    assert_eq!(stored(&game, "z"), Some(Tag::Int(2)));
}
