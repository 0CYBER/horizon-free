use super::*;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        // Keep any separate Windows build directory inside the fixture, even when TEMP
        // is under a non-ASCII user profile. Use pr_ready's ProgramData fallback.
        #[cfg(windows)]
        let base = if tool_safe(&base).is_err() {
            let public = std::env::var_os("ProgramData")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && tool_safe(path).is_ok())
                .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
            let base = fs::canonicalize(public).unwrap();
            tool_safe(&base).expect("Set TEMP to a writable ASCII path without spaces");
            base
        } else {
            base
        };
        loop {
            let path = base.join(format!(
                "horizon-location-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(checked_path(&fs::canonicalize(path).unwrap()).unwrap()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture: {e}"),
            }
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        // std removes links themselves rather than traversing their targets.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            reject_link(&path, &meta).unwrap(); // Never follow links while checking fixtures.
            if meta.is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().into(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

struct Fixture {
    temp: Temp,
    source: Roots,
    bootstrap: PathBuf,
    pointer: Vec<u8>,
    original: BTreeMap<PathBuf, Vec<u8>>,
}
impl Fixture {
    fn new(build: &str) -> Self {
        let temp = Temp::new();
        let source = Roots {
            launcher: temp.0.join("source/launcher"),
            game: temp.0.join("source/game"),
            build: temp.0.join("source").join(build),
        };
        for (root, name, bytes) in [
            (&source.launcher, "accounts.json", &b"{\"accounts\":[]}"[..]),
            (
                &source.game,
                "saves/dummy/level.dat",
                &b"\0\xff\x80world\0"[..],
            ),
            (
                &source.build,
                "cache/compiled.bin",
                &b"\0\x01\xfe\xffbuild"[..],
            ),
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            fs::create_dir_all(root.join("empty")).unwrap();
        }
        let bootstrap = temp.0.join("bootstrap/location.json");
        fs::create_dir(bootstrap.parent().unwrap()).unwrap();
        let pointer = serde_json::to_vec(&Location {
            version: 1,
            roots: source.clone(),
        })
        .unwrap();
        fs::write(&bootstrap, &pointer).unwrap();
        let original = snapshot(&temp.0.join("source"));
        Self {
            temp,
            source,
            bootstrap,
            pointer,
            original,
        }
    }
    fn target(&self) -> PathBuf {
        self.temp.0.join("destination")
    }
    fn unchanged(&self) {
        assert_eq!(snapshot(&self.temp.0.join("source")), self.original);
        for root in [&self.source.launcher, &self.source.game, &self.source.build] {
            assert_eq!(fs::read_dir(root.join("empty")).unwrap().count(), 0);
        }
    }
    fn old_pointer(&self) {
        assert_eq!(fs::read(&self.bootstrap).unwrap(), self.pointer);
        assert_eq!(
            resolve_at(&self.bootstrap).unwrap(),
            Some(self.source.clone())
        );
        self.unchanged();
    }
    fn migrated(&self, roots: &Roots) {
        assert_eq!(resolve_at(&self.bootstrap).unwrap(), Some(roots.clone()));
        for (source, dest) in [
            (&self.source.launcher, &roots.launcher),
            (&self.source.game, &roots.game),
            (&self.source.build, &roots.build),
        ] {
            assert_eq!(snapshot(source), snapshot(dest));
            assert_eq!(fs::read_dir(dest.join("empty")).unwrap().count(), 0);
        }
        self.unchanged();
    }
    fn refuse(&self, target: &Path) -> String {
        let error = migrate(&self.source, target, &self.bootstrap, |_| {}).unwrap_err();
        self.old_pointer();
        error
    }
}

#[test]
fn full_move_preserves_source_and_commits_separate_build() {
    let f = Fixture::new("build");
    let target = f.target();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&f.source.launcher, fs::Permissions::from_mode(0o750)).unwrap();
        fs::set_permissions(
            f.source.launcher.join("accounts.json"),
            fs::Permissions::from_mode(0o640),
        )
        .unwrap();
    }
    let roots = migrate(&f.source, &target, &f.bootstrap, |_| {}).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for root in [&f.source.launcher, &roots.launcher] {
            assert_eq!(
                fs::metadata(root).unwrap().permissions().mode() & 0o777,
                0o750
            );
            assert_eq!(
                fs::metadata(root.join("accounts.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
        }
    }
    assert_eq!(
        roots,
        Roots {
            launcher: target.join("launcher"),
            game: target.join("game"),
            build: target.join("build"),
        }
    );
    assert_ne!(fs::read(&f.bootstrap).unwrap(), f.pointer);
    f.migrated(&roots);
}

#[test]
fn full_move_preserves_aliased_and_nested_build_layouts() {
    for build in ["launcher", "launcher/native-build"] {
        let f = Fixture::new(build);
        let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
        assert_eq!(roots.build, f.target().join(build));
        assert!(!f.target().join("build").exists());
        f.migrated(&roots);
    }
}

#[test]
fn nonempty_and_file_destinations_preserve_pointer_and_contents() {
    let f = Fixture::new("build");
    let folder = f.temp.0.join("Игры with spaces");
    let file = f.temp.0.join("Existing file");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("sentinel"), b"keep folder contents").unwrap();
    fs::write(&file, b"keep file contents").unwrap();
    for target in [&folder, &file] {
        let error = f.refuse(target);
        assert!(error.contains("empty folder"), "{error}");
        assert!(error.contains(&target.display().to_string()), "{error}");
    }
    assert_eq!(fs::read_dir(&folder).unwrap().count(), 1);
    assert_eq!(
        fs::read(folder.join("sentinel")).unwrap(),
        b"keep folder contents"
    );
    assert_eq!(fs::read(file).unwrap(), b"keep file contents");
}

fn reusable(target: &Path) -> bool {
    staging_only(target).unwrap()
}

/// Fails a move into `target` after its copy, which leaves its stage behind. The new stage.
fn stopped_move(f: &Fixture, target: &Path) -> (String, PathBuf) {
    let before = kept_copies(target, None);
    let error = migrate_with_prepare(
        &f.source,
        target,
        &f.bootstrap,
        |_| {},
        |_, _| Err("fixture stop".into()),
    )
    .unwrap_err();
    assert!(error.contains("fixture stop"), "{error}");
    assert!(error.contains("Retry with the same folder"), "{error}");
    f.old_pointer();
    let new: Vec<_> = kept_copies(target, None)
        .into_iter()
        .filter(|stage| !before.contains(stage))
        .collect();
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(new[0].join("launcher/accounts.json").is_file());
    assert!(reusable(target));
    (error, new[0].clone())
}

#[test]
fn a_retry_leaves_an_earlier_stage_alone() {
    let f = Fixture::new("build");
    let target = f.target();
    let (_, old) = stopped_move(&f, &target);
    let staged = snapshot(&old);
    let moved = migrate(&f.source, &target, &f.bootstrap, |_| {}).unwrap();
    f.migrated(&moved);
    assert_eq!(snapshot(&old), staged);
    assert_eq!(kept_copies(&target, None), std::slice::from_ref(&old));
    assert!(kept_note(&target, None)
        .unwrap()
        .contains(&old.display().to_string()));
}

#[test]
fn two_earlier_stages_stay() {
    let f = Fixture::new("build");
    let target = f.target();
    let (_, first) = stopped_move(&f, &target);
    let (error, second) = stopped_move(&f, &target);
    assert!(error.contains("as they were"), "{error}");
    assert!(error.contains(&first.display().to_string()), "{error}");
    assert!(!error.contains(&second.display().to_string()), "{error}");
    let before = [snapshot(&first), snapshot(&second)];
    let moved = migrate(&f.source, &target, &f.bootstrap, |_| {}).unwrap();
    f.migrated(&moved);
    assert_eq!([snapshot(&first), snapshot(&second)], before);
    assert_eq!(kept_copies(&target, None).len(), 2);
}

#[test]
fn a_retry_leaves_reserved_build_folders_alone() {
    let f = Fixture::new("build");
    // On Windows this name reserves a horizon-build-* sibling, which the stopped move keeps.
    let target = f.temp.0.join("Игры");
    let planted = f.temp.0.join("horizon-build-planted/build");
    fs::create_dir_all(&planted).unwrap();
    fs::write(planted.join("compiled.bin"), b"other location").unwrap();
    stopped_move(&f, &target);
    let reserved = || {
        fs::read_dir(&f.temp.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("horizon-build-")
            })
            .map(|path| (snapshot(&path), path))
            .collect::<Vec<_>>()
    };
    let before = reserved();
    #[cfg(windows)]
    assert_eq!(before.len(), 2, "{before:?}");
    let moved = migrate(&f.source, &target, &f.bootstrap, |_| {}).unwrap();
    f.migrated(&moved);
    for (files, path) in &before {
        assert_eq!(&snapshot(path), files, "{}", path.display());
    }
    assert_eq!(
        fs::read(planted.join("compiled.bin")).unwrap(),
        b"other location"
    );
}

