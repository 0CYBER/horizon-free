//! Where packs keep their custom entity models and how a game reads one: the `.jem` files of
//! `emf/cem/` and `optifine/cem/` by model name, each read under [`MAX_BYTES`], parsed, its
//! warnings logged; and the PNGs beside them, the atlas names a model's texture takes (also one
//! outside `textures/`, which the atlas wouldn't walk). Shared by the mobs' and the block
//! entities' models.

use super::model::{EntityModel, MAX_BYTES, parse_jem};
use horizon_assets::resource_source::ResourceSource;
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;

/// Every `.jem` of the CEM folders by model name, sorted, each name's files best first:
/// `emf/cem` before `optifine/cem` whichever form either uses, and within one folder the flat
/// `<name>.jem` before `<name>/<name>.jem`.
pub fn jem_files(root: &ResourceSource) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for base in ["assets/minecraft/emf/cem", "assets/minecraft/optifine/cem"] {
        let (mut flat, mut nested) = (Vec::new(), Vec::new());
        for path in root.list(&format!("{base}/")) {
            let relative = &path[base.len() + 1..];
            if let Some(model) = relative
                .strip_suffix(".jem")
                .filter(|model| !model.contains('/'))
            {
                flat.push((model.to_owned(), path));
            } else if let Some((model, file)) = relative.split_once('/')
                && !model.ends_with(".jem")
                && file == format!("{model}.jem")
            {
                nested.push((model.to_owned(), path));
            }
        }
        for (model, path) in flat.into_iter().chain(nested) {
            out.entry(model).or_default().push(path);
        }
    }
    out
}

/// Every PNG of the CEM folders, its path from the resource root and its atlas name, in name
/// order: namespace, then `optifine/cem` before `emf/cem`, then the path. All of them, not only
/// the ones a loaded `.jem` names, so a model's texture is in the atlas whichever models load.
pub fn texture_files(root: &ResourceSource) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for namespace in root.namespaces("assets") {
        for folder in ["optifine/cem", "emf/cem"] {
            let base = format!("assets/{namespace}/{folder}");
            for path in root.list(&format!("{base}/")) {
                if path.ends_with(".png")
                    && let Some(name) = atlas_name(&path)
                {
                    out.push((path, name));
                }
            }
        }
    }
    out
}

/// The `.jem` at `path`, its warnings logged; None, with a warning, when it can't be read or
/// parsed.
pub fn load_jem(root: &ResourceSource, path: &str) -> Option<EntityModel> {
    let Some(text) = read_small(root, path) else {
        log::warn!("Horizon: custom entity model {path}: unreadable or over {MAX_BYTES} bytes");
        return None;
    };
    let jem = match parse_jem(path, &text, &mut |p: &str| read_small(root, p)) {
        Ok(jem) => jem,
        Err(e) => {
            log::warn!("Horizon: custom entity model {e}");
            return None;
        }
    };
    for warning in &jem.warnings {
        log::warn!("Horizon: custom entity model {warning}");
    }
    // `texture` resolves under `assets/<namespace>/` by construction (`paths::resolve`), so the
    // atlas always names it; only a missing file keeps it out.
    if let Some(texture) = &jem.texture
        && !root.contains(texture)
    {
        log::warn!(
            "Horizon: custom entity model {path}: its texture {texture} doesn't exist, \
             so not in the atlas: the vanilla one draws"
        );
    }
    Some(jem)
}

/// A pack file's text, bounded while reading and inflating archive entries.
fn read_small(root: &ResourceSource, path: &str) -> Option<String> {
    String::from_utf8(root.read_limited(path, MAX_BYTES as u64).ok()?).ok()
}

/// The atlas name of a texture resource path, the same one the entity atlas packs it under:
/// `assets/minecraft/textures/entity/x.png` is `entity/x` and
/// `assets/minecraft/optifine/cem/x.png` is `minecraft:optifine/cem/x`, as random variants'
/// names are ([`crate::random_textures::atlas_name`]). None for a path outside the resource
/// root's `assets/<namespace>/`.
pub fn atlas_name(path: &str) -> Option<String> {
    let (namespace, inner) = path.strip_prefix("assets/")?.split_once('/')?;
    Some(crate::random_textures::atlas_name(
        namespace,
        inner.strip_suffix(".png").unwrap_or(inner),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emf_files_win_and_a_flat_file_beats_its_folder() {
        let root = std::env::temp_dir().join(format!("horizon-jem-files-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for path in [
            // EMF's folder form over OptiFine's flat file.
            "optifine/cem/cow.jem",
            "emf/cem/cow/cow.jem",
            // One folder: the flat file over the folder form.
            "optifine/cem/pig.jem",
            "optifine/cem/pig/pig.jem",
            // A directory named `sheep.jem` is no file: the folder form stands.
            "optifine/cem/sheep.jem/readme.txt",
            "optifine/cem/sheep/sheep.jem",
        ] {
            let path = root.join("assets/minecraft").join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "{}").unwrap();
        }
        let files = jem_files(&ResourceSource::directory(root.clone()));
        fs::remove_dir_all(&root).unwrap();
        let paths = |paths: &[&str]| {
            paths
                .iter()
                .map(|p| format!("assets/minecraft/{p}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            files,
            BTreeMap::from([
                (
                    "cow".to_owned(),
                    paths(&["emf/cem/cow/cow.jem", "optifine/cem/cow.jem"])
                ),
                (
                    "pig".to_owned(),
                    paths(&["optifine/cem/pig.jem", "optifine/cem/pig/pig.jem"])
                ),
                ("sheep".to_owned(), paths(&["optifine/cem/sheep/sheep.jem"])),
            ])
        );
    }

    /// A model's texture takes the name the entity atlas packs it under, also beside its model
    /// outside `textures/`, and every PNG of the CEM folders is offered, named or not.
    #[test]
    fn cem_textures_take_the_names_the_atlas_packs_them_under() {
        let root =
            std::env::temp_dir().join(format!("horizon-cem-textures-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for path in [
            "assets/minecraft/optifine/cem/zombie2.png",
            "assets/minecraft/optifine/cem/zombie/zombie3.png",
            "assets/minecraft/emf/cem/cow.png",
            "assets/mymod/optifine/cem/x.png",
            "assets/minecraft/textures/entity/pig/pig.png",
            "assets/minecraft/optifine/cem/zombie.jem",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "png").unwrap();
        }
        let files = texture_files(&ResourceSource::directory(root.clone()));
        fs::remove_dir_all(&root).unwrap();
        let named = |path: &str, name: &str| (path.to_owned(), name.to_owned());
        assert_eq!(
            files,
            [
                named(
                    "assets/minecraft/optifine/cem/zombie/zombie3.png",
                    "minecraft:optifine/cem/zombie/zombie3"
                ),
                named(
                    "assets/minecraft/optifine/cem/zombie2.png",
                    "minecraft:optifine/cem/zombie2"
                ),
                named("assets/minecraft/emf/cem/cow.png", "minecraft:emf/cem/cow"),
                named("assets/mymod/optifine/cem/x.png", "mymod:optifine/cem/x"),
            ]
        );
        // A texture under `textures/` keeps the name the walked atlas already gives it.
        assert_eq!(
            atlas_name("assets/minecraft/textures/entity/pig/pig.png").as_deref(),
            Some("entity/pig/pig")
        );
        assert_eq!(
            atlas_name("assets/minecraft/optifine/cem/x.png").as_deref(),
            Some("minecraft:optifine/cem/x")
        );
        assert_eq!(atlas_name("optifine/cem/x.png"), None);
    }
}
