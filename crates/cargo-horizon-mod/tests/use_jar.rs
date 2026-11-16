use sha2::{Digest, Sha512};
use std::{fs, io::Write, process::Command};

#[test]
#[ignore = "daily"]
fn use_jar_records_exact_metadata_hash_and_order_without_copying_the_jar() {
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
        let result = Command::new(binary)
            .current_dir(&project)
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
            let repeat = Command::new(binary)
                .current_dir(&project)
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