#[test]
fn a_stage_beside_other_entries_is_refused() {
    let f = Fixture::new("build");
    let target = f.target();
    stopped_move(&f, &target);
    fs::write(target.join("sentinel"), b"keep").unwrap();
    let before = snapshot(&target);
    assert!(!reusable(&target));
    assert!(f.refuse(&target).contains("empty folder"));
    assert_eq!(snapshot(&target), before);
}

#[test]
fn a_stage_that_is_a_file_is_refused() {
    let f = Fixture::new("build");
    for name in [STAGING.to_owned(), format!("{STAGING}-old")] {
        let target = f.temp.0.join(format!("file{}", name.len()));
        fs::create_dir(&target).unwrap();
        fs::write(target.join(&name), b"not a folder").unwrap();
        assert!(!reusable(&target));
        assert!(f.refuse(&target).contains("empty folder"));
        assert_eq!(fs::read(target.join(&name)).unwrap(), b"not a folder");
    }
}

#[cfg(any(unix, windows))]
#[test]
fn a_linked_stage_is_refused_without_touching_its_target() {
    let f = Fixture::new("build");
    let outside = Temp::new();
    fs::write(outside.0.join("sentinel"), b"untouched").unwrap();
    for name in [STAGING.to_owned(), format!("{STAGING}-old")] {
        let target = f.temp.0.join(format!("linked{}", name.len()));
        fs::create_dir(&target).unwrap();
        dir_link(&outside.0, &target.join(&name));
        assert!(!reusable(&target));
        assert!(f.refuse(&target).contains("empty folder"));
        assert!(fs::symlink_metadata(target.join(&name)).is_ok());
    }
    assert_eq!(fs::read(outside.0.join("sentinel")).unwrap(), b"untouched");
}

#[test]
fn a_destination_inside_a_staging_folder_is_refused() {
    let f = Fixture::new("build");
    for target in [
        f.temp.0.join(STAGING),
        f.temp.0.join(STAGING).join("data"),
        f.temp.0.join("data").join(STAGING),
        f.temp.0.join(format!("{STAGING}-old")).join("data"),
    ] {
        let error = f.refuse(&target);
        assert!(error.contains("in its path"), "{error}");
        assert!(!target.exists());
    }
}

