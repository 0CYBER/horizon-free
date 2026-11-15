//! Compiler-driven discovery uses an isolated, per-pin source and metadata cache.
use super::{api, dependencies, internals, rust_toolchain};
use crate::i18n::trf;
use horizon_mod_package::build_data;
use horizon_mod_package::internals::{self as catalog, ListedInternal};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::Stdio,
};

#[derive(Debug)]
pub(crate) enum Error {
    SourceUnavailable { revision: String, reason: String },
    Check(String),
}

impl From<String> for Error {
    fn from(reason: String) -> Self {
        Self::Check(reason)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceUnavailable { revision, reason } => f.write_str(&trf(
                "E_INTERNAL_SOURCE_UNAVAILABLE: internals sync needs the source for {revision}. It is not cached and could not be downloaded: {error}",
                &[("revision", revision), ("error", reason)],
            )),
            Self::Check(reason) => f.write_str(reason),
        }
    }
}
impl std::error::Error for Error {}

/// No normal build reads this directory or its target. In particular, neither the attributes
/// nor the warning-bearing metadata can reach the normal build caches.
pub(crate) fn check(folder: &Path) -> Result<Vec<ListedInternal>, Error> {
    let catalog = pinned_catalog_with_modules(folder)?;
    discover_catalog(folder, &catalog.directory)
}

fn discover_catalog(folder: &Path, game: &Path) -> Result<Vec<ListedInternal>, Error> {
    let bundle = super::bundle_in(folder)?.ok_or_else(|| {
        trf(
            "{path} is not a native mod project.",
            &[("path", &folder.display().to_string())],
        )
    })?;
    let projects = [api::Project::read(&bundle).map_err(|e| e.to_string())?];
    let declared = [dependencies::from_manifest(
        &bundle.manifest.dependencies,
        horizon_mod_package::manifest::Side::Both,
    )];
    let inputs = [dependencies::Mod {
        id: &bundle.manifest.id,
        version: &bundle.manifest.version,
        dependencies: &declared[0],
        has_api: true,
        code: true,
    }];
    let resolution = dependencies::resolve_links(&inputs).map_err(|e| e.to_string())?;
    let cache = game
        .parent()
        .ok_or_else(|| "invalid discovery cache".to_owned())?
        .to_path_buf();
    crate::storage::mark_used(&cache).map_err(super::cache_error)?;
    let stage = cache.join("stage");
    prepare_stage(game, &stage)?;
    let mut listed = internals::Catalog::read(&stage)?;
    listed.discovery = true;
    api::stage(&stage, &projects, &declared, &resolution, &listed)?;
    let rust = rust_toolchain::install(|_| {})?;
    let mod_dir = stage.join("native-api").join(&bundle.manifest.id);
    let mut cmd = super::cargo(&rust, &stage, "check");
    cmd.args(["--message-format=json", "-p", &projects[0].package, "--lib"])
        .env("CARGO_TARGET_DIR", cache.join("target"))
        .env(
            "RUSTC_WRAPPER",
            std::env::current_exe().map_err(|e| e.to_string())?,
        )
        .env("HORIZON_INTERNALS_CHECK_MOD", &mod_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut used = BTreeSet::new();
    let mut errors = String::new();
    for line in BufReader::new(child.stdout.take().expect("piped")).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if let Some(message) = parse(&line, &mod_dir, &stage) {
            match message {
                Message::Used(kind, path) => {
                    used.insert((kind, path));
                }
                Message::Failure(text) => errors.push_str(&text),
            }
        }
    }
    if !child.wait().map_err(|e| e.to_string())?.success() {
        return Err(trf(
            "Internals discovery failed; the manifest was not changed. {error}",
            &[("error", &errors)],
        )
        .into());
    }
    used.into_iter()
        .filter(|(kind, _)| kind != "mod")
        .map(|(kind, path)| {
            catalog::resolve(&listed.internals, &bundle.manifest.id, &path, Some(&kind))
                .cloned()
                .map_err(|error| Error::Check(error.to_string()))
        })
        .collect()
}

/// The catalog directory is protected from concurrent native builds until this value drops.
pub struct DiscoveryCatalog {
    pub directory: PathBuf,
    _lease: crate::native_cache::Cache,
}

