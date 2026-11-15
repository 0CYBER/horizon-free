//! Explicitly trusted checkout installations, assembled by the release packager.
use crate::{
    backend,
    i18n::{tr, trf},
};
use sha2::{Digest, Sha256};
use std::{fs, path::Path, process::Command};

pub fn is_developer(version: &str) -> bool {
    version.starts_with("developer-")
}

pub fn uses_developer(version: &str) -> bool {
    is_developer(version)
        || crate::native_mods::about(version).is_some_and(|a| is_developer(&a.base))
        || crate::build_import::about(version).is_some_and(|a| is_developer(&a.base))
}

pub fn label(version: &str) -> Option<String> {
    uses_developer(version).then(|| {
        let base = crate::native_mods::about(version)
            .map(|a| a.base)
            .or_else(|| crate::build_import::about(version).map(|a| a.base))
            .unwrap_or_else(|| version.to_owned());
        let commit = revision(&base).unwrap_or_else(|_| tr("Unknown commit").into());
        trf(
            "Local unsigned developer build · {commit} · {id}",
            &[("commit", &commit), ("id", version)],
        )
    })
}

/// The instance row keeps the trust warning even before resources have been prepared.
pub fn instance_status(version: &str, prepared: bool) -> Option<String> {
    label(version).map(|label| {
        format!(
            "{label} · {}",
            if prepared {
                tr("Ready")
            } else {
                tr("Needs preparing")
            }
        )
    })
}

pub fn revision(version: &str) -> Result<String, String> {
    revision_in(&backend::version_dir(version))
}

pub(crate) fn revision_in(directory: &Path) -> Result<String, String> {
    let record = fs::read_to_string(directory.join("developer.json")).map_err(|e| e.to_string())?;
    #[derive(serde::Deserialize)]
    struct Record {
        revision: String,
    }
    let record: Record = serde_json::from_str(&record).map_err(|e| e.to_string())?;
    if record.revision.len() != 40 || !record.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(tr("Invalid developer build revision.").into());
    }
    Ok(record.revision)
}

// Do not follow symlinks: only the regular files the packager produced are installable.
fn digest_tree(root: &Path, hash: &mut Sha256) -> Result<(), String> {
    let mut entries = fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| tr("Developer package contains an invalid file name.").to_string())?;
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        if kind.is_dir() {
            hash.update(b"directory");
            digest_tree(&entry.path(), hash)?;
            hash.update(b"end");
        } else if kind.is_file() {
            let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
            hash.update(b"file");
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        } else {
            return Err(
                tr("Developer package must contain only regular files and directories.").into(),
            );
        }
    }
    Ok(())
}

/// The wasm standard library the packager copies from beside the game program, on every platform
/// (it is not an executable, so it has no `EXE_SUFFIX`).
const WASM_STD: &str = "horizon-wasm-std.wasm";

