//! The signing command as release.yml runs it.
use horizon_release_sign::{SIGNATURE, SUMS, SigningKey, signed_digest, trusted_keys};
use std::process::Command;

fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-release-sign-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("horizon-linux-x86_64.tar.gz"), b"linux build").unwrap();
    std::fs::write(dir.join("horizon-windows-x64.zip"), b"windows build").unwrap();
    dir
}

fn sign(dir: &std::path::Path, extra: &[&str], key: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_horizon-release-sign"));
    cmd.args(["sign", "--tag", "v2026.10.19"])
        .args(extra)
        .arg(dir.join("horizon-linux-x86_64.tar.gz"))
        .arg(dir.join("horizon-windows-x64.zip"))
        .env_remove("RELEASE_SIGNING_KEY")
        .env_remove("RELEASE_SIGNING_KEY_NEXT");
    if let Some(key) = key {
        cmd.env("RELEASE_SIGNING_KEY", key);
    }
    cmd.output().unwrap()
}

#[test]
fn a_dry_run_signs_with_a_throwaway_key() {
    let dir = folder("dry");
    let out = sign(&dir, &["--ephemeral"], None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join(SUMS).exists() && dir.join(SIGNATURE).exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_release_signs_with_the_secret_and_launchers_can_check_it() {
    let dir = folder("key");
    let (key, secret) = SigningKey::generate().unwrap();
    let out = sign(&dir, &[], Some(&secret));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains(&secret) && !String::from_utf8_lossy(&out.stderr).contains(&secret));
    if trusted_keys().is_empty() {
        // Before release-keys.txt lists a key: signed, with a warning naming the public key.
        assert!(out.status.success());
        assert!(stdout.contains(&key.public_base64()), "{stdout}");
        let sums = std::fs::read(dir.join(SUMS)).unwrap();
        let signature = std::fs::read_to_string(dir.join(SIGNATURE)).unwrap();
        let digest = signed_digest(
            "v2026.10.19",
            "horizon-windows-x64.zip",
            Some(&sums),
            Some(&signature),
            &[key.public()],
        );
        assert_eq!(
            digest,
            Ok(horizon_release_sign::sha256_hex(b"windows build"))
        );
    } else {
        // A key release-keys.txt doesn't list fails the release.
        assert!(!out.status.success());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_a_secret_it_publishes_unsigned_only_until_a_key_is_listed() {
    let dir = folder("none");
    let out = sign(&dir, &[], None);
    assert_eq!(out.status.success(), trusted_keys().is_empty());
    assert!(!dir.join(SIGNATURE).exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn keygen_writes_the_secret_to_a_pipe_and_the_public_key_to_a_file() {
    let dir = folder("keygen");
    let public = dir.join("key.pub");
    let out = Command::new(env!("CARGO_BIN_EXE_horizon-release-sign"))
        .args(["keygen", "--public-out"])
        .arg(&public)
        .output()
        .unwrap();
    assert!(out.status.success());
    let key = SigningKey::from_base64(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&public).unwrap().trim(),
        key.public_base64()
    );
    std::fs::remove_dir_all(dir).unwrap();
}