#[test]
fn a_held_destination_lock_stops_the_move_before_its_stage() {
    let f = Fixture::new("build");
    let target = f.target();
    fs::create_dir(&target).unwrap();
    let held = File::create(target.join(LOCK)).unwrap();
    held.try_lock().unwrap();
    let error = f.refuse(&target);
    assert!(error.contains("Another move"), "{error}");
    assert!(kept_copies(&target, None).is_empty());
    drop(held);
    // Released, the file left behind is accepted.
    assert!(reusable(&target));
    let moved = migrate(&f.source, &target, &f.bootstrap, |_| {}).unwrap();
    f.migrated(&moved);
    assert!(target.join(LOCK).is_file());
    assert!(kept_note(&target, None).is_none());
}

#[test]
fn a_lock_that_is_a_folder_is_refused() {
    let f = Fixture::new("build");
    let target = f.target();
    fs::create_dir_all(target.join(LOCK)).unwrap();
    assert!(!reusable(&target));
    assert!(f.refuse(&target).contains("empty folder"));
    assert!(target.join(LOCK).is_dir());
}

#[cfg(any(unix, windows))]
#[test]
fn a_linked_lock_is_refused() {
    let f = Fixture::new("build");
    let target = f.target();
    let outside = Temp::new();
    fs::create_dir(&target).unwrap();
    dir_link(&outside.0, &target.join(LOCK));
    assert!(!reusable(&target));
    assert!(f.refuse(&target).contains("empty folder"));
    assert!(fs::read_dir(&outside.0).unwrap().next().is_none());
}

#[test]
fn a_claim_beside_another_moves_data_is_given_up() {
    let f = Fixture::new("build");
    let target = f.target();
    fs::create_dir(&target).unwrap();
    // Another bootstrap's move puts its data there between the check and the claim.
    fs::write(target.join("theirs"), b"keep").unwrap();
    let error = claim_stage(&target).unwrap_err();
    assert!(error.contains("Another move"), "{error}");
    assert!(kept_copies(&target, None).is_empty());
    assert_eq!(fs::read(target.join("theirs")).unwrap(), b"keep");
}

#[test]
fn a_stage_without_its_game_folder_is_not_committed() {
    let f = Fixture::new("build");
    let target = f.target();
    let error = migrate_with_prepare(
        &f.source,
        &target,
        &f.bootstrap,
        |_| {},
        |stage, _| fs::remove_dir_all(&stage.game).map_err(|e| e.to_string()),
    )
    .unwrap_err();
    f.old_pointer();
    // The launcher folder was renamed out of the stage, so the same folder no longer works.
    assert!(target.join("launcher").is_dir());
    assert!(error.contains("choose an empty folder to retry"), "{error}");
    assert!(!reusable(&target));
}

#[test]
fn a_folder_already_at_the_destination_is_not_replaced() {
    let f = Fixture::new("build");
    let target = f.target();
    let error = migrate_with_prepare(
        &f.source,
        &target,
        &f.bootstrap,
        |_| {},
        |_, destination| {
            // Another move publishes its launcher folder there before this one renames.
            fs::create_dir(&destination.launcher).unwrap();
            fs::write(destination.launcher.join("theirs"), b"keep").unwrap();
            Ok(())
        },
    )
    .unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    f.old_pointer();
    assert_eq!(fs::read(target.join("launcher/theirs")).unwrap(), b"keep");
    let [stage] = &kept_copies(&target, None)[..] else {
        panic!("one stage");
    };
    assert!(stage.join("launcher/accounts.json").is_file());
}

#[test]
fn nested_same_and_ancestor_destinations_preserve_pointer() {
    let f = Fixture::new("build");
    for target in [
        f.source.game.join("nested"),
        f.source.launcher.clone(),
        f.temp.0.join("source"),
    ] {
        assert!(f.refuse(&target).contains("overlaps"));
    }
}

#[cfg(any(unix, windows))]
#[test]
fn source_entry_link_is_rejected_without_touching_its_target() {
    let f = Fixture::new("build");
    let outside = Temp::new();
    fs::write(outside.0.join("sentinel"), b"untouched").unwrap();
    let link = f.source.game.join("linked");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.0, &link).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(outside.0.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(output.status.success(), "junction setup: {output:?}");
    }
    struct Link(PathBuf);
    impl Drop for Link {
        fn drop(&mut self) {
            #[cfg(unix)]
            let _ = fs::remove_file(&self.0);
            #[cfg(windows)]
            let _ = fs::remove_dir(&self.0);
        }
    }
    let link = Link(link);
    let error = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap_err();
    drop(link);
    assert!(
        error.contains("Symbolic links and reparse points"),
        "{error}"
    );
    f.old_pointer();
    assert_eq!(fs::read(outside.0.join("sentinel")).unwrap(), b"untouched");
}

