//! `cargo horizon-mod`: make Horizon mods in Rust (`docs/mods.md`).
//!
//! - `new <path> [--name "Name"] [--sdk-path <dir>]`: a mod crate from the template.
//! - `build [--debug]`: the `.hmod` in `target/horizon-mod/`.
//! - `run [--game <horizon-client>] [-- <game args>]`: build, then play with
//!   the mod; run `build` again and the game reloads it.
//! - `test [--debug] [--hmod <file>] [<name>]`: build, then run the mod's
//!   `tests/*.toml` scripts in the sandbox, with no game.
//! - `check <file> [--json]`: what the registry checks of a submission
//!   (`.hmod`, `.hpack`, resource pack `.zip`), with a trial run for a mod.
//! - `registry build|review|check|list`: the registry repository's CI.
use horizon_mods::{
    Manifest, Package,
    manifest::{MANIFEST, component_imports, componentize},
    testing::{Script, TestGame},
};
mod check;
mod registry;

use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
};

const TARGET: &str = "wasm32-unknown-unknown";
const SDK_GIT: &str = "{ git = \"https://github.com/ludovic111/horizon\" }";

fn main() -> ExitCode {
    // Cargo runs `cargo-horizon-mod horizon-mod <args>`.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "horizon-mod") {
        args.remove(0);
    }
    let result = match args.first().map(String::as_str) {
        Some("new") => new(&args[1..]),
        Some("build") => build(!args.iter().any(|a| a == "--debug")).map(|p| {
            println!("Built {}", p.display());
        }),
        Some("run") => run(&args[1..]),
        Some("test") => test(&args[1..]),
        Some("check") => check(&args[1..]),
        Some("registry") => registry::main(&args[1..]),
        _ => Err("usage: cargo horizon-mod new <path> | build [--debug] | run [--game <horizon-client>] [-- <game args>] | test [--debug] [--hmod <file>] [<name>] | check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json] | registry build|review|check|list".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The value after `flag` in `args`.
fn flag<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(String::as_str)
}

fn new(args: &[String]) -> Result<(), String> {
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
    let sdk = match flag(args, "--sdk-path") {
        Some(p) => {
            let p = std::fs::canonicalize(p).map_err(|e| format!("--sdk-path {p}: {e}"))?;
            format!("{{ path = {:?} }}", p.display().to_string())
        }
        None => SDK_GIT.to_string(),
    };
    let fill = |t: &str| {
        t.replace("{crate}", &id.replace('_', "-"))
            .replace("{id}", &id)
            .replace("{name}", &name)
            .replace("{sdk}", &sdk)
    };
    let files = [
        ("Cargo.toml", include_str!("../template/Cargo.toml.in")),
        (MANIFEST, include_str!("../template/horizon-mod.toml")),
        ("src/lib.rs", include_str!("../template/src/lib.rs")),
        (".gitignore", include_str!("../template/gitignore")),
        (
            "tests/hello.toml",
            include_str!("../template/tests/hello.toml"),
        ),
    ];
    for (file, text) in files {
        let to = dir.join(file);
        std::fs::create_dir_all(to.parent().expect("a file in the mod"))
            .map_err(|e| e.to_string())?;
        std::fs::write(&to, fill(text)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Manifest::parse(&std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| e.to_string())?)?;
    println!(
        "Created {} (mod id {id}). Next: cd {path} && cargo horizon-mod run",
        dir.display()
    );
    Ok(())
}

/// Builds the crate in the current folder and packs `target/horizon-mod/<id>.hmod`.
fn build(release: bool) -> Result<PathBuf, String> {
    let manifest_text = std::fs::read_to_string(MANIFEST)
        .map_err(|e| format!("{MANIFEST}: {e} (run this in a mod's folder)"))?;
    let manifest = Manifest::parse(&manifest_text)?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args([
        "build",
        "--lib",
        "--target",
        TARGET,
        "--message-format=json-render-diagnostics",
    ]);
    if release {
        cmd.arg("--release");
    }
    let out = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "cargo build failed (is the target installed? rustup target add {TARGET})"
        ));
    }
    let core = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .flat_map(|m| m["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|f| f.as_str().map(PathBuf::from))
        .rfind(|f| f.extension().is_some_and(|e| e == "wasm"))
        .ok_or("the build made no .wasm: is the crate a cdylib?")?;
    let wasm = componentize(&std::fs::read(&core).map_err(|e| e.to_string())?)?;
    // The game refuses a component importing capabilities the manifest lacks.
    manifest.check_imports(component_imports(&wasm)?.iter().map(String::as_str))?;
    let target_dir = core.ancestors().nth(3).ok_or("unexpected target folder")?;
    let dir = target_dir.join("horizon-mod");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.hmod", manifest.id));
    // Write then rename, so a running game never reads half a file.
    let partial = path.with_extension("hmod.partial");
    std::fs::write(&partial, Package::write(&manifest_text, &wasm)?).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, &path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `check <file>`: the registry's checks of a submission, printed; fails
/// when one does.
fn check(args: &[String]) -> Result<(), String> {
    let file = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .ok_or("cargo horizon-mod check <file>")?;
    let path = Path::new(file);
    let kind = match flag(args, "--kind") {
        Some(k) => horizon_mods::index::Kind::from(k.to_string()),
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
        println!("{json}");
    }
    let report = report?;
    if !args.iter().any(|a| a == "--json") {
        print!("{}", report.text());
    }
    Ok(())
}

/// `test`: every `tests/*.toml` script (those whose name holds `<name>`)
/// against the built mod, or the `.hmod` given with `--hmod`.
fn test(args: &[String]) -> Result<(), String> {
    let hmod = match flag(args, "--hmod") {
        Some(p) => PathBuf::from(p),
        None => build(!args.iter().any(|a| a == "--debug"))?,
    };
    let filter = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--hmod"))
        .map(|(_, a)| a.as_str());
    let mut scripts: Vec<PathBuf> = std::fs::read_dir("tests")
        .map_err(|e| {
            format!(
                "tests/: {e} (a mod's tests are tests/<name>.toml: docs/mods.md, \"Testing a mod\")"
            )
        })?
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter(|p| filter.is_none_or(|f| p.to_string_lossy().contains(f)))
        .collect();
    scripts.sort();
    if scripts.is_empty() {
        return Err("no tests/*.toml to run".into());
    }
    println!(
        "running {} scripts against {}",
        scripts.len(),
        hmod.display()
    );
    let mut failed = 0;
    for path in &scripts {
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| Script::parse(&text))
            .and_then(|script| {
                let mut game = TestGame::load(&hmod)?;
                script.run(&mut game)?;
                Ok(script.steps.len())
            });
        match result {
            Ok(steps) => println!("test {} ... ok ({steps} steps)", path.display()),
            Err(e) => {
                failed += 1;
                println!("test {} ... FAILED\n  {e}", path.display());
            }
        }
    }
    let passed = scripts.len() - failed;
    if failed > 0 {
        return Err(format!("{failed} failed, {passed} passed"));
    }
    println!("test result: ok. {passed} passed");
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let path = build(true)?;
    let game = flag(args, "--game")
        .map(str::to_string)
        .or_else(|| std::env::var("HORIZON_CLIENT").ok())
        .unwrap_or_else(|| "horizon-client".into());
    let extra = args.iter().skip_while(|a| *a != "--").skip(1);
    println!(
        "Starting {game} with {} (rebuild with `cargo horizon-mod build` to reload it)",
        path.display()
    );
    let status = Command::new(&game)
        .arg("--dev-mod")
        .arg(&path)
        .args(extra)
        .status()
        .map_err(|e| {
            format!("{game}: {e} (pass --game <path to horizon-client> or set HORIZON_CLIENT)")
        })?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("the game exited with {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_is_a_valid_mod() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-new-{}", std::process::id()));
        let path = dir.join("My-Mod");
        new(&[path.display().to_string(), "--name".into(), "My Mod".into()]).unwrap();
        let manifest =
            Manifest::parse(&std::fs::read_to_string(path.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(
            (manifest.id.as_str(), manifest.name.as_str()),
            ("my_mod", "My Mod")
        );
        let cargo = std::fs::read_to_string(path.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"my-mod\"") && cargo.contains(SDK_GIT));
        let script = std::fs::read_to_string(path.join("tests/hello.toml")).unwrap();
        assert_eq!(Script::parse(&script).unwrap().steps.len(), 3);
        assert!(
            new(&[path.display().to_string()]).is_err(),
            "never overwrites"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
