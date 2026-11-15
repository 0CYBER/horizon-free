//! Runtime native outputs come from Cargo messages, never guessed linker arguments.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(tag = "reason", rename_all = "kebab-case")]
enum CargoEvent {
    BuildScriptExecuted {
        package_id: String,
        linked_paths: Vec<String>,
    },
    CompilerArtifact {
        package_id: String,
        filenames: Vec<PathBuf>,
        target: Target,
    },
    CompilerMessage {
        message: Diagnostic,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
}
#[derive(Deserialize)]
struct Diagnostic {
    rendered: Option<String>,
}

pub(super) struct Outputs {
    scripts: Vec<(String, Vec<String>)>,
    libraries: Vec<(String, Vec<PathBuf>)>,
    pub errors: Vec<String>,
}

fn dynamic(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    [".so", ".dll", ".dylib"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || name.contains(".so.")
}

impl Outputs {
    pub fn read(path: &Path) -> Result<Self, String> {
        let mut outputs = Self {
            scripts: Vec::new(),
            libraries: Vec::new(),
            errors: Vec::new(),
        };
        for line in BufReader::new(fs::File::open(path).map_err(|e| e.to_string())?).lines() {
            let line = line.map_err(|e| e.to_string())?;
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str(&line).map_err(|e| format!("Cargo build messages: {e}"))? {
                CargoEvent::BuildScriptExecuted {
                    package_id,
                    linked_paths,
                } => outputs.scripts.push((package_id, linked_paths)),
                CargoEvent::CompilerArtifact {
                    package_id,
                    filenames,
                    target,
                } if !target.kind.iter().any(|kind| kind == "proc-macro") => {
                    outputs.libraries.push((package_id, filenames))
                }
                CargoEvent::CompilerMessage { message } => {
                    if let Some(rendered) = message.rendered {
                        outputs.errors.push(rendered);
                    }
                }
                _ => {}
            }
        }
        Ok(outputs)
    }

    pub fn runtime(
        &self,
        runtime: &BTreeSet<String>,
        target: &Path,
    ) -> Result<Vec<PathBuf>, String> {
        let roots = [target.to_path_buf(), crate::rust_toolchain::cargo_home()]
            .into_iter()
            .filter(|root| root.exists())
            .map(|root| root.canonicalize().map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut candidates = Vec::new();
        for (id, files) in &self.libraries {
            if runtime.contains(id) {
                candidates.extend(files.iter().filter(|file| dynamic(file)).cloned());
            }
        }
        for (_, paths) in self.scripts.iter().filter(|(id, _)| runtime.contains(id)) {
            for search in paths {
                let (kind, path) = search.split_once('=').unwrap_or(("native", search));
                let path = Path::new(path);
                if !path.is_absolute() {
                    return Err(format!(
                        "Native dependency emitted relative library search path {}",
                        path.display()
                    ));
                }
                let canonical = path
                    .canonicalize()
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                if !roots.iter().any(|root| canonical.starts_with(root)) {
                    continue;
                } // System libraries stay system libraries.
                if kind == "framework" {
                    return Err(format!(
                        "Bundled framework output {} is not supported by native mod packaging",
                        path.display()
                    ));
                }
                for entry in fs::read_dir(canonical).map_err(|e| e.to_string())? {
                    let file = entry.map_err(|e| e.to_string())?.path();
                    if dynamic(&file) {
                        candidates.push(file);
                    }
                }
            }
        }
        let mut files = BTreeMap::new();
        for path in candidates {
            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
            if !roots.iter().any(|root| canonical.starts_with(root)) {
                return Err(format!(
                    "Native output {} escapes the managed build",
                    path.display()
                ));
            }
            let name = path
                .file_name()
                .ok_or("Native output has no filename")?
                .to_owned();
            if let Some(previous) = files.get(&name) {
                if fs::read(previous).map_err(|e| e.to_string())?
                    != fs::read(&path).map_err(|e| e.to_string())?
                {
                    return Err(format!(
                        "Conflicting native runtime outputs named {}",
                        name.to_string_lossy()
                    ));
                }
            } else {
                files.insert(name, path);
            }
        }
        Ok(files.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_outputs_exclude_host_tools_and_preserve_native_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        // read_dir returns canonical-prefixed paths on Windows. Match that representation
        // without canonicalizing individual files, which would erase Unix aliases.
        fs::create_dir(&target).unwrap();
        let target = target.canonicalize().unwrap();
        let library = target.join("libnative.so.1");
        fs::write(&library, b"native").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&library, target.join("libnative.so")).unwrap();
        let host = dir.path().join("host.so");
        fs::write(&host, b"host").unwrap();
        let messages = dir.path().join("messages");
        let rows = [
            serde_json::json!({"reason":"build-script-executed","package_id":"runtime","linked_paths":[format!("native={}",target.display())]}),
            serde_json::json!({"reason":"compiler-artifact","package_id":"host","filenames":[host],"target":{"kind":["proc-macro"]}}),
        ];
        fs::write(
            &messages,
            rows.iter()
                .map(|row| row.to_string() + "\n")
                .collect::<String>(),
        )
        .unwrap();
        let files = Outputs::read(&messages)
            .unwrap()
            .runtime(&BTreeSet::from(["runtime".into()]), &target)
            .unwrap();
        assert!(files.contains(&library));
        assert!(!files.contains(&host));
        #[cfg(unix)]
        assert!(files.contains(&target.join("libnative.so")));
    }
    #[test]
    fn escaping_and_conflicting_runtime_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir_all(target.join("one")).unwrap();
        fs::create_dir_all(target.join("two")).unwrap();
        let one = target.join("one/library.so");
        let two = target.join("two/library.so");
        fs::write(&one, b"one").unwrap();
        fs::write(&two, b"two").unwrap();
        let mut outputs = Outputs {
            scripts: vec![],
            libraries: vec![("runtime".into(), vec![one, two])],
            errors: vec![],
        };
        let runtime = BTreeSet::from(["runtime".into()]);
        assert!(outputs
            .runtime(&runtime, &target)
            .unwrap_err()
            .contains("Conflicting"));
        let outside = dir.path().join("outside.so");
        fs::write(&outside, b"outside").unwrap();
        outputs.libraries[0].1 = vec![outside];
        assert!(outputs
            .runtime(&runtime, &target)
            .unwrap_err()
            .contains("escapes"));
        outputs.libraries.clear();
        outputs.scripts = vec![("runtime".into(), vec!["native=relative".into()])];
        assert!(outputs
            .runtime(&runtime, &target)
            .unwrap_err()
            .contains("relative"));
    }
}
