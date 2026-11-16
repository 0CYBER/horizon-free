//! Library identities are produced by compilation, then hashed at publication.
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Library {
    pub file: String,
    pub sha256: String,
    pub kind: String,
    #[serde(rename = "mod")]
    pub mod_id: String,
    pub identity: String,
}

pub(super) fn stamp(
    path: &Path,
    build: &str,
    id: &str,
    version: &str,
    patch: bool,
) -> Result<(), String> {
    let row = Library {
        file: path
            .file_name()
            .ok_or("invalid library path")?
            .to_string_lossy()
            .into_owned(),
        sha256: String::new(),
        kind: if patch { "patch" } else { "mod" }.into(),
        mod_id: id.into(),
        identity: format!("{build} {id} {version} {}", crate::rust_toolchain::VERSION),
    };
    fs::write(
        path.with_extension("identity.json"),
        serde_json::to_vec(&row).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub(super) fn validate_exports(path: &Path, id: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Ok(());
    }
    let parse = || -> Result<usize, String> {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let pe = object::read::pe::PeFile64::parse(bytes.as_slice()).map_err(|e| e.to_string())?;
        Ok(pe
            .export_table()
            .map_err(|e| e.to_string())?
            .map_or(0, |table| table.addresses().len()))
    };
    let count = parse().map_err(|e| {
        crate::i18n::trf(
            "Could not inspect native mod {mod}'s DLL exports: {error}",
            &[("mod", id), ("error", &e)],
        )
    })?;
    refuse_exports(id, count)
}

fn refuse_exports(id: &str, count: usize) -> Result<(), String> {
    if count > 65_535 {
        return Err(crate::i18n::trf(
            "Native mod {mod} has {count} DLL exports; Windows permits at most 65,535.",
            &[("mod", id), ("count", &count.to_string())],
        ));
    }
    Ok(())
}

pub(super) fn collect(paths: &[std::path::PathBuf]) -> Result<Vec<Library>, String> {
    use sha2::{Digest, Sha256};
    let mut rows = Vec::new();
    for path in paths {
        let stamp = path.with_extension("identity.json");
        if !stamp.exists() {
            continue;
        }
        let mut row: Library = serde_json::from_slice(&fs::read(stamp).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        row.sha256 = format!(
            "{:x}",
            Sha256::digest(fs::read(path).map_err(|e| e.to_string())?)
        );
        rows.push(row);
    }
    // Stable partition keeps dependency order within the mod libraries.
    rows.sort_by_key(|row| row.kind == "patch");
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_limit_names_the_mod_and_count() {
        assert!(refuse_exports("heavy", 65_535).is_ok());
        let error = refuse_exports("heavy", 65_536).unwrap_err();
        assert!(error.contains("heavy"), "{error}");
        assert!(error.contains("65536"), "{error}");
    }

    #[test]
    fn records_hashes_and_dependency_order_with_patch_last() {
        let tmp = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["dependency", "patch", "dependent"]
            .iter()
            .map(|n| tmp.path().join(n))
            .collect();
        for (i, p) in paths.iter().enumerate() {
            fs::write(p, b"abc").unwrap();
            stamp(
                p,
                "build",
                ["dependency", "horizon_patch", "dependent"][i],
                "1",
                i == 1,
            )
            .unwrap();
        }
        let rows = collect(&paths).unwrap();
        assert_eq!(
            rows.iter().map(|r| r.mod_id.as_str()).collect::<Vec<_>>(),
            ["dependency", "dependent", "horizon_patch"]
        );
        assert_eq!(
            rows[0].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let json = serde_json::to_value(&rows).unwrap();
        assert_eq!(json[0]["mod"], "dependency");
        assert_eq!(json[2]["kind"], "patch");
        assert_eq!(
            rows[0].identity,
            format!("build dependency 1 {}", crate::rust_toolchain::VERSION)
        );
    }
}
