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

pub fn run(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err("usage: cargo horizon-mod use-jar <path>".into());
    };
    let manifest_path = Path::new(MANIFEST);
    let text = fs::read_to_string(manifest_path).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let manifest = Manifest::parse(&text)?;
    let path = Path::new(path);
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let hash = horizon_port_pack::fetch::hash_reader::<sha2::Sha512>(file, io::sink())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let identity = horizon_port_pack::jar::read_identity(path)?;
    let jar = JarRef::new(identity.id, identity.version, format!("{hash:x}"))?;
    let mut listed = false;
    for existing in manifest
        .jar
        .iter()
        .filter(|entry| entry.sha512() == jar.sha512())
    {
        if existing != &jar {
            return Err(format!(
                "{MANIFEST}: this hash lists {} {}, but {} identifies {} {}; correct the existing entry",
                existing.id(),
                existing.version(),
                path.display(),
                jar.id(),
                jar.version()
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