pub fn install(checkout: &Path, program: &Path, trusted: bool) -> Result<String, String> {
    if !trusted {
        return Err(tr("Developer installation runs the checkout's build code and an unsigned game with your rights. Pass --trust-unsigned only if you trust both.").into());
    }
    let checkout = checkout.canonicalize().map_err(|e| e.to_string())?;
    let program = program.canonicalize().map_err(|e| e.to_string())?;
    if program.file_name().and_then(|n| n.to_str()) != Some(backend::CLIENT)
        || !crate::build_import::holds_game(&program)
    {
        return Err(tr("Choose a real horizon-client executable built for this computer.").into());
    }
    let mut missing = Vec::new();
    for name in ["horizon-ctl", "horizon-prepare", "horizon-server"] {
        let name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        if !program.with_file_name(&name).is_file() {
            missing.push(name);
        }
    }
    if !checkout.join("exporters/manifest.json").is_file() {
        missing.push("exporters/manifest.json".into());
    }
    // The packager copies the wasm standard library from beside the game, whatever mods follow.
    let wasm_std = program.with_file_name(WASM_STD);
    let wasm_std_missing = !wasm_std.is_file();
    if wasm_std_missing {
        missing.push(WASM_STD.into());
    }
    if !missing.is_empty() {
        let mut message = trf(
            "Missing developer packaging inputs: {files}. Build companions with cargo build -p horizon-client-lib --bin horizon-ctl -p horizon-prepare -p horizon-server. Run horizon-prepare compile-exporters --root CHECKOUT, then copy its server, client and manifest.json into CHECKOUT/exporters/.",
            &[("files", &missing.join(", "))],
        );
        if wasm_std_missing {
            message.push(' ');
            message.push_str(&trf("Generate the wasm standard library from the same checkout with cargo run --locked -p cargo-horizon-mod -- wasm-std {path}", &[("path", &wasm_std.display().to_string())]));
        }
        return Err(message);
    }
    let (revision, snapshot) = committed_snapshot(&checkout)?;
    let _publication =
        crate::run_leases::installation().map_err(crate::native_mods::cache_error)?;
    let root = backend::version_dir("");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix(".developer-")
        .tempdir_in(&root)
        .map_err(|e| e.to_string())?;
    let package = staging.path().join("package");
    let status = Command::new("cargo")
        .current_dir(&checkout)
        .args(["run", "--locked", "-p", "xtask", "--", "package"])
        .arg(&program)
        .arg(&package)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(tr(
            "The checkout's release packager failed; no developer build was installed.",
        )
        .into());
    }
    if !horizon_prepare::exporters::available(&package) {
        return Err(tr("The packaged exporters are missing, changed or do not match the checkout sources. Recompile the exporters before installing.").into());
    }
    fs::write(
        package.join("developer.json"),
        serde_json::json!({"revision": revision}).to_string(),
    )
    .map_err(|e| e.to_string())?;
    fs::write(package.join("developer-source.tar"), snapshot).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    digest_tree(&package, &mut hash)?;
    let tag = format!("developer-{:x}", hash.finalize());
    let destination = root.join(&tag);
    if !destination.exists() {
        crate::run_leases::publish(&package).map_err(crate::native_mods::cache_error)?;
        fs::write(package.join(".installed"), "").map_err(|e| e.to_string())?;
        fs::rename(package, destination).map_err(|e| e.to_string())?;
    }
    Ok(tag)
}