#[cfg(any(unix, windows))]
#[test]
fn game_cache_links_are_left_out_of_the_move() {
    let f = Fixture::new("build");
    let assets = f.source.launcher.join("versions/x/assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("sentinel"), b"untouched").unwrap();
    struct Link(PathBuf);
    impl Drop for Link {
        fn drop(&mut self) {
            #[cfg(unix)]
            let _ = fs::remove_file(&self.0);
            #[cfg(windows)]
            let _ = fs::remove_dir(&self.0);
        }
    }
    let mut links = Vec::new();
    for instance in [
        f.source.game.clone(),
        f.source.game.join("instances/a"),
        f.source.game.join("ports/p/home"),
    ] {
        let stamp = instance.join(".horizon-cache/mod-content/0123456789abcdef");
        fs::create_dir_all(&stamp).unwrap();
        fs::write(stamp.join("STAMP"), b"stamp").unwrap();
        let link = stamp.join("assets");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&assets, &link).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(assets.to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(output.status.success(), "junction setup: {output:?}");
        }
        links.push(Link(link));
    }
    fs::write(f.source.game.join("instances/a/options.txt"), b"options").unwrap();
    // A world may carry the cache's name; only the game's own caches are left out.
    let world = f.source.game.join("saves/.horizon-cache");
    fs::create_dir_all(&world).unwrap();
    fs::write(world.join("level.dat"), b"world").unwrap();
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    drop(links);
    assert!(!roots.game.join(".horizon-cache").exists());
    assert!(!roots.game.join("instances/a/.horizon-cache").exists());
    assert!(!roots.game.join("ports/p/home/.horizon-cache").exists());
    assert_eq!(
        fs::read(roots.game.join("saves/.horizon-cache/level.dat")).unwrap(),
        b"world"
    );
    assert_eq!(
        fs::read(roots.game.join("instances/a/options.txt")).unwrap(),
        b"options"
    );
    assert_eq!(
        fs::read(roots.game.join("saves/dummy/level.dat")).unwrap(),
        b"\0\xff\x80world\0"
    );
    assert_eq!(fs::read(assets.join("sentinel")).unwrap(), b"untouched");
    assert_eq!(
        fs::read(roots.launcher.join("versions/x/assets/sentinel")).unwrap(),
        b"untouched"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn other_launchers_links_are_left_out_of_the_move() {
    let f = Fixture::new("build");
    let outside = Temp::new();
    for dir in ["libraries", "assets"] {
        fs::create_dir(outside.0.join(dir)).unwrap();
        fs::write(outside.0.join(dir).join("sentinel"), b"untouched").unwrap();
    }
    fs::write(outside.0.join("client.jar"), b"jar").unwrap();
    struct Link(PathBuf);
    impl Drop for Link {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            #[cfg(windows)]
            let _ = fs::remove_dir(&self.0);
        }
    }
    let linked = f.source.launcher.join("linked-minecraft");
    let version = linked.join("versions/1.0");
    fs::create_dir_all(&version).unwrap();
    fs::write(version.join("1.0.json"), b"{}").unwrap();
    let mut links = Vec::new();
    for dir in ["libraries", "assets"] {
        let link = linked.join(dir);
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.0.join(dir), &link).unwrap();
        #[cfg(windows)]
        {
            let output = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(link.to_string_lossy().replace('/', "\\"))
                .arg(outside.0.join(dir).to_string_lossy().replace('/', "\\"))
                .output()
                .unwrap();
            assert!(output.status.success(), "junction setup: {output:?}");
        }
        links.push(Link(link));
    }
    #[cfg(unix)]
    {
        let jar = version.join("1.0.jar");
        std::os::unix::fs::symlink(outside.0.join("client.jar"), &jar).unwrap();
        links.push(Link(jar));
    }
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    drop(links);
    let moved = roots.launcher.join("linked-minecraft");
    for path in ["libraries", "assets", "versions/1.0/1.0.jar"] {
        assert!(fs::symlink_metadata(moved.join(path)).is_err(), "{path}");
    }
    assert_eq!(
        fs::read(moved.join("versions/1.0/1.0.json")).unwrap(),
        b"{}"
    );
    assert_eq!(
        fs::read(roots.launcher.join("accounts.json")).unwrap(),
        b"{\"accounts\":[]}"
    );
    for dir in ["libraries", "assets"] {
        assert_eq!(
            fs::read(outside.0.join(dir).join("sentinel")).unwrap(),
            b"untouched"
        );
    }
    assert_eq!(fs::read(outside.0.join("client.jar")).unwrap(), b"jar");
}

/// Links the folder `link` to `target`: a symlink on Unix, a junction on Windows.
#[cfg(any(unix, windows))]
fn dir_link(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(output.status.success(), "junction setup: {output:?}");
    }
}

#[cfg(any(unix, windows))]
#[test]
fn server_runtime_moves_with_its_release() {
    let f = Fixture::new("build");
    let root = f.source.launcher.join("server-runtime");
    let files = root.join("game-files/horizon-client");
    fs::create_dir_all(files.join("1.0")).unwrap();
    fs::write(files.join("1.0/data"), b"files").unwrap();
    // Two releases with their links; `current` picks the first, which is not the newest.
    for tag in ["v1", "v2"] {
        let release = root.join("releases").join(tag);
        fs::create_dir_all(release.join(".local")).unwrap();
        fs::write(release.join("horizon-server"), tag).unwrap();
        dir_link(&files, &release.join(".local/horizon-client"));
    }
    fs::create_dir_all(root.join("releases/v3/.local")).unwrap();
    fs::write(root.join("current.switching"), b"mine").unwrap();
    dir_link(&root.join("releases/v1"), &root.join("current"));
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    let moved = roots.launcher.join("server-runtime");
    let canonical = |path: PathBuf| fs::canonicalize(path).unwrap();
    assert_eq!(
        canonical(moved.join("current")),
        canonical(moved.join("releases/v1"))
    );
    assert_eq!(
        fs::read(moved.join("current/horizon-server")).unwrap(),
        b"v1"
    );
    for tag in ["v1", "v2"] {
        let local = moved
            .join("releases")
            .join(tag)
            .join(".local/horizon-client");
        assert_eq!(fs::read(local.join("1.0/data")).unwrap(), b"files");
        assert_eq!(
            canonical(local),
            canonical(moved.join("game-files/horizon-client"))
        );
    }
    assert_eq!(
        canonical(root.join("current")),
        canonical(root.join("releases/v1"))
    );
    // A release with a plain `.local` gets no link, and a player's own `.switching` file moves.
    assert!(fs::read_dir(moved.join("releases/v3/.local"))
        .unwrap()
        .next()
        .is_none());
    assert_eq!(fs::read(moved.join("current.switching")).unwrap(), b"mine");
}