impl DiscoveryCatalog {
    /// Copy both build-data files the discovery wrote while its lease is held; refuse a reused
    /// destination.
    pub fn snapshot(&self, destination: &Path) -> Result<PathBuf, String> {
        fs::create_dir(destination).map_err(|e| e.to_string())?;
        for name in [build_data::INTERNALS, build_data::POINTS] {
            fs::copy(self.directory.join(name), destination.join(name))
                .map_err(|e| e.to_string())?;
        }
        Ok(destination.to_path_buf())
    }
}

/// Discover the pinned game with only this mod's modules, using the launcher's managed tools.
/// Hold the returned catalog while reading its internals.json and points.json files.
pub fn pinned_catalog_with_modules(folder: &Path) -> Result<DiscoveryCatalog, String> {
    super::require_consent().map_err(|e| e.to_string())?;
    let lease = crate::native_cache::Cache::acquire(&crate::backend::build_dir())
        .map_err(super::cache_error)?;
    super::clear_log()?;
    let directory = catalog_for_mod(folder, &super::root().join("internals-sync"))
        .map_err(|e| e.to_string())?;
    Ok(DiscoveryCatalog {
        directory,
        _lease: lease,
    })
}

/// Return the pinned game's discovery tree and catalog pair with only this mod's modules
/// applied. Each module discovery starts from pristine pinned source, never a staged tree.
/// The launcher owns source downloads, Rust installation and horizon-mixins management.
fn catalog_for_mod(folder: &Path, cache_root: &Path) -> Result<PathBuf, Error> {
    catalog_for_mod_with(
        folder,
        cache_root,
        super::source,
        |bundle, prepared, input, fresh| {
            let rust = rust_toolchain::install(|_| {})?;
            let tool = super::tool(&rust, &bundle.native().mixins, &|_| {})?;
            super::run(
                super::apply(&rust, &tool, prepared, input, fresh),
                "horizon-mixins",
                |_| {},
            )
        },
    )
}

// Keep downloads and the external executable at the boundary so fixtures exercise the actual
// module-input preparation, source staging and catalog publication without installing tools.
fn catalog_for_mod_with(
    folder: &Path,
    cache_root: &Path,
    source: impl FnOnce(&str) -> Result<PathBuf, String>,
    apply: impl FnOnce(&super::Bundle, &Path, &Path, &Path) -> Result<(), String>,
) -> Result<PathBuf, Error> {
    let bundle = super::bundle_in(folder)?.ok_or_else(|| "not a native mod project".to_owned())?;
    let native = bundle.native();
    let cache = if bundle.needs_mixins() {
        cache_root
            .join(&native.horizon)
            .join("modules")
            .join(&bundle.manifest.id)
    } else {
        cache_root.join(&native.horizon)
    };
    let game = cache.join(if bundle.needs_mixins() {
        "module-game"
    } else {
        "game"
    });
    if !bundle.needs_mixins() && game.join(".discovery-ready").is_file() {
        return Ok(game);
    }
    let source = source(&native.horizon).map_err(|reason| Error::SourceUnavailable {
        revision: native.horizon.clone(),
        reason,
    })?;
    let fresh = cache.join("game.new");
    if fresh.exists() {
        fs::remove_dir_all(&fresh).map_err(|e| e.to_string())?;
    }
    if bundle.needs_mixins() {
        let input = cache.join("mixins.incoming");
        super::stage_mixins(std::slice::from_ref(&bundle), &input)?;
        let prepared = cache.join("mixins.source");
        super::prepare_mixins_source(&source, &prepared)?;
        apply(&bundle, &prepared, &input, &fresh)
            .map_err(|error| format!("{}: {error}", bundle.manifest.id))?;
    } else {
        super::sync(&source, &fresh)?;
    }
    horizon_platform::mod_build::api_gen::staging::stage(
        &fresh,
        &fresh.join("points.json"),
        &fresh.join(internals::INTERNALS),
        horizon_platform::mod_build::api_gen::staging::Mode::Discovery,
        &horizon_platform::mod_build::api_gen::staging::Build {
            commit: native.horizon.clone(),
        },
    )?;
    fs::write(fresh.join(".discovery-ready"), "").map_err(|e| e.to_string())?;
    if game.exists() {
        fs::remove_dir_all(&game).map_err(|e| e.to_string())?;
    }
    fs::rename(&fresh, &game).map_err(|e| e.to_string())?;
    Ok(game)
}

