//! Add an exact, original Java jar as a passive asset/data source.
use horizon_mod_package::{
    Manifest,
    manifest::{JarRef, MANIFEST},
};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

const USAGE: &str = "usage: cargo horizon-mod use-jar <path> [--take <prefix>]...";

/// The game's jar store: `$HORIZON_HOME/jars`, else `<platform data dir>/horizon/jars`
/// (matching `horizon_client_render::logging::game_home().join("jars")`).
pub fn default_jar_store() -> PathBuf {
    let home = match std::env::var_os("HORIZON_HOME").filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => horizon_mod_package::selection::data_dir_with(|key| std::env::var_os(key))
            .join("horizon"),
    };
    home.join("jars")
}

/// Copy `source` into `store` at `jar.cache_path(store)` atomically, verifying its SHA-512.
pub fn stage_jar(source: &Path, jar: &JarRef, store: &Path) -> Result<PathBuf, String> {
    let dest = jar.cache_path(store);
    if dest.is_file() {
        let existing = fs::File::open(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        let hash = horizon_port_pack::fetch::hash_reader::<sha2::Sha512>(existing, io::sink())
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        if format!("{hash:x}") == jar.sha512() {
            return Ok(dest);
        }
    }
    fs::create_dir_all(store).map_err(|e| format!("{}: {e}", store.display()))?;
    let partial = store.join(format!(
        "{}.jar.{}.partial",
        jar.sha512(),
        std::process::id()
    ));
    let mut reader = fs::File::open(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| format!("{}: {e}", partial.display()))?;
    let staged = (|| -> Result<(), String> {
        let hash = horizon_port_pack::fetch::hash_reader::<sha2::Sha512>(&mut reader, &mut out)
            .map_err(|e| format!("{}: {e}", source.display()))?;
        let got = format!("{hash:x}");
        if got != jar.sha512() {
            return Err(format!(
                "{}: sha512 changed while staging (expected {}, got {got})",
                source.display(),
                jar.sha512()
            ));
        }
        out.sync_all()
            .map_err(|e| format!("{}: {e}", partial.display()))?;
        drop(out);
        fs::rename(&partial, &dest).map_err(|e| format!("{}: {e}", dest.display()))
    })();
    if staged.is_err() {
        let _ = fs::remove_file(&partial);
    }
    staged?;
    Ok(dest)
}

/// Stage any `[[jar]]` entries needed for `cargo horizon-mod run` into the game's
/// jar store, using `LaunchArguments::parse` so `--home` follows the exact same
/// rules as `horizon-client` (last `--home` wins; values consumed by other flags
/// are not misread as `--home`). Fails loudly before launch if a listed jar is
/// missing from `default_store` and the target store.
pub fn stage_for_run(mod_dir: &Path, extra: &[String], default_store: &Path) -> Result<(), String> {
    let manifest_path = mod_dir.join(MANIFEST);
    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&text)?;
    if manifest.jar.is_empty() {
        return Ok(());
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let default_home = default_store.parent().unwrap_or(default_store);
    let launch = horizon_mod_package::selection::LaunchArguments::parse(
        extra.iter().cloned(),
        &cwd,
        default_home,
        None,
    )
    .map_err(|e| e.to_string())?;
    if launch.is_information_request() {
        return Ok(());
    }
    let target_store = launch.context.home.join("jars");
    let mut missing = Vec::new();
    for jar in &manifest.jar {
        let cached = jar.cache_path(default_store);
        if cached.is_file() {
            if target_store != default_store {
                stage_jar(&cached, jar, &target_store)?;
            }
        } else if !jar.cache_path(&target_store).is_file() {
            missing.push(format!(
                "jar {} {} is listed in {MANIFEST} but not staged in {}; run `cargo horizon-mod use-jar <original.jar>`",
                jar.id(),
                jar.version(),
                default_store.display()
            ));
        }
    }
    if !missing.is_empty() {
        return Err(missing.join("\n"));
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let Some((path, rest)) = args.split_first() else {
        return Err(USAGE.into());
    };
    let mut take = Vec::new();
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        match (arg.as_str(), rest.next()) {
            ("--take", Some(prefix)) => take.push(prefix.clone()),
            _ => return Err(USAGE.into()),
        }
    }
    use_jar_in(Path::new("."), Path::new(path), take, &default_jar_store())
}

fn use_jar_in(mod_dir: &Path, path: &Path, take: Vec<String>, store: &Path) -> Result<(), String> {
    let manifest_path = mod_dir.join(MANIFEST);
    let text = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let manifest = Manifest::parse(&text)?;
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let hash = horizon_port_pack::fetch::hash_reader::<sha2::Sha512>(file, io::sink())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let identity = horizon_port_pack::jar::read_identity(path)?;
    let mut jar = JarRef::new(identity.id, identity.version, format!("{hash:x}"))?;
    if !take.is_empty() {
        jar = jar.with_take(take)?;
        // The hash pins these files, so an entry that keeps nothing now never will.
        let names = jar_names(path)?;
        for entry in jar.take().unwrap_or_default() {
            let only = jar.clone().with_take(vec![entry.clone()])?;
            if !names.iter().any(|name| only.takes(name)) {
                return Err(format!(
                    "{}: take entry {entry:?} matches no file in the jar",
                    path.display()
                ));
            }
        }
    }
    let mut listed = false;
    for existing in manifest
        .jar
        .iter()
        .filter(|entry| entry.sha512() == jar.sha512())
    {
        if existing.identity() != jar.identity() {
            return Err(format!(
                "{MANIFEST}: this hash lists {} {}, but {} identifies {} {}; correct the existing entry",
                existing.id(),
                existing.version(),
                path.display(),
                jar.id(),
                jar.version()
            ));
        }
        if existing.take() != jar.take() {
            return Err(format!(
                "{MANIFEST}: {} {} is already listed with take {:?}; edit its entry to change what it takes",
                jar.id(),
                jar.version(),
                existing.take().unwrap_or_default()
            ));
        }
        listed = true;
    }
    if listed {
        let staged = stage_jar(path, &jar, store)?;
        println!(
            "Jar {} {} is already listed (staged at {})",
            jar.id(),
            jar.version(),
            staged.display()
        );
        return Ok(());
    }
    let entry = toml::to_string(&jar).map_err(|e| e.to_string())?;
    let candidate = format!("{text}\n[[jar]]\n{entry}");
    Manifest::parse(&candidate).map_err(|e| format!(
        "{MANIFEST}: cannot append a jar entry; use [[jar]] tables rather than an inline jar array: {e}"
    ))?;
    let staged = stage_jar(path, &jar, store)?;
    let partial = manifest_path.with_extension(format!("toml.{}.partial", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| format!("{}: {e}", partial.display()))?;
    let result = (|| -> io::Result<()> {
        file.write_all(candidate.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, &manifest_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result.map_err(|e| format!("{MANIFEST}: {e}"))?;
    println!(
        "Added jar {} {} (staged at {})",
        jar.id(),
        jar.version(),
        staged.display()
    );
    Ok(())
}

/// Every file name in the jar at `path`.
fn jar_names(path: &Path) -> Result<Vec<String>, String> {
    let error = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = fs::File::open(path).map_err(|e| error(&e))?;
    let archive = zip::ZipArchive::new(file).map_err(|e| error(&e))?;
    let mut names = archive
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| error(&e))?;
    // A directory entry holds nothing, so a take that matches only one keeps nothing.
    names.retain(|name| !name.ends_with('/'));
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fabric_jar(path: &Path, id: &str, version: &str) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("fabric.mod.json", options).unwrap();
        write!(
            zip,
            r#"{{"schemaVersion":1,"id":"{id}","version":"{version}"}}"#
        )
        .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn use_jar_stages_hashed_bytes_and_stage_for_run_copies_to_custom_home() {
        let tmp = tempfile::tempdir().unwrap();
        let mod_dir = tmp.path().join("mod");
        let default_store = tmp.path().join("home/jars");
        let first_home = tmp.path().join("first-home");
        let last_home = tmp.path().join("last-home");
        let eq_home = tmp.path().join("eq-home");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(
            mod_dir.join(MANIFEST),
            "id = 'demo'\nname = 'Demo'\nversion = '0.1.0'\napi = '0.2'\ncapabilities = ['registries']\n",
        )
        .unwrap();
        let jar_path = tmp.path().join("example.jar");
        make_fabric_jar(&jar_path, "example_mod", "1.2.3");

        use_jar_in(&mod_dir, &jar_path, Vec::new(), &default_store).unwrap();
        let manifest =
            Manifest::parse(&fs::read_to_string(mod_dir.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(manifest.jar.len(), 1);
        let cached = manifest.jar[0].cache_path(&default_store);
        assert!(
            cached.is_file(),
            "expected staged jar at {}",
            cached.display()
        );
        assert_eq!(fs::read(&cached).unwrap(), fs::read(&jar_path).unwrap());

        // Repeated `--home` uses the last value, matching `horizon-client` / `LaunchArguments`.
        let extra = vec![
            "--home".into(),
            first_home.display().to_string(),
            "--home".into(),
            last_home.display().to_string(),
        ];
        stage_for_run(&mod_dir, &extra, &default_store).unwrap();
        assert!(
            !manifest.jar[0]
                .cache_path(&first_home.join("jars"))
                .exists()
        );
        let last_cached = manifest.jar[0].cache_path(&last_home.join("jars"));
        assert!(last_cached.is_file());
        assert_eq!(
            fs::read(&last_cached).unwrap(),
            fs::read(&jar_path).unwrap()
        );

        // Corrupted cached bytes in a custom `--home` are replaced on re-stage,
        // and another option's value (`--world --home`) is not misread as `--home`.
        let eq_cached = manifest.jar[0].cache_path(&eq_home.join("jars"));
        fs::create_dir_all(eq_cached.parent().unwrap()).unwrap();
        fs::write(&eq_cached, b"corrupted").unwrap();
        let re_stage_extra = vec![
            "--world".into(),
            "--home".into(),
            "--home".into(),
            eq_home.display().to_string(),
        ];
        stage_for_run(&mod_dir, &re_stage_extra, &default_store).unwrap();
        assert_eq!(fs::read(&eq_cached).unwrap(), fs::read(&jar_path).unwrap());

        // Missing jar in default_store fails loudly with an actionable `use-jar` hint.
        let empty_store = tmp.path().join("empty-home/jars");
        let err = stage_for_run(&mod_dir, &[], &empty_store).unwrap_err();
        assert!(
            err.contains("example_mod") && err.contains("cargo horizon-mod use-jar"),
            "unexpected error: {err}"
        );
    }
}
