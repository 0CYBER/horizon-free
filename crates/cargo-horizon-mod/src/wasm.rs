//! `build --wasm`: the mod in the current folder built to `mod.wasm`, and `wasm-std`: the game's
//! standard library member.
//!
//! A wasm mod is a member of its dependency group, linked at load by the game
//! (`horizon-mod-wasm`): one memory, one copy of std and of each library mod. So every member is
//! a position-independent shared module built by one pinned toolchain, with std built from
//! source (`-Zbuild-std`) so its symbols are the same in every member. This tool is also the
//! link step (cargo calls it as the linker, with `HORIZON_MOD_LINK` set): the std member links
//! std whole and exports it; a mod leaves std and its library mods out, to resolve to theirs.
use horizon_mod_package::manifest::{MANIFEST, WASM, sha512_of};
use serde_json::Value;
mod group;
pub use group::rustc;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The one toolchain every wasm mod and the game's std member are built with: the pin CI and the
/// launcher's native-mod toolchain use too (`tools/ci/rust-version`).
pub const TOOLCHAIN: &str = include_str!("../../../tools/ci/rust-version").trim_ascii();
const TARGET: &str = "wasm32-unknown-unknown";
/// Set when cargo runs this tool as the linker: `std` or `member`.
pub const LINK_ENV: &str = "HORIZON_MOD_LINK";
const LLD_ENV: &str = "HORIZON_MOD_LLD";
/// The library mods' crate names, comma-separated, that a member leaves out.
const PROVIDED_ENV: &str = "HORIZON_MOD_PROVIDED";
const PROFILE: &str = "horizon-wasm";
const FLAGS: &str = "-Crelocation-model=pic -Zshare-generics=n -Zdefault-visibility=hidden --cfg=horizon_shared_artifacts_v2";

/// Guest boundary exports, kept even though they originate in an rlib dependency.
const GUEST_EXPORTS: &[&str] = &["horizon_layout", "horizon_in", "horizon_hook"];

/// The pinned toolchain's version line, its sysroot's linker, installing it first if needed.
struct Toolchain {
    version: String,
    lld: PathBuf,
}

/// `tool` (`cargo`, `rustc`) of the pinned toolchain: through rustup when there is one, else
/// the one on `PATH`, which must then be that toolchain.
pub(crate) fn pinned(tool: &str) -> Command {
    let mut command = if rustup_found() {
        let mut c = Command::new("rustup");
        c.args(["run", TOOLCHAIN, tool]);
        c
    } else {
        let home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/opt/rustup"));
        let directory = home.join("toolchains");
        let installed = std::fs::read_dir(directory).ok().and_then(|entries| {
            entries.filter_map(Result::ok).find_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                let binary = entry.path().join("bin").join(tool);
                (name.starts_with(&format!("{TOOLCHAIN}-")) && binary.is_file()).then_some(binary)
            })
        });
        Command::new(installed.unwrap_or_else(|| PathBuf::from(tool)))
    };
    command.env_remove("RUSTC").env_remove("RUSTC_WRAPPER");
    command
}

fn rustup_found() -> bool {
    Command::new("rustup").arg("--version").output().is_ok()
}

