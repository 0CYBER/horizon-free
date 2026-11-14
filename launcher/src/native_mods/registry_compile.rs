//! Cargo builds each mod as a Rust dylib; the SDK executable remains unchanged.
use super::{api, dependencies, internals, prebuilt, registry_plan, registry_rustc};
use crate::{
    i18n::{tr, trf},
    rust_toolchain::Toolchain,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(super) struct Options<'a> {
    pub consent: Option<&'a dyn Fn(&super::RegistryInventory) -> bool>,
    pub adapter: &'a Path,
}

fn compiler_plan(
    rust: &Toolchain,
    game: &Path,
    bindings: BTreeMap<PathBuf, registry_rustc::Bindings>,
) -> registry_rustc::Plan {
    registry_rustc::Plan {
        rustc: rust.rustc(),
        target: rust.host.target.into(),
        cache_wrapper: std::env::var_os("RUSTC_WRAPPER").map(PathBuf::from),
        workspace_wrapper: std::env::var_os("RUSTC_WORKSPACE_WRAPPER").map(PathBuf::from),
        // Transitive proc-macro metadata lives in host, just as on the direct SDK route.
        search: vec![game.join("deps"), game.join("host")],
        native: game.join("native"),
        crates: bindings,
    }
}

/// `out` is the mods folder's staging folder, rewritten every build; `target_dir` is cargo's build
/// folder, kept between builds of the same game build (see `prebuilt::compile`).
#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    rust: &Toolchain,
    game: &Path,
    out: &Path,
    target_dir: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    options: Options<'_>,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let sdk = prebuilt::identity(game)?;
    let lock = sdk.registry_lock.as_ref().ok_or_else(|| tr("The prebuilt SDK does not declare a locked registry graph; install an SDK that supports registry-bearing native mods").to_string())?;
    let mut patches = sdk.registry_patches.clone().ok_or_else(|| {
        tr("The prebuilt SDK does not declare game registry patches; install a newer SDK")
            .to_string()
    })?;
    let sdk_root = game.canonicalize().map_err(|e| e.to_string())?;
    for (_, registry) in patches.iter_mut() {
        for (_, spec) in registry
            .as_table_mut()
            .ok_or_else(|| tr("Invalid SDK patch registry").to_string())?
            .iter_mut()
        {
            let spec = spec
                .as_table_mut()
                .ok_or_else(|| tr("Invalid SDK patch").to_string())?;
            let relative = spec
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| tr("SDK patch has no path").to_string())?;
            let path = sdk_root.join(relative).canonicalize().map_err(|e| {
                trf(
                    "SDK patch {path}: {error}",
                    &[("path", relative), ("error", &e.to_string())],
                )
            })?;
            if !path.starts_with(&sdk_root) {
                return Err(tr("SDK patch escapes the verified SDK").into());
            }
            spec.insert("path".into(), path.to_string_lossy().into_owned().into());
        }
    }
    let links = sdk.native_links.as_ref().ok_or_else(|| tr("The prebuilt SDK does not declare native library providers; registry-bearing mods cannot be linked safely").to_string())?;
    let catalog = internals::Catalog::read(game)?;
    let generated = catalog
        .prepare(projects.iter().map(|project| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    // The last build's staging, kept aside until this one's is written: a file whose bytes didn't
    // change gets its old modified time back, so cargo's fingerprints reuse what it built from it.
    let previous = out.with_extension("old");
    if previous.exists() {
        fs::remove_dir_all(&previous).map_err(|e| e.to_string())?;
    }
    if out.exists() {
        fs::rename(out, &previous).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let template = api::template(&game.join(super::api_route::TEMPLATE))?;
    let mut bindings = BTreeMap::new();
    let mut members = Vec::new();
    let mut wrapper_deps = toml::Table::new();
    let deps = game.join("deps");
    let packages: BTreeMap<_, _> = projects
        .iter()
        .map(|project| {
            (
                project.bundle.manifest.id.as_str(),
                project.package.as_str(),
            )
        })
        .collect();
    for (position, &index) in resolution.order.iter().enumerate() {
        let project = &projects[index];
        let bundle = project.bundle;
        let id = &bundle.manifest.id;
        // Validate the whole local manifest closure without copying or executing its code.
        let inert = out.join("validation").join(id);
        super::registry_generate::stage(bundle, &inert)?;
        let relative = format!("native-api/{id}");
        let dest = out.join(&relative);
        super::copy_bundle(bundle, &dest)?;
        let mut todo = vec![PathBuf::new()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(local) = todo.pop() {
            if !seen.insert(local.clone()) {
                continue;
            }
            let path = dest.join(&local).join("Cargo.toml");
            let mut manifest: toml::Table = fs::read_to_string(&path)
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
            super::registry_generate::rewrite_dependencies(
                bundle,
                &local,
                &mut manifest,
                &mut todo,
            )?;
            if let Some(targets) = manifest
                .get_mut("target")
                .and_then(toml::Value::as_table_mut)
            {
                for (_, target) in targets.iter_mut() {
                    if let Some(table) = target.as_table_mut() {
                        super::registry_generate::rewrite_dependencies(
                            bundle, &local, table, &mut todo,
                        )?;
                    }
                }
            }
            if local.as_os_str().is_empty() {
                let targets: Vec<_> = declared[index]
                    .iter()
                    .map(|dependency| dependencies::Package {
                        dependency,
                        package: packages.get(dependency.id.as_str()).copied(),
                    })
                    .collect();
                let enabled = dependencies::rewrite(id, &mut manifest, &targets)
                    .map_err(|e| e.to_string())?;
                let mut dependency =
                    toml::toml! { path = (relative.clone()) package = (project.package.clone()) };
                if !enabled.is_empty() {
                    dependency.insert(
                        "features".into(),
                        enabled
                            .into_iter()
                            .map(toml::Value::String)
                            .collect::<Vec<_>>()
                            .into(),
                    );
                }
                wrapper_deps.insert(format!("mod_{position}"), dependency.into());
                let lib = manifest
                    .entry("lib")
                    .or_insert_with(|| toml::Table::new().into())
                    .as_table_mut()
                    .ok_or(tr("Invalid lib table"))?;
                lib.insert(
                    "crate-type".into(),
                    vec![toml::Value::String("dylib".into())].into(),
                );
                let root = dest.join(
                    lib.get("path")
                        .and_then(toml::Value::as_str)
                        .unwrap_or("src/lib.rs"),
                );
                let entry = root
                    .parent()
                    .ok_or(tr("Invalid library root"))?
                    .join("__horizon_entry.rs");
                fs::write(
                    &entry,
                    prebuilt::library_entry(
                        &template,
                        &[&bundle.manifest],
                        &catalog,
                        sdk.commit(),
                        id,
                        &bundle.manifest.version.to_string(),
                        &[],
                    ),
                )
                .map_err(|e| e.to_string())?;
                let mut source = fs::read_to_string(&root).map_err(|e| e.to_string())?;
                prebuilt::check_entry_root(&source, id)?;
                source.push_str("\n#[allow(unsafe_code)]\n#[path = \"__horizon_entry.rs\"] mod __horizon_entry;\n");
                fs::write(root, source).map_err(|e| e.to_string())?;
            }
            fs::write(path, manifest.to_string()).map_err(|e| e.to_string())?;
        }
        let mut externs: BTreeMap<String, PathBuf> = BTreeMap::from([
            ("mod_api".into(), prebuilt::library(&deps, "mod_api")?),
            ("mod_vocab".into(), prebuilt::library(&deps, "mod_vocab")?),
        ]);
        for (name, path) in &externs {
            prebuilt::refuse_closure_rlib(id, name, path)?;
        }
        for (name, package) in project.api_dependencies() {
            if package != "internals" {
                let path = prebuilt::library(&deps, package)?;
                prebuilt::refuse_closure_rlib(id, package, &path)?;
                externs.insert(name, path);
            }
        }
        if internals::needed(&bundle.manifest)
            || project
                .api_dependencies()
                .iter()
                .any(|(_, package)| *package == "internals")
        {
            let facade = &generated[id.as_str()];
            let facade_relative = internals::relative(id);
            let facade_dir = out.join(&facade_relative);
            fs::create_dir_all(facade_dir.join("src")).map_err(|e| e.to_string())?;
            fs::write(facade_dir.join("src/lib.rs"), &facade.source).map_err(|e| e.to_string())?;
            let facade_package = internals::package(id);
            let manifest = toml::toml! { [package] name = (facade_package.clone()) version = "0.0.0" edition = "2024" };
            fs::write(facade_dir.join("Cargo.toml"), manifest.to_string())
                .map_err(|e| e.to_string())?;
            let mut facade_externs = BTreeMap::new();
            for name in facade
                .crates
                .iter()
                .map(String::as_str)
                .chain(["horizon_client"])
            {
                let path = prebuilt::facade_library(&deps, name, &sdk)?;
                prebuilt::refuse_closure_rlib(id, name, &path)?;
                facade_externs.insert(name.to_owned(), path);
            }
            bindings.insert(
                facade_dir,
                registry_rustc::Bindings {
                    externs: facade_externs,
                    link_args: Vec::new(),
                },
            );
            members.push(facade_relative.clone());
            let mut manifest: toml::Table = fs::read_to_string(dest.join("Cargo.toml"))
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e| format!("Cargo.toml: {e}"))?;
            let dependencies = manifest
                .entry("dependencies")
                .or_insert_with(|| toml::Table::new().into())
                .as_table_mut()
                .ok_or("invalid dependencies")?;
            let aliases: Vec<_> = project
                .api_dependencies()
                .into_iter()
                .filter(|(_, package)| *package == "internals")
                .map(|(name, _)| name)
                .collect();
            for alias in aliases {
                dependencies.insert(alias, toml::toml! { path = (format!("../../{facade_relative}")) package = (facade_package.clone()) }.into());
            }
            fs::write(dest.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
            wrapper_deps.insert(
                format!("mod_{position}_internals"),
                toml::toml! { path = facade_relative package = facade_package }.into(),
            );
        }
        let mut link_args = sdk.link_args.clone();
        if cfg!(target_os = "linux") {
            link_args.push("-Wl,--disable-new-dtags".into());
            link_args.push("-Wl,-rpath,$ORIGIN".into());
        }
        if cfg!(target_os = "macos") {
            link_args.push("-Wl,-rpath,@loader_path".into());
        }
        bindings.insert(dest, registry_rustc::Bindings { externs, link_args });
        members.push(relative);
    }
    // `[[native.mixin]]` rows: the patched crates join the workspace beside the mods, whose
    // crates their helpers call, and compile against the SDK like a mod.
    let patch = super::patch::prepare(rust, game, &out.join("fnr"), projects, &sdk)?;
    if let Some(patch) = &patch {
        for name in &patch.names {
            let dir = patch.dir.join(name);
            let mut manifest = toml::toml! { [package] name = (name.clone()) version = "0.0.0" edition = "2024" [lib] path = "src/lib.rs" };
            let mut dependencies = toml::Table::new();
            for bundle in &patch.bundles {
                let id = &bundle.manifest.id;
                let project = projects
                    .iter()
                    .find(|project| project.bundle.manifest.id == *id)
                    .expect("a prepared mod is a project");
                let krate = project.crate_name();
                dependencies.insert(
                    krate,
                    toml::toml! { path = (out.join("native-api").join(id).display().to_string()) package = (project.package.clone()) }
                        .into(),
                );
            }
            manifest.insert("dependencies".into(), dependencies.into());
            let features: toml::Table = sdk
                .game_features()
                .iter()
                .map(|feature| (feature.clone(), toml::Value::Array(Vec::new())))
                .collect();
            manifest.insert("features".into(), features.into());
            fs::write(dir.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
            let externs: BTreeMap<_, _> =
                super::patch::sdk_externs(game, &sdk)?.into_iter().collect();
            for (krate, path) in &externs {
                prebuilt::refuse_closure_rlib("horizon_patch", krate, path)?;
            }
            bindings.insert(
                dir,
                registry_rustc::Bindings {
                    externs,
                    link_args: Vec::new(),
                },
            );
            let relative = format!(
                "fnr/{}/{name}",
                patch
                    .dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            );
            members.push(relative.clone());
            wrapper_deps.insert(
                name.clone(),
                toml::toml! { path = (relative) features = (sdk.game_features().iter().cloned().map(toml::Value::String).collect::<Vec<_>>()) }.into(),
            );
        }
        // The tool's workspace files would make the crates a second workspace inside this one.
        for file in ["Cargo.toml", "Cargo.lock"] {
            let _ = fs::remove_file(patch.dir.join(file));
        }
    }
    fs::create_dir_all(out.join("src")).map_err(|e| e.to_string())?;
    let installs = patch
        .as_ref()
        .map(|patch| patch.installs.as_slice())
        .unwrap_or(&[]);
    let entry = prebuilt::library_entry(
        &template,
        &[],
        &catalog,
        sdk.commit(),
        "horizon_patch",
        "0",
        installs,
    );
    fs::write(out.join("src/lib.rs"), entry).map_err(|e| e.to_string())?;
    let mut manifest = toml::toml! { [package] name = "horizon_patch" version = "0.0.0" edition = "2024" [lib] crate-type = ["dylib"] [workspace] resolver = "2" };
    manifest
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .expect("generated workspace")
        .insert(
            "members".into(),
            members
                .into_iter()
                .map(toml::Value::String)
                .collect::<Vec<_>>()
                .into(),
        );
    manifest.insert("dependencies".into(), wrapper_deps.into());
    fs::write(out.join("Cargo.toml"), manifest.to_string()).map_err(|e| e.to_string())?;
    let declared_paths = projects
        .iter()
        .map(|project| {
            (
                project.bundle.manifest.id.as_str(),
                project
                    .bundle
                    .manifest
                    .dependencies
                    .keys()
                    .map(String::as_str)
                    .collect(),
            )
        })
        .collect();
    let wanted = super::crates::wanted(out, &declared_paths)?;
    let mut plan = registry_plan::Plan::read(lock, projects, &wanted)?;
    plan.authorize_game_patches(&patches);
    registry_plan::apply_patches(out, &patches)?;
    fs::write(
        out.join("Cargo.lock"),
        plan.mods.seed("version=4\npackage=[]\n")?,
    )
    .map_err(|e| e.to_string())?;
    plan.fetch_with_patches(rust, out, &patches)?;
    let metadata = super::registry_inventory::read(rust, out)
        .map_err(|error| plan.explain_resolution(error))?;
    plan.check_mods(&super::registry_lock::RegistryLock::parse(
        &super::read_lock(out)?,
    )?)?;
    plan.allowed.check_partial_resolution(
        &super::registry_lock::RegistryLock::parse(&super::read_lock(out)?)?,
        &plan.game,
    )?;
    let mut game_names = plan.game.game_package_names();
    for registry in patches.values() {
        for name in registry.as_table().expect("validated SDK patches").keys() {
            game_names.remove(name);
        }
    }
    super::crates::check_game_graph_names(out, &super::read_lock(out)?, &game_names)?;
    let inventory = metadata.inventory(&super::registry_lock::RegistryLock::default(), links)?;
    if inventory.added_crates > 0 && !options.consent.is_some_and(|ask| ask(&inventory)) {
        return Err(crate::i18n::tr("Registry dependency compilation was not approved").into());
    }
    let mut externs = prebuilt::wrapper_externs(&deps, &template)?;
    externs.insert("mod_vocab".into(), prebuilt::library(&deps, "mod_vocab")?);
    for (name, path) in &externs {
        prebuilt::refuse_closure_rlib("horizon_patch", name, path)?;
    }
    let mut link_args = sdk.link_args.clone();
    if cfg!(target_os = "linux") {
        link_args.push("-Wl,--disable-new-dtags".into());
        link_args.push("-Wl,-rpath,$ORIGIN".into());
    }
    if cfg!(target_os = "macos") {
        link_args.push("-Wl,-rpath,@executable_path".into());
    }
    bindings.insert(
        out.to_path_buf(),
        registry_rustc::Bindings { externs, link_args },
    );
    if previous.exists() {
        keep_unchanged_times(out, &previous).map_err(|e| e.to_string())?;
        fs::remove_dir_all(&previous).map_err(|e| e.to_string())?;
    }
    let mut command = super::cargo(rust, out, "build");
    command
        .args([
            "--workspace",
            "--lib",
            "--offline",
            "--locked",
            "--release",
            "--target",
            rust.host.target,
        ])
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--message-format=json-render-diagnostics");
    // Cargo's per-crate timings (`target/cargo-timings/`), for the check that measures where a
    // registry build's time goes; a player's kept build folder doesn't collect them.
    if std::env::var_os("HORIZON_NATIVE_CARGO_TIMINGS").is_some() {
        command.arg("--timings");
    }
    let wrapper = compiler_plan(rust, game, bindings);
    wrapper.configure(&mut command, &out.join("rustc-plan.json"), options.adapter)?;
    let messages = out.join("cargo-messages.jsonl");
    let result = super::run_checked(
        command,
        tr("Compiling registry mod libraries"),
        |_| {},
        Some(fs::File::create(&messages).map_err(|e| e.to_string())?),
    );
    let outputs = super::registry_runtime::Outputs::read(&messages)?;
    if let Some(patch) = &patch {
        patch.log()?;
    }
    super::append_log(&fs::read_to_string(&messages).map_err(|e| e.to_string())?)?;
    if let Err(failure) = result {
        let mut error = format!("{}\n{}", failure.error, outputs.errors.join("\n"));
        if error.contains("E0277") || error.contains("E0308") {
            error.push_str(&format!("\n\n{}", crate::i18n::tr("SDK values keep the SDK's Rust crate identities. A registry-built copy of a trait or type is not interchangeable, even at the same version. Type mismatches are refused; SDK libraries are never substituted.")));
        }
        return Err(error);
    }
    let binary = game.join("bin").join(if cfg!(windows) {
        "horizon-client.exe"
    } else {
        "horizon-client"
    });
    let release = target_dir.join(rust.host.target).join("release");
    let mut runtime = prebuilt::runtime_libraries(rust, &sdk, game)?;
    for &index in &resolution.order {
        let project = &projects[index];
        let library = release.join(prebuilt::dylib_name(&project.crate_name()));
        super::library_record::validate_exports(&library, &project.bundle.manifest.id)?;
        super::library_record::stamp(
            &library,
            sdk.commit(),
            &project.bundle.manifest.id,
            &project.bundle.manifest.version.to_string(),
            false,
        )?;
        runtime.push(library);
    }
    let library = release.join(prebuilt::dylib_name("horizon_patch"));
    super::library_record::validate_exports(&library, "horizon_patch")?;
    super::library_record::stamp(&library, sdk.commit(), "horizon_patch", "0", true)?;
    runtime.push(library);
    for library in outputs.runtime(&metadata.runtime_packages()?, target_dir)? {
        if let Some(existing) = runtime
            .iter()
            .find(|existing| existing.file_name() == library.file_name())
        {
            if fs::read(existing).map_err(|e| e.to_string())?
                != fs::read(&library).map_err(|e| e.to_string())?
            {
                return Err(format!(
                    "Native runtime output {} conflicts with an immutable SDK library",
                    library.display()
                ));
            }
        } else {
            runtime.push(library);
        }
    }
    Ok((binary, runtime))
}

/// Gives each file under `staged` whose bytes equal the file at the same place under `previous`
/// that file's modified time.
fn keep_unchanged_times(staged: &Path, previous: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(staged)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let old = previous.join(entry.file_name());
        if kind.is_dir() {
            if old.is_dir() {
                keep_unchanged_times(&entry.path(), &old)?;
            }
        } else if kind.is_file() {
            let Ok(meta) = fs::symlink_metadata(&old) else {
                continue;
            };
            if meta.is_file()
                && meta.len() == entry.metadata()?.len()
                && fs::read(&old)? == fs::read(entry.path())?
            {
                fs::File::options()
                    .write(true)
                    .open(entry.path())?
                    .set_modified(meta.modified()?)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// A restaged file whose bytes didn't change keeps its old modified time, so cargo reuses
    /// what it built from it; a changed or new file keeps its new one.
    #[test]
    fn only_unchanged_staged_files_keep_their_old_time() {
        let root = tempfile::tempdir().unwrap();
        let (old, new) = (root.path().join("stage.old"), root.path().join("stage"));
        for dir in [&old, &new] {
            fs::create_dir_all(dir.join("src")).unwrap();
        }
        let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        for (name, text) in [("src/lib.rs", "same"), ("Cargo.toml", "old")] {
            let path = old.join(name);
            fs::write(&path, text).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(then)
                .unwrap();
        }
        fs::write(new.join("src/lib.rs"), "same").unwrap();
        fs::write(new.join("Cargo.toml"), "new").unwrap();
        fs::write(new.join("src/added.rs"), "added").unwrap();
        keep_unchanged_times(&new, &old).unwrap();
        let modified = |name: &str| fs::metadata(new.join(name)).unwrap().modified().unwrap();
        assert_eq!(modified("src/lib.rs"), then);
        assert_ne!(modified("Cargo.toml"), then);
        assert_ne!(modified("src/added.rs"), then);
    }

    use super::*;

    #[test]
    #[ignore = "daily"]
    fn registry_adapter_compiles_facade_with_transitive_host_macro_metadata() {
        use std::{ffi::OsString, process::Command};
        let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/registry-facade-tests");
        fs::create_dir_all(&target).unwrap();
        let scratch = tempfile::tempdir_in(target).unwrap();
        let game = scratch.path().join("sdk");
        for folder in ["deps", "host", "native"] {
            fs::create_dir_all(game.join(folder)).unwrap();
        }
        let sysroot = Command::new("rustc")
            .arg("--print")
            .arg("sysroot")
            .output()
            .unwrap();
        assert!(sysroot.status.success());
        let rust = Toolchain {
            dir: PathBuf::from(String::from_utf8(sysroot.stdout).unwrap().trim()),
            host: crate::rust_toolchain::host().expect("supported test host"),
        };
        let macros = game.join("host").join(format!(
            "{}sdk_macros{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ));
        let macro_source = scratch.path().join("macros.rs");
        fs::write(&macro_source, "extern crate proc_macro; #[proc_macro_derive(Marker)] pub fn marker(_: proc_macro::TokenStream) -> proc_macro::TokenStream { proc_macro::TokenStream::new() }").unwrap();
        let built = Command::new(rust.rustc())
            .args([
                "--crate-name",
                "sdk_macros",
                "--crate-type",
                "proc-macro",
                "--edition",
                "2024",
            ])
            .arg(&macro_source)
            .arg("-o")
            .arg(&macros)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let engine = game.join("deps/libhorizon_client-fixture.rmeta");
        let engine_source = scratch.path().join("engine.rs");
        fs::write(
            &engine_source,
            "pub use sdk_macros::Marker; #[derive(Marker)] pub struct Game;",
        )
        .unwrap();
        let built = Command::new(rust.rustc())
            .args([
                "--crate-name",
                "horizon_client",
                "--crate-type",
                "rlib",
                "--edition",
                "2024",
                "--emit",
                "metadata",
                "-C",
                "prefer-dynamic",
                "--target",
                rust.host.target,
            ])
            .arg(&engine_source)
            .arg("--extern")
            .arg(format!("sdk_macros={}", macros.display()))
            .arg("-o")
            .arg(&engine)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let facade = scratch.path().join("facade");
        fs::create_dir_all(&facade).unwrap();
        let source = facade.join("lib.rs");
        fs::write(&source, "pub use horizon_client::Game;").unwrap();
        let mut plan = compiler_plan(
            &rust,
            &game,
            BTreeMap::from([(
                facade.clone(),
                registry_rustc::Bindings {
                    externs: BTreeMap::from([("horizon_client".into(), engine)]),
                    link_args: vec![],
                },
            )]),
        );
        plan.cache_wrapper = None;
        plan.workspace_wrapper = None;
        let args: Vec<OsString> = vec![
            rust.rustc().into_os_string(),
            source.into_os_string(),
            "--crate-name".into(),
            "facade".into(),
            "--crate-type".into(),
            "rlib".into(),
            "--edition".into(),
            "2024".into(),
            "--target".into(),
            rust.host.target.into(),
            "--emit".into(),
            "metadata".into(),
            "-o".into(),
            scratch.path().join("facade.rmeta").into_os_string(),
        ];
        let (program, arguments) = plan
            .arguments(args.clone(), Some(facade.clone()), scratch.path())
            .unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        // Render a real SDK-route executable workspace. Its SDK libraries are
        // binary externs, not Cargo source dependencies, including the host that
        // the SDK-owned template calls after the layer split.
        for name in [
            "mod_api",
            "mod_vocab",
            "mod_points",
            "horizon_mod_package",
            "horizon_mod_host",
        ] {
            let source = scratch.path().join(format!("{name}.rs"));
            fs::write(
                &source,
                if name == "horizon_mod_host" {
                    "pub fn main_with() {}"
                } else {
                    ""
                },
            )
            .unwrap();
            let result = Command::new(rust.rustc())
                .args([
                    "--crate-name",
                    name,
                    "--crate-type",
                    "rlib",
                    "--edition",
                    "2024",
                    "--emit",
                    "metadata",
                    "--target",
                    rust.host.target,
                ])
                .arg(source)
                .arg("-o")
                .arg(game.join(format!("deps/lib{name}-fixture.rmeta")))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        fs::write(game.join(prebuilt::TEMPLATE), "fn main() { horizon_mod_host::main_with(); }\n/* MOD_ENTRIES */\n/* MOD_SLOTS */\n/* BUILT_MODS */\n").unwrap();
        let template = api::template(&game.join(prebuilt::TEMPLATE)).unwrap();
        let wrapper = scratch.path().join("wrapper");
        fs::create_dir_all(wrapper.join("src")).unwrap();
        fs::write(
            wrapper.join("Cargo.toml"),
            "[package]\nname='registry-game'\nversion='0.0.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            wrapper.join("src/main.rs"),
            api::main_rs(&template, &[], &internals::Catalog::default()),
        )
        .unwrap();
        let bindings = prebuilt::wrapper_externs(&game.join("deps"), &template).unwrap();
        let mut wrapper_plan = compiler_plan(
            &rust,
            &game,
            BTreeMap::from([(
                wrapper.clone(),
                registry_rustc::Bindings {
                    externs: bindings,
                    link_args: vec![],
                },
            )]),
        );
        wrapper_plan.cache_wrapper = None;
        wrapper_plan.workspace_wrapper = None;
        let mut wrapper_args = args.clone();
        wrapper_args[1] = wrapper.join("src/main.rs").into_os_string();
        wrapper_args[3] = "registry_game".into();
        wrapper_args[5] = "bin".into();
        *wrapper_args.last_mut().unwrap() = scratch.path().join("wrapper.rmeta").into_os_string();
        let (program, arguments) = wrapper_plan
            .arguments(wrapper_args.clone(), Some(wrapper.clone()), scratch.path())
            .unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        wrapper_plan
            .crates
            .get_mut(&wrapper)
            .unwrap()
            .externs
            .remove("horizon_mod_host");
        let (program, arguments) = wrapper_plan
            .arguments(wrapper_args, Some(wrapper), scratch.path())
            .unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("E0433"));
        // The old registry route supplied deps alone, unlike the direct SDK route.
        plan.search = vec![game.join("deps")];
        let (program, arguments) = plan.arguments(args, Some(facade), scratch.path()).unwrap();
        let result = Command::new(program).args(arguments).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("E0463"));
    }

    #[test]
    #[ignore = "requires an explicitly prepared private SDK, mod and launcher executable"]
    fn real_sdk_registry_compile() {
        let required =
            |name| PathBuf::from(std::env::var_os(name).expect("explicit fixture path required"));
        let game = required("HORIZON_TEST_REGISTRY_SDK");
        let folder = required("HORIZON_TEST_REGISTRY_MOD");
        let out = required("HORIZON_TEST_REGISTRY_OUT");
        let adapter = required("HORIZON_TEST_REGISTRY_ADAPTER");
        let bundle = super::super::bundle_in(&folder).unwrap().unwrap();
        let projects = [api::Project::read(&bundle).unwrap()];
        let declared = [Vec::new()];
        let inputs = [dependencies::Mod {
            id: &bundle.manifest.id,
            version: &bundle.manifest.version,
            dependencies: &declared[0],
            has_api: true,
            code: true,
        }];
        let resolution = dependencies::resolve(&inputs).unwrap();
        let rust = crate::rust_toolchain::install(|_| {}).unwrap();
        let approve = std::env::var_os("HORIZON_TEST_REGISTRY_DECLINE").is_none();
        let result = compile(
            &rust,
            &game,
            &out,
            &out.join("target"),
            &projects,
            &declared,
            &resolution,
            Options {
                consent: Some(&|inventory| {
                    eprintln!("{}\n{}", inventory.label(), inventory.detail());
                    approve
                }),
                adapter: &adapter,
            },
        );
        if approve {
            let (binary, _) = result.unwrap();
            assert!(binary.is_file());
            eprintln!("compiled wrapper {}", binary.display());
        } else {
            assert!(result.unwrap_err().contains("not approved"));
            assert!(!out.join("cargo-messages.jsonl").exists());
            assert!(!out.join("target").exists());
            eprintln!(
                "detailed consent declined before target compilation or Cargo build messages"
            );
        }
    }
}
