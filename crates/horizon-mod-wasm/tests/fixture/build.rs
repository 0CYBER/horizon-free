//! Like wire-loopback's folder_modules: sorted, absolute-path declarations from one flat folder.
//! The fixture also keeps its root, scanner-only points and nested math data in that folder;
//! those are not independent guest modules. No source parsing or extra build dependencies.
use std::{fs, path::Path};

pub fn modules(directory: &Path) -> Result<String, String> {
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() || path.file_name().is_some_and(|name| name == "mod.rs") {
            return Err(format!(
                "{}: put each guest module in a file of its own here, not a folder or mod.rs",
                path.display()
            ));
        }
        if path.extension().is_some_and(|extension| extension == "rs")
            && !path.file_name().is_some_and(|name| {
                name == "lib.rs" || name == "points.rs" || name == "strict_math_vectors.rs"
            })
        {
            paths.push(path);
        }
    }
    paths.sort();
    let mut code = String::new();
    for path in paths {
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: guest module name is not UTF-8", path.display()))?;
        let absolute = path
            .canonicalize()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        // Keep the math checker's original private visibility; registrations stay public.
        let visibility = if name == "strict_math_checks" {
            ""
        } else {
            "pub "
        };
        code.push_str(&format!(
            "#[path = {:?}]\n{visibility}mod {name};\n",
            absolute.to_string_lossy()
        ));
    }
    Ok(code)
}

pub fn generate(manifest: &Path, out: &Path) {
    let directory = manifest.join("src");
    println!("cargo:rerun-if-changed={}", directory.display());
    let code = modules(&directory)
        .unwrap_or_else(|error| panic!("discovering wasm fixture modules: {error}"));
    let output = out.join("wasm_fixture_modules.rs");
    if fs::read_to_string(&output).ok().as_deref() != Some(&code) {
        fs::write(&output, code)
            .unwrap_or_else(|error| panic!("writing {}: {error}", output.display()));
    }
}

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    generate(&manifest, &out);
}