pub(crate) fn committed_snapshot(checkout: &Path) -> Result<(String, Vec<u8>), String> {
    let revision = Command::new("git")
        .current_dir(checkout)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| e.to_string())?;
    if !revision.status.success() {
        return Err(tr("The developer checkout has no HEAD commit.").into());
    }
    let revision = String::from_utf8(revision.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .to_owned();
    let clean = Command::new("git")
        .current_dir(checkout)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output()
        .map_err(|e| e.to_string())?;
    if !clean.status.success() || !clean.stdout.is_empty() {
        return Err(tr("The developer checkout has dirty tracked inputs. Commit your changes first, then rebuild the game and retry.").into());
    }
    let snapshot = Command::new("git")
        .current_dir(checkout)
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.eol=lf",
            "archive",
            "--format=tar",
            "HEAD",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !snapshot.status.success() {
        return Err(tr("Could not snapshot the developer checkout's committed source.").into());
    }
    Ok((revision, snapshot.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn digest(root: &Path) -> Vec<u8> {
        let mut hash = Sha256::new();
        digest_tree(root, &mut hash).unwrap();
        hash.finalize().to_vec()
    }
    #[test]
    fn dirty_tracked_checkout_is_refused_but_local_commit_is_snapshotted() {
        let folder = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .current_dir(folder.path())
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q"]);
        fs::write(folder.path().join("input"), "committed").unwrap();
        git(&["add", "input"]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "local only",
        ]);
        let (commit, snapshot) = committed_snapshot(folder.path()).unwrap();
        assert_eq!(commit.len(), 40);
        let output = tempfile::tempdir().unwrap();
        tar::Archive::new(snapshot.as_slice())
            .unpack(output.path())
            .unwrap();
        assert_eq!(
            fs::read_to_string(output.path().join("input")).unwrap(),
            "committed"
        );
        fs::write(folder.path().join("input"), "dirty").unwrap();
        assert!(committed_snapshot(folder.path())
            .unwrap_err()
            .contains("Commit your changes first"));
    }
    /// A checkout with the exporters manifest, and a game program beside the given companions.
    fn packaging_inputs(
        companions: &[&str],
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let folder = tempfile::tempdir().unwrap();
        let checkout = folder.path().join("checkout");
        fs::create_dir_all(checkout.join("exporters")).unwrap();
        fs::write(checkout.join("exporters/manifest.json"), "{}").unwrap();
        let bin = folder.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let magic: &[u8] = if cfg!(windows) {
            b"MZ"
        } else if cfg!(target_os = "macos") {
            &[0xcf, 0xfa, 0xed, 0xfe]
        } else {
            b"ELF"
        };
        let program = bin.join(backend::CLIENT);
        fs::write(
            &program,
            [magic, &horizon_mod_package::game_identity::MARK[..]].concat(),
        )
        .unwrap();
        for name in companions {
            fs::write(
                bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
                "",
            )
            .unwrap();
        }
        (folder, checkout, program)
    }
    const COMPANIONS: [&str; 3] = ["horizon-ctl", "horizon-prepare", "horizon-server"];
    const REMEDY: &str = "Build companions with cargo build -p horizon-client-lib --bin horizon-ctl -p horizon-prepare -p horizon-server. Run horizon-prepare compile-exporters --root CHECKOUT, then copy its server, client and manifest.json into CHECKOUT/exporters/.";
    #[test]
    fn a_missing_wasm_standard_library_is_named_with_its_generation_command_before_packaging() {
        let (_folder, checkout, program) = packaging_inputs(&COMPANIONS);
        let error = install(&checkout, &program, true).unwrap_err();
        let expected_path = program
            .canonicalize()
            .unwrap()
            .with_file_name("horizon-wasm-std.wasm");
        assert_eq!(
            error,
            format!(
                "Missing developer packaging inputs: horizon-wasm-std.wasm. {REMEDY} Generate the wasm standard library from the same checkout with cargo run --locked -p cargo-horizon-mod -- wasm-std {}",
                expected_path.display()
            )
        );
    }
    #[test]
    fn the_wasm_standard_library_is_reported_beside_the_other_missing_inputs() {
        let (_folder, checkout, program) = packaging_inputs(&["horizon-ctl"]);
        let error = install(&checkout, &program, true).unwrap_err();
        let missing = [
            format!("horizon-prepare{}", std::env::consts::EXE_SUFFIX),
            format!("horizon-server{}", std::env::consts::EXE_SUFFIX),
            "horizon-wasm-std.wasm".to_owned(),
        ]
        .join(", ");
        assert!(
            error.starts_with(&format!("Missing developer packaging inputs: {missing}. {REMEDY} Generate the wasm standard library")),
            "{error}"
        );
    }
    #[test]
    fn a_present_wasm_standard_library_passes_the_preflight_without_a_hint() {
        let (_folder, checkout, program) = packaging_inputs(&COMPANIONS);
        fs::write(program.with_file_name("horizon-wasm-std.wasm"), "").unwrap();
        // The checkout is no git repository, so the install stops at its snapshot, before the packager.
        assert_eq!(
            install(&checkout, &program, true).unwrap_err(),
            "The developer checkout has no HEAD commit."
        );
        let (_other, checkout, program) = packaging_inputs(&["horizon-ctl", "horizon-prepare"]);
        fs::write(program.with_file_name("horizon-wasm-std.wasm"), "").unwrap();
        let error = install(&checkout, &program, true).unwrap_err();
        assert!(
            error.starts_with("Missing developer packaging inputs: horizon-server"),
            "{error}"
        );
        assert!(
            !error.contains("Generate the wasm standard library"),
            "{error}"
        );
    }
    #[test]
    fn consent_precedes_checkout_access() {
        let error = install(Path::new("missing"), Path::new("missing"), false).unwrap_err();
        assert!(error.contains("--trust-unsigned"));
    }
    #[test]
    fn digest_covers_names_contents_and_nested_inputs() {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path();
        fs::create_dir(root.join("exporters")).unwrap();
        fs::write(root.join("horizon-client"), b"game A").unwrap();
        fs::write(root.join("exporters/manifest.json"), b"manifest A").unwrap();
        let original = digest(root);
        assert_eq!(original, digest(root));
        fs::write(root.join("horizon-client"), b"game B").unwrap();
        assert_ne!(original, digest(root));
        fs::write(root.join("horizon-client"), b"game A").unwrap();
        fs::write(root.join("exporters/manifest.json"), b"manifest B").unwrap();
        assert_ne!(original, digest(root));
        fs::write(root.join("exporters/manifest.json"), b"manifest A").unwrap();
        fs::rename(
            root.join("exporters/manifest.json"),
            root.join("exporters/other.json"),
        )
        .unwrap();
        assert_ne!(original, digest(root));
    }
    #[test]
    #[cfg(unix)]
    fn package_symlinks_are_refused() {
        let folder = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc/passwd", folder.path().join("outside")).unwrap();
        assert!(digest_tree(folder.path(), &mut Sha256::new()).is_err());
    }
}