pub(crate) fn output(mut command: Command) -> Result<String, String> {
    let out = command.output().map_err(|e| format!("{command:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn toolchain() -> Result<Toolchain, String> {
    let version = || {
        output({
            let mut c = pinned("rustc");
            c.arg("-vV");
            c
        })
    };
    let info = match version() {
        Ok(info) => info,
        Err(_) if rustup_found() => {
            println!("Installing Rust {TOOLCHAIN} with rust-src for {TARGET}…");
            let status = Command::new("rustup")
                .args([
                    "toolchain",
                    "install",
                    TOOLCHAIN,
                    "--profile",
                    "minimal",
                    "--component",
                    "rust-src",
                    "--target",
                    TARGET,
                ])
                .status()
                .map_err(|e| format!("rustup: {e}"))?;
            if !status.success() {
                return Err(format!("rustup couldn't install Rust {TOOLCHAIN}"));
            }
            version()?
        }
        Err(e) => return Err(e),
    };
    let field = |name: &str| {
        info.lines()
            .find_map(|l| l.strip_prefix(name))
            .map(str::trim)
            .ok_or(format!("rustc -vV: no {name}"))
    };
    if field("release:")? != TOOLCHAIN {
        return Err(format!(
            "wasm mods build with Rust {TOOLCHAIN}, found {}: install rustup, or put {TOOLCHAIN} first on PATH",
            field("release:")?
        ));
    }
    let host = field("host:")?;
    let sysroot = output({
        let mut c = pinned("rustc");
        c.args(["--print", "sysroot"]);
        c
    })?;
    Ok(Toolchain {
        version: format!("{} {}", field("release:")?, field("commit-hash:")?),
        lld: Path::new(sysroot.trim()).join(format!("lib/rustlib/{host}/bin/rust-lld")),
    })
}

/// The shared compiler policy for both mod members and the game's standard library.
fn cargo_command(
    action: &str,
    role: &str,
    root: &Path,
    target_dir: &Path,
    toolchain: &Toolchain,
) -> Result<Command, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = pinned("cargo");
    command
        .args([
            action,
            "--lib",
            "--target",
            TARGET,
            "--profile",
            PROFILE,
            "-Zbuild-std=std,panic_abort",
        ])
        .args([
            "--config",
            &format!("profile.{PROFILE}.inherits=\"release\""),
        ])
        .args(["--config", &format!("profile.{PROFILE}.panic=\"abort\"")])
        .args(["--config", &format!("profile.{PROFILE}.codegen-units=1")])
        .args([
            "--config",
            &format!("profile.{PROFILE}.build-override.codegen-units=1"),
        ])
        .current_dir(root)
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTC_WRAPPER", &exe)
        .env("HORIZON_GROUP_WRAPPER", "{}")
        .env("HORIZON_STD_ARTIFACTS", target_dir.join("std-artifacts"))
        .env("HORIZON_BUILD_ROOT", target_dir)
        .env("RUSTC_BOOTSTRAP", "1")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env("RUSTFLAGS", FLAGS)
        .env("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER", &exe)
        .env(LINK_ENV, role)
        .env(LLD_ENV, &toolchain.lld);
    Ok(command)
}

fn lib_name(package: &Value) -> Option<String> {
    package["targets"].as_array()?.iter().find_map(|t| {
        let lib = t["kind"]
            .as_array()?
            .iter()
            .any(|k| k == "lib" || k == "cdylib" || k == "rlib");
        lib.then(|| t["name"].as_str().map(|n| n.replace('-', "_")))?
    })
}

/// The game's std member, built into `target_dir`.
fn std_member(target_dir: &Path, toolchain: &Toolchain) -> Result<Vec<u8>, String> {
    let dir = target_dir.join("horizon-wasm-std");
    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"horizon-wasm-std\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join("src/lib.rs"),
        "// The group's one std: linked whole and exported. The array is the group's shared state: word 0\n// points at its handler table, word 1 counts its cache budget, word 2 guards eviction, words 3.. are eviction callbacks\n// (`horizon_mod_api::wire::guest`).\n#[unsafe(no_mangle)]\npub static horizon_group_state: [core::sync::atomic::AtomicUsize; 32] =\n    [const { core::sync::atomic::AtomicUsize::new(0) }; 32];\n",
    )
    .map_err(|e| e.to_string())?;
    let mut command = cargo_command("rustc", "std", &dir, target_dir, toolchain)?;
    command.args([
        "--crate-type",
        "cdylib",
        "--",
        "-Zdefault-visibility=interposable",
    ]);
    if !command.status().map_err(|e| e.to_string())?.success() {
        return Err("Cargo std build failed".into());
    }
    std::fs::read(
        target_dir
            .join(TARGET)
            .join(PROFILE)
            .join("horizon_wasm_std.wasm"),
    )
    .map_err(|e| e.to_string())
}

