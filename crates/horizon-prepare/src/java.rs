//! The JDK that compiles and runs the vanilla exporters. They call Mojang's own code, so they
//! stay Java; everything around them is here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub struct Jdk {
    pub javac: PathBuf,
    pub java: PathBuf,
}

const TIMEOUT: Duration = Duration::from_secs(3600);

fn executable(dir: &Path, name: &str) -> PathBuf {
    dir.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    })
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| executable(&dir, name))
        .find(|p| p.is_file())
}

/// The major version `javac -version` reports ("javac 25.0.1" → 25; "javac 1.8.0" → 8).
fn javac_major(javac: &Path) -> Result<u32, String> {
    let output = Command::new(javac)
        .arg("-version")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", javac.display()))?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    let version = text
        .split_whitespace()
        .skip_while(|w| *w != "javac")
        .nth(1)
        .ok_or_else(|| {
            format!(
                "{} -version printed no version: {}",
                javac.display(),
                text.trim()
            )
        })?;
    let mut parts = version.split(['.', '-', '+', '_']);
    let first: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Ok(if first == 1 {
        parts.next().and_then(|p| p.parse().ok()).unwrap_or(0)
    } else {
        first
    })
}

/// The folders JDK installers and tools put JDKs in, each JDK one folder below (#666: a
/// player installed Java 25 but JAVA_HOME and the PATH still named an older one).
fn jdk_parents() -> Vec<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let mut parents = Vec::new();
    if cfg!(windows) {
        let vendors = [
            "Eclipse Adoptium",
            "Java",
            "Microsoft",
            "Zulu",
            "BellSoft",
            "Amazon Corretto",
            "Semeru",
            "OpenJDK",
        ];
        for base in ["ProgramFiles", "ProgramW6432"].into_iter().filter_map(var) {
            parents.extend(vendors.iter().map(|v| base.join(v)));
        }
    } else if cfg!(target_os = "macos") {
        parents.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        parents.extend(
            home.iter()
                .map(|h| h.join("Library/Java/JavaVirtualMachines")),
        );
    } else {
        parents.extend(["/usr/lib/jvm", "/usr/java", "/usr/local/lib/jvm"].map(PathBuf::from));
    }
    parents.extend(
        home.iter()
            .flat_map(|h| [h.join(".jdks"), h.join(".sdkman/candidates/java")]),
    );
    parents
}

/// The JDKs (folders with `bin/javac`) one level below `parents`; a macOS bundle's home is
/// its `Contents/Home`. Those named for `major` come first, then the others newest name first.
fn installed_jdks(parents: &[PathBuf], major: i64) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = parents
        .iter()
        .filter_map(|parent| fs::read_dir(parent).ok())
        .flatten()
        .flatten()
        .map(|entry| {
            let bundle = entry.path().join("Contents/Home");
            if bundle.is_dir() {
                bundle
            } else {
                entry.path()
            }
        })
        .filter(|home| executable(&home.join("bin"), "javac").is_file())
        .collect();
    let named = |home: &Path| {
        home.iter().any(|part| {
            part.to_string_lossy()
                .split(|c: char| !c.is_ascii_digit())
                .any(|n| n == major.to_string())
        })
    };
    homes.sort_by(|a, b| named(b).cmp(&named(a)).then_with(|| b.cmp(a)));
    homes.dedup();
    homes
}

