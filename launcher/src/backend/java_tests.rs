use super::*;

#[test]
fn selected_jdk_reaches_external_preparers() {
    const CHILD: &str = "HORIZON_LAUNCHER_JAVA_TEST";
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        let home = java(25, true, &[], &root.join("missing.json"), &|_| {}).unwrap();
        assert_eq!(
            fs::canonicalize(&home).unwrap(),
            fs::canonicalize(root.join("jdk25")).unwrap()
        );
        // An older preparer invokes Java tools by name; both must now resolve to this JDK.
        for name in ["java", "javac"] {
            let mut cmd = crate::process::command(name);
            with_java(&mut cmd, &home);
            let output = cmd.arg("-version").output().unwrap();
            assert!(output.status.success());
            let text = String::from_utf8_lossy(&output.stdout).into_owned()
                + &String::from_utf8_lossy(&output.stderr);
            assert!(text.contains("25.0.1"), "{name}: {text}");
            assert_eq!(
                cmd.get_envs()
                    .find(|(key, _)| *key == "JAVA_HOME")
                    .unwrap()
                    .1,
                Some(home.as_os_str())
            );
        }
        return;
    }

    let root = std::env::temp_dir().join(format!("horizon-launcher-java-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let executable =
        |bin: &Path, name: &str| bin.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let probe = executable(&root, "probe");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/horizon-prepare/tests/support/java_probe.rs");
    let output = crate::process::command("rustc")
        .arg(source)
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
        ("jdk25", Some("25.0.1"), "25.0.1"),
    ] {
        let bin = root.join(home).join("bin");
        fs::create_dir_all(&bin).unwrap();
        for (name, version) in [("javac", compiler), ("java", Some(runtime))] {
            if let Some(version) = version {
                fs::copy(&probe, executable(&bin, name)).unwrap();
                fs::write(bin.join(format!("{name}.version")), version).unwrap();
            }
        }
    }
    let output = crate::process::command(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "backend::java_tests::selected_jdk_reaches_external_preparers",
            "--nocapture",
        ])
        .env(CHILD, &root)
        .env_remove("JAVA_HOME")
        .env(
            "PATH",
            std::env::join_paths([root.join("jre21/bin"), root.join("jdk25/bin")]).unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(root).unwrap();
}