/// `wasm-std <out>`: the std member the game ships beside its executable.
pub fn wasm_std(args: &[String]) -> Result<(), String> {
    let out = args
        .first()
        .ok_or("cargo horizon-mod wasm-std <out.wasm>")?;
    let toolchain = toolchain()?;
    let target = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("target/horizon-mod");
    std::fs::write(out, std_member(&target, &toolchain)?).map_err(|e| format!("{out}: {e}"))?;
    if let Some(input) = super::flag(&args[1..], "--catalog") {
        let path = Path::new(out)
            .parent()
            .unwrap_or(Path::new("."))
            .join("points.json");
        let bytes = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

fn registrations(
    manifest: &toml_edit::DocumentMut,
    catalog: Option<&Path>,
) -> Result<(String, String), String> {
    let mut parsed = manifest.clone();
    if !parsed.contains_key("wasm") {
        let mut wasm = toml_edit::Table::new();
        wasm["sha512"] = toml_edit::value("0".repeat(128));
        wasm["toolchain"] = toml_edit::value("pending");
        parsed["wasm"] = toml_edit::Item::Table(wasm);
    }
    let parsed = horizon_mod_package::Manifest::parse(&parsed.to_string())?;
    let (registrations, point_names) = if parsed.inject.is_empty() {
        (String::new(), "[]".to_owned())
    } else {
        let std_member = std::env::var_os("HORIZON_WASM_STD")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .map(|p| p.with_file_name("horizon-wasm-std.wasm"))
            });
        let default_catalog = std_member.and_then(|p| p.parent().map(|p| p.join("points.json")));
        let path = catalog.map(Path::to_path_buf).or(default_catalog).ok_or(
            "[[inject]] requires --catalog <points.json> or HORIZON_WASM_STD beside points.json",
        )?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e}; supply --catalog <points.json>", path.display()))?;
        let (_, json): (_, Value) =
            horizon_mod_package::build_data::read(horizon_mod_package::build_data::POINTS, &text)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        let points: Vec<horizon_mod_point_emit::Point> =
            serde_json::from_value(json["points"].clone())
                .map_err(|e| format!("{}: {e}", path.display()))?;
        let expressions = horizon_mod_point_emit::api(&parsed, &points, "crate::")
            .map_err(|e| format!("{}:{e}", parsed.id))?;
        let names = horizon_mod_point_emit::places(&parsed, &points)
            .map_err(|e| format!("{}:{e}", parsed.id))?;
        let registrations = expressions
            .iter()
            .map(|e| {
                format!("mod_points::register(&{e}).expect(\"validated guest API listener\");\n")
            })
            .collect();
        (
            registrations,
            serde_json::to_string(&names).map_err(|e| e.to_string())?,
        )
    };
    Ok((registrations, point_names))
}

pub fn build(root: &Path, catalog: Option<&Path>, additional: &[PathBuf]) -> Result<(), String> {
    group::build(root, catalog, additional)
}

fn member_export(name: &str) -> bool {
    !matches!(name, "__horizon_mod_hooks" | "__dso_handle" | "__tls_base")
        && !rustc_demangle::try_demangle(name)
            .is_ok_and(|symbol| format!("{symbol:#}").starts_with("__rustc::"))
}

fn group_export(name: &str, krate: &str) -> bool {
    if GUEST_EXPORTS.contains(&name)
        || name == "horizon_layout_ext"
        || name == "horizon_mod_points_register"
        || name == "horizon_mod_points_seal"
        || name.starts_with("__wasm_")
    {
        return true;
    }
    let Ok(symbol) = rustc_demangle::try_demangle(name) else {
        return false;
    };
    let path = format!("{symbol:#}");
    path.starts_with(&format!("{krate}::"))
        || (path.starts_with('<')
            && path
                .split(">::")
                .next()
                .is_some_and(|receiver| receiver.contains(&format!("{krate}::"))))
}

