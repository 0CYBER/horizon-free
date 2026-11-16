use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src");
    let mut files: Vec<_> = fs::read_dir("src")
        .expect("example sources")
        .map(|entry| entry.expect("example entry").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "rs") && path.file_stem().unwrap() != "lib"
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no standalone book examples discovered");
    let mut modules = String::new();
    for file in files {
        let name = file
            .file_stem()
            .unwrap()
            .to_str()
            .expect("UTF-8 module name");
        assert!(
            name.bytes()
                .all(|c| c.is_ascii_lowercase() || c == b'_' || c.is_ascii_digit()),
            "invalid example module name: {name}"
        );
        modules.push_str(&format!(
            "#[path = {:?}] pub mod {name};\n",
            file.canonicalize().expect("example path")
        ));
    }
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR")).join("example_modules.rs"),
        modules,
    )
    .expect("example declarations");
}
