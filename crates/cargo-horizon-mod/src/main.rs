//! `cargo horizon-mod`: make Horizon content mods (`docs/modpacks.md`): blocks,
//! items, data and assets, no code. Code mods are native (docs/modding/src/how-to/native-mods.md).
//!
//! - `new <path> [--name "Name"]`: a content mod from the template; with
//!   `--template native|wasm [--horizon <build>]`, a code mod pinned to the current Nightly's
//!   SDK unless `--horizon` names another build (`nightly-<date>` or a sha), with that SDK fetched
//!   into `.horizon/sdk/` (`sdk_fetch.rs`).
//! - `update [--horizon <build>]`: repin this native mod and diff/refresh declared internals.
//! - `internals sync [--repo <path>]`: sync declarations for native or wasm mods.
//! - `internals add <path>… [--kind <kind>]`: declare internals from the prebuilt catalog.
//! - `build`: the `.hmod` in `target/horizon-mod/`, with the mod's `assets/`
//!   and `data/`; with `--wasm`, a Rust mod built to `mod.wasm` first (`wasm.rs`).
//! - `wasm-std <out.wasm>`: the standard library member the game ships for wasm mods.
//! - `run [--game <horizon-client>] [-- <game args>]`: build, then play with
//!   the mod.
//! - `check [--uses]` in a mod folder: `cargo check` for the host and wasm32, the package
//!   rules, and what the mod uses of its pinned build (`sdk_cmds.rs`).
//! - `check <file> [--json]`: what the registry checks of a submission
//!   (`.hmod`, `.hpack`, resource pack `.zip`, `.hshader`).
//! - `hooks <text> [--all]`: search the pinned build's hooks (or every item) offline.
//! - `sdk diff <build> <build>`: what changed in the surface between two builds.
//! - `fits [<file.hmod>] [--build <name>]…`: whether the mod fits each build; exit 0 when all
//!   do, 1 when one does not, 2 when it cannot be told.
//! - `registry build|review|check|list`: the registry repository's CI.
//! - `use-jar <path>`: list an exact original Java jar as a passive asset/data source.
use horizon_mod_package::{Content, Manifest, Package, manifest::MANIFEST};
mod bounded;
mod check;
mod internals;
mod jar;
mod layouts;
mod registry;
mod sdk_cmds;
mod sdk_fetch;
mod uses_table;
mod wasm;

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn is_compiler_wrapper(name: &str) -> bool {
    name.strip_suffix(".exe").unwrap_or(name).ends_with("rustc")
}