#[cfg(any(unix, windows))]
#[test]
fn server_runtime_current_elsewhere_is_refused() {
    let f = Fixture::new("build");
    let outside = Temp::new();
    fs::write(outside.0.join("sentinel"), b"untouched").unwrap();
    // No `releases` folder: `current` is still checked on its own.
    let root = f.source.launcher.join("server-runtime");
    fs::create_dir_all(&root).unwrap();
    dir_link(&outside.0, &root.join("current"));
    let error = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap_err();
    assert!(error.contains("a link Horizon did not make"), "{error}");
    assert_eq!(fs::read(&f.bootstrap).unwrap(), f.pointer);
    assert_eq!(fs::read(outside.0.join("sentinel")).unwrap(), b"untouched");
    #[cfg(unix)]
    fs::remove_file(root.join("current")).unwrap();
    #[cfg(windows)]
    fs::remove_dir(root.join("current")).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn a_linked_server_runtime_is_refused() {
    let f = Fixture::new("build");
    let elsewhere = f.source.launcher.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    dir_link(&elsewhere, &f.source.launcher.join("server-runtime"));
    let error = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap_err();
    assert!(error.contains("a link Horizon did not make"), "{error}");
    assert_eq!(fs::read(&f.bootstrap).unwrap(), f.pointer);
}

#[cfg(unix)]
#[test]
fn server_game_files_through_a_linked_folder_are_refused() {
    let f = Fixture::new("build");
    let root = f.source.launcher.join("server-runtime");
    fs::create_dir_all(root.join("releases/v1/.local")).unwrap();
    fs::create_dir_all(f.source.launcher.join("elsewhere/horizon-client")).unwrap();
    // A relative link the copy would keep, so the game files would end outside the runtime.
    std::os::unix::fs::symlink("../elsewhere", root.join("game-files")).unwrap();
    dir_link(
        &root.join("game-files/horizon-client"),
        &root.join("releases/v1/.local/horizon-client"),
    );
    let error = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap_err();
    assert!(error.contains("a link Horizon did not make"), "{error}");
    assert_eq!(fs::read(&f.bootstrap).unwrap(), f.pointer);
}

#[cfg(unix)]
#[test]
fn server_links_spelled_through_a_linked_parent_move() {
    let f = Fixture::new("build");
    // The launcher spelled its folder through a link above it, as `~/.local/share` can be.
    let alias = f.temp.0.join("alias");
    std::os::unix::fs::symlink(f.temp.0.join("source"), &alias).unwrap();
    let spelled = Roots {
        launcher: alias.join("launcher"),
        ..f.source.clone()
    };
    let root = spelled.launcher.join("server-runtime");
    fs::create_dir_all(root.join("releases/v1/.local")).unwrap();
    fs::create_dir_all(root.join("game-files/horizon-client")).unwrap();
    dir_link(
        &root.join("game-files/horizon-client"),
        &root.join("releases/v1/.local/horizon-client"),
    );
    dir_link(&root.join("releases/v1"), &root.join("current"));
    let roots = migrate(&spelled, &f.target(), &f.bootstrap, |_| {}).unwrap();
    let moved = roots.launcher.join("server-runtime");
    assert_eq!(
        fs::read_link(moved.join("current")).unwrap(),
        moved.join("releases/v1")
    );
}

#[test]
fn real_files_where_other_launchers_links_go_are_moved() {
    let f = Fixture::new("build");
    let linked = f.source.launcher.join("linked-minecraft");
    for (path, bytes) in [
        ("assets/indexes/1.json", &b"assets"[..]),
        ("libraries/a.jar", b"library"),
        ("versions/1.0/1.0.jar", b"jar"),
    ] {
        fs::create_dir_all(linked.join(path).parent().unwrap()).unwrap();
        fs::write(linked.join(path), bytes).unwrap();
    }
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    assert_eq!(snapshot(&f.source.launcher), snapshot(&roots.launcher));
    assert_eq!(snapshot(&linked).len(), 3);
}

#[cfg(unix)]
#[test]
fn relative_link_inside_the_root_is_made_again() {
    let f = Fixture::new("build");
    let runtime = f.source.launcher.join("runtime");
    fs::create_dir_all(runtime.join("lib")).unwrap();
    fs::create_dir_all(runtime.join("bin")).unwrap();
    fs::write(runtime.join("lib/libjvm.so"), b"jvm").unwrap();
    std::os::unix::fs::symlink("../lib/libjvm.so", runtime.join("bin/libjvm.so")).unwrap();
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    let link = roots.launcher.join("runtime/bin/libjvm.so");
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("../lib/libjvm.so"));
    assert_eq!(fs::read(link).unwrap(), b"jvm");
}