/// One argument of the response file rustc hands the linker, one per line. On Windows rustc
/// quotes each line the Windows way (`"D:\dir\a b.rlib"`, a quote inside as `\"`, backslashes
/// doubled only in front of a quote); elsewhere
/// it escapes `\` and space with a backslash.
fn response_file_arg(line: &str) -> String {
    let mut arg = String::new();
    if let Some(quoted) = line
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        // A run of backslashes is literal unless a quote follows it: then each pair is one
        // backslash and an odd one left over escapes the quote.
        let mut backslashes = 0;
        for c in quoted.chars() {
            match c {
                '\\' => backslashes += 1,
                '"' => {
                    arg.extend(std::iter::repeat_n('\\', backslashes / 2));
                    arg.push('"');
                    backslashes = 0;
                }
                _ => {
                    arg.extend(std::iter::repeat_n('\\', backslashes));
                    arg.push(c);
                    backslashes = 0;
                }
            }
        }
        // The closing quote follows these, so they were doubled too.
        arg.extend(std::iter::repeat_n('\\', backslashes / 2));
        return arg;
    }
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        arg.push(if c == '\\' {
            chars.next().unwrap_or(c)
        } else {
            c
        });
    }
    arg
}

/// The link step, run by cargo as the linker: rust-lld linking a shared module, then rustc's
/// own metadata sections dropped.
pub fn link(role: &str, args: Vec<String>) -> Result<(), String> {
    let lld = std::env::var(LLD_ENV).map_err(|_| format!("{LLD_ENV} unset"))?;
    let provided: Vec<String> = std::env::var(PROVIDED_ENV)
        .unwrap_or_default()
        .split(',')
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    let crate_of = |arg: &str| -> Option<String> {
        let file = Path::new(arg).file_name()?.to_str()?;
        let stem = file.strip_prefix("lib")?.strip_suffix(".rlib")?;
        Some(
            stem.rsplit_once('-')
                .map_or(stem, |(name, _)| name)
                .to_owned(),
        )
    };
    let std_artifacts = PathBuf::from(
        std::env::var_os("HORIZON_STD_ARTIFACTS").ok_or("HORIZON_STD_ARTIFACTS unset")?,
    );
    let mut args = args
        .into_iter()
        .flat_map(|arg| match arg.strip_prefix('@') {
            Some(file) => std::fs::read_to_string(file)
                .map(|t| t.lines().map(response_file_arg).collect())
                .unwrap_or_else(|_| vec![arg]),
            None => vec![arg],
        });
    let mut out = Vec::new();
    let mut output = None;
    while let Some(arg) = args.next() {
        let krate = crate_of(&arg);
        let std = Path::new(&arg)
            .file_name()
            .is_some_and(|name| std_artifacts.join(name).is_file());
        if arg == "-o" {
            output = args.next();
            out.extend(["-o".to_owned(), output.clone().unwrap_or_default()]);
        } else if role == "std" && std {
            out.extend([
                "--whole-archive".to_owned(),
                arg,
                "--no-whole-archive".to_owned(),
            ]);
        } else if role == "member"
            && krate
                .as_deref()
                .is_some_and(|k| std || provided.iter().any(|p| p == k))
        {
            continue;
        } else if role == "std" && arg == "--gc-sections" {
            out.push("--no-gc-sections".to_owned());
        } else {
            out.push(arg);
        }
    }
    let explicit_exports: std::collections::BTreeSet<_> = out
        .iter()
        .filter_map(|arg| arg.strip_prefix("--export=").map(str::to_owned))
        .collect();
    out.extend(
        [
            "--shared",
            "--strip-all",
            "--unresolved-symbols=import-dynamic",
        ]
        .map(str::to_owned),
    );
    if role == "std" || std::env::var_os("HORIZON_GROUP_PACKAGE").is_some() {
        out.push("--export-all".to_owned());
    }
    if role != "std" {
        // A member's own code takes its functions' addresses as table offsets, not through
        // `GOT.func`; with `--export-all` those functions are exported, and without local
        // binding the linker resolves such an address to slot 0, another function.
        out.push("-Bsymbolic".to_owned());
        for name in GUEST_EXPORTS {
            out.push(format!("--export={name}"));
        }
        if std::env::var("HORIZON_GUEST_REGISTRATIONS").is_ok_and(|s| !s.is_empty()) {
            out.push("--export=horizon_layout_ext".to_owned());
        }
    }
    let status = Command::new(&lld)
        .args(&out)
        .status()
        .map_err(|e| format!("{lld}: {e}"))?;
    if !status.success() {
        return Err(format!("rust-lld failed ({status})"));
    }
    let output = output.ok_or("rustc gave no -o")?;
    let bytes = std::fs::read(&output).map_err(|e| e.to_string())?;
    // Core starts execute during instantiation, before the group can fill self GOT entries.
    // Preserve the real start as an explicit initialization phase instead.
    let start = wasmparser::Parser::new(0)
        .parse_all(&bytes)
        .find_map(|payload| match payload {
            Ok(wasmparser::Payload::StartSection { func, .. }) => Some(func),
            _ => None,
        });
    let mut module = wasm_encoder::Module::new();
    let mut private_exports = Vec::new();
    let krate = if role == "member" {
        std::env::var("CARGO_CRATE_NAME").map_err(|_| "Cargo did not identify the member crate")?
    } else {
        String::new()
    };
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        let payload = payload.map_err(|e| e.to_string())?;
        if let wasmparser::Payload::CustomSection(c) = &payload
            && c.name().starts_with(".rmeta")
        {
            continue;
        }
        if matches!(payload, wasmparser::Payload::StartSection { .. }) {
            continue;
        }
        // The hook factory is an implementation detail of this member, not a group symbol.
        // Every mod defines it, so publishing it would make unrelated mods collide.
        if let wasmparser::Payload::ExportSection(reader) = &payload {
            let mut section = wasm_encoder::ExportSection::new();
            let mut has_start = false;
            for export in reader.clone() {
                let export = export.map_err(|e| e.to_string())?;
                if export.name == "__wasm_start" {
                    if start.is_some_and(|index| {
                        export.index != index || export.kind != wasmparser::ExternalKind::Func
                    }) {
                        return Err("linker start export does not name its core start".into());
                    }
                    has_start = true;
                }
                if role == "member"
                    && !group_export(export.name, &krate)
                    && !explicit_exports.contains(export.name)
                {
                    private_exports.push(export.name.to_owned());
                }
                if role != "member" || member_export(export.name) {
                    let kind = wasm_encoder::ExportKind::from(export.kind);
                    section.export(export.name, kind, export.index);
                }
            }
            if let Some(start) = start
                && !has_start
            {
                section.export("__wasm_start", wasm_encoder::ExportKind::Func, start);
            }
            module.section(&section);
            continue;
        }
        // A member imports the host ABI (`horizon`) and its group (`env`, `GOT.*`).
        // Any other import a crate declared (a JS runtime's, from `getrandom` under `zip`,
        // some under `env` itself) is never called in the game: it becomes a group import the
        // linker stubs to trap, named for this member, as the linker makes one stub per
        // importer and two members' would collide.
        if let wasmparser::Payload::ImportSection(reader) = &payload {
            let member = Path::new(&output)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            let mut section = wasm_encoder::ImportSection::new();
            for import in reader.clone().into_imports() {
                let import = import.map_err(|e| e.to_string())?;
                let js = import.name.starts_with("__wbindgen") || import.name.starts_with("__wbg");
                let ours = !js
                    && (import.module == "env"
                        || import.module == "horizon"
                        || import.module.starts_with("GOT."));
                let ty =
                    wasm_encoder::EntityType::try_from(import.ty).map_err(|e| format!("{e:?}"))?;
                if ours {
                    section.import(import.module, import.name, ty);
                } else {
                    let name = format!("{member}:{}.{}", import.module, import.name);
                    section.import("env", &name, ty);
                }
            }
            module.section(&section);
            continue;
        }
        if let Some((id, range)) = payload.as_section() {
            module.section(&wasm_encoder::RawSection {
                id,
                data: &bytes[range.start as usize..range.end as usize],
            });
        }
    }
    if role == "member" {
        module.section(&wasm_encoder::CustomSection {
            name: "horizon.private".into(),
            data: serde_json::to_vec(&private_exports)
                .map_err(|e| e.to_string())?
                .into(),
        });
    }
    std::fs::write(&output, module.finish()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod response_file_tests {
    use super::response_file_arg;
    #[test]
    fn windows_quoted_lines_lose_their_quotes_and_keep_path_backslashes() {
        assert_eq!(response_file_arg(r#""--export""#), "--export");
        assert_eq!(
            response_file_arg(r#""D:\dir\a b\libx-1.rlib""#),
            r"D:\dir\a b\libx-1.rlib"
        );
        assert_eq!(response_file_arg(r#""a\"b""#), r#"a"b"#);
        assert_eq!(response_file_arg(r#""a\\\"b""#), r#"a\"b"#);
        assert_eq!(response_file_arg(r#""dir\\""#), r"dir\");
    }
    #[test]
    fn gnu_escaped_lines_lose_their_escapes() {
        assert_eq!(response_file_arg("--export"), "--export");
        assert_eq!(
            response_file_arg(r"/tmp/a\ b/libx.rlib"),
            "/tmp/a b/libx.rlib"
        );
        assert_eq!(response_file_arg(r"a\\b"), r"a\b");
    }
}

#[cfg(test)]
mod export_tests {
    use super::{group_export, member_export};
    #[test]
    fn compiler_allocator_shims_are_private_but_library_symbols_are_public() {
        assert!(!member_export(
            "_RNvCs5fGHjzWXRav_7___rustc35___rust_no_alloc_shim_is_unstable_v2"
        ));
        assert!(!member_export("__horizon_mod_hooks"));
        assert!(!member_export("__dso_handle"));
        assert!(!member_export("__tls_base"));
        assert!(member_export(
            "_RNvCs5Y1sYENNevt_10shared_lib12shared_value"
        ));
        assert!(member_export("horizon_hook"));
    }
    #[test]
    fn only_owned_rust_symbols_are_published_to_the_group() {
        let own = "_RNvCs5Y1sYENNevt_10shared_lib12shared_value";
        assert!(group_export(own, "shared_lib"));
        assert!(!group_export(own, "other"));
        let method = "_RNvXs1_CskfiYWmmseqF_18wasm_test_observerNtB5_9DependentNtNtCscdi4KFDYmTw_7mod_api9generated15BlockEntityPart17block_entity_tick";
        assert!(group_export(method, "wasm_test_observer"));
        assert!(!group_export(method, "wasm_test_library"));
        let upstream = "_RNvMs2_NtNtCscdi4KFDYmTw_7mod_api5parts14block_entitiesDNtB7_9RegistrarEL_17block_entity_type";
        assert!(!group_export(upstream, "wasm_test_observer"));
        assert!(group_export("horizon_hook", "wasm_test_observer"));
    }
}

#[cfg(test)]
mod refusal_tests {
    #[test]
    fn wasm_registration_refusal_names_mod_target_and_type() {
        let dir = std::env::temp_dir().join(format!("horizon-wire-refusal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let catalog = dir.join("points.json");
        std::fs::write(&catalog, r#"{"schema":1,"build":{"commit":"c"},"points":[{"target":"game::tick","name":"tick","at_return":true,"api_refusal":"argument 1 `value` (`&RegistrationData`) has no API type yet"}]}"#).unwrap();
        let manifest = "id = 'addon'\nname = 'Addon'\nversion = '1.0.0'\napi = '0.2'\ncapabilities = ['registries']\n[[inject]]\nid = 'before'\nat = 'game::tick'\nkind = 'head'\nwith = 'before'\nhash = 'pin'\n".parse().unwrap();
        assert_eq!(
            super::registrations(&manifest, Some(&catalog)).unwrap_err(),
            "addon:before: no API listener at game::tick: argument 1 `value` (`&RegistrationData`) has no API type yet"
        );
    }
}
