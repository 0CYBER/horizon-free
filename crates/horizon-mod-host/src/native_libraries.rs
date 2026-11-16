//! Checked, process-lifetime native library loading.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{ffi::CStr, path::Path};

/// Commit used for the pinned SDK ABI.
pub const GAME_BUILD_ID: &str = env!("HORIZON_GAME_BUILD_ID");
/// Compiler release used for the pinned Rust ABI.
pub const RUSTC_VERSION: &str = env!("HORIZON_RUSTC_VERSION");

#[derive(Deserialize)]
struct Record {
    libraries: Vec<Library>,
}
#[derive(Deserialize)]
struct Library {
    file: String,
    sha256: String,
    #[serde(rename = "mod")]
    id: String,
    identity: String,
}

fn verify(folder: &Path, libraries: &[Library]) -> Result<(), String> {
    for library in libraries {
        if library.identity.split_whitespace().next() != Some(GAME_BUILD_ID)
            || library.identity.split_whitespace().last() != Some(RUSTC_VERSION)
        {
            return Err(format!(
                "mod {}: incompatible build identity {:?}; game {GAME_BUILD_ID} rustc {RUSTC_VERSION}",
                library.id, library.identity
            ));
        }
        // File names are untrusted build-record input, not paths outside this installation.
        if Path::new(&library.file).components().count() != 1
            || !matches!(
                Path::new(&library.file).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(format!(
                "mod {}: invalid library file {}",
                library.id, library.file
            ));
        }
        // The launcher installs real files (copies or hard links), never symlinks: a link could
        // lead out of the installation.
        let path = folder.join(&library.file);
        let is_link = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("mod {}: {}: {error}", library.id, library.file))?
            .file_type()
            .is_symlink();
        if is_link {
            return Err(format!(
                "mod {}: library file {} is a symbolic link",
                library.id, library.file
            ));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("mod {}: {}: {error}", library.id, library.file))?;
        let actual = format!("{:x}", Sha256::digest(bytes));
        if actual != library.sha256 {
            return Err(format!(
                "mod {}: library {} hash mismatch (expected {}, found {actual})",
                library.id, library.file, library.sha256
            ));
        }
    }
    Ok(())
}

fn load(folder: &Path) -> Result<Vec<&'static mod_api::__private::native::NativeMod>, String> {
    let path = folder.join("native-mods.json");
    let record = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Record>(&bytes)
            .map_err(|e| format!("native-mods.json: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("native-mods.json: {e}")),
    };
    verify(folder, &record.libraries)?;
    let mut loaded = Vec::new();
    for row in &record.libraries {
        let path = folder.join(&row.file);
        // SAFETY: loading native code is an explicit trust boundary. Keep every handle forever,
        // including on refusal, so returned Rust references and function pointers never dangle.
        let library: libloading::Library = unsafe {
            #[cfg(windows)]
            {
                libloading::os::windows::Library::load_with_flags(&path, 0x100 | 0x1000)
                    .map(Into::into)
            }
            #[cfg(not(windows))]
            {
                libloading::Library::new(&path)
            }
        }
        .map_err(|e| format!("mod {}: {e}", row.id))?;
        let library = Box::leak(Box::new(library));
        unsafe {
            let build = library
                .get::<unsafe extern "C" fn() -> *const core::ffi::c_char>(
                    b"horizon_native_build\0",
                )
                .map_err(|e| format!("mod {}: {e}", row.id))?;
            let pointer = build();
            if pointer.is_null() {
                return Err(format!("mod {}: null build identity", row.id));
            }
            let identity = CStr::from_ptr(pointer)
                .to_str()
                .map_err(|e| format!("mod {}: {e}", row.id))?;
            if identity != row.identity
                || identity.split_whitespace().next() != Some(GAME_BUILD_ID)
                || identity.split_whitespace().last() != Some(RUSTC_VERSION)
            {
                return Err(format!(
                    "mod {}: identity {identity:?}, expected {:?}; game {GAME_BUILD_ID} rustc {RUSTC_VERSION}",
                    row.id, row.identity
                ));
            }
        }
        loaded.push((row, library));
    }
    // No registration factory is called until every library has passed identity validation.
    loaded
        .into_iter()
        .map(|(row, library)| unsafe {
            let entry = library
                .get::<unsafe fn() -> &'static mod_api::__private::native::NativeMod>(
                    b"horizon_native_mod_v1\0",
                )
                .map_err(|e| format!("mod {}: {e}", row.id))?;
            Ok(entry())
        })
        .collect()
}

