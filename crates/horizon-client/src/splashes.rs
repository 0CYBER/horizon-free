//! The title screen's splash pool: `SplashManager.prepare` with NeoForge's
//! `NeoForgeSplashHooks.loadSplashes`. Built from the resource stack on each
//! resource reload; the title picks from it (`title::choose_splash`).

use horizon_assets::resource_source::ResourceSource;
use std::path::Path;

const VANILLA: &str = "assets/minecraft/texts/splashes.txt";

/// The splash lines, one per line, for `stack` (vanilla, content mods and the
/// enabled packs, merged). Every namespace's top `texts/splashes.txt` joins in
/// namespace order, duplicates kept. When `skip_if_modified` and the top
/// `minecraft` file isn't the one in `vanilla` (the game's own resources), only
/// that file is read (NeoForge's `skipModdedSplashesIfVanillaModified`). Empty
/// when no pack has the `minecraft` file, as `getResourceOrThrow` fails then.
pub fn pool(stack: &ResourceSource, vanilla: &Path, skip_if_modified: bool) -> String {
    let Ok(top) = stack.read_to_string(VANILLA) else {
        return String::new();
    };
    if skip_if_modified && !from_vanilla(stack, vanilla) {
        return top;
    }
    // `listResources("texts", ...)`: each namespace's winning file, by id.
    let mut pool = String::new();
    for namespace in stack.namespaces("assets") {
        let name = format!("assets/{namespace}/texts/splashes.txt");
        if !stack.contains(&name) {
            continue;
        }
        match stack.read_to_string(&name) {
            Ok(text) => {
                pool.push_str(&text);
                if !pool.is_empty() && !pool.ends_with('\n') {
                    pool.push('\n');
                }
            }
            Err(error) => log::warn!("Horizon: invalid texts/splashes.txt in {namespace}: {error}"),
        }
    }
    pool
}

/// Whether the top `minecraft` splashes come from the game's own resources
/// (the built-in vanilla pack), not from a resource pack or a mod.
fn from_vanilla(stack: &ResourceSource, vanilla: &Path) -> bool {
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    stack
        .folder_of(VANILLA)
        .is_some_and(|folder| same(&folder, vanilla))
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::resource_source::Resource;
    use std::collections::BTreeMap;

    /// The game's own resources with one splash line, and a stack whose
    /// `minecraft` file is `minecraft` (vanilla's when `None`) plus `others`.
    fn stack(
        minecraft: Option<&str>,
        others: &[(&str, &str)],
    ) -> (tempfile::TempDir, ResourceSource) {
        let vanilla = tempfile::tempdir().unwrap();
        let file = vanilla.path().join(VANILLA);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "Vanilla!").unwrap();
        let own = ResourceSource::directory(vanilla.path().to_path_buf());
        let mut files = BTreeMap::from([(
            VANILLA.to_owned(),
            match minecraft {
                None => Resource::file(own, VANILLA.to_owned()),
                Some(text) => Resource::bytes(text.as_bytes().to_vec()),
            },
        )]);
        files.extend(others.iter().map(|(name, text)| {
            (
                (*name).to_owned(),
                Resource::bytes(text.as_bytes().to_vec()),
            )
        }));
        (vanilla, ResourceSource::indexed(files))
    }

    #[test]
    fn a_content_mods_lines_follow_vanillas() {
        let (vanilla, stack) = stack(
            None,
            &[(
                "assets/testmod/texts/splashes.txt",
                "First mod line!\nSecond mod line!\n",
            )],
        );
        let pool = pool(&stack, vanilla.path(), true);
        let picks: Vec<_> = (0..3)
            .map(|index| crate::title::choose_splash(&pool, index, (6, 15)))
            .collect();
        let expected = ["Vanilla!", "First mod line!", "Second mod line!"];
        assert_eq!(picks, expected.map(|line| Some(line.to_owned())));
    }

    #[test]
    fn a_packs_own_minecraft_splashes_replace_the_pool() {
        let (vanilla, stack) = stack(
            Some("Pack line!"),
            &[("assets/testmod/texts/splashes.txt", "Mod line!")],
        );
        assert_eq!(
            pool(&stack, vanilla.path(), false),
            "Pack line!
Mod line!
"
        );
        let pool = pool(&stack, vanilla.path(), true);
        for index in 0..2 {
            assert_eq!(
                crate::title::choose_splash(&pool, index, (6, 15)).as_deref(),
                Some("Pack line!")
            );
        }
        assert_eq!(
            crate::title::choose_splash(&pool, 0, (12, 24)).as_deref(),
            Some("Merry X-mas!")
        );
    }
}
