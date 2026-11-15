//! A data pack's own item and block tags, at load and at run time.
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::{geometry::Vec3, position::ChunkPos};
use std::sync::Arc;

/// `ReloadableServerRegistries.reload` binds the packs' tags before
/// `ServerFunctionLibrary` compiles the functions (`ReloadableServerResources`
/// creates the commands from `lookupWithUpdatedTags`, then reloads the
/// function library), so a function whose `if items`/`if block` names a tag
/// its own pack defines loads and, run, tests the tag's members.
#[test]
#[ignore = "daily"]
fn a_packs_own_item_and_block_tags_admit_and_match_its_functions() {
    use crate::game::game_tests::tests::server_game;
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let dir = std::env::temp_dir().join(format!(
        "horizon-pack-tags-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let pack = dir.join("datapacks/tag_pack");
    let write = |path: &str, body: &str| {
        let path = pack.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    let (major, minor) = horizon_core::version::DATA_PACK_FORMAT;
    write(
        "pack.mcmeta",
        &serde_json::json!({"pack": {
            "description": "Own tags", "pack_format": major,
            "min_format": [major, minor], "max_format": [major, minor],
        }})
        .to_string(),
    );
    write(
        "data/p/tags/item/shiny.json",
        r#"{"values":["minecraft:diamond"]}"#,
    );
    write(
        "data/p/tags/block/stopper.json",
        r#"{"values":["minecraft:stone"]}"#,
    );
    write(
        "data/p/function/t.mcfunction",
        "execute if items entity @s weapon.mainhand #p:shiny run scoreboard players add #item p 1\n\
         execute if block ~ ~ ~ #p:stopper run scoreboard players add #block p 1\n",
    );
    game.world_root = dir.clone();
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(4.5, 200.0, 4.5)).unwrap(),
    );
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("scoreboard objectives add p dummy"));
    assert!(game.run_command("reload"));
    assert!(
        game.fns.store.errors.is_empty(),
        "{:?}",
        game.fns.store.errors
    );
    assert!(
        game.fns.store.functions.contains_key("p:t"),
        "the pack's function loaded against its own tags"
    );
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    let dirt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:dirt")
        .unwrap();
    game.world.set(4, 200, 4, stone).unwrap();
    let score = |game: &Game<'_>, key: &str| {
        game.score
            .board
            .score(key, "p")
            .map(|s| s.value)
            .unwrap_or(0)
    };
    assert!(game.run_command("item replace entity @s weapon.mainhand with minecraft:diamond"));
    assert!(game.run_command("function p:t"));
    assert_eq!(score(&game, "#item"), 1, "the diamond is in #p:shiny");
    assert_eq!(score(&game, "#block"), 1, "the stone is in #p:stopper");
    // The same tag, a member it doesn't hold: the function still runs, and
    // neither test matches.
    game.world.set(4, 200, 4, dirt).unwrap();
    assert!(game.run_command("item replace entity @s weapon.mainhand with minecraft:stick"));
    assert!(game.run_command("function p:t"));
    assert_eq!(score(&game, "#item"), 1, "a stick is not in #p:shiny");
    assert_eq!(score(&game, "#block"), 1, "dirt is not in #p:stopper");
    // Both match again.
    game.world.set(4, 200, 4, stone).unwrap();
    assert!(game.run_command("item replace entity @s weapon.mainhand with minecraft:diamond"));
    assert!(game.run_command("function p:t"));
    assert_eq!(score(&game, "#item"), 2);
    assert_eq!(score(&game, "#block"), 2);
    let _ = std::fs::remove_dir_all(&dir);
}
