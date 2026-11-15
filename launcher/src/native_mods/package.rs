//! Validated flat packages and launcher-owned horizon-mixins input.
use super::{Bundle, MANIFEST};
use horizon_mod_package::mod_surface::uses::USES_FILE;
use horizon_mod_package::{manifest, Package};
use std::{fs, path::Path};

pub(super) fn read(dir: &Path) -> Result<Option<Bundle>, String> {
    for old in [
        "horizon-native.json",
        "horizon-mixin.toml",
        "native",
        "rust",
    ] {
        if dir.join(old).exists() {
            return Err(format!("{}: unsupported package path {old}", dir.display()));
        }
    }
    if !dir.join(manifest::MANIFEST).is_file() {
        return Ok(None);
    }
    let package = Package::read(dir)?;
    if package.manifest.native.is_none()
        && (package.manifest.wasm.is_none() || package.manifest.inject.is_empty())
    {
        return Ok(None);
    }
    if let Some(state) = package
        .manifest
        .native
        .as_ref()
        .and_then(|native| native.state.first())
    {
        return Err(crate::i18n::trf(
            "E_MIXIN_STATE_LAYOUT: {mod}:{state}: native state changes the prebuilt game's layout; use an attachment instead",
            &[("mod", &package.manifest.id), ("state", &state.id)],
        ));
    }
    package.directory_files(dir)?;
    Ok(Some(Bundle {
        manifest: package.manifest,
        dir: dir.to_owned(),
    }))
}

