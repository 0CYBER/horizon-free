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
        rewrite_dependencies(bundle, &relative, &mut manifest, &mut todo)?;
        if let Some(targets) = manifest
            .get_mut("target")
            .and_then(toml::Value::as_table_mut)
        {
            for target in targets
                .iter_mut()
                .filter_map(|(_, value)| value.as_table_mut())
            {
                rewrite_dependencies(bundle, &relative, target, &mut todo)?;
            }
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

pub(super) fn rewrite_dependencies(
    bundle: &Bundle,
    relative: &Path,
    table: &mut toml::Table,
    todo: &mut Vec<PathBuf>,
) -> Result<(), String> {
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
            if let Some(&(name, _)) = api::PACKAGES.iter().find(|(name, _)| *name == package) {
                api::sdk_source(key, name, dep)?;
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
    Ok(())
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
}
