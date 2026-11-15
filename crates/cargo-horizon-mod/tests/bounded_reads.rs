use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/bounded-{name}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str]) -> String {
        self.run_launcher(args, "horizon-launcher")
    }
    fn run_launcher(&self, args: &[&str], launcher: &str) -> String {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .args(args)
            .env("HORIZON_LAUNCHER", launcher)
            .env("HORIZON_SDK_BASE", self.0.join("no-sdk"))
            .current_dir(&self.0)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let start = Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(10) {
                child.kill().unwrap();
                panic!("command hung: {args:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        String::from_utf8(output.stderr).unwrap()
    }
    fn manifest(&self) {
        fs::write(self.0.join("horizon-mod.toml"), format!("id='example'\nname='Test'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n", "a".repeat(40), "b".repeat(40))).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn large(path: &Path, limit: u64) {
    fs::File::create(path).unwrap().set_len(limit + 1).unwrap();
}
const COMMANDS: &[&[&str]] = &[
    &["inject", "add", "lower::run", "--kind", "head"],
    &["inject", "update"],
    &["inject", "sync", "--repo", "game"],
    &["internals", "add", "lower::run"],
    &["internals", "sync"],
];
#[test]
#[ignore = "daily"]
fn bounded_manifest_commands() {
    let f = Fixture::new("manifest");
    let path = f.0.join("horizon-mod.toml");
    let limit = horizon_mod_package::manifest::MAX_MANIFEST;
    large(&path, limit);
    for args in COMMANDS {
        let error = f.run(args);
        assert!(
            error.contains("horizon-mod.toml") && error.contains(&limit.to_string()),
            "{error}"
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), limit + 1);
    }
}
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn bounded_special_manifests() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new("special");
    let path = f.0.join("horizon-mod.toml");
    symlink("/dev/zero", &path).unwrap();
    for args in COMMANDS {
        assert!(f.run(args).contains("not a regular file"));
    }
    fs::remove_file(&path).unwrap();
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    for args in COMMANDS {
        assert!(f.run(args).contains("not a regular file"));
    }
}
#[test]
#[ignore = "daily"]
fn bounded_catalog_commands() {
    for (name, relative, commands) in [
        (
            "points",
            format!("{}/build-data/points.json", "a".repeat(40)),
            &COMMANDS[..2],
        ),
        (
            "old",
            "old/build-data/internals.json".into(),
            &COMMANDS[1..2],
        ),
        (
            "internals",
            format!("{}/build-data/internals.json", "a".repeat(40)),
            &COMMANDS[..2],
        ),
    ] {
        let f = Fixture::new(name);
        f.manifest();
        let cache = f.0.join(".horizon/cache");
        let current = cache.join("a".repeat(40)).join("build-data");
        fs::create_dir_all(&current).unwrap();
        fs::write(cache.join("a".repeat(40)).join("build"), "a".repeat(40)).unwrap();
        let header = r#""schema":1,"build":{"commit":"c"}"#;
        fs::write(
            current.join("internals.json"),
            format!(r#"{{{header},"internals":[]}}"#),
        )
        .unwrap();
        fs::write(
            current.join("points.json"),
            format!(r#"{{{header},"sites":[]}}"#),
        )
        .unwrap();
        let path = cache.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        large(&path, 256 << 20);
        let before = fs::read(f.0.join("horizon-mod.toml")).unwrap();
        for args in commands {
            let error = f.run(args);
            assert!(
                error.contains(path.file_name().unwrap().to_str().unwrap())
                    && error.contains("268435456"),
                "{error}"
            );
            assert_eq!(fs::metadata(&path).unwrap().len(), (256 << 20) + 1);
            assert_eq!(fs::read(f.0.join("horizon-mod.toml")).unwrap(), before);
        }
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn bounded_discovery_reread() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("discovery");
    f.manifest();
    let launcher = f.0.join("launcher");
    fs::write(
        &launcher,
        "#!/bin/sh\ntruncate -s 33554433 horizon-mod.toml\necho '[]'\n",
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let error = f.run_launcher(COMMANDS[4], launcher.to_str().unwrap());
    assert!(
        error.contains("horizon-mod.toml") && error.contains("33554432"),
        "{error}"
    );
    assert_eq!(
        fs::metadata(f.0.join("horizon-mod.toml")).unwrap().len(),
        33554433
    );
}
