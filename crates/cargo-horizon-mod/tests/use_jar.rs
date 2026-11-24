use sha2::{Digest, Sha512};
use std::{fs, io::Write, process::Command};

#[test]
#[ignore = "daily"]
fn use_jar_records_exact_metadata_hash_order_and_stages_jar() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "target/use-jar-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let binary = env!("CARGO_BIN_EXE_cargo-horizon-mod");
    let created = Command::new(binary)
        .current_dir(&root)
        .args(["new", "port"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let project = root.join("port");
    let manifest = project.join("horizon-mod.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    for (name, metadata_name, metadata) in [
        (
            "first.jar",
            "fabric.mod.json",
            r#"{"id":"fabric-api","version":"1.2"}"#,
        ),
        (
            "second.jar",
            "META-INF/neoforge.mods.toml",
            "[[mods]]\nmodId = 'original_mod'\nversion = '3'\n",
        ),
        ("unknown.jar", "README.txt", "no mod identity"),
    ] {
        let path = root.join(name);
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        zip.start_file(metadata_name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(metadata.as_bytes()).unwrap();
        zip.start_file(
            "example/Unused.class",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"not executable; ignored by metadata reader")
            .unwrap();
        zip.finish().unwrap();
        let before = fs::read_to_string(&manifest).unwrap();
        let home = root.join("home");
        let result = Command::new(binary)
            .current_dir(&project)
            .env("HORIZON_HOME", &home)
            .arg("use-jar")
            .arg(&path)
            .output()
            .unwrap();
        if name == "unknown.jar" {
            assert!(!result.status.success());
            assert_eq!(fs::read_to_string(&manifest).unwrap(), before);
        } else {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let added = fs::read_to_string(&manifest).unwrap();
            let parsed = horizon_mod_package::Manifest::parse(&added).unwrap();
            let jar = parsed.jar.last().unwrap();
            assert_eq!(
                jar.sha512(),
                format!("{:x}", Sha512::digest(fs::read(&path).unwrap()))
            );
            assert!(jar.cache_path(&home.join("jars")).is_file());
            let repeat = Command::new(binary)
                .current_dir(&project)
                .env("HORIZON_HOME", &home)
                .arg("use-jar")
                .arg(&path)
                .output()
                .unwrap();
            assert!(repeat.status.success());
            assert_eq!(fs::read_to_string(&manifest).unwrap(), added);
        }
        assert!(!project.join(name).exists());
    }
    let text = fs::read_to_string(&manifest).unwrap();
    assert!(text.starts_with(&original));
    let parsed = horizon_mod_package::Manifest::parse(&text).unwrap();
    assert_eq!(
        parsed
            .jar
            .iter()
            .map(|jar| (jar.id(), jar.version()))
            .collect::<Vec<_>>(),
        [("fabric-api", "1.2"), ("original_mod", "3")]
    );
    let inline = format!(
        "jar = [{{ id = 'fabric-api', version = '1.2', sha512 = '{}' }}]\n{original}",
        parsed.jar[0].sha512()
    );
    for (input, jar_name, message) in [
        (
            text.replace("fabric-api", "wrong_id"),
            "first.jar",
            "this hash lists",
        ),
        (inline, "second.jar", "cannot append a jar entry"),
        (
            format!("unknown_field = true\n{original}"),
            "first.jar",
            "unknown field",
        ),
    ] {
        fs::write(&manifest, &input).unwrap();
        let result = Command::new(binary)
            .current_dir(&project)
            .env("HORIZON_HOME", root.join("home"))
            .arg("use-jar")
            .arg(root.join(jar_name))
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains(message));
        assert_eq!(fs::read_to_string(&manifest).unwrap(), input);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "daily"]
fn use_jar_take_records_checked_prefixes_and_refuses_one_matching_nothing() {
    // Removed on drop, so a failing run leaves nothing behind.
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let binary = env!("CARGO_BIN_EXE_cargo-horizon-mod");
    let created = Command::new(binary)
        .current_dir(root)
        .args(["new", "port"])
        .output()
        .unwrap();
    assert!(created.status.success());
    let project = root.join("port");
    let manifest = project.join("horizon-mod.toml");
    let jar = root.join("original.jar");
    let mut zip = zip::ZipWriter::new(fs::File::create(&jar).unwrap());
    for (name, text) in [
        (
            "META-INF/neoforge.mods.toml",
            "[[mods]]\nmodId = 'original_mod'\nversion = '6'\n",
        ),
        ("assets/original_mod/lang/en_us.json", "{}"),
        ("assets/minecraft/atlases/blocks.json", "{}"),
        ("data/original_mod/recipe/a.json", "{}"),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.add_directory("assets/empty/", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.finish().unwrap();
    let use_jar = |take: &[&str]| {
        let mut command = Command::new(binary);
        command
            .current_dir(&project)
            .env("HORIZON_HOME", root.join("home"))
            .arg("use-jar")
            .arg(&jar);
        for prefix in take {
            command.args(["--take", prefix]);
        }
        command.output().unwrap()
    };
    let before = fs::read_to_string(&manifest).unwrap();
    for (take, message) in [
        (
            &["assets/original_mod", "assets/nothing_here"][..],
            "matches no file",
        ),
        (&["assets/empty"], "matches no file"),
        (&["assets/original_mod/"], "plain path prefix"),
        (&["META-INF"], "must start with assets/"),
    ] {
        let refused = use_jar(take);
        assert!(!refused.status.success(), "{take:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&refused.stderr)
        );
        assert_eq!(fs::read_to_string(&manifest).unwrap(), before);
    }
    let take = ["assets/original_mod", "assets/minecraft/atlases"];
    let added = use_jar(&take);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let parsed =
        horizon_mod_package::Manifest::parse(&fs::read_to_string(&manifest).unwrap()).unwrap();
    let listed = parsed.jar.last().unwrap();
    assert_eq!(listed.take().unwrap(), take);
    assert_eq!(listed.side(), horizon_mod_package::manifest::Side::Client);
    assert!(use_jar(&take).status.success());
    let other = use_jar(&["assets/original_mod"]);
    assert!(!other.status.success());
    assert!(String::from_utf8_lossy(&other.stderr).contains("already listed with take"));
}
