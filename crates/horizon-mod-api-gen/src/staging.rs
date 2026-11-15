//! One source staging pass for the launcher and `horizon-stage`.
use crate::{internals, points, replace};
/// The build a staged copy is of, named in each build-data file's header.
pub use point_shape::build_data::Build;
use point_shape::build_data::Document;
use std::{path::Path, time::Instant};

/// Discovery instrumentation must never be used to build or publish a game.
#[derive(Clone, Copy)]
pub enum Mode {
    /// Widen visibility, mark injection points and give functions their replacement slots,
    /// without diagnostic attributes.
    Build,
    /// Add internal-use diagnostics for an isolated metadata-only mod check.
    Discovery,
}

/// Counts and catalog produced by source staging.
pub struct Staged {
    /// Injection point counts.
    pub points: points::Staged,
    /// Replacement slot counts.
    pub replace: replace::Staged,
    /// Exposed internal signatures and changed visibility counts.
    pub internals: internals::Widened,
}

/// Expose internals, mark points, give functions their replacement slots and write the build
/// data (`points.json`, `internals.json`, `loops.json`, `functions.json`, each with the header
/// naming `build`) in a
/// disposable source copy. Internals precede points so their catalog contains only the
/// original game items, and points precede the slots so only game functions are points.
/// A pass stamps the copy before edits; even a failed pass requires fresh source for a retry.
pub fn stage(
    root: &Path,
    points_json: &Path,
    internals_json: &Path,
    mode: Mode,
    build: &Build,
) -> Result<Staged, String> {
    let stamp = root.join(".horizon-source-staged");
    if stamp.exists() {
        return Err("source tree is already staged; use a fresh copy of the source".into());
    }
    std::fs::write(
        &stamp,
        "Source staging started. Use a fresh copy to stage again.\n",
    )
    .map_err(|error| format!("{}: {error}", stamp.display()))?;
    // Listed before any pass rewrites the source: the markers are the source's.
    let started = Instant::now();
    #[derive(serde::Serialize)]
    struct Loops {
        loops: Vec<crate::loops::PublishedLoop>,
    }
    let loops = Loops {
        loops: crate::loops::collect(root)?,
    };
    eprintln!(
        "stage_phase=loops seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    // Indexed before any pass edits a file: its spans and hashes are of the source a mixin
    // build downloads.
    let started = Instant::now();
    let generic: std::collections::BTreeSet<String> = loops
        .loops
        .iter()
        .map(|l| crate::functions::key(&l.generic_target()))
        .collect();
    let mut functions = crate::functions::index(root, &generic)?;
    eprintln!(
        "stage_phase=functions seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    write(
        &points_json.with_file_name(crate::loops::FILE),
        &Document::new(build, loops),
    )?;
    let started = Instant::now();
    let mut internals = match mode {
        Mode::Build => internals::widen(root)?,
        Mode::Discovery => internals::discover(root)?,
    };
    internals
        .entries
        .extend(crate::crossing::foreign_internals());
    #[derive(serde::Serialize)]
    struct Internals<'a> {
        internals: &'a [internals::Internal],
    }
    write(
        internals_json,
        &Document::new(
            build,
            Internals {
                internals: &internals.entries,
            },
        ),
    )?;
    eprintln!(
        "stage_phase=internals seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let wire = crate::build::wire_types(root)?;
    eprintln!(
        "stage_phase=wire seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let (points, records) = points::stage_with_manifest(root, &wire)?;
    eprintln!(
        "stage_phase=points seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    #[derive(serde::Serialize)]
    struct Manifest {
        points: Vec<points::GamePoint>,
        /// The `mod_api` types with a `Wire` layout: what an engine point's API record may hold.
        wire: std::collections::BTreeSet<String>,
    }
    write(
        points_json,
        &Document::new(
            build,
            Manifest {
                points: records,
                wire,
            },
        ),
    )?;
    eprintln!(
        "stage_phase=point_manifest seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    let started = Instant::now();
    let replace = replace::stage(root)?;
    eprintln!(
        "stage_phase=replace seconds={:.3}",
        started.elapsed().as_secs_f64()
    );
    functions.retain_slotted(&replace.slots, &generic);
    // One line: the index is the largest build-data file, and only programs read it.
    let path = points_json.with_file_name(crate::functions::FILE);
    let text = serde_json::to_string(&Document::new(build, &functions))
        .map_err(|error| error.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Staged {
        points,
        replace,
        internals,
    })
}

fn write(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, text + "\n").map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(root: &Path, points: &Path, internals: &Path, mode: Mode) -> Result<Staged, String> {
        let build = Build {
            commit: "c".repeat(40),
        };
        super::stage(root, points, internals, mode, &build)
    }

    #[test]
    fn staging_marks_points_and_slots_and_refuses_reuse() {
        // Each iteration starts from fresh source, not the preceding staged fixture.
        for copy in 0..3 {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().to_path_buf();
            let source = root.join("crates/horizon-client/src/game.rs");
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            std::fs::write(
                &source,
                "impl Game<'_> { fn unit(&self) {} fn optional(&self) -> Option<u32> { None } }",
            )
            .unwrap();
            std::fs::write(source.parent().unwrap().join("entry.rs"), "").unwrap();
            std::fs::write(
                source.parent().unwrap().join("lib.rs"),
                "mod game; mod entry;\n",
            )
            .unwrap();
            std::fs::write(
                root.join("crates/horizon-client/Cargo.toml"),
                "[package]\nname = 'horizon-client'\n[dependencies]\nhorizon-engine-slots = { path = '../horizon-engine-slots' }\n",
            )
            .unwrap();
            std::fs::write(
                root.join("Cargo.lock"),
                "version = 4\n\n[[package]]\nname = \"horizon-client\"\nversion = \"0.1.0\"\n",
            )
            .unwrap();
            let lists = root.join("crates/horizon-mod-api-gen");
            std::fs::create_dir_all(&lists).unwrap();
            std::fs::write(lists.join("internals.txt"), "").unwrap();
            let package = root.join("crates/horizon-engine-slots");
            std::fs::create_dir_all(&package).unwrap();
            std::fs::write(
                package.join("Cargo.toml"),
                "[package]\nname = 'horizon-engine-slots'\n",
            )
            .unwrap();
            let output = root.join("points.json");
            if copy == 2 {
                // Refuse retries even when a late output failure leaves edited source.
                std::fs::create_dir(&output).unwrap();
                assert!(stage(&root, &output, &root.join("internals.json"), Mode::Build).is_err());
            } else {
                stage(&root, &output, &root.join("internals.json"), Mode::Build).unwrap();
                let bytes = std::fs::read(&output).unwrap();
                assert_eq!(bytes.last(), Some(&b'\n'));
                let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                // Every build-data file opens with the header naming the build.
                for file in point_shape::build_data::FILES {
                    let text = std::fs::read_to_string(root.join(file)).unwrap();
                    let (build, _): (Build, serde_json::Value) =
                        point_shape::build_data::read(file, &text).unwrap();
                    assert_eq!(build.commit, "c".repeat(40), "{file}");
                }
                let points = manifest["points"].as_array().unwrap();
                for (name, returns) in [("unit", "cancel"), ("optional", "result")] {
                    let point = points.iter().find(|point| point["name"] == name).unwrap();
                    assert_eq!(point["returns"], returns);
                    assert_eq!(point["signature_hash"].as_str().unwrap().len(), 32);
                }
            }
            let before = std::fs::read(&source).unwrap();
            let text = String::from_utf8_lossy(&before);
            assert!(text.contains("horizon_mod_api_macros"));
            // The slots follow the points: a pass that failed between them left none.
            assert_eq!(
                text.contains("slot = 0") && text.contains("__fnr_unit"),
                copy != 2
            );
            for mode in [Mode::Build, Mode::Discovery] {
                let error = stage(&root, &output, &root.join("internals.json"), mode)
                    .err()
                    .expect("staging twice must fail");
                assert_eq!(
                    error,
                    "source tree is already staged; use a fresh copy of the source"
                );
                assert_eq!(std::fs::read(&source).unwrap(), before);
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}
