//! Which code mod owns each block, so a turned-off mod's blocks keep still.
//!
//! Options > Mods turns a code mod off by skipping its hooks; its content stays registered and
//! its blocks stay in the world. Without the hooks the engine would fall back to the template's
//! own behaviour on them (ticks, shape updates, neighbour reactions, block events, entity
//! contacts, block entity tickers) and could rewrite or remove them. The level answers
//! `LevelAccess::template_suppressed` from [`suppressed`] and leaves such a block as it is until
//! the mod is on again.
//!
//! The table belongs to the registry it describes. A content root records which mod owns each of
//! its namespaces under its prepared folder ([`stage`]); the world tables loaded from that folder
//! copy it into their registry ([`bind`]). A later reload that fails before its world tables load
//! leaves the registry in use as it is.
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

/// A state no code mod owns.
const NONE: u16 = u16::MAX;

/// Each staged prepared root with its mods' namespaces, each with its mod's id.
static STAGED: Mutex<Vec<(PathBuf, HashMap<String, String>)>> = Mutex::new(Vec::new());

/// Records the mods (id, owned namespaces) whose content the root prepared in `root` holds.
pub(super) fn stage<'a>(root: &Path, mods: impl IntoIterator<Item = (&'a str, &'a [String])>) {
    let owners = mods
        .into_iter()
        .flat_map(|(id, namespaces)| namespaces.iter().map(move |ns| (ns.clone(), id.to_owned())))
        .collect();
    let mut staged = STAGED.lock().unwrap_or_else(|e| e.into_inner());
    staged.retain(|(path, _)| path != root);
    staged.push((root.to_path_buf(), owners));
}

/// Gives `registry`, loaded from the root prepared in `root`, the owners staged for that root:
/// per state, the dispatch index of the code mod whose namespace it is in. Content-only mods have
/// no code to turn off and own nothing here. Once per registry; a registry with none set never
/// suppresses a block.
pub(in crate::game) fn bind(registry: &BlockRegistry, root: &Path) {
    if registry.code_block_owners.get().is_some() {
        return;
    }
    let staged = STAGED.lock().unwrap_or_else(|e| e.into_inner());
    let Some((_, owners)) = staged.iter().find(|(path, _)| path == root) else {
        return;
    };
    let index: HashMap<&str, u16> = owners
        .iter()
        .filter_map(|(namespace, id)| {
            let index = mod_api::__private::mod_index(id)?;
            Some((namespace.as_str(), u16::try_from(index).ok()?))
        })
        .collect();
    let table = if index.is_empty() {
        Box::default()
    } else {
        (0..registry.len())
            .map(|i| {
                registry
                    .state(BlockStateId(i as u32))
                    .and_then(|state| state.name.split_once(':'))
                    .and_then(|(namespace, _)| index.get(namespace).copied())
                    .unwrap_or(NONE)
            })
            .collect()
    };
    let _ = registry.code_block_owners.set(table);
}

pub(in crate::game) use horizon_client::block_contact::template_suppressed as suppressed;

#[cfg(test)]
mod tests {
    use super::*;

    struct Quiet;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Quiet {}

    #[test]
    fn a_turned_off_code_mods_blocks_are_suppressed_until_it_is_on_again() {
        let facts = r#"{"defaults":{"minecraft:ice":0,"frost:ice":1,"frost_extra:slush":2,"pack:stone":3},"states":[
            ["i",false,false,"","minecraft:ice"],
            ["f",false,false,"","frost:ice"],
            ["s",false,false,"","frost_extra:slush"],
            ["p",false,false,"","pack:stone"]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        let state = |name: &str| registry.default_state(name).unwrap();
        let root = std::env::temp_dir().join("code_block_owners_test_root");
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("frost", Box::new(Quiet))].into();
        mod_platform::testing::with_mods(mods, || {
            let frost = ["frost".to_string(), "frost_extra".to_string()];
            let pack = ["pack".to_string()];
            stage(&root, [("frost", &frost[..]), ("pack", &pack[..])]);
            // Staged for another root: never this registry's.
            stage(&root.join("other"), [("frost", &pack[..])]);
            bind(&registry, &root);

            assert!(
                !suppressed(&registry, state("frost:ice")),
                "an enabled mod's block"
            );
            assert!(mod_api::__private::set_mod_enabled("frost", false));
            assert!(suppressed(&registry, state("frost:ice")));
            assert!(
                suppressed(&registry, state("frost_extra:slush")),
                "every owned namespace"
            );
            assert!(!suppressed(&registry, state("minecraft:ice")));
            assert!(
                !suppressed(&registry, state("pack:stone")),
                "a content-only mod has nothing to turn off"
            );
            assert!(
                !suppressed(&registry, BlockStateId(99)),
                "a state past the table"
            );
            assert!(mod_api::__private::set_mod_enabled("frost", true));
            assert!(!suppressed(&registry, state("frost:ice")));
        });
    }
}
