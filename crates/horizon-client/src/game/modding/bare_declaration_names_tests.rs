//! arismoko/horizon#3120: a bare name is the declaring mod's own, so a block entity type built with
//! `.blocks(&["press"])` holds the block `press` the same mod declares.
use std::{collections::BTreeSet, fs, path::Path};

use horizon_client::mod_content::{BLOCK_ENTITY_TYPES, accept_declared, build};
use horizon_mod_package::enabled::EnabledMods;
use mod_api::Side;
use mod_api::{__private::game::RegistrationContextNew, BlocksEvent, Hooks, Registrar};

use crate::game::{Game, api_values::TryIntoEngine};

struct Press;
#[mod_api::__subscriptions]
impl Hooks for Press {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.block_entity_type("press").blocks(&["press"]).build();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .blocks
            .push(mod_api::Block::new("press", "minecraft:stone"));
    }
}

/// The least of a prepared resource folder the content overlay reads.
fn base(root: &Path) {
    fs::create_dir_all(root.join("assets/minecraft/blockstates")).unwrap();
    for (file, text) in [
        (
            "facts.json",
            r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
        ),
        (
            "gameplay-facts.json",
            r#"{"states":[{"item":"minecraft:stone"}]}"#,
        ),
        (
            "registries.json",
            r#"{"minecraft:item":{"entries":{}},"minecraft:block":{"entries":{}},
                "minecraft:block_entity_type":{"entries":{"minecraft:sign":{"protocol_id":8}}}}"#,
        ),
        ("items.json", "{}"),
        ("assets/minecraft/blockstates/stone.json", "{}"),
    ] {
        fs::write(root.join(file), text).unwrap();
    }
}

#[test]
fn a_bare_block_name_binds_the_type_to_the_mods_own_block() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("press_mod", Box::new(Press))].into();
    let (blocks, types) = mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        let context = mod_api::RegistrationContext::new();
        let mut blocks = Vec::new();
        Game::native_blocks(&context, &mut blocks);
        assert!(mod_api::__private::take_declaration_refusals(&context).is_empty());
        let blocks: Vec<horizon_mod_package::content::Block> = blocks
            .iter()
            .map(|block| block.try_into_engine(&()).unwrap())
            .collect();
        (blocks, crate::game::api_values::block_entity_declarations())
    });
    let mut content = Vec::new();
    let errors = accept_declared(&mut content, &blocks, &[], &types, &BTreeSet::new());
    assert!(errors.is_empty(), "{errors:?}");
    let scratch = tempfile::tempdir().unwrap();
    let base_root = scratch.path().join("base");
    base(&base_root);
    let inventory = EnabledMods::new([("press_mod".to_owned(), Side::Both)]);
    let root = scratch.path().join("overlay");
    // Build the declaration table without starting a detached cache prune.
    build(&base_root, Path::new(""), &content, &root, &inventory).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(BLOCK_ENTITY_TYPES)).unwrap(),
        r#"{"press_mod:press":"press_mod:press"}"#
    );
    scratch.close().unwrap();
}
