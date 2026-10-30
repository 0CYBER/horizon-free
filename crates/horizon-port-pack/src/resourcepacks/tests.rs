use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "horizon-resourcepacks-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn work(&self) -> PathBuf {
        self.0.join("work")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
fn archive(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}
fn imported(source: &Path, work: &Path) -> Result<Report, String> {
    pipeline::import(source, work, &|_| {})?;
    Ok(Report::load(work))
}

#[test]
fn mrpack_client_override_wins_and_server_and_unrelated_overrides_stay_out() {
    let t = Temp::new();
    let src = t.0.join("pack.mrpack");
    archive(
        &src,
        &[
            (
                "modrinth.index.json",
                br#"{"name":"Small","files":[{"path":"shaderpacks/later.zip"}]}"#,
            ),
            ("overrides/resourcepacks/look.zip", b"common"),
            ("client-overrides/resourcepacks/look.zip", b"client"),
            ("server-overrides/resourcepacks/server.zip", b"server"),
            ("overrides/options.txt", b"do not copy"),
        ],
    );
    let report = imported(&src, &t.work()).unwrap();
    assert_eq!(
        fs::read(t.work().join("resourcepacks/look.zip")).unwrap(),
        b"client"
    );
    assert!(!t.work().join("resourcepacks/server.zip").exists());
    assert!(!t.work().join("options.txt").exists());
    assert_eq!(
        report.files["resourcepacks/look.zip"].state,
        State::Available
    );
    assert_eq!(
        report.files["shaderpacks/later.zip"].state,
        State::Unsupported
    );
}

#[test]
fn curseforge_custom_root_and_directory_pack() {
    let t = Temp::new();
    let src = t.0.join("pack.zip");
    archive(
        &src,
        &[
            (
                "manifest.json",
                br#"{"name":"Custom","overrides":"custom/root","files":[]}"#,
            ),
            ("custom/root/resourcepacks/Pretty/pack.mcmeta", b"{}"),
            (
                "custom/root/resourcepacks/Pretty/assets/test/a.txt",
                b"texture",
            ),
        ],
    );
    imported(&src, &t.work()).unwrap();
    assert_eq!(
        fs::read(t.work().join("resourcepacks/Pretty/assets/test/a.txt")).unwrap(),
        b"texture"
    );
    assert_eq!(available(&t.work()).len(), 1);
}

#[test]
fn resource_only_instance_and_repeated_import_keep_edits() {
    let t = Temp::new();
    let src = t.0.join("instance");
    put(&src.join("mmc-pack.json"), b"{}");
    put(
        &src.join(".minecraft/resourcepacks/Look/pack.mcmeta"),
        b"{}",
    );
    put(&src.join(".minecraft/resourcepacks/look.zip"), b"zip");
    imported(&src, &t.work()).unwrap();
    put(&t.work().join("resourcepacks/look.zip"), b"local edit");
    let report = imported(&src, &t.work()).unwrap();
    assert_eq!(
        report.files["resourcepacks/look.zip"].state,
        State::Conflict
    );
    assert_eq!(
        fs::read(t.work().join("resourcepacks/look.zip")).unwrap(),
        b"local edit"
    );
    assert_eq!(available(&t.work()).len(), 2);
}

#[test]
fn hostile_archive_paths_and_cross_layer_collisions_are_rejected_before_staging() {
    for name in [
        "../escape.zip",
        "/absolute.zip",
        "C:drive.zip",
        "a\\..\\escape.zip",
        "CON.zip",
        "space.zip ",
    ] {
        let t = Temp::new();
        let src = t.0.join("bad.mrpack");
        archive(
            &src,
            &[
                ("modrinth.index.json", b"{}"),
                (&format!("overrides/resourcepacks/{name}"), b"bad"),
            ],
        );
        assert!(imported(&src, &t.work()).is_err(), "{name}");
        assert!(available(&t.work()).is_empty());
        assert!(!t.work().join("escape.zip").exists());
    }
    let t = Temp::new();
    let src = t.0.join("bad.mrpack");
    archive(
        &src,
        &[
            ("modrinth.index.json", b"{}"),
            ("overrides/resourcepacks/a.zip", b"first"),
            (
                "client-overrides/resourcepacks/a.zip/pack.mcmeta",
                b"second",
            ),
        ],
    );
    assert!(imported(&src, &t.work()).is_err());
    assert!(available(&t.work()).is_empty());
}

#[test]
fn duplicate_names_case_aliases_and_byte_limits() {
    let mut files = Files::new();
    add(&mut files, "A.zip", &b"one"[..], &mut 0).unwrap();
    assert!(add(&mut files, "A.zip", &b"two"[..], &mut 0).is_err());
    add(&mut files, "a.zip", &b"two"[..], &mut 0).unwrap();
    assert!(validate(&files).is_err());
    assert!(read_limited(&b"12345"[..], 4).is_err());
    assert_eq!(read_limited(&b"1234"[..], 4).unwrap(), b"1234");
}

#[test]
fn profile_copy_is_idempotent_and_preserves_whole_directory_edits() {
    let t = Temp::new();
    put(&t.work().join("resourcepacks/Look/pack.mcmeta"), b"{}");
    put(&t.work().join("resourcepacks/Look/assets/a"), b"original");
    put(&t.work().join("resourcepacks/look.zip"), b"zip");
    let home = t.0.join("home");
    let first = install(&t.work(), &home).unwrap();
    assert_eq!(first.files.len(), 2);
    put(&home.join("resourcepacks/Look/assets/a"), b"edit");
    let second = install(&t.work(), &home).unwrap();
    assert_eq!(second.files["resourcepacks/Look"].state, State::Conflict);
    assert_eq!(
        second.files["resourcepacks/look.zip"].state,
        State::Available
    );
    assert_eq!(
        fs::read(home.join("resourcepacks/Look/assets/a")).unwrap(),
        b"edit"
    );
    assert!(!home.join("options.txt").exists());
}

#[cfg(unix)]
#[test]
fn symlinks_at_source_and_destination_are_rejected() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let src = t.0.join("instance");
    put(&src.join("resourcepacks/good.zip"), b"good");
    put(&t.0.join("outside.zip"), b"outside");
    symlink(t.0.join("outside.zip"), src.join("resourcepacks/link.zip")).unwrap();
    assert!(imported(&src, &t.work()).is_err());
    fs::remove_file(src.join("resourcepacks/link.zip")).unwrap();
    imported(&src, &t.work()).unwrap();
    let home = t.0.join("home");
    fs::create_dir(&home).unwrap();
    symlink(&src, home.join("resourcepacks")).unwrap();
    assert!(install(&t.work(), &home).is_err());
    assert_eq!(
        fs::read(src.join("resourcepacks/good.zip")).unwrap(),
        b"good"
    );
}
