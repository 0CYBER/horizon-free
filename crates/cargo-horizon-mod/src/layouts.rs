//! Cached host-side layout probe for packaged mod types.
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_root: PathBuf,
    resolve: Resolve,
}
#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}
#[derive(Deserialize)]
struct Node {
    id: String,
    dependencies: Vec<String>,
}
#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    source: Option<String>,
    version: String,
    targets: Vec<Target>,
}
#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
    src_path: PathBuf,
}

fn resolved_features(
    tool: Tool,
    root: &Path,
    owner: &Package,
    package: &Package,
) -> Result<Vec<String>, String> {
    // Metadata enables defaults on all workspace members; tree selects our mod.
    let bytes = output(
        tool("cargo")
            .args([
                "tree",
                "--package",
                &owner.id,
                "--invert",
                &package.id,
                "--depth",
                "0",
                "--prefix",
                "none",
                "--format",
                "{f}",
                "--edges",
                "normal,build",
            ])
            .current_dir(root),
    )?;
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    Ok(text
        .trim()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

fn output(command: &mut Command) -> Result<Vec<u8>, String> {
    let result = command.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    Ok(result.stdout)
}
fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if fs::read(path).ok().as_deref() != Some(bytes) {
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Runs a Rust tool (`cargo`, `rustc`): the launcher's pinned toolchain outside tests.
type Tool<'a> = &'a dyn Fn(&str) -> Command;

/// Computes layout identities using the mod's resolved dependencies on the host target, with the
/// launcher's pinned toolchain.
pub fn build(root: &Path) -> Result<(), String> {
    build_with(&crate::wasm::pinned, root)
}

fn build_with(tool: Tool, root: &Path) -> Result<(), String> {
    if !root.join("Cargo.toml").is_file() {
        return Ok(());
    }
    // `cargo metadata` prints plain Windows paths, without the verbatim prefix
    // `canonicalize` adds, so member manifests are compared in that form.
    let root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    let declaration = horizon_mod_package::Manifest::parse(
        &fs::read_to_string(root.join(horizon_mod_package::manifest::MANIFEST))
            .map_err(|e| e.to_string())?,
    )?;
    let cargo: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let lib = root.join(
        cargo
            .get("lib")
            .and_then(|v| v.get("path"))
            .and_then(toml::Value::as_str)
            .unwrap_or("src/lib.rs"),
    );
    if declaration.dependencies.is_empty()
        && lib.is_file()
        && mod_source::wire_types::public_types("probe", &lib)?.is_empty()
    {
        let destination = root.join("layouts.json");
        if destination.exists() {
            return Err("layouts: existing layouts.json would lose all recorded types; remove it explicitly after reviewing the public API change".into());
        }
        return Ok(());
    }
    let metadata: Metadata = serde_json::from_slice(&output(
        tool("cargo")
            .args(["metadata", "--format-version", "1"])
            .current_dir(&root),
    )?)
    .map_err(|e| e.to_string())?;
    let owner = metadata
        .packages
        .iter()
        .find(|p| p.manifest_path == root.join("Cargo.toml"))
        .ok_or("mod is not a Cargo package")?;
    let mut needed = std::collections::BTreeSet::new();
    let mut pending = vec![owner.id.clone()];
    while let Some(id) = pending.pop() {
        if needed.insert(id.clone())
            && let Some(node) = metadata.resolve.nodes.iter().find(|n| n.id == id)
        {
            pending.extend(node.dependencies.iter().cloned());
        }
    }
    let mut declarations = Vec::new();
    for package in metadata.packages.iter().filter(|p| needed.contains(&p.id)) {
        let directory = package
            .manifest_path
            .parent()
            .ok_or("package manifest has no directory")?;
        let Ok(text) = fs::read_to_string(directory.join(horizon_mod_package::manifest::MANIFEST))
        else {
            continue;
        };
        let manifest = horizon_mod_package::Manifest::parse(&text)?;
        let Some(target) = package.targets.iter().find(|t| {
            t.kind
                .iter()
                .any(|k| matches!(k.as_str(), "lib" | "rlib" | "dylib" | "cdylib"))
        }) else {
            continue;
        };
        let types =
            mod_source::wire_types::public_types(&target.name.replace('-', "_"), &target.src_path)?;
        if !types.is_empty() {
            declarations.push((package, manifest.id, types));
        }
    }
    let destination = root.join("layouts.json");
    if declarations.is_empty() {
        if destination.exists() {
            return Err("layouts: existing layouts.json would lose all recorded types; remove it explicitly after reviewing the public API change".into());
        }
        return Ok(());
    }
    let api = metadata
        .packages
        .iter()
        .find(|p| p.name == "horizon-mod-api")
        .ok_or("crossing types need horizon-mod-api")?;
    let stage = root.join("target/horizon-mod/layout-probe");
    fs::create_dir_all(stage.join("src")).map_err(|e| e.to_string())?;
    let mut manifest = String::from(
        "[package]\nname = \"horizon-layout-probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\n",
    );
    let mut api_dependency = toml::map::Map::new();
    api_dependency.insert("package".into(), api.name.clone().into());
    api_dependency.insert("default-features".into(), false.into());
    api_dependency.insert(
        "features".into(),
        toml::Value::Array(
            resolved_features(tool, &root, owner, api)?
                .into_iter()
                .map(toml::Value::String)
                .collect(),
        ),
    );
    match api.source.as_deref() {
        Some(source) if source.starts_with("git+") => {
            let (url, rev) = source[4..]
                .rsplit_once('#')
                .ok_or("API git source has no revision")?;
            api_dependency.insert(
                "git".into(),
                url.split('?')
                    .next()
                    .ok_or("API git source has no URL")?
                    .into(),
            );
            api_dependency.insert("rev".into(), rev.into());
        }
        Some(_) => {
            api_dependency.insert("version".into(), format!("={}", api.version).into());
        }
        None => {
            api_dependency.insert(
                "path".into(),
                api.manifest_path
                    .parent()
                    .ok_or("API has no directory")?
                    .to_string_lossy()
                    .into_owned()
                    .into(),
            );
        }
    }
    manifest.push_str(&format!(
        "mod_api = {}\n",
        toml::Value::Table(api_dependency)
    ));
    manifest.push_str("serde_json = \"1\"\n");
    let mut source = String::from(
        "use mod_api::wire::Wire;\nfn main() { let mut libraries = std::collections::BTreeMap::new();\n",
    );
    for (index, (package, id, types)) in declarations.iter().enumerate() {
        let alias = format!("library_{index}");
        manifest.push_str(&format!(
            "{alias} = {{ package = {:?}, default-features = false, features = {:?}, path = {:?} }}\n",
            package.name,
            resolved_features(tool, &root, owner, package)?,
            package
                .manifest_path
                .parent()
                .ok_or("no mod directory")?
                .to_string_lossy()
        ));
        source.push_str("let mut types = std::collections::BTreeMap::new();\n");
        for ty in types {
            let (_, suffix) = ty.split_once("::").ok_or("type has no crate")?;
            source.push_str(&format!("types.insert({ty:?}, format!(\"{{:016x}}\", <{alias}::{suffix} as Wire>::layout(&mut Vec::new())));\n"));
        }
        source.push_str(&format!("libraries.insert({id:?}, types);\n"));
    }
    source.push_str("println!(\"{}\", serde_json::to_string(&libraries).unwrap()); }\n");
    save(&stage.join("Cargo.toml"), manifest.as_bytes())?;
    save(&stage.join("src/main.rs"), source.as_bytes())?;
    let lock = metadata.workspace_root.join("Cargo.lock");
    if lock.is_file() {
        save(
            &stage.join("Cargo.lock"),
            &fs::read(lock).map_err(|e| e.to_string())?,
        )?;
    }
    let rustc = output(tool("rustc").arg("-vV"))?;
    let rustc = String::from_utf8(rustc).map_err(|e| e.to_string())?;
    let host = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc did not report its host target")?;
    let bytes = output(
        tool("cargo")
            .args(["run", "--quiet", "--target", host, "--manifest-path"])
            .arg(stage.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(root.join("target/horizon-mod/probe-target"))
            .env("CARGO_INCREMENTAL", "0"),
    )?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("layout probe: {e}"))?;
    save(
        &destination,
        &serde_json::to_vec(&json).map_err(|e| e.to_string())?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolchain running the tests: CI has no launcher-pinned toolchain installed.
    fn build(root: &Path) -> Result<(), String> {
        let tool = |name: &str| match name {
            "cargo" => Command::new(env!("CARGO")),
            other => Command::new(other),
        };
        build_with(&tool, root)
    }

    /// The probe manifest carries each dependency's resolved features, and a refusal keeps the
    /// recorded layouts, without compiling the probe: `cargo run` is answered with fixed layouts,
    /// every other cargo command runs for real.
    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn the_probe_builds_dependencies_with_their_resolved_features() {
        use std::os::unix::fs::PermissionsExt;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/layout-features-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let library = root.join("library");
        fs::create_dir_all(library.join("src")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        let api = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../horizon-mod-api")
            .canonicalize()
            .unwrap();
        let stub = root.join("cargo-stub");
        fs::write(
            &stub,
            format!(
                "#!/bin/sh\nif [ \"$1\" = run ]; then echo '{{\"library\":{{\"library::Shared\":\"0000000000000001\"}}}}'; else exec {:?} \"$@\"; fi\n",
                env!("CARGO")
            ),
        )
        .unwrap();
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
        let tool = |name: &str| match name {
            "cargo" => Command::new(&stub),
            other => Command::new(other),
        };
        let header = |id: &str| {
            format!(
                "id = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n"
            )
        };
        fs::write(library.join("Cargo.toml"), format!("[package]\nname = \"library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nhorizon-mod-api = {{ path = {:?} }}\n[features]\ndefault = [\"wide\"]\nwide = []\nextra = []\n", api.to_string_lossy())).unwrap();
        fs::write(library.join("horizon-mod.toml"), header("library")).unwrap();
        fs::write(
            library.join("src/lib.rs"),
            "#[derive(mod_api::Wire)] pub struct Shared { pub value: u16 }\n",
        )
        .unwrap();
        fs::write(
            root.join("horizon-mod.toml"),
            format!("{}[dependencies]\nlibrary = \"^0.1\"\n", header("addon")),
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        let staged = |dependency: &str| {
            fs::write(root.join("Cargo.toml"), format!("[package]\nname = \"addon\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nhorizon-mod-api = {{ path = {:?} }}\nlibrary = {{ path = \"library\"{dependency} }}\n", api.to_string_lossy())).unwrap();
            build_with(&tool, &root).unwrap();
            let manifest =
                fs::read_to_string(root.join("target/horizon-mod/layout-probe/Cargo.toml"))
                    .unwrap();
            manifest
                .lines()
                .find(|line| line.starts_with("library_0 = "))
                .unwrap()
                .split(", path")
                .next()
                .unwrap()
                .to_owned()
        };
        assert_eq!(
            staged(""),
            r#"library_0 = { package = "library", default-features = false, features = ["default", "wide"]"#
        );
        assert_eq!(
            staged(", default-features = false"),
            r#"library_0 = { package = "library", default-features = false, features = []"#
        );
        assert_eq!(
            staged(", default-features = false, features = [\"extra\"]"),
            r#"library_0 = { package = "library", default-features = false, features = ["extra"]"#
        );
        let recorded = fs::read(root.join("layouts.json")).unwrap();
        assert_eq!(
            recorded,
            br#"{"library":{"library::Shared":"0000000000000001"}}"#
        );
        fs::write(
            library.join("src/lib.rs"),
            "#[cfg(unix)] #[derive(mod_api::Wire)] pub struct Shared;\n",
        )
        .unwrap();
        assert!(
            build_with(&tool, &root)
                .unwrap_err()
                .contains("library::Shared")
        );
        assert_eq!(
            recorded,
            fs::read(root.join("layouts.json")).unwrap(),
            "a refusal keeps the recorded layouts"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// The real probe end to end: compiles `mod_api` and the mod's dependencies, so it runs outside
    /// the fast checks.
    #[test]
    #[ignore = "compiles the real layout probe (minutes cold)"]
    fn field_changes_invalidate_the_cached_probe() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/layout-probe-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        let api = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../horizon-mod-api")
            .canonicalize()
            .unwrap();
        fs::write(root.join("Cargo.toml"), format!("[package]\nname = \"layout-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nhorizon-mod-api = {{ path = {:?} }}\n", api.to_string_lossy())).unwrap();
        fs::write(root.join("horizon-mod.toml"), "id = \"fixture\"\nname = \"Fixture\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n").unwrap();
        fs::write(root.join("src/lib.rs"), "#[derive(mod_api::Wire)] pub struct Record { pub field: u32 }\nmod private { #[derive(mod_api::Wire)] pub struct Secret; }\n").unwrap();
        let start = std::time::Instant::now();
        build(&root).unwrap();
        eprintln!("layout probe cold: {:?}", start.elapsed());
        let before = fs::read(root.join("layouts.json")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(value["fixture"].as_object().unwrap().len(), 1);
        let start = std::time::Instant::now();
        build(&root).unwrap();
        eprintln!("layout probe warm: {:?}", start.elapsed());
        assert_eq!(before, fs::read(root.join("layouts.json")).unwrap());
        fs::write(
            root.join("src/lib.rs"),
            "#[derive(mod_api::Wire)] pub struct Record { pub field: u64 }\n",
        )
        .unwrap();
        build(&root).unwrap();
        assert_ne!(before, fs::read(root.join("layouts.json")).unwrap());
        fs::write(root.join("src/lib.rs"), "pub struct Local;\n").unwrap();
        assert!(
            build(&root)
                .unwrap_err()
                .contains("lose all recorded types")
        );
        fs::remove_file(root.join("layouts.json")).unwrap();
        let library = root.join("library");
        fs::create_dir_all(library.join("src")).unwrap();
        fs::write(library.join("Cargo.toml"), format!("[package]\nname = \"library\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nhorizon-mod-api = {{ path = {:?} }}\n", api.to_string_lossy())).unwrap();
        fs::write(library.join("horizon-mod.toml"), "id = \"library\"\nname = \"Library\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n").unwrap();
        fs::write(
            library.join("src/lib.rs"),
            "#[derive(mod_api::Wire)] pub struct Shared { pub value: u16 }\n",
        )
        .unwrap();
        let mut cargo = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        cargo.push_str("library = { path = \"library\" }\n");
        fs::write(root.join("Cargo.toml"), cargo).unwrap();
        let mut manifest = fs::read_to_string(root.join("horizon-mod.toml")).unwrap();
        manifest.push_str("[dependencies]\nlibrary = \"^0.1\"\n");
        fs::write(root.join("horizon-mod.toml"), manifest).unwrap();
        build(&root).unwrap();
        let recorded: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join("layouts.json")).unwrap()).unwrap();
        assert!(recorded["library"]["library::Shared"].as_str().is_some());
        assert!(recorded.get("fixture").is_none());
        let lib_manifest = library.join("Cargo.toml");
        let mut text = fs::read_to_string(&lib_manifest).unwrap();
        text.push_str("\n[features]\ndefault = [\"wide\"]\nwide = []\n");
        fs::write(&lib_manifest, text).unwrap();
        fs::write(library.join("src/lib.rs"), "#[derive(mod_api::Wire)] pub struct Shared { pub value: u16, #[cfg(feature = \"wide\")] pub extra: u64 }").unwrap();
        build(&root).unwrap();
        let wide = fs::read(root.join("layouts.json")).unwrap();
        let manifest_path = root.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path).unwrap().replace(
            "path = \"library\"",
            "path = \"library\", default-features = false",
        );
        fs::write(&manifest_path, text).unwrap();
        build(&root).unwrap();
        assert_ne!(wide, fs::read(root.join("layouts.json")).unwrap());
        let text = fs::read_to_string(&manifest_path).unwrap().replace(
            "default-features = false",
            "default-features = false, features = [\"wide\"]",
        );
        fs::write(&manifest_path, text).unwrap();
        build(&root).unwrap();
        assert_eq!(wide, fs::read(root.join("layouts.json")).unwrap());
        let recorded = fs::read(root.join("layouts.json")).unwrap();
        fs::write(
            library.join("src/lib.rs"),
            "#[cfg(unix)] #[derive(mod_api::Wire)] pub struct Shared;",
        )
        .unwrap();
        assert!(build(&root).unwrap_err().contains("library::Shared"));
        assert_eq!(recorded, fs::read(root.join("layouts.json")).unwrap());
    }
}
