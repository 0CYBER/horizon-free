//! Publish native installations before exposing their completion marker.
use super::*;

pub(super) fn install(
    base: &str,
    version: &str,
    target: Target,
    binary: &Path,
    libraries: &[PathBuf],
    about: &serde_json::Value,
    mods: &[Bundle],
) -> Result<(), String> {
    let _publication = crate::run_leases::installation().map_err(cache_error)?;
    let from = backend::version_dir(base);
    let partial = target.directory(&format!(".{version}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    copy_base(&from, &partial, target, target == Target::Client)?;
    let installed = partial.join(target.binary());
    fs::copy(binary, &installed).map_err(|e| format!("The built game: {e}"))?;
    // The prebuilt game's libraries load from beside the executable.
    for library in libraries {
        let name = library.file_name().ok_or("invalid library path")?;
        fs::copy(library, partial.join(name)).map_err(|e| format!("{}: {e}", library.display()))?;
    }
    let mut about = about.clone();
    about["libraries"] =
        serde_json::to_value(library_record::collect(libraries)?).map_err(|e| e.to_string())?;
    fs::write(partial.join("native-mods.json"), about.to_string()).map_err(|e| e.to_string())?;
    for bundle in mods {
        package::copy(
            bundle,
            &partial
                .join(horizon_mod_package::mod_list::NATIVE_DIR)
                .join(&bundle.manifest.id),
        )?;
    }
    if let Err(cancelled) = check_cancelled() {
        let _ = fs::remove_dir_all(&partial);
        return Err(cancelled);
    }
    let dir = target.directory(version);
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&partial, &dir).map_err(|e| e.to_string())?;
    crate::run_leases::publish(&dir).map_err(cache_error)?;
    fs::write(dir.join(".installed"), "").map_err(|e| e.to_string())
}