#[cfg(unix)]
#[test]
fn relative_link_leaving_the_root_is_still_refused() {
    for target in [
        "../../outside",
        "lib/../../outside",
        "./lib",
        "lib/./a",
        "lib//a",
        "lib/",
        "/etc",
        "../..",
    ] {
        let f = Fixture::new("build");
        let folder = f.source.launcher.join("runtime");
        fs::create_dir_all(folder.join("lib")).unwrap();
        std::os::unix::fs::symlink(target, folder.join("link")).unwrap();
        let error = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap_err();
        assert!(
            error.contains("Symbolic links and reparse points"),
            "{target}: {error}"
        );
        assert_eq!(fs::read(&f.bootstrap).unwrap(), f.pointer);
    }
}

#[cfg(unix)]
struct PermissionsGuard(PathBuf, fs::Permissions);
#[cfg(unix)]
impl Drop for PermissionsGuard {
    fn drop(&mut self) {
        fs::set_permissions(&self.0, self.1.clone()).unwrap();
    }
}

#[cfg(any(unix, windows))]
#[test]
fn unreadable_source_keeps_old_pointer_and_source_bytes() {
    let f = Fixture::new("build");
    let path = f.source.game.join("saves/dummy/level.dat");
    #[cfg(windows)]
    let guard = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap()
    };
    #[cfg(unix)]
    let guard = {
        use std::os::unix::fs::PermissionsExt;
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        if unsafe { geteuid() } == 0 {
            eprintln!("SKIP unreadable_source_keeps_old_pointer_and_source_bytes: root bypasses chmod(000); run as an unprivileged user");
            return;
        }
        let guard = PermissionsGuard(path.clone(), fs::metadata(&path).unwrap().permissions());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o0)).unwrap();
        guard
    };
    assert!(
        File::open(&path).is_err(),
        "fixture must deny an actual read"
    );
    let mut committing = false;
    let error = migrate(&f.source, &f.target(), &f.bootstrap, |s| {
        committing |= s == "Committing data location"
    })
    .unwrap_err();
    drop(guard);
    assert!(!committing, "copy must fail before pointer commit");
    assert!(
        error.contains(&checked_path(&path).unwrap().display().to_string()),
        "{error}"
    );
    f.old_pointer();
}

#[test]
fn stale_worker_cannot_commit_after_successful_migration() {
    let f = Fixture::new("build");
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    let pointer = fs::read(&f.bootstrap).unwrap();
    let stale_target = f.temp.0.join("stale-worker");
    let error = migrate(&f.source, &stale_target, &f.bootstrap, |_| {}).unwrap_err();
    assert!(error.contains("Data location changed"), "{error}");
    assert_eq!(fs::read(&f.bootstrap).unwrap(), pointer);
    assert!(!stale_target.exists());
    f.migrated(&roots);
}

/// #18138: a child process another thread spawns while a migration holds its lock has a copy of
/// the lock's descriptor until its exec. The lock is released when the migration ends all the
/// same: closing the file alone left it taken through the copy, and the next migration read
/// "Cannot lock".
#[cfg(unix)]
#[test]
fn migration_lock_is_released_while_a_copy_of_its_descriptor_is_open() {
    let f = Fixture::new("build");
    let lease = MigrationLease::take(&f.bootstrap).unwrap();
    let copy = lease.0.try_clone().unwrap();
    assert!(f.refuse(&f.target()).contains("Cannot lock"));
    drop(lease);
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    f.migrated(&roots);
    drop(copy);
}

#[test]
fn shared_lock_blocks_migration_until_all_readers_release() {
    let f = Fixture::new("build");
    let first = acquire_lock_at(&f.bootstrap).unwrap();
    let second = acquire_lock_at(&f.bootstrap).unwrap();
    assert!(f.refuse(&f.target()).contains("Cannot lock"));
    // A parallel fork may still hold a pre-exec copy of either descriptor: unlock, not only close.
    first.unlock().unwrap();
    drop(first);
    assert!(f.refuse(&f.target()).contains("Cannot lock"));
    assert!(!f.target().exists());
    second.unlock().unwrap();
    drop(second);
    f.old_pointer();
    let roots = migrate(&f.source, &f.target(), &f.bootstrap, |_| {}).unwrap();
    f.migrated(&roots);
}

#[cfg(windows)]
#[test]
fn pointer_commit_failure_keeps_old_pointer_and_retries() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new("build");
    // Allow reads/writes, but deny replacement of the existing pointer.
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(1 | 2)
        .open(&f.bootstrap)
        .unwrap();
    let mut committing = false;
    let error = migrate(&f.source, &f.target(), &f.bootstrap, |s| {
        committing |= s == "Committing data location"
    })
    .unwrap_err();
    drop(handle);
    assert!(committing, "must fail during pointer commit: {error}");
    assert!(
        error.contains(&f.bootstrap.display().to_string()),
        "{error}"
    );
    f.old_pointer();
    let roots = migrate(&f.source, &f.temp.0.join("retry"), &f.bootstrap, |_| {}).unwrap();
    f.migrated(&roots);
}

#[cfg(windows)]
#[test]
fn canonical_windows_paths_are_validated_from_the_complete_root() {
    let path = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    let plain = checked_path(&path).unwrap();
    assert_eq!(std::fs::canonicalize(&plain).unwrap(), path);
    assert_eq!(checked_path(&plain).unwrap(), plain);
}

