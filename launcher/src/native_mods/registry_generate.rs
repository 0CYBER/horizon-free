//! Explicit, index-only generation of the registry lock consumed by native staging.
use super::{api, Bundle};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Copies only inert manifests. Source files, build scripts and Cargo config never enter it.
pub(super) fn stage(bundle: &Bundle, output: &Path) -> Result<(), String> {
    let mut todo = vec![PathBuf::new()];
    let mut seen = BTreeSet::new();
    while let Some(relative) = todo.pop() {
        if !seen.insert(relative.clone()) {
            continue;
        }
        let input = bundle.dir.join(&relative);
        let manifest_path = input.join("Cargo.toml");
        if super::crates::is_link_or_junction(
            &fs::symlink_metadata(&manifest_path).map_err(|e| e.to_string())?,
        ) {
            return Err("linked dependency manifest is refused".into());
        }
        let mut manifest: toml::Table = fs::read_to_string(&manifest_path)
            .map_err(|e| e.to_string())?
            .parse()
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        for section in ["patch", "replace", "workspace"] {
            if manifest.contains_key(section) {
                return Err(format!(
                    "{}: [{section}] is refused",
                    manifest_path.display()
                ));
            }
        }
        for config in [".cargo/config", ".cargo/config.toml"] {
            if input.join(config).exists() {
                return Err(format!(
                    "{}: Cargo source configuration is refused",
                    input.display()
                ));
            }
        }
        let mut named = rewrite_dependencies(bundle, &relative, &mut manifest, &mut todo)?;
        if let Some(targets) = manifest
            .get_mut("target")
            .and_then(toml::Value::as_table_mut)
        {
            for target in targets
                .iter_mut()
                .filter_map(|(_, value)| value.as_table_mut())
            {
                named.extend(rewrite_dependencies(bundle, &relative, target, &mut todo)?);
            }
        }
        // A proc macro compiles for the host, where the SDK's game libraries don't link.
        let proc_macro = manifest
            .get("lib")
            .and_then(|lib| lib.get("proc-macro"))
            .and_then(toml::Value::as_bool)
            == Some(true);
        if proc_macro && !named.is_empty() {
            return Err(crate::i18n::trf(
                "{crate}: a proc-macro crate can't depend on the mod API; keep the API calls in the crates its macros expand into",
                &[("crate", &relative.display().to_string())],
            ));
        }
        manifest.remove("bin");
        manifest.remove("example");
        manifest.remove("test");
        manifest.remove("bench");
        manifest.remove("lib");
        let package = manifest
            .get_mut("package")
            .and_then(toml::Value::as_table_mut)
            .ok_or("Cargo.toml: missing package")?;
        package.insert("build".into(), false.into());
        for key in ["autobins", "autoexamples", "autotests", "autobenches"] {
            package.insert(key.into(), false.into());
        }
        manifest.insert("lib".into(), toml::toml! { path = "inert.rs" }.into());
        if relative.as_os_str().is_empty() {
            manifest.insert("workspace".into(), toml::Table::new().into());
        }
        let dest = output.join(relative);
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        fs::write(dest.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
        fs::write(dest.join("inert.rs"), "").map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Validates and removes the SDK dependencies of the crate at `relative` inside the mod (the root
/// crate's or a local crate's: each names the mod's one SDK copy from its own folder), queues its
/// local path dependencies in `todo`, and returns the SDK packages it named, each with the name
/// its code calls it by (`mod_api`, or a renamed dependency's key). The build supplies those as
/// compiled SDK libraries instead. The per-mod internals facade is the root crate's alone.
pub(super) fn rewrite_dependencies(
    bundle: &Bundle,
    relative: &Path,
    table: &mut toml::Table,
    todo: &mut Vec<PathBuf>,
) -> Result<Vec<(String, &'static str)>, String> {
    let mut named = Vec::new();
    for kind in [
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "dev_dependencies",
        "build_dependencies",
    ] {
        let Some(deps) = table.get_mut(kind).and_then(toml::Value::as_table_mut) else {
            continue;
        };
        let mut sdk = Vec::new();
        for (key, dep) in deps.iter_mut() {
            let get = |field| dep.get(field).and_then(toml::Value::as_str);
            let package = get("package").unwrap_or(key);
            if let Some(&(name, library)) = api::PACKAGES.iter().find(|(name, _)| *name == package)
            {
                api::sdk_source_at(relative, key, name, dep)?;
                let local = !relative.as_os_str().is_empty();
                if local && name == api::INTERNALS_PACKAGE {
                    return Err(crate::i18n::trf(
                        "{key}: {package} belongs in the mod's root crate, not the local crate {crate}",
                        &[
                            ("key", key),
                            ("package", name),
                            ("crate", &relative.display().to_string()),
                        ],
                    ));
                }
                // The build always supplies the SDK, so no feature can switch it off. Taking a
                // local crate's optional one out would leave its `dep:` and `/` entries in
                // `[features]` naming a dependency the manifest no longer has, which Cargo
                // rejects. The root crate may keep one: its `[features]` are refused outright
                // (`prebuilt`), so nothing can name it, and existing mods keep building.
                if local && dep.get("optional").and_then(toml::Value::as_bool) == Some(true) {
                    return Err(crate::i18n::trf(
                        "{manifest}: {key} can't be optional; the build always supplies the mod API",
                        &[
                            ("manifest", &relative.join("Cargo.toml").display().to_string()),
                            ("key", key),
                        ],
                    ));
                }
                // The root crate's feature requests are refused before the build (`prebuilt`):
                // the SDK is compiled once, with its pinned features, for every crate.
                let own_features = dep
                    .get("features")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|f| !f.is_empty())
                    || dep.get("default-features").and_then(toml::Value::as_bool) == Some(false);
                if local && own_features {
                    return Err(super::Refusal::Features {
                        mod_id: bundle.manifest.id.clone(),
                    }
                    .to_string());
                }
                let alias = if key == name {
                    library.to_owned()
                } else {
                    key.replace('-', "_")
                };
                named.push((alias, library));
                sdk.push(key.clone());
                continue;
            }
            for selector in [
                "git",
                "rev",
                "tag",
                "branch",
                "registry",
                "registry-index",
                "workspace",
            ] {
                if dep.get(selector).is_some() {
                    return Err(format!("{key}: {selector} source is refused"));
                }
            }
            if let Some(path) = dep.get("path") {
                let path = path.as_str().ok_or("dependency path must be a string")?;
                let inside = super::crates::local_path(&bundle.dir, relative, path)?;
                todo.push(inside);
            }
        }
        for key in sdk {
            deps.remove(&key);
        }
    }
    Ok(named)
}

pub(super) fn generate(bundle: &Bundle, update: bool) -> Result<(), String> {
    // Project owns the original API declaration and source-policy boundary.
    api::Project::read(bundle).map_err(|e| e.to_string())?;
    let destination = bundle.dir.join("Cargo.lock");
    if destination.exists() && !update {
        return Err(
            "Cargo.lock already exists; use --update-lock to replace its pins explicitly".into(),
        );
    }
    let work = tempfile::tempdir_in(super::root()).map_err(|e| e.to_string())?;
    stage(bundle, work.path())?;
    let rust = crate::rust_toolchain::install(|_| {})?;
    let command = super::cargo(&rust, work.path(), "generate-lockfile");
    super::run(command, "Resolving registry dependency pins", |_| {})?;
    let text = super::read_lock(work.path())?;
    super::registry_lock::RegistryLock::parse(&text)?;
    // Refusals and resolution errors above never mutate the existing lock.
    let mut pending = tempfile::NamedTempFile::new_in(&bundle.dir).map_err(|e| e.to_string())?;
    use std::io::Write;
    pending
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    pending.as_file().sync_all().map_err(|e| e.to_string())?;
    pending.persist(destination).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Bundle {
        super::super::tests::write_bundle(
            root,
            "registry_fixture",
            "compile_error!(\"must never compile\");",
        );
        fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\nversion='0.1.0'\n[dependencies]\nhorizon-mod-api={ path='.horizon/sdk/api/crates/horizon-mod-api' }\nitoa='1'\n").unwrap();
        super::super::bundle_in(root).unwrap().unwrap()
    }
    #[test]
    fn the_sdk_is_removed_only_in_its_path_form_and_sources_are_inert() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        fs::write(input.path().join("build.rs"), "panic!(\"do not run\");").unwrap();
        let before = fs::read(input.path().join("Cargo.toml")).unwrap();
        stage(&bundle, output.path()).unwrap();
        let generated = fs::read_to_string(output.path().join("Cargo.toml")).unwrap();
        assert!(!generated.contains("horizon-mod-api"));
        assert!(generated.contains("itoa"));
        assert!(!output.path().join("build.rs").exists());
        assert!(!output.path().join("src").exists());
        assert_eq!(fs::read(input.path().join("Cargo.toml")).unwrap(), before);
        let original = String::from_utf8(before).unwrap();
        for (form, refusal) in [
            (
                format!(
                    "{{ git='https://github.com/horizon-rust/horizon', rev='{}' }}",
                    "a".repeat(40)
                ),
                "E_SDK_SOURCE: horizon-mod-api: the SDK is no longer named by git",
            ),
            (
                "'0.1'".to_owned(),
                "E_SDK_SOURCE: horizon-mod-api: a version names no source",
            ),
            (
                "{ path='../horizon/crates/horizon-mod-api' }".to_owned(),
                "E_SDK_SOURCE: horizon-mod-api: the path must be \".horizon/sdk/api/crates/horizon-mod-api\"",
            ),
        ] {
            fs::write(
                input.path().join("Cargo.toml"),
                original.replace("{ path='.horizon/sdk/api/crates/horizon-mod-api' }", &form),
            )
            .unwrap();
            let error = stage(&bundle, output.path()).unwrap_err();
            assert!(error.starts_with(refusal), "{error}");
        }
    }
    #[test]
    fn source_refusals_leave_existing_lock_untouched() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::write(input.path().join("Cargo.lock"), "original pins").unwrap();
        assert!(generate(&bundle, false)
            .unwrap_err()
            .contains("--update-lock"));
        for extra in [
            "[patch.crates-io]\nitoa={path='outside'}\n",
            "[replace]\n'itoa:1.0.18'={path='outside'}\n",
        ] {
            fs::write(
                input.path().join("Cargo.toml"),
                format!("{original}\n{extra}"),
            )
            .unwrap();
            assert!(stage(&bundle, output.path()).is_err());
            assert_eq!(
                fs::read_to_string(input.path().join("Cargo.lock")).unwrap(),
                "original pins"
            );
        }
    }
    #[test]
    fn local_dependencies_are_traversed_but_cannot_escape() {
        let input = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::create_dir(input.path().join("local")).unwrap();
        fs::write(
            input.path().join("local/Cargo.toml"),
            "[package]\nname='helper'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}\nhelper={{path='local'}}\n"),
        )
        .unwrap();
        stage(&bundle, output.path()).unwrap();
        assert!(output.path().join("local/inert.rs").is_file());
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}\nhelper={{path='../outside'}}\n"),
        )
        .unwrap();
        assert!(stage(&bundle, output.path())
            .unwrap_err()
            .contains("outside the mod"));
    }
    #[test]
    fn local_crates_name_the_sdk_from_their_own_folder() {
        let input = tempfile::tempdir().unwrap();
        let bundle = fixture(input.path());
        let original = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        fs::write(
            input.path().join("Cargo.toml"),
            format!("{original}helper={{path='src/crates/helper'}}\n"),
        )
        .unwrap();
        let helper = input.path().join("src/crates/helper");
        fs::create_dir_all(&helper).unwrap();
        let write_helper = |dependencies: &str| {
            fs::write(
                helper.join("Cargo.toml"),
                format!(
                    "[package]\nname='helper'\nversion='0.1.0'\n[dependencies]\n{dependencies}\n"
                ),
            )
            .unwrap();
        };
        let rewrite = |relative: &str| {
            let path = input.path().join(relative).join("Cargo.toml");
            let mut manifest: toml::Table = fs::read_to_string(path).unwrap().parse().unwrap();
            let mut todo = Vec::new();
            let named =
                rewrite_dependencies(&bundle, Path::new(relative), &mut manifest, &mut todo);
            (named, manifest, todo)
        };

        // The SDK three folders up, under its own name and renamed: both are taken out of the
        // manifest and named for the build, which supplies them as SDK libraries.
        write_helper(
            "horizon-mod-api={path='../../../.horizon/sdk/api/crates/horizon-mod-api'}\n\
             points={package='horizon-mod-points',path='../../../.horizon/sdk/api/crates/horizon-mod-points'}\n\
             itoa='1'",
        );
        let (named, manifest, _) = rewrite("src/crates/helper");
        assert_eq!(
            named.unwrap(),
            [
                ("mod_api".to_owned(), "mod_api"),
                ("points".to_owned(), "mod_points")
            ]
        );
        let dependencies = manifest["dependencies"].as_table().unwrap();
        assert_eq!(dependencies.keys().collect::<Vec<_>>(), ["itoa"]);
        let output = tempfile::tempdir().unwrap();
        stage(&bundle, output.path()).unwrap();
        assert!(output.path().join("src/crates/helper/inert.rs").is_file());

        // The root crate's form, read from the local crate's folder, names no SDK.
        write_helper("horizon-mod-api={path='.horizon/sdk/api/crates/horizon-mod-api'}");
        let error = rewrite("src/crates/helper").0.unwrap_err();
        assert!(
            error.starts_with("E_SDK_SOURCE: horizon-mod-api: the path must be \"../../../.horizon/sdk/api/crates/horizon-mod-api\""),
            "{error}"
        );

        // The internals facade is generated for the mod's crate only.
        write_helper(
            "horizon-internals={path='../../../.horizon/sdk/api/crates/horizon-internals'}",
        );
        let error = rewrite("src/crates/helper").0.unwrap_err();
        assert!(error.contains("belongs in the mod's root crate"), "{error}");

        // The SDK is built once with its pinned features: a local crate can't ask for others,
        // and gets the root crate's refusal for it.
        for request in ["features=['extra']", "default-features=false"] {
            write_helper(&format!(
                "horizon-mod-api={{path='../../../.horizon/sdk/api/crates/horizon-mod-api',{request}}}"
            ));
            let error = rewrite("src/crates/helper").0.unwrap_err();
            assert_eq!(
                error,
                crate::native_mods::Refusal::Features {
                    mod_id: bundle.manifest.id.clone()
                }
                .to_string()
            );
        }

        // A local crate's SDK can't be optional: removing it would leave a feature such as
        // `dep:api` naming a dependency that is gone. Refused before Cargo runs, naming the crate.
        write_helper(
            "api={package='horizon-mod-api',path='../../../.horizon/sdk/api/crates/horizon-mod-api',optional=true}\n[features]\nwith-api=['dep:api']",
        );
        let error = rewrite("src/crates/helper").0.unwrap_err();
        assert_eq!(
            error,
            "src/crates/helper/Cargo.toml: api can't be optional; the build always supplies the mod API"
        );
        assert_eq!(stage(&bundle, output.path()).unwrap_err(), error);
        write_helper("itoa='1'");

        // The root crate may keep an optional SDK, as before: its `[features]` are refused
        // outright, so nothing names it once it is taken out.
        let root = fs::read_to_string(input.path().join("Cargo.toml")).unwrap();
        let optional = root.replace(
            "horizon-mod-api={ path='.horizon/sdk/api/crates/horizon-mod-api' }",
            "horizon-mod-api={ path='.horizon/sdk/api/crates/horizon-mod-api', optional=true }",
        );
        assert_ne!(optional, root);
        fs::write(input.path().join("Cargo.toml"), optional).unwrap();
        let (named, manifest, _) = rewrite("");
        assert_eq!(named.unwrap(), [("mod_api".to_owned(), "mod_api")]);
        assert!(!manifest["dependencies"]
            .as_table()
            .unwrap()
            .contains_key("horizon-mod-api"));
        stage(&bundle, output.path()).unwrap();
        fs::write(input.path().join("Cargo.toml"), root).unwrap();

        // A proc macro runs on the host, where the SDK's libraries don't link: refused before
        // the build, naming the crate. One that doesn't name the SDK is fine.
        let proc_macro = |dependencies: &str| {
            fs::write(
                helper.join("Cargo.toml"),
                format!(
                    "[package]\nname='helper'\nversion='0.1.0'\n[lib]\nproc-macro=true\n[dependencies]\n{dependencies}\n"
                ),
            )
            .unwrap();
            stage(&bundle, output.path())
        };
        let error =
            proc_macro("horizon-mod-api={path='../../../.horizon/sdk/api/crates/horizon-mod-api'}")
                .unwrap_err();
        assert!(
            error.starts_with("src/crates/helper: a proc-macro crate can't depend on the mod API"),
            "{error}"
        );
        proc_macro("itoa='1'").unwrap();

        // The root crate still names its SDK from the mod's folder, and queues its local crates.
        let (named, _, todo) = rewrite("");
        assert_eq!(named.unwrap(), [("mod_api".to_owned(), "mod_api")]);
        assert_eq!(todo, [PathBuf::from("src/crates/helper")]);
    }
}