/// Finds a JDK with `javac` of at least `major`, the version research-lock.json pins:
/// `JAVA_HOME` when it holds one, else `javac` and `java` on the PATH (as the Python preparer
/// ran them), else, on macOS, the JDK `/usr/libexec/java_home -v <major>` reports, else a JDK
/// in the folders installers put them in (`jdk_parents`).
pub fn find(major: i64) -> Result<Jdk, String> {
    let need = format!(
        "Java {major} is needed (a JDK with javac, e.g. Eclipse Temurin {major}). \
         Install it, or set JAVA_HOME to it, and try again."
    );
    let mut candidates = Vec::new();
    if let Some(home) = std::env::var_os("JAVA_HOME").filter(|h| !h.is_empty()) {
        let bin = Path::new(&home).join("bin");
        candidates.push((
            executable(&bin, "javac"),
            executable(&bin, "java"),
            "JAVA_HOME",
        ));
    }
    if let (Some(javac), Some(java)) = (on_path("javac"), on_path("java")) {
        candidates.push((javac, java, "PATH"));
    }
    if cfg!(target_os = "macos") {
        if let Ok(out) = Command::new("/usr/libexec/java_home")
            .args(["-v", &major.to_string()])
            .stderr(Stdio::null())
            .output()
        {
            let home = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !home.is_empty() {
                let bin = Path::new(&home).join("bin");
                candidates.push((
                    executable(&bin, "javac"),
                    executable(&bin, "java"),
                    "/usr/libexec/java_home",
                ));
            }
        }
    }
    for home in installed_jdks(&jdk_parents(), major) {
        let bin = home.join("bin");
        candidates.push((
            executable(&bin, "javac"),
            executable(&bin, "java"),
            "an installed JDK",
        ));
    }
    let mut problems = Vec::new();
    for (javac, java, from) in candidates {
        if !javac.is_file() || !java.is_file() {
            problems.push(format!(
                "{from} ({}) has no javac and java",
                javac.parent().unwrap().display()
            ));
            continue;
        }
        match javac_major(&javac) {
            Ok(found) if i64::from(found) >= major => return Ok(Jdk { javac, java }),
            Ok(found) => problems.push(format!("{from} has Java {found} ({})", javac.display())),
            Err(e) => problems.push(e),
        }
    }
    if problems.is_empty() {
        Err(format!("{need} No javac was found."))
    } else {
        Err(format!("{need} Found: {}.", problems.join("; ")))
    }
}

/// A classpath with the platform's separator.
pub fn classpath<'a>(paths: impl IntoIterator<Item = &'a Path>) -> String {
    let separator = if cfg!(windows) { ";" } else { ":" };
    paths
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(separator)
}

impl Jdk {
    /// `javac --release <major> -proc:none` with Python's `subprocess.run(..., check=True,
    /// stdout=log, stderr=STDOUT, timeout=3600)`.
    pub fn javac(&self, major: i64, args: &[String], log: &fs::File) -> Result<(), String> {
        let mut command = Command::new(&self.javac);
        command
            .args(["--release", &major.to_string(), "-proc:none"])
            .args(args);
        run(command, log)
    }

    pub fn java(&self, args: &[String], cwd: Option<&Path>, log: &fs::File) -> Result<(), String> {
        let mut command = Command::new(&self.java);
        command.args(args);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        run(command, log)
    }
}

fn run(mut command: Command, log: &fs::File) -> Result<(), String> {
    let out = log.try_clone().map_err(|e| e.to_string())?;
    let err = log.try_clone().map_err(|e| e.to_string())?;
    let program = Path::new(command.get_program())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
        .map_err(|e| format!("Cannot start {program}: {e}"))?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("{program} failed ({status})"))
            };
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} took over an hour and was stopped"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_jdks_prefer_the_pinned_major() {
        let dir = std::env::temp_dir().join(format!("horizon-prepare-jdks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let parent = dir.join("Eclipse Adoptium");
        for jdk in [
            "jdk-21.0.5.11-hotspot",
            "jdk-25.0.1.8-hotspot",
            "jdk-26.0.0-hotspot",
            "jre-25.0.1",
        ] {
            let bin = parent.join(jdk).join("bin");
            fs::create_dir_all(&bin).unwrap();
            if jdk.starts_with("jdk") {
                fs::write(executable(&bin, "javac"), b"").unwrap();
            }
        }
        // A macOS bundle keeps its home in Contents/Home.
        let bundle = dir.join("JavaVirtualMachines/temurin-25.jdk/Contents/Home");
        fs::create_dir_all(bundle.join("bin")).unwrap();
        fs::write(executable(&bundle.join("bin"), "javac"), b"").unwrap();
        let found = installed_jdks(
            &[
                parent.clone(),
                dir.join("JavaVirtualMachines"),
                dir.join("missing"),
            ],
            25,
        );
        assert_eq!(
            found,
            [
                bundle,
                parent.join("jdk-25.0.1.8-hotspot"),
                parent.join("jdk-26.0.0-hotspot"),
                parent.join("jdk-21.0.5.11-hotspot"),
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
