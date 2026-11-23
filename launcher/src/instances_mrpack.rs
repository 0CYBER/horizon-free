//! A modded Modrinth pack (`.mrpack`) as a Java instance, the way Prism Launcher imports one:
//! the Minecraft version and the mod loader its index names in `dependencies` (`minecraft`,
//! `fabric-loader`, `quilt-loader`, `forge`, `neoforge`: Modrinth's format,
//! <https://support.modrinth.com/en/articles/8802351-modrinth-modpack-format-mrpack>), its
//! `overrides/` and `client-overrides/` copied into the instance's folder, and each of its
//! `files` downloaded and checked against its hash ([`crate::import`]). The game and the loader
//! come with the first Play, for an account that owns the game.
use crate::import;
use crate::instances::{self, Instance};
use crate::loaders::{Kind, Loader};
use serde_json::Value;
use std::path::Path;

/// The Minecraft version and the loader (none for a vanilla pack) a pack's index names.
pub fn game_of(index: &Value) -> Result<(String, Option<Loader>), String> {
    let dependencies = &index["dependencies"];
    let minecraft = dependencies["minecraft"]
        .as_str()
        .ok_or("The pack doesn't name its Minecraft version.")?;
    let mut loader = None;
    for (key, version) in dependencies.as_object().into_iter().flatten() {
        let kind = match key.as_str() {
            "minecraft" => continue,
            "fabric-loader" => Kind::Fabric,
            "quilt-loader" => Kind::Quilt,
            "forge" => Kind::Forge,
            "neoforge" => Kind::NeoForge,
            other => {
                return Err(format!(
                    "The pack needs {other}, which Horizon can't install."
                ))
            }
        };
        if loader.is_some() {
            return Err("The pack names more than one mod loader.".into());
        }
        let version = version
            .as_str()
            .ok_or_else(|| format!("The pack's {key} has no version."))?;
        if !plausible_version(version) {
            return Err(format!(
                "The pack's {key} version isn't one Horizon can use."
            ));
        }
        loader = Some(Loader {
            kind,
            version: version.to_string(),
        });
    }
    Ok((minecraft.to_string(), loader))
}

/// Whether `version` looks like a loader release (`0.16.9`, `0.27.1-beta.1`, `47.3.0`,
/// `21.1.77`, `1.12.2-14.23.5.2860`). The loader's version goes into a URL and a folder name, so
/// a pack's must not carry a path or a query.
fn plausible_version(version: &str) -> bool {
    (1..=64).contains(&version.len())
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
        && version != "."
        && version != ".."
}

/// Makes a Java instance from the `.mrpack` `file`, named after the pack.
pub fn import(file: &Path, progress: &dyn Fn(&str, f32)) -> Result<Instance, String> {
    let (minecraft, loader) = game_of(&import::mrpack_index(file)?)?;
    let versions = crate::minecraft_versions::load()?;
    let minecraft = versions.find(&minecraft)?.id.clone();
    let made_id = std::cell::RefCell::new(None);
    let made = import::import_mrpack_into(file, progress, &|name| {
        let made = instances::create_java(name, &minecraft)?;
        *made_id.borrow_mut() = Some(made.id.clone());
        match &loader {
            Some(loader) => instances::edit(&made.id, |m| {
                m.java.get_or_insert_with(Default::default).loader = Some(loader.clone())
            }),
            None => Ok(made),
        }
    });
    // A pack that failed half way leaves no instance behind.
    if let (Err(_), Some(id)) = (&made, made_id.into_inner()) {
        let _ = instances::delete(&id);
    }
    made
}

#[cfg(test)]
#[path = "instances_mrpack/tests.rs"]
mod tests;