// The pinned game never changes in this cache. Keep Cargo.lock after the first check:
// horizon-world watches it, so restoring the pristine lock would rebuild the entire game
// every time. Reset only the workspace membership and the generated mod source copies.
// The first copy is published by a rename, so an interrupted copy is never taken as warm.
fn prepare_stage(game: &Path, stage: &Path) -> Result<(), String> {
    if !stage.is_dir() {
        let fresh = stage.with_extension("new");
        if fresh.exists() {
            fs::remove_dir_all(&fresh).map_err(|e| e.to_string())?;
        }
        super::sync(game, &fresh)?;
        fs::rename(&fresh, stage).map_err(|e| e.to_string())?;
    } else {
        fs::copy(game.join("Cargo.toml"), stage.join("Cargo.toml")).map_err(|e| e.to_string())?;
    }
    for relative in ["native-api", "native-internals", "crates/native-game"] {
        let path = stage.join(relative);
        if path.exists() {
            fs::remove_dir_all(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Entry point used only when Cargo launches this executable as the discovery rustc wrapper.
/// Force the lint for the selected mod, including macro expansions and local allow/deny
/// attributes, without enabling warnings in the game's own crates or other dependencies.
pub(crate) fn rustc_wrapper() -> Option<i32> {
    let selected = std::env::var_os("HORIZON_INTERNALS_CHECK_MOD")?;
    let mut args = std::env::args_os().skip(1);
    let compiler = args.next()?;
    if Some(&compiler) != std::env::var_os("RUSTC").as_ref() {
        return None;
    }
    let mut cmd = crate::process::command(compiler);
    cmd.args(args);
    if std::env::var_os("CARGO_MANIFEST_DIR").as_ref() == Some(&selected) {
        cmd.arg("--force-warn=deprecated");
    }
    Some(match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("rustc: {error}");
            1
        }
    })
}

#[derive(Deserialize)]
struct Diagnostic {
    level: String,
    message: String,
    code: Option<Code>,
    spans: Vec<Span>,
    rendered: Option<String>,
}
#[derive(Deserialize)]
struct Code {
    code: String,
}
#[derive(Deserialize)]
struct Span {
    file_name: PathBuf,
    #[serde(default)]
    expansion: Option<Box<Expansion>>,
}
#[derive(Deserialize)]
struct Expansion {
    span: Span,
}
#[derive(Deserialize)]
struct CargoMessage {
    reason: String,
    message: Option<Diagnostic>,
    manifest_path: Option<PathBuf>,
}

enum Message {
    Used(String, String),
    Failure(String),
}

// Rustc paths are workspace-relative (or absolute). Macro expansions can point at the
// dependency's macro body: follow the expansion back to the mod's call site, never a string prefix.
fn owned(span: &Span, mod_dir: &Path, stage: &Path) -> bool {
    let path = if span.file_name.is_absolute() {
        span.file_name.clone()
    } else {
        stage.join(&span.file_name)
    };
    path.starts_with(mod_dir)
        || span
            .expansion
            .as_ref()
            .is_some_and(|expansion| owned(&expansion.span, mod_dir, stage))
}

fn parse(line: &str, mod_dir: &Path, stage: &Path) -> Option<Message> {
    let event: CargoMessage = serde_json::from_str(line).ok()?;
    if event.reason != "compiler-message" {
        return None;
    }
    let diagnostic = event.message?;
    if diagnostic.level == "error" {
        return Some(Message::Failure(
            diagnostic.rendered.unwrap_or(diagnostic.message),
        ));
    }
    if event
        .manifest_path
        .as_deref()
        .is_none_or(|path| path.parent() != Some(mod_dir))
    {
        return None;
    }
    if diagnostic.code?.code != "deprecated"
        || !diagnostic
            .spans
            .iter()
            .any(|span| owned(span, mod_dir, stage))
    {
        return None;
    }
    let (_, note) = diagnostic.message.rsplit_once("horizon-internal:")?;
    let (kind, path) = note.split_once(':')?;
    Some(Message::Used(kind.into(), path.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovery_cache_keeps_the_resolved_lock_and_game_sources_but_replaces_mods() {
        let root = super::super::tests::dir("discovery-cache");
        let game = root.join("game");
        let stage = root.join("stage");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        fs::write(game.join("Cargo.lock"), "original").unwrap();
        fs::write(game.join("game.rs"), "struct Game;").unwrap();
        prepare_stage(&game, &stage).unwrap();
        let modified = fs::metadata(stage.join("game.rs"))
            .unwrap()
            .modified()
            .unwrap();
        fs::write(stage.join("Cargo.lock"), "resolved with mod dependencies").unwrap();
        fs::write(stage.join("Cargo.toml"), "with a generated mod").unwrap();
        fs::create_dir_all(stage.join("native-api/old/src")).unwrap();
        fs::write(stage.join("native-api/old/src/lib.rs"), "old mod").unwrap();
        prepare_stage(&game, &stage).unwrap();
        assert_eq!(
            fs::read(stage.join("Cargo.lock")).unwrap(),
            b"resolved with mod dependencies"
        );
        assert_eq!(
            fs::metadata(stage.join("game.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert_eq!(
            fs::read(stage.join("Cargo.toml")).unwrap(),
            fs::read(game.join("Cargo.toml")).unwrap()
        );
        assert!(!stage.join("native-api").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_cache_discards_an_interrupted_first_copy() {
        let root = super::super::tests::dir("discovery-interrupted");
        let game = root.join("game");
        let stage = root.join("stage");
        fs::create_dir_all(game.join("crates")).unwrap();
        fs::write(game.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        fs::write(game.join("internals.json"), "[]").unwrap();
        fs::write(game.join("crates/lib.rs"), "struct Game;").unwrap();
        // A copy cut off after its first file.
        let partial = root.join("stage.new");
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("Cargo.toml"), "[workspace]\nmembers=[]\n").unwrap();
        prepare_stage(&game, &stage).unwrap();
        assert_eq!(fs::read(stage.join("internals.json")).unwrap(), b"[]");
        assert!(stage.join("crates/lib.rs").is_file());
        assert!(!partial.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_filters_diagnostics_by_crate_span_and_lint_and_keeps_macro_calls() {
        let stage = Path::new("/cache/stage");
        let own = stage.join("native-api/example");
        let mut event = json!({"reason":"compiler-message", "manifest_path":own.join("Cargo.toml"), "message": {
            "level":"warning", "message":"use of deprecated method: horizon-internal:method:horizon_client::game::Thing::method",
            "code":{"code":"deprecated"}, "spans":[{"file_name":"native-api/example/src/lib.rs"}], "rendered":null
        }});
        assert!(
            matches!(parse(&event.to_string(), &own, stage), Some(Message::Used(kind, path)) if kind == "method" && path.ends_with("Thing::method"))
        );
        event["message"]["spans"][0] = json!({"file_name":"crates/dependency/src/lib.rs", "expansion":{"span":{"file_name":"native-api/example/src/lib.rs"}}});
        assert!(matches!(
            parse(&event.to_string(), &own, stage),
            Some(Message::Used(..))
        ));
        event["manifest_path"] = json!(stage.join("crates/dependency/Cargo.toml"));
        assert!(parse(&event.to_string(), &own, stage).is_none());
        event["manifest_path"] = json!(own.join("Cargo.toml"));
        event["message"]["spans"][0] = json!({"file_name":"native-api/example_other/src/lib.rs"});
        assert!(parse(&event.to_string(), &own, stage).is_none());
        event["message"]["spans"][0] = json!({"file_name":own.join("src/lib.rs")});
        event["message"]["code"]["code"] = json!("unused");
        assert!(parse(&event.to_string(), &own, stage).is_none());
        event["message"]["level"] = json!("error");
        assert!(matches!(
            parse(&event.to_string(), &own, stage),
            Some(Message::Failure(..))
        ));
    }
}

#[cfg(test)]
#[path = "internals_sync_tests.rs"]
mod catalog_tests;

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn catalog_snapshot_survives_a_second_discovery_between_reads() {
        let dir = std::env::temp_dir().join(format!("catalog-snapshot-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let shared = dir.join("shared");
        fs::create_dir_all(&shared).unwrap();
        for name in ["internals.json", "points.json"] {
            fs::write(shared.join(name), "first").unwrap();
        }
        let lease = crate::native_cache::Cache::acquire(&dir.join("cache")).unwrap();
        let catalog = DiscoveryCatalog {
            directory: shared.clone(),
            _lease: lease,
        };
        let snapshot = dir.join("snapshot");
        let snapshot = catalog.snapshot(&snapshot).unwrap();
        drop(catalog);
        assert_eq!(
            fs::read_to_string(snapshot.join("internals.json")).unwrap(),
            "first"
        );
        // The next discovery replaces both published files once the first lease ends.
        for name in ["internals.json", "points.json"] {
            fs::write(shared.join(name), "second").unwrap();
        }
        assert_eq!(
            fs::read_to_string(snapshot.join("points.json")).unwrap(),
            "first"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
