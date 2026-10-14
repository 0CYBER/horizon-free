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

/// Finds a JDK with `javac` of at least `major`, the version research-lock.json pins:
/// `JAVA_HOME` when it holds one, else `javac` and `java` on the PATH (as the Python preparer
/// ran them), else, on macOS, the JDK `/usr/libexec/java_home -v <major>` reports.
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
