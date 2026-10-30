//! Each scenario runs in its own process so Java discovery sees a controlled environment.
use horizon_prepare::java;
use std::{env, fs, path::Path, process::Command};

const TEST: &str = "selects_a_compatible_compiler_and_runtime";
const CHILD: &str = "HORIZON_JAVA_SELECTION_CHILD";
const ROOT: &str = "HORIZON_JAVA_SELECTION_ROOT";

fn tool(bin: &Path, name: &str) -> std::path::PathBuf {
    bin.join(format!("{name}{}", env::consts::EXE_SUFFIX))
}

#[test]
fn selects_a_compatible_compiler_and_runtime() {
    if let Ok(scenario) = env::var(CHILD) {
        let root = std::path::PathBuf::from(env::var_os(ROOT).unwrap());
        if scenario.ends_with("error") {
            // Use a future compiler version to keep real installed JDKs out of the result.
            let error = java::find(255, &[]).err().expect("accepted an old runtime");
            let runtime = if scenario == "legacy-runtime-error" {
                8
            } else {
                21
            };
            assert!(error.contains(&format!("java {runtime} (")), "{error}");
            return;
        }
        let jdk = if scenario == "runtime-only" {
            java::find_runtime(25, &[])
        } else {
            java::find(25, &[])
        }
        .unwrap();
        let expected = root.join(if scenario == "runtime-only" {
            "jre25"
        } else if scenario == "target-release" {
            "newer"
        } else {
            "jdk25"
        });
        for (actual, name) in [(&jdk.javac, "javac"), (&jdk.java, "java")] {
            if scenario == "runtime-only" && name == "javac" {
                continue;
            }
            assert_eq!(
                fs::canonicalize(actual).unwrap(),
                fs::canonicalize(tool(&expected.join("bin"), name)).unwrap()
            );
        }
        assert_eq!(
            fs::canonicalize(jdk.home()).unwrap(),
            fs::canonicalize(expected).unwrap()
        );
        let log_path = root.join(format!("{scenario}.log"));
        let log = fs::File::create(&log_path).unwrap();
        jdk.java(&[], None, &log).unwrap();
        assert_eq!(fs::read_to_string(log_path).unwrap().trim(), "java 25.0.1");
        return;
    }

    let root = env::temp_dir().join(format!("horizon-java-selection-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let probe = tool(&root, "probe");
    let output = Command::new("rustc")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support/java_probe.rs"))
        .arg("-o")
        .arg(&probe)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for (home, compiler, runtime) in [
        ("jre21", None, "21.0.1"),
        ("jre25", None, "25.0.1"),
        ("jdk21", Some("21.0.1"), "21.0.1"),
        ("jdk25", Some("25.0.1"), "25.0.1"),
        ("mixed", Some("25.0.1"), "21.0.1"),
        ("failing", Some("25.0.1"), "25.0.1"),
        ("newer", Some("26-ea+1"), "25.0.1"),
        ("future", Some("255.0.1"), "21.0.1"),
        ("legacy", Some("255.0.1"), "1.8.0_442"),
    ] {
        let bin = root.join(home).join("bin");
        fs::create_dir_all(&bin).unwrap();
        for (name, version) in [("javac", compiler), ("java", Some(runtime))] {
            if let Some(version) = version {
                fs::copy(&probe, tool(&bin, name)).unwrap();
                fs::write(bin.join(format!("{name}.version")), version).unwrap();
            }
        }
    }
    fs::write(root.join("failing/bin/java.exit"), "7").unwrap();
    let scenarios = vec![
        ("runtime-only", Some("jre25"), vec!["jre21", "jre25"]),
        ("older-runtime", None, vec!["jre21", "jdk25"]),
        ("older-jdk", None, vec!["jdk21", "jdk25"]),
        ("incompatible-home", Some("mixed"), vec!["jdk25"]),
        ("failed-runtime", Some("failing"), vec!["jdk25"]),
        ("target-release", Some("newer"), vec![]),
        ("runtime-error", Some("future"), vec![]),
        ("legacy-runtime-error", Some("legacy"), vec![]),
    ];
    #[cfg(unix)]
    let scenarios = {
        let mut scenarios = scenarios;
        let bin = root.join("shims/bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(tool(&root.join("jdk25/bin"), "javac"), tool(&bin, "javac"))
            .unwrap();
        std::os::unix::fs::symlink(tool(&root.join("jre21/bin"), "java"), tool(&bin, "java"))
            .unwrap();
        scenarios.push(("symlinked-path", None, vec!["shims"]));
        scenarios
    };
    for (scenario, home, path) in scenarios {
        let mut child = Command::new(env::current_exe().unwrap());
        child
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, scenario)
            .env(ROOT, &root)
            .env(
                "PATH",
                env::join_paths(path.iter().map(|home| root.join(home).join("bin"))).unwrap(),
            )
            .env("HOME", &root)
            .env("USERPROFILE", &root)
            .env("ProgramFiles", &root)
            .env("ProgramW6432", &root)
            .env_remove("JAVA_HOME");
        if let Some(home) = home {
            child.env("JAVA_HOME", root.join(home));
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{scenario}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::remove_dir_all(root).unwrap();
}
