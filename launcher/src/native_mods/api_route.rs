//! Build each API-only native mod as its own Rust dylib from SDK metadata.
//! The executable is fixed and the engine archive is never read.
use super::{api, dependencies, internals, prebuilt};
use crate::rust_toolchain::Toolchain;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The template the mods library is written from, in the `api` slice.
pub(super) const TEMPLATE: &str = "native_mod_entry.rs.template";
/// The folder of the runtime slice holding the fixed executable.
const BIN: &str = "bin";

/// Compile the mods in dependency order against API metadata and the runtime's dynamic
/// closure. Return the unchanged executable and its runtime libraries, followed by mod dylibs.
#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    rust: &Toolchain,
    runtime: &Path,
    api: &Path,
    out: &Path,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    on_mod: &dyn Fn(&str, usize, usize),
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let sdk = prebuilt::identity(runtime)?;
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let deps = api.join("deps");
    let host = api.join("host");
    let runtime_deps = runtime.join("deps");
    let search =
        |edition: &str| prebuilt::rustc_with(rust, &[&deps, &host, &runtime_deps, out], edition);
    let mut libraries = HashMap::new();
    for (package, name) in api::PACKAGES {
        if package != api::INTERNALS_PACKAGE {
            libraries.insert(name, prebuilt::library(&runtime_deps, name)?);
        }
    }
    libraries.insert("mod_vocab", prebuilt::library(&runtime_deps, "mod_vocab")?);
    let mut rlibs: HashMap<&str, PathBuf> = HashMap::new();
    for (n, &index) in resolution.order.iter().enumerate() {
        let project = &projects[index];
        on_mod(&project.bundle.manifest.id, n + 1, resolution.order.len());
        let (rlib, _) = prebuilt::compile_mod(
            project,
            &declared[index],
            out,
            search,
            |package| &libraries[package],
            &rlibs,
            &prebuilt::library_entry(
                &api::template(&api.join(TEMPLATE))?,
                &[&project.bundle.manifest],
                &internals::Catalog::read(api)?,
                sdk.commit(),
                &project.bundle.manifest.id,
                &project.bundle.manifest.version.to_string(),
                &[],
            ),
            sdk.commit(),
        )?;
        rlibs.insert(project.bundle.manifest.id.as_str(), rlib);
    }
    let mut installed = prebuilt::runtime_libraries(rust, &sdk, runtime)?;
    for &index in &resolution.order {
        installed.push(rlibs[projects[index].bundle.manifest.id.as_str()].clone());
    }
    Ok((runtime.join(BIN).join(crate::backend::CLIENT), installed))
}
