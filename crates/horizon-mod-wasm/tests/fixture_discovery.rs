#[allow(dead_code)]
#[path = "fixture/build.rs"]
mod fixture_build;

#[test]
fn fixture_modules_are_sorted_and_preserve_support_file_layout() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    for name in [
        "zebra.rs",
        "strict_math_checks.rs",
        "apple.rs",
        "lib.rs",
        "strict_math_vectors.rs",
    ] {
        std::fs::write(root.join(name), "").unwrap();
    }
    let absolute = root.canonicalize().unwrap();
    assert_eq!(
        fixture_build::modules(root).unwrap(),
        format!(
            "#[path = {:?}]\npub mod apple;\n#[path = {:?}]\nmod strict_math_checks;\n#[path = {:?}]\npub mod zebra;\n",
            absolute.join("apple.rs").to_str().unwrap(),
            absolute.join("strict_math_checks.rs").to_str().unwrap(),
            absolute.join("zebra.rs").to_str().unwrap(),
        )
    );
}

#[test]
fn fixture_modules_refuse_unsupported_layout_by_name() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["mod.rs", "nested", "notes.txt", "extensionless"] {
        let path = directory.path().join(name);
        if name == "nested" {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::write(&path, "").unwrap();
        }
        let error = fixture_build::modules(directory.path()).unwrap_err();
        assert!(error.contains(name), "{error}");
        if name == "nested" {
            std::fs::remove_dir(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }
}

#[cfg(unix)]
#[test]
fn fixture_modules_refuse_non_utf8_ancestor_by_name() {
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().unwrap();
    let root = directory
        .path()
        .join(std::ffi::OsString::from_vec(b"non_utf8_\xff".to_vec()));
    std::fs::create_dir(&root).unwrap();
    let module = root.join("guest.rs");
    std::fs::write(&module, "").unwrap();

    let error = fixture_build::modules(&root).unwrap_err();
    assert!(error.contains("guest module path is not UTF-8"), "{error}");
    assert!(error.contains("non_utf8_"), "{error}");
    assert!(error.contains("guest.rs"), "{error}");
}

#[test]
fn every_guest_source_file_is_included_by_name() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture/src");
    let generated = fixture_build::modules(&source).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|extension| extension == "rs") {
            let name = path.file_stem().unwrap().to_str().unwrap();
            // The root, scanner input and math checker's nested data are deliberately not roots.
            if !matches!(name, "lib" | "points" | "strict_math_vectors") {
                assert!(
                    generated.contains(&format!("mod {name};")),
                    "missing guest module {name}"
                );
            }
        }
    }
}