#[test]
fn a_missing_or_unreadable_location_is_reported_before_the_window_opens() {
    let temp = Temp::new();
    let bootstrap = temp.0.join("location.json");
    let roots = Roots {
        launcher: temp.0.join("unplugged/launcher"),
        game: temp.0.join("unplugged/game"),
        build: temp.0.join("unplugged/build"),
    };
    let pointer = serde_json::to_vec(&Location {
        version: 1,
        roots: roots.clone(),
    })
    .unwrap();
    fs::write(&bootstrap, &pointer).unwrap();
    let error = startup_error_at(&bootstrap).expect("a missing folder must reach the window");
    assert!(error.contains("is missing"), "{error}");

    // A damaged pointer is the other way the launcher finds no data folder.
    fs::write(&bootstrap, b"not json").unwrap();
    let damaged = startup_error_at(&bootstrap).expect("a damaged pointer must reach the window");
    assert!(damaged.contains("Invalid"), "{damaged}");
}

#[test]
fn using_the_default_folder_keeps_the_pointer_beside_it() {
    let temp = Temp::new();
    let bootstrap = temp.0.join("location.json");
    let source = Roots {
        launcher: temp.0.join("source/launcher"),
        game: temp.0.join("source/game"),
        build: temp.0.join("source/build"),
    };
    for root in [&source.launcher, &source.game, &source.build] {
        fs::create_dir_all(root).unwrap();
    }
    let pointer = serde_json::to_vec(&Location {
        version: 1,
        roots: source.clone(),
    })
    .unwrap();
    fs::write(&bootstrap, &pointer).unwrap();

    use_default_at(&bootstrap).unwrap();

    assert!(!bootstrap.exists());
    assert_eq!(
        fs::read(temp.0.join("location.json.old")).unwrap(),
        pointer,
        "the old pointer stays recoverable"
    );
    for root in [&source.launcher, &source.game, &source.build] {
        assert!(root.is_dir(), "{} is not deleted", root.display());
    }
}

#[cfg(unix)]
#[test]
fn a_config_folder_behind_a_link_still_opens() {
    // A dotfiles-managed ~/.config is a link: players who never moved their data must start
    // without an error, and a pointer kept there must still resolve and lock.
    let temp = Temp::new();
    let real = temp.0.join("dotfiles/config");
    fs::create_dir_all(&real).unwrap();
    let config = temp.0.join("config");
    std::os::unix::fs::symlink(&real, &config).unwrap();
    let bootstrap = config.join("horizon-launcher-location/location.json");

    assert_eq!(resolve_at(&bootstrap), Ok(None));
    assert_eq!(startup_error_at(&bootstrap), None);
    drop(acquire_lock_at(&bootstrap).unwrap());

    let roots = Roots {
        launcher: temp.0.join("data/launcher"),
        game: temp.0.join("data/game"),
        build: temp.0.join("data/build"),
    };
    for root in [&roots.launcher, &roots.game, &roots.build] {
        fs::create_dir_all(root).unwrap();
    }
    let pointer = serde_json::to_vec(&Location {
        version: 1,
        roots: roots.clone(),
    })
    .unwrap();
    fs::write(&bootstrap, pointer).unwrap();
    assert_eq!(resolve_at(&bootstrap), Ok(Some(roots)));

    // The lock behind the link is the one a worker takes through the real path.
    let reader = acquire_lock_at(&bootstrap).unwrap();
    assert!(lock_at(&real.join("horizon-launcher-location/location.json"), true).is_err());
    drop(reader);
}