/// Start the fixed executable, registering checked native libraries before the engine seal.
pub fn main() {
    crate::prepare_main_stack();
    crate::exit_if_compile_child();
    let startup = horizon_client::entry::prepare();
    let result = std::env::current_exe()
        .map_err(|e| e.to_string())
        .and_then(|exe| load(exe.parent().expect("executable parent")));
    let libraries = result.unwrap_or_else(|error| {
        eprintln!("Horizon: refusing native startup: {error}");
        std::process::exit(1);
    });
    let mut entries = Vec::new();
    let mut slots = Vec::new();
    let mut built = Vec::new();
    let mut installs = Vec::new();
    // Every patch copy is installed and checked before any mod's hook factory runs, as the
    // generated `main` did: a factory may call a patched function, and a refused install stops
    // the start before any mod code beyond the installs has run.
    for library in &libraries {
        installs.extend((library.installs)());
    }
    horizon_client::entry::installed(&installs);
    for library in libraries {
        entries.extend((library.entries)());
        slots.extend((library.slots)());
        built.extend_from_slice(library.built);
    }
    crate::main_with(
        entries,
        &slots,
        Box::leak(built.into_boxed_slice()),
        startup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_phase_refuses_missing_corrupt_and_traversal_by_mod_name() {
        let folder = std::env::temp_dir().join(format!("native-hash-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut row = Library {
            file: "test.so".into(),
            sha256: format!("{:x}", Sha256::digest(b"valid")),
            id: "example".into(),
            identity: format!("{GAME_BUILD_ID} example 1 {RUSTC_VERSION}"),
        };
        assert!(
            verify(&folder, std::slice::from_ref(&row))
                .unwrap_err()
                .contains("example")
        );
        std::fs::write(folder.join("test.so"), b"valid").unwrap();
        verify(&folder, std::slice::from_ref(&row)).unwrap();
        for identity in [
            format!("wrong example 1 {RUSTC_VERSION}"),
            format!("{GAME_BUILD_ID} example 1 wrong"),
        ] {
            let valid = std::mem::replace(&mut row.identity, identity);
            let error = verify(&folder, std::slice::from_ref(&row)).unwrap_err();
            assert!(error.contains("example") && error.contains("identity"));
            row.identity = valid;
        }
        assert!(serde_json::from_str::<Record>("{}").is_err());
        assert!(serde_json::from_str::<Record>(r#"{"libraries": []}"#).is_ok());
        std::fs::write(folder.join("test.so"), b"corrupt").unwrap();
        assert!(
            verify(&folder, std::slice::from_ref(&row))
                .unwrap_err()
                .contains("example")
        );
        // A link with the right bytes is still refused: it could lead out of the installation.
        #[cfg(unix)]
        {
            std::fs::write(folder.join("real.so"), b"valid").unwrap();
            std::os::unix::fs::symlink(folder.join("real.so"), folder.join("link.so")).unwrap();
            row.file = "link.so".into();
            let error = verify(&folder, std::slice::from_ref(&row)).unwrap_err();
            assert!(
                error.contains("example") && error.contains("symbolic link"),
                "{error}"
            );
        }
        row.file = "../test.so".into();
        assert!(verify(&folder, &[row]).unwrap_err().contains("example"));
        std::fs::remove_dir_all(folder).unwrap();
    }
}