fn main() -> ExitCode {
    // The uses scan parses a mod's source in a child, this program started again: run its part
    // and exit.
    let entry = mod_surface::child::child_entry();
    // Cargo runs `cargo-horizon-mod horizon-mod <args>`.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if std::env::var_os("HORIZON_GROUP_WRAPPER").is_some()
        // `rustc`, or `rustc.exe` on Windows.
        && args
            .first()
            .is_some_and(|a| is_compiler_wrapper(a))
    {
        return match wasm::rustc(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    // Cargo runs this tool as the linker of a `build --wasm`.
    if let Ok(role) = std::env::var(wasm::LINK_ENV) {
        return match wasm::link(&role, args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().is_some_and(|a| a == "horizon-mod") {
        args.remove(0);
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new(&args[1..]).and_then(|code| match code {
            Some(dir) => fetched(&dir),
            None => Ok(()),
        }),
        Some("build") if args.get(1).is_some_and(|a| a == "--wasm") => fetched(Path::new("."))
            .and_then(|()| wasm::build(Path::new("."), &args[2..].windows(2).filter(|a| a[0] == "--with").map(|a| PathBuf::from(&a[1])).collect::<Vec<_>>()))
            .and_then(|()| build(entry)),
        Some("wasm-std") => wasm::wasm_std(&args[1..]),
        Some("build") => {
            let sdk = if code_project(Path::new(".")) {
                fetched(Path::new("."))
            } else {
                Ok(())
            };
            sdk.and_then(|()| build(entry))
        }
        Some("run") => run(&args[1..]).map_err(|e| e.to_string()),
        Some("check") if args.get(1).is_none_or(|a| a.starts_with("--")) => {
            sdk_cmds::check_mod(entry, &args[1..])
        }
        Some("check") => check(&args[1..], &mut io::stdout().lock(), &mut io::stderr().lock()),
        Some("hooks") => sdk_cmds::hooks(&args[1..]),
        Some("sdk") => sdk_cmds::sdk(&args[1..]),
        Some("fits") => return sdk_cmds::fits(entry, &args[1..]),
        Some("registry") => registry::main(&args[1..]),
        Some("use-jar") => jar::run(&args[1..]),
        Some("internals") => internals::run(&args[1..]),
        Some("update") => {
            sdk_cmds::update_report(entry, Path::new("."), flag(&args[1..], "--horizon"))
                .and_then(|()| {
                    update(Path::new("."), flag(&args[1..], "--horizon"))
                })
                .map(|()| println!("Run `cargo horizon-mod check`."))
        }
        _ => Err("usage: cargo horizon-mod new <path> [--name \"Name\"] [--template content|native|wasm [--horizon <build>]] | update [--horizon <build>] | internals sync [--repo <path>] | internals add <path>… [--kind <kind>] | build [--wasm] | wasm-std <out.wasm> | run [--game <horizon-client>] [-- <game args>] | check [--uses] | check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json] | hooks <text> [--all] | sdk diff <build> <build> | fits [<file.hmod>] [--build <name>]… | registry build|review|check|list | use-jar <path>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Whether `dir` is a code mod's project: one with a `Cargo.toml`, which depends on the SDK.
fn code_project(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file()
}

/// The SDK the project at `dir` pins, fetched into its `.horizon/sdk/` when missing or stale; a
/// project inside a game checkout builds against that tree instead.
fn fetched(dir: &Path) -> Result<(), String> {
    if let Some(tree) = sdk_fetch::game_tree(dir)? {
        println!("Building against the game checkout {}", tree.display());
        return Ok(());
    }
    let sdk = sdk_fetch::ensure_sdk(dir)?;
    println!(
        "SDK {} (build {}) in {}",
        sdk.name,
        sdk.build,
        sdk.dir.display()
    );
    Ok(())
}

/// The value after `flag` in `args`.
fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

/// Makes the mod `args` asks for; a code mod's folder is returned, for its SDK to be fetched.
fn new(args: &[String]) -> Result<Option<PathBuf>, String> {
    let path = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod new <path>")?;
    let dir = Path::new(path);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    let base = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("the path needs a folder name")?;
    let id: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let name = flag(args, "--name").map_or_else(|| base.replace(['-', '_'], " "), str::to_string);
    let fill = |t: &str| t.replace("{id}", &id).replace("{name}", &name);
    match flag(args, "--template") {
        None | Some("content") => {}
        Some(template @ ("native" | "wasm")) => {
            new_code(
                dir,
                &id,
                template == "native",
                flag(args, "--horizon"),
                fill,
            )?;
            return Ok(Some(dir.to_owned()));
        }
        Some(_) => return Err("--template is content, native or wasm".into()),
    }
    // A block copying a vanilla one, with its model, name, loot, recipe and tag.
    let files = [
        (
            MANIFEST,
            include_str!("../template/content/horizon-mod.toml"),
        ),
        (
            "data/{id}/horizon/block/glowing_amethyst.json",
            include_str!("../template/content/block.json"),
        ),
        (
            "assets/{id}/blockstates/glowing_amethyst.json",
            include_str!("../template/content/blockstate.json"),
        ),
        (
            "assets/{id}/models/block/glowing_amethyst.json",
            include_str!("../template/content/model.json"),
        ),
        (
            "assets/{id}/items/glowing_amethyst.json",
            include_str!("../template/content/item.json"),
        ),
        (
            "assets/{id}/lang/en_us.json",
            include_str!("../template/content/lang.json"),
        ),
        (
            "data/{id}/loot_table/blocks/glowing_amethyst.json",
            include_str!("../template/content/loot.json"),
        ),
        (
            "data/{id}/recipe/glowing_amethyst.json",
            include_str!("../template/content/recipe.json"),
        ),
        (
            "data/minecraft/tags/block/mineable/pickaxe.json",
            include_str!("../template/content/pickaxe.json"),
        ),
        (".gitignore", include_str!("../template/gitignore")),
    ];
    for (file, text) in files {
        let file = fill(file);
        let file = file.as_str();
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let manifest =
        Manifest::parse(&std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string())?)?;
    Content::read(dir, &manifest)?;
    println!(
        "Created {} (mod id {id}). Next: cd {path} && cargo horizon-mod build, then copy \
         target/horizon-mod/{id}.hmod into your game folder's horizon-mods/ \
         (docs/modding/src/start/first-content-mod.md)",
        dir.display()
    );
    Ok(None)
}

/// The build a code mod pins: `--horizon`'s (a sha or a Nightly name), or the current Nightly's,
/// the build players have.
fn pin(horizon: Option<&str>) -> Result<String, String> {
    sdk_fetch::resolve(
        &sdk_fetch::Base::from_env(),
        horizon.unwrap_or(horizon_release_sign::NIGHTLY),
    )
}

/// Repins the native mod, reporting signature changes before writing. Its `Cargo.toml` names the
/// SDK by path, so only the pin moves; the next command fetches the new SDK.
fn update(dir: &Path, horizon: Option<&str>) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest = Manifest::parse(&text)?;
    let Some(native) = manifest.native.as_ref() else {
        // A wasm mod pins `[package.metadata.horizon] sdk`; `repin` moves it and drops the old SDK.
        let horizon = pin(horizon)?;
        sdk_fetch::repin(dir, &horizon)?;
        println!("Pinned to Horizon {horizon}");
        return Ok(());
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let horizon = pin(horizon)?;
    if !manifest.internals.is_empty() {
        // arismoko/horizon#2593: these become the pinned catalogs with the mod's own modules applied.
        let old = internals::load(dir, &native.horizon)?;
        let new = internals::load(dir, &horizon)?;
        print!("{}", internals::refresh(&manifest, &mut doc, &old, &new)?);
    }
    internals::set_string(
        doc["native"]["horizon"].as_value_mut().expect("native pin"),
        &horizon,
    );
    let candidate = doc.to_string();
    internals::atomic_write_manifest(&path, candidate)?;
    // The old pin's SDK goes; the next command fetches this one.
    sdk_fetch::repin(dir, &horizon)?;
    println!("Pinned to Horizon {horizon}");
    Ok(())
}

/// A flat code mod project, native or wasm, pinned to the build it is made for. Its `Cargo.toml`
/// names the SDK by path (`.horizon/sdk/api/crates/horizon-mod-api`); a native mod pins its
/// build in `[native] horizon`, a wasm mod in `[package.metadata.horizon] sdk`.
fn new_code(
    dir: &Path,
    id: &str,
    native: bool,
    horizon: Option<&str>,
    fill: impl Fn(&str) -> String,
) -> Result<(), String> {
    if id.len() > 64 {
        return Err(format!("mod id {id}: 64 characters at most"));
    }
    let horizon = pin(horizon)?;
    let horizon = horizon.as_str();
    let (manifest, metadata) = if native {
        (
            format!(
                "{}\n[native]\ngame = \"26.3\"\nhorizon = \"{horizon}\"\nmixins = \"{}\"\n",
                include_str!("../template/native/horizon-mod.toml"),
                horizon_mod_package::mixins_tool::REV,
            ),
            String::new(),
        )
    } else {
        (
            include_str!("../template/wasm/horizon-mod.toml").to_owned(),
            format!(
                "\n# The SDK build this mod is made for: `cargo horizon-mod update` moves it.\n[package.metadata.horizon]\nsdk = \"{horizon}\"\n"
            ),
        )
    };
    let cargo = include_str!("../template/native/api.toml").replace("{metadata}", &metadata);
    let files = [
        ("Cargo.toml", cargo.as_str()),
        ("src/lib.rs", include_str!("../template/native/lib.rs")),
        (MANIFEST, manifest.as_str()),
        (".gitignore", include_str!("../template/gitignore")),
    ];
    for (file, text) in files {
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    for folder in ["assets", "data"] {
        std::fs::create_dir_all(dir.join(folder)).map_err(|e| e.to_string())?;
    }
    let next = if native {
        format!(
            "horizon-launcher native-build {} --instance <instance> --trust-native-code",
            dir.display()
        )
    } else {
        format!("cd {} && cargo horizon-mod build --wasm", dir.display())
    };
    println!(
        "Created {} ({} mod id {id}). Your editor resolves horizon-mod-api once the SDK is in .horizon/sdk/: \
         any cargo horizon-mod command fetches it in a fresh clone. Next: {next}",
        dir.display(),
        if native { "native" } else { "wasm" },
    );
    Ok(())
}

/// Packs the mod in the current folder into `target/horizon-mod/<id>.hmod`: the manifest and
/// the files in `assets/` and `data/`, and for a code mod its source and its uses table, scanned
/// against the pinned build (none for a mod made for exactly one build, or built against the game
/// checkout it lies in, which has no published surface to scan against).
fn build(entry: mod_surface::child::ChildEntry) -> Result<(), String> {
    let root = Path::new(".");
    let package = Package::read(root)?;
    let code = package.manifest.native.is_some() || package.manifest.wasm.is_some();
    let uses = if code && sdk_fetch::game_tree(root)?.is_none() {
        let sdk = sdk_fetch::ensure_sdk(root)?;
        match uses_table::scan(entry, root, &uses_table::surface_of(&sdk)?)? {
            uses_table::Scanned::Uses(uses) => Some(uses.table(sdk.build).to_json().into_bytes()),
            uses_table::Scanned::Exact(_) => None,
            uses_table::Scanned::Unread(why) => {
                println!(
                    "note: no uses table packed ({why}); the package counts as made for build {} only.",
                    sdk.name
                );
                None
            }
        }
    } else {
        None
    };
    let path = pack_into(root, &root.join("target/horizon-mod"), uses.as_deref())?;
    println!("Built {}", path.display());
    if package.manifest.native.is_some()
        && let Some(hint) = uses_table::package_hint(&path)
    {
        eprintln!("{hint}");
    }
    Ok(())
}

fn build_in(root: &Path) -> Result<PathBuf, String> {
    pack_into(root, &root.join("target/horizon-mod"), None)
}

fn pack_into(root: &Path, dir: &Path, uses: Option<&[u8]>) -> Result<PathBuf, String> {
    let package = Package::read(root)?;
    if package.manifest.native.is_some() || package.manifest.wasm.is_some() {
        layouts::build(root)?;
    }
    let bytes = Package::pack_directory_with(root, uses)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.hmod", package.manifest.id));
    // Write then rename, so a running game never reads half a file.
    let partial = path.with_extension("hmod.partial");
    std::fs::write(&partial, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `check <file>`: the registry's checks of a submission, printed; fails
/// when one does.
fn check(
    args: &[String],
    stdout: &mut dyn io::Write,
    stderr: &mut dyn io::Write,
) -> Result<(), String> {
    let file = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod check <file>")?;
    let path = Path::new(file);
    let kind = match flag(args, "--kind") {
        Some(k) => horizon_mod_package::index::Kind::from(k.to_string()),
        None => check::kind_of(path)
            .ok_or("pass --kind mod|modpack|resourcepack: the extension doesn't say")?,
    };
    let signature = flag(args, "--signature")
        .map(|p| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")))
        .transpose()?;
    let report = check::check(path, kind, flag(args, "--key"), signature.as_deref());
    if args.iter().any(|a| a == "--json") {
        let json = match &report {
            Ok(r) => serde_json::json!({"ok": true, "report": r.json()}),
            Err(e) => serde_json::json!({"ok": false, "error": e}),
        };
        writeln!(stdout, "{json}").map_err(|e| e.to_string())?;
        return report.map(|_| ());
    }
    let report = report?;
    write!(stdout, "{}", report.text()).map_err(|e| e.to_string())?;
    if report.kind == horizon_mod_package::index::Kind::NativeMod
        && let Some(hint) = uses_table::package_hint(path)
    {
        writeln!(stderr, "{hint}").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug)]
enum RunError {
    CodeMod,
    Build(String),
    Launch { game: String, error: std::io::Error },
    Exit(std::process::ExitStatus),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CodeMod => f.write_str(
                "code mods run through the launcher's native build: horizon-launcher native-build . --trust-native-code (docs/modding/src/how-to/native-mods.md)",
            ),
            Self::Build(error) => f.write_str(error),
            Self::Launch { game, error } => write!(
                f,
                "{game}: {error} (pass --game <path to horizon-client> or set HORIZON_CLIENT)"
            ),
            Self::Exit(status) => write!(f, "the game exited with {status}"),
        }
    }
}

impl std::error::Error for RunError {}

fn run(args: &[String]) -> Result<(), RunError> {
    if Path::new("Cargo.toml").is_file() {
        return Err(RunError::CodeMod);
    }
    // `run` packs content mods only (a code mod goes through the launcher), which carry no table.
    let path = build_in(Path::new(".")).map_err(RunError::Build)?;
    let game = flag(args, "--game")
        .map(str::to_string)
        .or_else(|| std::env::var("HORIZON_CLIENT").ok())
        .unwrap_or_else(|| "horizon-client".into());
    let extra = args.iter().skip_while(|a| *a != "--").skip(1);
    println!("Starting {game} with {}", path.display());
    let status = Command::new(&game)
        .arg("--dev-mod")
        .arg(&path)
        .args(extra)
        .status()
        .map_err(|error| RunError::Launch { game, error })?;
    status.success().then_some(()).ok_or(RunError::Exit(status))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_package_check_keeps_the_json_output_contract() {
        use serde_json::{Value, json};
        use sha2::{Digest, Sha256};
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("greeter.hmod");
        let pin = "0123456789abcdef0123456789abcdef01234567";
        let manifest = format!(
            "id = 'greeter'\nname = 'Greeter'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n\
             [native]\ngame = '26.3'\nhorizon = '{pin}'\nmixins = '{pin}'\n"
        );
        let cargo = b"[package]\nname = 'greeter'\nversion = '0.1.0'\n\
                      [dependencies]\nhorizon-mod-api = { path = '.horizon/sdk/api/crates/horizon-mod-api' }\n";
        let bytes =
            Package::write_files(&manifest, &[("Cargo.toml", cargo), ("src/lib.rs", b"")]).unwrap();
        fs::write(&path, &bytes).unwrap();

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let args = [path.to_str().unwrap().to_owned(), "--json".into()];
        check(&args, &mut stdout, &mut stderr).unwrap();

        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap(),
            json!({
                "ok": true,
                "report": {
                    "kind": "native-mod",
                    "id": "greeter",
                    "version": "0.1.0",
                    "api": format!("native-1/{pin}/{pin}"),
                    "side": null,
                    "capabilities": [],
                    "game": "26.3",
                    "requires": [],
                    "sha256": format!("{:x}", Sha256::digest(&bytes)),
                    "size": bytes.len(),
                    "signed": false,
                    "wasm": false
                }
            })
        );
        assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
    }

    #[test]
    fn the_content_template_builds_and_checks_with_no_code() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-content-{}", std::process::id()));
        let path = dir.join("gems");
        let new_args = [
            path.display().to_string(),
            "--template".into(),
            "content".into(),
        ];
        new(&new_args).unwrap();
        assert!(!path.join("Cargo.toml").exists());
        let hmod = build_in(&path).unwrap();
        assert_eq!(hmod, path.join("target/horizon-mod/gems.hmod"));
        let package = Package::read(&hmod).unwrap();
        let content = Content::read(&hmod, &package.manifest).unwrap();
        assert!(
            content
                .files
                .contains(&"data/gems/horizon/block/glowing_amethyst.json".into())
        );
        assert_eq!(content.files.len(), 8, "{:?}", content.files);
        let report = check::check(&hmod, check::kind_of(&hmod).unwrap(), None, None).unwrap();
        assert!(
            report.text().contains("no code: content only"),
            "{}",
            report.text()
        );
        let executable = path.join("assets/gems/mod.wasm");
        std::fs::write(&executable, b"\0asm").unwrap();
        assert!(
            build_in(&path)
                .unwrap_err()
                .contains("unsupported executable file")
        );
        std::fs::remove_file(executable).unwrap();
        // Code is native now.
        std::fs::write(path.join("Cargo.toml"), "").unwrap();
        assert!(build_in(&path).unwrap_err().contains("native"));
        assert!(
            new(&[
                dir.join("x").display().to_string(),
                "--template".into(),
                "lua".into()
            ])
            .is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn native_build_and_check_share_the_content_executable_boundary() {
        use horizon_mod_package::manifest::ContentFileError;
        use std::io::Write;

        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("native-content-contract-{}", std::process::id()));
        let path = dir.join("mixed");
        let _ = std::fs::remove_dir_all(&dir);
        new(&[
            path.display().to_string(),
            "--template".into(),
            "native".into(),
            "--horizon".into(),
            "0123456789abcdef0123456789abcdef01234567".into(),
        ])
        .unwrap();
        let asset = "assets/mixed/program.bin";
        std::fs::create_dir_all(path.join("assets/mixed")).unwrap();
        std::fs::write(path.join(asset), b"ordinary asset").unwrap();
        // Only selected content ships, not the Git project's executable hooks.
        std::fs::create_dir_all(path.join(".git/hooks")).unwrap();
        std::fs::write(path.join(".git/hooks/pre-commit"), b"#!/bin/sh\nexit 0\n").unwrap();
        let hmod = build_in(&path).unwrap();
        let kind = check::kind_of(&hmod).unwrap();
        assert!(check::check(&hmod, kind.clone(), None, None).is_ok());

        // Model an externally supplied archive with executable bytes under an asset name.
        let original = std::fs::read(&hmod).unwrap();
        let mut input = zip::ZipArchive::new(std::io::Cursor::new(original)).unwrap();
        let mut output = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        for i in 0..input.len() {
            let mut entry = input.by_index(i).unwrap();
            let name = entry.name().unwrap().into_owned();
            output.start_file(&name, options).unwrap();
            if name == asset {
                output.write_all(b"MZ-not-an-asset").unwrap();
            } else {
                std::io::copy(&mut entry, &mut output).unwrap();
            }
        }
        std::fs::write(&hmod, output.finish().unwrap().into_inner()).unwrap();
        let expected = ContentFileError::Executable(asset.into()).to_string();
        assert_eq!(
            check::check(&hmod, kind, None, None).err().unwrap(),
            expected
        );
        std::fs::remove_file(&hmod).unwrap();
        std::fs::write(path.join(asset), b"MZ-not-an-asset").unwrap();
        assert_eq!(build_in(&path).unwrap_err(), expected);
        assert!(!hmod.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_native_template_packs_into_an_hmod_the_registry_accepts() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-native-{}", std::process::id()));
        let path = dir.join("machine-mod");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let args = [
            path.display().to_string(),
            "--template".into(),
            "native".into(),
            "--horizon".into(),
            commit.into(),
        ];
        assert_eq!(new(&args).unwrap(), Some(path.clone()));
        let api = std::fs::read_to_string(path.join("Cargo.toml")).unwrap();
        // The SDK by path, never git: the build is the manifest's pin.
        assert!(
            api.contains(
                r#"horizon-mod-api = { path = ".horizon/sdk/api/crates/horizon-mod-api" }"#
            ),
            "{api}"
        );
        assert!(
            !api.contains("git") && !api.contains("[package.metadata.horizon]"),
            "{api}"
        );
        assert_eq!(sdk_fetch::pin(&path).unwrap(), commit);
        let hmod = build_in(&path).unwrap();
        let bundle = horizon_mod_package::index::native::Bundle::read(&hmod).unwrap();
        assert_eq!(bundle.manifest.id, "machine_mod");
        let names: Vec<_> = bundle.files.keys().map(String::as_str).collect();
        assert_eq!(names, ["Cargo.toml", "horizon-mod.toml", "src/lib.rs"]);
        assert!(path.join("horizon-mod.toml").is_file());
        for old in [
            "api",
            "mixins",
            "content",
            "native",
            "rust",
            "horizon-native.json",
            "horizon-mixin.toml",
        ] {
            assert!(!path.join(old).exists());
        }
        assert_eq!(
            check::kind_of(&hmod),
            Some(horizon_mod_package::index::Kind::NativeMod)
        );
        assert_eq!(check::kind_of(&dir.join("machine_mod.hnmod")), None);
        let report = check::check(&hmod, check::kind_of(&hmod).unwrap(), None, None).unwrap();
        assert!(report.text().contains("machine_mod"), "{}", report.text());
        assert!(
            new(&[
                dir.join("bad").display().to_string(),
                "--template".into(),
                "native".into(),
                "--horizon".into(),
                "main".into(),
            ])
            .unwrap_err()
            .contains("is not a build")
        );
        let newer = "89abcdef0123456789abcdef0123456789abcdef";
        update(&path, Some(newer)).unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(path.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest.native.unwrap().horizon, newer);
        assert_eq!(
            std::fs::read_to_string(path.join("Cargo.toml")).unwrap(),
            api
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_wasm_template_pins_its_build_in_the_cargo_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter");
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let args = [
            path.display().to_string(),
            "--template".into(),
            "wasm".into(),
            "--horizon".into(),
            commit.into(),
        ];
        assert_eq!(new(&args).unwrap(), Some(path.clone()));
        let manifest =
            Manifest::parse(&std::fs::read_to_string(path.join(MANIFEST)).unwrap()).unwrap();
        assert!(manifest.native.is_none() && manifest.wasm.is_none());
        assert_eq!(sdk_fetch::pin(&path).unwrap(), commit);
        let api = std::fs::read_to_string(path.join("Cargo.toml")).unwrap();
        assert!(
            api.contains(".horizon/sdk/api/crates/horizon-mod-api"),
            "{api}"
        );
        // The SDK the editor reads is never part of the mod or its repository.
        let ignored = std::fs::read_to_string(path.join(".gitignore")).unwrap();
        assert!(ignored.lines().any(|line| line == "/.horizon"), "{ignored}");
    }
}

#[cfg(test)]
mod wrapper_suffix_tests {
    #[test]
    fn compiler_names_keep_suffix_contract() {
        for (name, expected) in [
            ("diagnostic-rustc", true),
            ("xrustc", true),
            ("rustc", true),
            ("/path/to/rustc", true),
            ("rustc.exe", true),
            ("diagnostic-rustc.exe", true),
            ("rustc.custom", false),
            ("rustc-wrapper", false),
            ("cargo-horizon-mod", false),
            ("/rustc/tool", false),
        ] {
            assert_eq!(super::is_compiler_wrapper(name), expected, "{name}");
        }
    }
}
