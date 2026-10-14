//! HORIZON_PREPARE_SOURCES_SHA256: a digest of this crate's sources, which the launcher compares
//! with a version folder's copy to know whether it may prepare that version in-process. Keep the
//! algorithm the same as `horizon_prepare::sources_sha256`.

use sha2::Digest;
use std::fs;
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for name in ["Cargo.toml", "build.rs"] {
        println!("cargo:rerun-if-changed={name}");
        files.push((name.to_string(), fs::read(dir.join(name)).unwrap()));
    }
    println!("cargo:rerun-if-changed=src");
    let mut sources: Vec<String> = fs::read_dir(dir.join("src"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    sources.sort();
    for name in sources {
        println!("cargo:rerun-if-changed=src/{name}");
        files.push((
            format!("src/{name}"),
            fs::read(dir.join("src").join(&name)).unwrap(),
        ));
    }
    let mut hash = sha2::Sha256::new();
    for (name, data) in files {
        let data: Vec<u8> = data.into_iter().filter(|b| *b != b'\r').collect();
        hash.update(format!("{name}\0{}\0", data.len()));
        hash.update(&data);
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    println!("cargo:rustc-env=HORIZON_PREPARE_SOURCES_SHA256={hex}");
}
