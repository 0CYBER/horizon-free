use std::{fs, process::Command};

#[test]
#[ignore = "daily"]
fn git_project_build_checks_only_the_files_it_packages() {
    let root = std::env::temp_dir().join(format!("horizon-content-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let run = |command: &mut Command| {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let binary = env!("CARGO_BIN_EXE_cargo-horizon-mod");
    run(Command::new(binary)
        .current_dir(&root)
        .args(["new", "ruby"]));
    let project = root.join("ruby");
    run(Command::new("git")
        .current_dir(&project)
        .args(["init", "--quiet"]));
    // Arbitrary project files, not a list of special ignored directory names.
    fs::create_dir_all(project.join("work")).unwrap();
    fs::write(project.join("work/cache.bin"), b"\0asm").unwrap();
    run(Command::new(binary).current_dir(&project).arg("build"));
    run(Command::new(binary).current_dir(&project).arg("build"));
    let path = project.join("target/horizon-mod/ruby.hmod");
    let package = horizon_mod_package::Package::read(&path).unwrap();
    assert_eq!(package.manifest.id, "ruby");
    let zip = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
    assert!(zip.file_names().all(|name| {
        let name = name.unwrap();
        name == "horizon-mod.toml" || name.starts_with("assets/") || name.starts_with("data/")
    }));
    fs::write(project.join("assets/ruby/renamed.bin"), b"\0asm").unwrap();
    let output = Command::new(binary)
        .current_dir(&project)
        .arg("build")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported executable file"));
    fs::remove_dir_all(root).unwrap();
}