/// A link above the data folder (a ~/.local/share on another disk) is not refused: only the root
/// itself may not be one.
#[cfg(unix)]
#[test]
fn a_link_above_the_root_is_followed() {
    let base = std::env::temp_dir().join(format!("horizon-dl-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(base.join("disk/data")).unwrap();
    std::os::unix::fs::symlink(base.join("disk"), base.join("share")).unwrap();
    let checked = checked_path(&base.join("share/data")).unwrap();
    assert_eq!(checked, fs::canonicalize(base.join("disk/data")).unwrap());
    std::os::unix::fs::symlink(base.join("disk/data"), base.join("disk/linked")).unwrap();
    assert!(
        checked_path(&base.join("share/linked")).is_err(),
        "the root itself"
    );
    let _ = fs::remove_dir_all(&base);
}

/// No unit test reaches the player's real folders: a test's launcher data, build folders and
/// native build log are all under the temporary folder ([`test_home`]), or under the home a run
/// names in `HORIZON_LAUNCHER_HOME`, which takes its place (`backend::data_dir`;
/// `tools/pr_ready.py` names one for the launcher's tests, #16877).
#[test]
fn unit_tests_never_resolve_the_real_profile() {
    let paths = [
        crate::backend::data_dir(),
        crate::backend::build_dir(),
        crate::native_mods::log_path(),
    ];
    // The named home as given (the data folder) and as the build folder resolves it.
    let homes = match std::env::var_os("HORIZON_LAUNCHER_HOME").filter(|h| !h.is_empty()) {
        Some(home) => {
            let home = PathBuf::from(home);
            let real = home.canonicalize().map(crate::backend::plain);
            vec![real.unwrap_or_else(|_| home.clone()), home]
        }
        None => vec![test_home()],
    };
    for path in paths {
        assert!(
            homes.iter().any(|home| path.starts_with(home)),
            "{} is outside {}",
            path.display(),
            homes[0].display()
        );
    }
}

#[cfg(windows)]
#[test]
fn windows_moves_unicode_and_spaced_data_with_a_physical_ascii_build_folder() {
    for build in ["build", "launcher", "launcher/native-build"] {
        let f = Fixture::new(build);
        let missing = f.temp.0.join("My Games").join("Horizon");
        let existing = f.temp.0.join("Users/Ирина/AppData/Local/Horizon");
        let ordinary = f.temp.0.join("Horizon_Data-2");
        fs::create_dir_all(&existing).unwrap();
        let mut source = f.source.clone();
        for target in [&missing, &existing, &ordinary] {
            let moved = migrate(&source, target, &f.bootstrap, |_| {}).unwrap();
            assert_eq!(moved.launcher, target.join("launcher"));
            assert_eq!(moved.game, target.join("game"));
            assert_eq!(
                resolve_at(&f.bootstrap).unwrap(),
                Some(moved.clone()),
                "restart"
            );
            let physical_build = checked_path(&moved.build).unwrap();
            tool_safe(&physical_build).unwrap();
            assert_eq!(physical_build, moved.build, "not an alias");
            if target == &ordinary {
                assert_eq!(moved.build, target.join("build"));
            } else {
                assert!(within(&moved.build, &f.temp.0));
                assert!(!within(&moved.build, target));
            }
            f.migrated(&moved);
            source = moved;
        }
    }
}

#[cfg(windows)]
#[test]
fn separate_build_copy_and_commit_failures_keep_the_original_pointer() {
    use std::os::windows::fs::OpenOptionsExt;
    for commit_failure in [false, true] {
        let f = Fixture::new("build");
        let locked = if commit_failure {
            f.bootstrap.clone()
        } else {
            f.source.build.join("cache/compiled.bin")
        };
        let guard = OpenOptions::new()
            .read(true)
            .share_mode(if commit_failure { 1 | 2 } else { 0 })
            .open(locked)
            .unwrap();
        let target = f.temp.0.join("Игры with spaces");
        let mut committing = false;
        let error = migrate(&f.source, &target, &f.bootstrap, |s| {
            committing |= s == "Committing data location"
        })
        .unwrap_err();
        drop(guard);
        assert_eq!(committing, commit_failure);
        assert!(error.contains("Original data was kept"));
        assert!(error.contains("horizon-build-"), "{error}");
        f.old_pointer();
        let retry = f.temp.0.join("Retry Игры");
        let moved = migrate(&f.source, &retry, &f.bootstrap, |_| {}).unwrap();
        f.migrated(&moved);
    }
}

#[cfg(windows)]
#[test]
fn separate_build_is_staged_without_publishing_until_preparation_succeeds() {
    let f = Fixture::new("build");
    let target = f.temp.0.join("Игры");
    let error = migrate_with_prepare(
        &f.source,
        &target,
        &f.bootstrap,
        |_| {},
        |stage, final_roots| {
            assert_eq!(stage.build, final_roots.build);
            assert_eq!(
                fs::read(stage.build.join("cache/compiled.bin")).unwrap(),
                b"\0\x01\xfe\xffbuild"
            );
            assert_eq!(resolve_at(&f.bootstrap).unwrap(), Some(f.source.clone()));
            Err("fixture preparation failure".into())
        },
    )
    .unwrap_err();
    assert!(error.contains("fixture preparation failure"));
    assert!(error.contains("horizon-build-"));
    f.old_pointer();
    let retry = f.temp.0.join("Retry Игры");
    let moved = migrate(&f.source, &retry, &f.bootstrap, |_| {}).unwrap();
    f.migrated(&moved);
    let offline = moved.build.with_file_name("disconnected");
    fs::rename(&moved.build, &offline).unwrap();
    assert!(resolve_at(&f.bootstrap).unwrap_err().contains("missing"));
    fs::rename(offline, &moved.build).unwrap();
    assert_eq!(resolve_at(&f.bootstrap).unwrap(), Some(moved));
}

#[test]
fn a_move_stopped_between_files_keeps_everything() {
    let f = Fixture::new("build");
    let target = f.target();
    let lease = MigrationLease::take(&f.bootstrap).unwrap();
    let mut files = 0;
    let error = migrate_locked(
        &f.source,
        &target,
        &f.bootstrap,
        |message| {
            files += usize::from(message == progress::FILE);
            if files == 1 {
                return Err("fixture window closed".into());
            }
            Ok(())
        },
        |_, _| Ok(()),
        &lease,
    )
    .unwrap_err();
    drop(lease);
    assert!(error.contains("fixture window closed"), "{error}");
    assert_eq!(files, 1);
    f.old_pointer();
    assert_eq!(kept_copies(&target, None).len(), 1);
    assert!(reusable(&target));
}

/// A stop asked for while `prepare` runs is honoured before the first rename; one asked for
/// after that is not, and the move commits.
#[test]
fn a_stop_is_honoured_only_before_the_renames() {
    for late in [false, true] {
        let f = Fixture::new("build");
        let target = f.target();
        let lease = MigrationLease::take(&f.bootstrap).unwrap();
        let stop = std::cell::Cell::new(false);
        let result = migrate_locked(
            &f.source,
            &target,
            &f.bootstrap,
            |message| {
                if stop.get() {
                    return Err("fixture window closed".into());
                }
                // The last report: a stop asked for now is already past the last stop point.
                stop.set(late && message == progress::COMMIT);
                Ok(())
            },
            |_, _| {
                stop.set(!late);
                Ok(())
            },
            &lease,
        );
        drop(lease);
        if late {
            f.migrated(&result.unwrap());
        } else {
            let error = result.unwrap_err();
            assert!(error.contains("fixture window closed"), "{error}");
            f.old_pointer();
            assert!(!target.join("launcher").exists());
            assert!(reusable(&target));
        }
    }
}
