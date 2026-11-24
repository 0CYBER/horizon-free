//! Add an exact, original Java jar as a passive asset/data source.
use horizon_mod_package::{
    Manifest,
    manifest::{JarRef, MANIFEST},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};

const USAGE: &str = "usage: cargo horizon-mod use-jar <path> [--take <prefix>]...";

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
    let manifest_path = Path::new(MANIFEST);
    let text = fs::read_to_string(manifest_path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = Manifest::parse(&text)?;
    let path = Path::new(path);
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
        println!("Jar {} {} is already listed", jar.id(), jar.version());
        return Ok(());
    }
    let entry = toml::to_string(&jar).map_err(|e| e.to_string())?;
    let candidate = format!("{text}\n[[jar]]\n{entry}");
    Manifest::parse(&candidate).map_err(|e| format!(
        "{MANIFEST}: cannot append a jar entry; use [[jar]] tables rather than an inline jar array: {e}"
    ))?;
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
        fs::rename(&partial, manifest_path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result.map_err(|e| format!("{MANIFEST}: {e}"))?;
    println!("Added jar {} {}", jar.id(), jar.version());
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
