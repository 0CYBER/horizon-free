use super::*;
use std::fs;

struct Home(PathBuf);
impl Home {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let home = std::env::temp_dir().join(format!(
            "horizon-cleanup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&home).unwrap();
        Self(home)
    }
    fn file(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"old compiler output").unwrap();
        path
    }
    fn dir(&self) -> Dir {
        Dir::open_ambient_dir(&self.0, cap_std::ambient_authority()).unwrap()
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A directory link as the game makes one: a symlink on Unix, a junction on Windows.
#[cfg(unix)]
fn link_dir(from: &Path, to: &Path) {
    std::os::unix::fs::symlink(from, to).unwrap();
}
#[cfg(windows)]
fn link_dir(from: &Path, to: &Path) {
    // mklink takes plain paths: no forward slash and no verbatim prefix.
    let plain = |path: &Path| {
        let path = path.to_string_lossy().replace('/', "\\");
        path.trim_start_matches(r"\\?\").to_owned()
    };
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(plain(to))
        .arg(plain(from))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// #17911: a link the game made in its cache is not followed when the cache is searched for a
/// running build's locks, so the cache can be removed whole. Where none is expected, a link is
/// still refused.
#[test]
fn a_game_made_link_holds_none_of_the_units_locks() {
    let home = Home::new();
    let outside = Home::new();
    outside.file("version/textures/.cargo-lock");
    home.file("resourcepacks/0123456789abcdef/pack.json");
    link_dir(
        &outside.0.join("version/textures"),
        &home.0.join("resourcepacks/0123456789abcdef/textures"),
    );

    assert!(matches!(
        contains_operational_locks(&home.dir(), Links::Refused),
        Err(Error::Unsafe(_))
    ));
    assert!(!contains_operational_locks(&home.dir(), Links::GameMade).unwrap());
    home.file("resourcepacks/0123456789abcdef/.cargo-lock");
    assert!(contains_operational_locks(&home.dir(), Links::GameMade).unwrap());
    assert!(outside.0.join("version/textures/.cargo-lock").exists());
}

#[test]
fn late_cargo_is_refused_before_removal_and_can_start_after_completion() {
    let home = Home::new();
    let output = home.file("target/debug/output");
    let directory = home.dir();
    let plan = Plan::pin(&directory).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(home.0.join("target/debug/.cargo-lock"))
        .unwrap();
    held.lock().unwrap();
    fs::write(&output, b"late Cargo output").unwrap();
    assert!(matches!(plan.clear(&directory), Err(Error::Busy)));
    assert_eq!(fs::read(&output).unwrap(), b"late Cargo output");
    held.unlock().unwrap();
    let plan = Plan::pin(&directory).unwrap();
    let late = home.file("target/new-profile/new-output");
    plan.clear_observed(&directory, &|_| {}, &|| {
        if !output.exists() {
            assert!(matches!(held.try_lock(), Err(fs::TryLockError::WouldBlock)));
            assert!(fs::read_dir(&home.0).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".trash-")));
        }
    })
    .unwrap();
    assert!(!output.exists());
    assert!(late.exists());
    held.try_lock().unwrap();
    fs::write(&output, b"started after completed removal").unwrap();
    held.unlock().unwrap();
    assert_eq!(
        fs::read(output).unwrap(),
        b"started after completed removal"
    );
}

#[cfg(unix)]
#[test]
fn swapped_directory_and_file_links_never_delete_external_data() {
    let home = Home::new();
    let outside = Home::new();
    let precious = outside.file("world");
    home.file("cache/output");
    let selected = home.file("selected");
    let directory = home.dir();
    let plan = Plan::pin(&directory).unwrap();
    fs::rename(home.0.join("cache"), home.0.join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside.0, home.0.join("cache")).unwrap();
    fs::remove_file(&selected).unwrap();
    std::os::unix::fs::symlink(&precious, &selected).unwrap();
    assert!(plan.clear(&directory).is_err());
    assert_eq!(fs::read(precious).unwrap(), b"old compiler output");
    assert!(selected
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn existing_profile_lock_refuses_pin_before_any_deletion() {
    let home = Home::new();
    let output = home.file("target/debug/output");
    Plan::pin(&home.dir()).unwrap();
    let lock = home.0.join("target/debug/.cargo-lock");
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock)
        .unwrap();
    held.lock().unwrap();
    assert!(matches!(Plan::pin(&home.dir()), Err(Error::Busy)));
    assert!(output.exists());
}

#[test]
fn pin_file_clears_only_one_file_and_refuses_lock_or_nested_names() {
    let home = Home::new();
    let selected = home.file("selected");
    let kept = home.file("kept");
    home.file("nested/output");
    home.file(".build.lock");
    let directory = home.dir();
    for name in [
        ".cargo-lock",
        ".package-cache",
        ".package-cache-mutate",
        ".build.lock",
        "nested/output",
    ] {
        assert!(Plan::pin_file(&directory, Path::new(name)).is_err());
    }
    Plan::pin_file(&directory, Path::new("selected"))
        .unwrap()
        .clear(&directory)
        .unwrap();
    assert!(!selected.exists());
    assert!(kept.exists());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&kept, home.0.join("link")).unwrap();
        assert!(Plan::pin_file(&directory, Path::new("link")).is_err());
        assert!(kept.exists());
    }
}

#[cfg(target_os = "linux")]
#[test]
fn flat_cache_fits_a_small_descriptor_limit() {
    if std::env::var_os("HORIZON_CLEANUP_FD_CHILD").is_none() {
        use std::os::unix::process::CommandExt;
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--include-ignored",
                "--exact",
                "cache_cleanup::tests::flat_cache_fits_a_small_descriptor_limit",
                "--nocapture",
            ])
            .env("HORIZON_CLEANUP_FD_CHILD", "1");
        // Only the isolated child gets the lowered limit, never another test's process.
        unsafe {
            child.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 1024,
                    rlim_max: 1024,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let output = child.output().unwrap();
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let home = Home::new();
    for index in 0..5000 {
        home.file(&format!("target/profile-{index}/output"));
    }
    let mut deep = PathBuf::new();
    for _ in 0..220 {
        deep.push("d");
    }
    home.file(deep.join("output").to_str().unwrap());
    let directory = home.dir();
    let peak = std::cell::Cell::new(0);
    let observe = || {
        let descriptors = fs::read_dir("/proc/self/fd").unwrap().count();
        peak.set(peak.get().max(descriptors));
    };
    let plan = Plan::pin_observed(&directory, &observe).unwrap();
    assert_eq!(plan.outputs.len(), 5001);
    plan.clear_observed(&directory, &|_| {}, &observe).unwrap();
    assert!(
        peak.get() <= 3 * 220 + 64,
        "peak descriptors: {}",
        peak.get()
    );
    eprintln!(
        "Observed peak: {} descriptors for 5,000 sibling profiles and depth 220",
        peak.get()
    );
    assert!(!home.0.join("target/profile-4999/output").exists());
    assert!(!home.0.join(deep).join("output").exists());
}

#[test]
fn root_lock_is_held_before_enumeration() {
    let home = Home::new();
    home.file("output");
    let observed = std::cell::Cell::new(false);
    Plan::pin_observed(&home.dir(), &|| {
        let actor = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(home.0.join(".cargo-lock"))
            .unwrap();
        assert!(matches!(
            actor.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        observed.set(true);
    })
    .unwrap();
    assert!(observed.get());
}

#[cfg(unix)]
#[test]
fn linked_lock_is_not_followed_even_when_its_target_is_inside_the_root() {
    let home = Home::new();
    let kept = home.file("kept");
    std::os::unix::fs::symlink("kept", home.0.join(".cargo-lock")).unwrap();
    assert!(Plan::pin(&home.dir()).is_err());
    assert_eq!(fs::read(kept).unwrap(), b"old compiler output");
}

#[cfg(windows)]
#[test]
fn pinned_root_cannot_be_renamed_away_on_windows() {
    let home = Home::new();
    let output = home.file("output");
    let _plan = Plan::pin(&home.dir()).unwrap();
    assert!(fs::rename(&home.0, home.0.with_extension("held")).is_err());
    assert!(output.exists());
}

#[cfg(unix)]
#[test]
fn replacing_the_root_path_does_not_redirect_plan_or_trash() {
    let home = Home::new();
    home.file("output");
    let directory = home.dir();
    let plan = Plan::pin(&directory).unwrap();
    let moved = home.0.with_extension("held");
    fs::rename(&home.0, &moved).unwrap();
    let precious = home.file("output");
    plan.clear(&directory).unwrap();
    assert_eq!(fs::read(precious).unwrap(), b"old compiler output");
    assert!(!moved.join("output").exists());
    fs::remove_dir_all(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[test]
fn package_cache_leases_refuse_pinning_and_late_clear() {
    for name in &LOCKS[1..] {
        let home = Home::new();
        let output = home.file("cache/output");
        let lock = home.file(name);
        let owner = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(lock)
            .unwrap();
        owner.lock().unwrap();
        assert!(matches!(Plan::pin(&home.dir()), Err(Error::Busy)));
        assert!(output.exists());
        owner.unlock().unwrap();
        let plan = Plan::pin(&home.dir()).unwrap();
        owner.lock().unwrap();
        fs::write(&output, b"package-cache owner").unwrap();
        assert!(matches!(plan.clear(&home.dir()), Err(Error::Busy)));
        assert_eq!(fs::read(output).unwrap(), b"package-cache owner");
        owner.unlock().unwrap();
    }
}

/// #18669: eviction chose an instance's cache while the instance was idle. A launch that
/// claimed the instance since keeps the cache, and a launch while a unit of it is being deleted
/// is refused as busy until the delete ends.
#[test]
fn a_launch_and_a_delete_in_one_instance_never_overlap() {
    let home = Home::new();
    home.file(".horizon-cache/resourcepacks/0123456789abcdef/pack.json");
    let cache = home.0.join(".horizon-cache");
    let launch = || {
        let mut game = std::process::Command::new("game");
        game.current_dir(&home.0);
        crate::run_leases::shared(&game)
    };
    // The launch's claims, taken before the game starts and held while it runs.
    let running = launch().unwrap();
    assert!(matches!(
        delete_game_unit_in(&cache, Links::GameMade, Some(&home.0)),
        Err(Error::Busy)
    ));
    assert!(cache
        .join("resourcepacks/0123456789abcdef/pack.json")
        .is_file());
    drop(running);
    // A delete under way holds the instance: the launch is refused until it ends.
    let deleting = claim_instance(&home.0).unwrap();
    assert!(matches!(launch(), Err(Error::Busy)));
    drop(deleting);
    drop(launch().unwrap());
}
