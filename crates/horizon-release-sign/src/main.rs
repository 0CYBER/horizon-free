//! horizon-release-sign: signs a release's checksums in release.yml and launcher.yml, checks a
//! downloaded release, and makes new keys (docs/security.md, "Signed updates").
//!
//!     horizon-release-sign sign --tag v2026.10.19 dist/*.zip dist/*.tar.gz
//!     horizon-release-sign sign --tag v2026.10.19 --ephemeral dist/*      # a dry run: a throwaway key
//!     horizon-release-sign verify --tag v2026.10.19 <folder>
//!     horizon-release-sign keygen --public-out key.pub | gh secret set RELEASE_SIGNING_KEY --env release
//!
//! `sign` reads the private keys from RELEASE_SIGNING_KEY (and RELEASE_SIGNING_KEY_NEXT while
//! rotating), writes SHA256SUMS and SHA256SUMS.sig beside the first file, and checks the result
//! the way the launcher does, against the keys built in from release-keys.txt. Until that file
//! lists a key it publishes SHA256SUMS alone with a warning; once it does, a missing or unknown
//! signing key fails the release instead of publishing something launchers would refuse.

use horizon_release_sign::{
    KEY_VARIABLES, SIGNATURE, SUMS, Sha256, SigningKey, parse_sums, sums_file, trusted_keys, verify,
};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("sign") => sign(&args[1..]),
        Some("verify") => check_folder(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        _ => Err(
            "usage: horizon-release-sign sign --tag <tag> [--ephemeral] <file>... | \
                  verify --tag <tag> <folder> | keygen --public-out <file>"
                .into(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("horizon-release-sign: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--name value` from `args`, and the rest.
fn option(args: &[String], name: &str) -> (Option<String>, Vec<String>) {
    let mut value = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            value = it.next().cloned();
        } else {
            rest.push(arg.clone());
        }
    }
    (value, rest)
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::default();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            return Ok(hash.hex());
        }
        hash.update(&buffer[..n]);
    }
}

fn sign(args: &[String]) -> Result<(), String> {
    let (tag, rest) = option(args, "--tag");
    let tag = tag.ok_or("sign needs --tag")?;
    let ephemeral = rest.iter().any(|a| a == "--ephemeral");
    let files: Vec<PathBuf> = rest
        .iter()
        .filter(|a| *a != "--ephemeral")
        .map(PathBuf::from)
        .collect();
    let first = files.first().ok_or("sign needs the files to sign")?;
    let folder = first.parent().unwrap_or(Path::new("."));
    let mut listed = Vec::new();
    for file in &files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{}: no file name", file.display()))?;
        if name != SUMS && name != SIGNATURE {
            listed.push((name.to_string(), file_hash(file)?));
        }
    }
    let sums = sums_file(&listed);
    std::fs::write(folder.join(SUMS), &sums).map_err(|e| e.to_string())?;

    let trusted = trusted_keys();
    let keys: Vec<SigningKey> = if ephemeral {
        vec![SigningKey::generate()?.0]
    } else {
        KEY_VARIABLES
            .iter()
            .filter_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
            .map(|secret| SigningKey::from_base64(&secret))
            .collect::<Result<_, _>>()?
    };
    if keys.is_empty() {
        if !trusted.is_empty() {
            return Err(format!(
                "no signing key (the release environment's {}): launchers would refuse an unsigned \
                 release, so it is not published",
                KEY_VARIABLES[0]
            ));
        }
        println!(
            "::warning::{tag}: published with {SUMS} but unsigned: release-keys.txt lists no key yet"
        );
        return Ok(());
    }
    let signature: String = keys
        .iter()
        .map(|key| key.sign(&tag, sums.as_bytes()) + "\n")
        .collect();
    std::fs::write(folder.join(SIGNATURE), &signature).map_err(|e| e.to_string())?;
    let check_with = if ephemeral || trusted.is_empty() {
        if !ephemeral {
            println!(
                "::warning::{tag}: signed, but release-keys.txt lists no key yet: add {} to it",
                keys[0].public_base64()
            );
        }
        keys.iter().map(SigningKey::public).collect()
    } else {
        trusted
    };
    verify(&tag, sums.as_bytes(), &signature, &check_with).map_err(|_| {
        "the signing key is not one release-keys.txt lists: launchers would refuse this release"
            .to_string()
    })?;
    println!(
        "{tag}: {} files signed with {} key(s){}",
        listed.len(),
        keys.len(),
        if ephemeral {
            " (a throwaway key: dry run)"
        } else {
            ""
        }
    );
    Ok(())
}

/// Checks a folder holding a release's assets with the built-in keys, as the launcher does.
fn check_folder(args: &[String]) -> Result<(), String> {
    let (tag, rest) = option(args, "--tag");
    let tag = tag.ok_or("verify needs --tag")?;
    let folder = PathBuf::from(rest.first().ok_or("verify needs the release's folder")?);
    let read = |name: &str| std::fs::read(folder.join(name)).map_err(|e| format!("{name}: {e}"));
    let sums = read(SUMS)?;
    let signature = String::from_utf8(read(SIGNATURE)?).map_err(|e| e.to_string())?;
    let listed = verify(&tag, &sums, &signature, &trusted_keys())?;
    for (name, hex) in &listed {
        horizon_release_sign::check(name, hex, &file_hash(&folder.join(name))?)?;
    }
    let text = String::from_utf8_lossy(&sums);
    println!(
        "{tag}: signature good, {} files match",
        parse_sums(&text)?.len()
    );
    Ok(())
}

/// Prints a new private key to stdout, which must be a pipe (into `gh secret set`), never a
/// terminal, and writes its public key to `--public-out`.
fn keygen(args: &[String]) -> Result<(), String> {
    let (public_out, _) = option(args, "--public-out");
    let public_out = public_out.ok_or("keygen needs --public-out <file>")?;
    if std::io::stdout().is_terminal() {
        return Err(
            "keygen writes the private key to its output: pipe it into `gh secret set`".into(),
        );
    }
    let (key, secret) = SigningKey::generate()?;
    std::fs::write(&public_out, key.public_base64() + "\n").map_err(|e| e.to_string())?;
    let mut out = std::io::stdout().lock();
    out.write_all(secret.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}