/// Generate tool input in the build's incoming copy, never in a modder's project.
pub(super) fn tool_manifest(bundle: &Bundle) -> Result<(), String> {
    let mut manifest = toml::Table::new();
    manifest.insert(
        "mod".into(),
        toml::Value::Table(toml::Table::from_iter([
            ("id".into(), bundle.manifest.id.clone().into()),
            ("version".into(), bundle.manifest.version.clone().into()),
        ])),
    );
    manifest.insert(
        "compatibility".into(),
        toml::Value::Table(toml::Table::from_iter([(
            "horizon_revision".into(),
            bundle.native().horizon.clone().into(),
        )])),
    );
    for (name, rows) in [
        ("injection", toml::Value::try_from(&bundle.native().mixin)),
        ("bridge", toml::Value::try_from(&bundle.native().bridge)),
        ("state", toml::Value::try_from(&bundle.native().state)),
    ] {
        let rows = rows.map_err(|error| error.to_string())?;
        if rows.as_array().is_some_and(|rows| !rows.is_empty()) {
            manifest.insert(name.into(), rows);
        }
    }
    if !bundle.native().module.is_empty() {
        manifest.insert(
            "module".into(),
            toml::Value::try_from(&bundle.native().module).map_err(|e| e.to_string())?,
        );
    }
    fs::write(
        bundle.dir.join(MANIFEST),
        toml::to_string(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn checked_files(bundle: &Bundle) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    Package::read(&bundle.dir)?.directory_files(&bundle.dir)
}

pub(super) fn files(bundle: &Bundle) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    Ok(checked_files(bundle)?
        .into_keys()
        .map(|name| {
            let path = bundle.dir.join(&name);
            (name, path)
        })
        .collect())
}

/// The package's files and its uses table, which is no source file: a copy is judged against
/// another game build as the package was.
pub(super) fn copy(bundle: &Bundle, to: &Path) -> Result<(), String> {
    let mut files = checked_files(bundle)?;
    if let Ok(table) = fs::read(bundle.dir.join(USES_FILE)) {
        files.insert(USES_FILE.to_owned(), table);
    }
    copy_files(to, files)
}

/// Only code enters the tool; runtime resource packs stay with the package.
pub(super) fn copy_tool_input(bundle: &Bundle, to: &Path) -> Result<(), String> {
    copy_files(
        to,
        checked_files(bundle)?.into_iter().filter(|(name, _)| {
            manifest::source_file(name)
                || bundle.native().mixin.iter().any(|row| row.helper == *name)
        }),
    )
}

fn copy_files(to: &Path, files: impl IntoIterator<Item = (String, Vec<u8>)>) -> Result<(), String> {
    for (name, bytes) in files {
        let path = to.join(name);
        fs::create_dir_all(path.parent().ok_or("invalid destination")?)
            .map_err(|e| e.to_string())?;
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Check target-crossing edges before the native-only dependency resolver.
///
/// A package that can't be read stops the scan, naming the mod: every enabled native mod compiles
/// into one game binary, and every other native refusal stops that build the same way. Leaving
/// one mod out would quietly build a different set than the player enabled.
pub(super) fn checked_bundles(folder: &Path) -> Result<Vec<Bundle>, String> {
    let paths = horizon_mod_package::find(folder)
        .into_iter()
        .chain(horizon_mod_package::find(
            &folder
                .parent()
                .unwrap_or(folder)
                .join(horizon_mod_package::mod_list::DIR),
        ));
    if let Some((id, error)) = horizon_mod_package::selection::package_errors(paths)
        .into_iter()
        .next()
    {
        return Err(format!("{id}: {error}"));
    }
    let found = super::scan_bundles(folder)?;
    let wasm: Vec<_> = horizon_mod_package::find(folder)
        .into_iter()
        .chain(horizon_mod_package::find(
            &folder
                .parent()
                .unwrap_or(folder)
                .join(horizon_mod_package::mod_list::DIR),
        ))
        .filter_map(|path| horizon_mod_package::Package::read(&path).ok())
        .filter(|package| package.manifest.wasm.is_some())
        .collect();
    for bundle in &found {
        let dependent = horizon_mod_package::Package::read(&bundle.dir)?;
        for library in &wasm {
            if !dependent
                .manifest
                .dependencies
                .contains_key(&library.manifest.id)
            {
                continue;
            }
            let reason = if let Some(name) = dependent
                .layouts
                .first_difference(&library.manifest.id, &library.layouts)
            {
                crate::i18n::trf(
                    "{mod}: layout mismatch in library {library} for type {type}",
                    &[
                        ("mod", &dependent.manifest.id),
                        ("library", &library.manifest.id),
                        ("type", name),
                    ],
                )
            } else {
                crate::i18n::trf("{library} is a wasm mod; native mod {mod} depends on it: mixed native and wasm dependencies aren't supported yet", &[("library", &library.manifest.id), ("mod", &dependent.manifest.id)])
            };
            return Err(reason);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod mixed_tests {
    use super::*;

    #[test]
    fn malformed_layout_retains_identity() {
        let why = refusal("malformed");
        assert!(why.contains("lib: layouts.json is malformed"), "{why}");
    }
    fn refusal(hash: &str) -> String {
        refusal_with(hash)
    }

    fn refusal_with(hash: &str) -> String {
        let home = tempfile::tempdir().unwrap();
        let addon = home.path().join("addon");
        let library = home.path().join("lib");
        fs::create_dir_all(addon.join("src")).unwrap();
        fs::create_dir_all(&library).unwrap();
        fs::write(
            addon.join("Cargo.toml"),
            "[package]\nname='addon'\nversion='1.0.0'\n",
        )
        .unwrap();
        fs::write(addon.join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        let header = |id| {
            format!("id='{id}'\nname='{id}'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n")
        };
        fs::write(
            addon.join(manifest::MANIFEST),
            format!(
                "{}[dependencies]\nlib='*'\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                header("addon"),
                "a".repeat(40),
                "b".repeat(40)
            ),
        )
        .unwrap();
        let module = b"\x00\x61\x73\x6d\x01\x00\x00\x00\x00\x0f\x08\x64\x79\x6c\x69\x6e\x6b\x2e\x30\x01\x04\x00\x00\x00\x00\x01\x04\x01\x60\x00\x00\x03\x02\x01\x00\x07\x2e\x03\x0e\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x6c\x61\x79\x6f\x75\x74\x00\x00\x0a\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x69\x6e\x00\x00\x0c\x68\x6f\x72\x69\x7a\x6f\x6e\x5f\x68\x6f\x6f\x6b\x00\x00\x0a\x04\x01\x02\x00\x0b";
        fs::write(library.join("mod.wasm"), module).unwrap();
        fs::write(
            library.join(manifest::MANIFEST),
            format!(
                "{}[wasm]\nsha512='{}'\ntoolchain='t'\nbuilt_with='t'\n",
                header("lib"),
                horizon_mod_package::manifest::sha512_of(module),
            ),
        )
        .unwrap();
        fs::write(
            addon.join("layouts.json"),
            br#"{"lib":{"T":"0123456789abcdef"}}"#,
        )
        .unwrap();
        fs::write(
            library.join("layouts.json"),
            format!(r#"{{"lib":{{"T":"{hash}"}}}}"#),
        )
        .unwrap();

        if hash == "malformed" {
            fs::write(library.join("layouts.json"), b"{").unwrap();
        }
        checked_bundles(home.path()).expect_err("mixed dependency must refuse")
    }

    #[test]
    fn native_to_wasm_layout_mismatch_names_the_type() {
        assert_eq!(
            refusal("1123456789abcdef"),
            "addon: layout mismatch in library lib for type T"
        );
    }

    #[test]
    fn native_to_wasm_matching_layout_is_still_unsupported() {
        assert_eq!(refusal("0123456789abcdef"), "lib is a wasm mod; native mod addon depends on it: mixed native and wasm dependencies aren't supported yet");
    }
}

#[cfg(test)]
mod state_tests {
    #[test]
    fn native_state_is_refused_before_staging() {
        let dir = super::super::tests::dir("state-refusal");
        super::super::tests::write_bundle(&dir, "stateful", "");
        let path = dir.join(horizon_mod_package::manifest::MANIFEST);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("\n[[native.state]]\nid='autosave'\ntarget='game::Game'\nty='u32'\ninitial='0'\nconstructor=[]\n");
        std::fs::write(path, text).unwrap();
        let error = super::read(&dir).err().unwrap();
        assert!(error.contains("E_MIXIN_STATE_LAYOUT"), "{error}");
        assert!(error.contains("attachment"), "{error}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
