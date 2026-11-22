#[test]
fn repository_trust_is_isolated_and_unknown_repositories_fail_closed() {
    let (upstream, _) = SigningKey::generate().unwrap();
    let (other, _) = SigningKey::generate().unwrap();
    let upstream_text = upstream.public_base64();
    let keys = |repo| parse_keys(repository_keys(repo, &upstream_text)).unwrap();
    let sums = sums_file(&[("game.tar.gz".into(), "ab".repeat(32))]);
    let tag = "nightly-2026-11-10-abcdef0";
    let repository = "horizon-rust/horizon";
    assert!(
        verify(
            tag,
            sums.as_bytes(),
            &upstream.sign(tag, sums.as_bytes()),
            &keys(repository)
        )
        .is_ok()
    );
    assert!(
        verify(
            tag,
            sums.as_bytes(),
            &other.sign(tag, sums.as_bytes()),
            &keys(repository)
        )
        .is_err()
    );
    assert!(
        verify(
            "nightly",
            sums.as_bytes(),
            &upstream.sign(tag, sums.as_bytes()),
            &keys(repository)
        )
        .is_err()
    );
    assert!(keys("someone/horizon").is_empty());
    assert!(keys("unknown/horizon").is_empty());
    assert!(trusted_keys_for("unknown/horizon").is_empty());
    assert_eq!(
        trusted_keys_for("horizon-rust/horizon"),
        parse_keys(KEYS).unwrap()
    );
}

/// Runs in both profiles (CI runs it with `--release` too): the variable's key is trusted for an SDK
/// exactly when `debug_assertions` is on, and never for a game or launcher release.
#[test]
fn sdk_key_variable_is_honoured_only_with_debug_assertions() {
    let (signer, _) = SigningKey::generate().unwrap();
    let public = parse_keys(&signer.public_base64()).unwrap()[0];
    // SAFETY: this is the only test that touches the variable, and nothing else here reads it.
    unsafe { std::env::set_var(SDK_KEY_VAR, signer.public_base64()) };
    let repository = "horizon-rust/horizon";
    let trusted = sdk_keys_for(repository).contains(&public);
    assert_eq!(trusted, cfg!(debug_assertions));
    assert_eq!(
        sdk_keys_for(repository).len(),
        trusted_keys_for(repository).len() + usize::from(cfg!(debug_assertions))
    );
    assert!(!trusted_keys_for(repository).contains(&public));
    let sums = sums_file(&[("sdk.json".into(), "ab".repeat(32))]);
    let signature = signer.sign("sdk-abc", sums.as_bytes());
    assert_eq!(
        verify(
            "sdk-abc",
            sums.as_bytes(),
            &signature,
            &sdk_keys_for(repository)
        )
        .is_ok(),
        cfg!(debug_assertions)
    );
    let (second, _) = SigningKey::generate().unwrap();
    // Unparseable, empty, and more than one key: none is trusted (debug builds also report it on stderr).
    for bad in [
        "not a key".to_string(),
        String::new(),
        format!("{}\n{}", signer.public_base64(), second.public_base64()),
    ] {
        unsafe { std::env::set_var(SDK_KEY_VAR, &bad) };
        assert_eq!(sdk_keys_for(repository), trusted_keys_for(repository));
    }
    unsafe { std::env::remove_var(SDK_KEY_VAR) };
}

use super::*;

fn signed(tag: &str) -> (SigningKey, String, String) {
    let (key, _) = SigningKey::generate().unwrap();
    let sums = sums_file(&[
        ("horizon-linux-x86_64.tar.gz".into(), sha256_hex(b"linux")),
        ("horizon-windows-x64.zip".into(), sha256_hex(b"windows")),
    ]);
    let signature = key.sign(tag, sums.as_bytes()) + "\n";
    (key, sums, signature)
}

#[test]
fn an_sdk_signature_never_passes_for_a_game_release_nor_the_reverse() {
    let build = "0123456789abcdef0123456789abcdef01234567";
    let sdk_tag = format!("{SDK_PREFIX}{build}");
    let (key, sums, sdk_signature) = signed(&sdk_tag);
    let keys = [key.public()];
    let asset = "horizon-linux-x86_64.tar.gz";
    let sdk =
        |signature: &str| sdk_digest(build, asset, Some(sums.as_bytes()), Some(signature), &keys);
    assert_eq!(sdk(&sdk_signature), Ok(sha256_hex(b"linux")));
    // The SDK's list, signed for its tag, is refused as a game release: under its own tag by
    // name, and under a game release's tag by its signature.
    for tag in [sdk_tag.as_str(), "v2026.11.13"] {
        let refused = signed_digest(
            tag,
            asset,
            Some(sums.as_bytes()),
            Some(&sdk_signature),
            &keys,
        );
        assert!(refused.is_err(), "{tag}");
    }
    let nightly = "nightly-2026-11-13-0123456";
    assert!(
        build_digest(
            Channel::Nightly,
            nightly,
            asset,
            Some(sums.as_bytes()),
            Some(&sdk_signature),
            &keys
        )
        .is_err()
    );
    // A game release's or a Nightly's signature is refused as the SDK's, and so is another
    // build's SDK.
    for tag in ["v2026.11.13", NIGHTLY, nightly] {
        assert!(sdk(&key.sign(tag, sums.as_bytes())).is_err(), "{tag}");
    }
    let other = format!("{SDK_PREFIX}{}", "f".repeat(40));
    assert!(sdk(&key.sign(&other, sums.as_bytes())).is_err());
    assert!(
        sdk_digest(
            "0123456",
            asset,
            Some(sums.as_bytes()),
            Some(&sdk_signature),
            &keys
        )
        .is_err()
    );
}

#[test]
fn the_built_in_keys_file_parses() {
    assert!(parse_keys(KEYS).is_ok());
}

#[test]
fn releases_from_before_signing_keep_their_pinned_digests() {
    let mut seen = std::collections::HashSet::new();
    for line in LEGACY.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(parts.len(), 3, "{line}");
        assert_eq!(parts[2].len(), 64, "{line}");
        assert!(
            parts[2]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{line}"
        );
        assert!(seen.insert((parts[0], parts[1])), "listed twice: {line}");
    }
    if let Some((tag, asset, hex)) = legacy_entries(LEGACY).next() {
        assert_eq!(legacy_digest(tag, asset), Some(hex));
        assert_eq!(legacy_digest(tag, "not-an-asset.zip"), None);
    }
    assert_eq!(
        legacy_digest("v2099.01.01", "horizon-windows-x64.zip"),
        None
    );
}

#[test]
fn a_signed_release_gives_its_digests() {
    let (key, sums, signature) = signed("v2026.10.19");
    let digest = signed_digest(
        "v2026.10.19",
        "horizon-windows-x64.zip",
        Some(sums.as_bytes()),
        Some(&signature),
        &[key.public()],
    );
    assert_eq!(digest, Ok(sha256_hex(b"windows")));
    assert!(check("x", &sha256_hex(b"windows"), &sha256_hex(b"windows")).is_ok());
    assert!(check("x", &sha256_hex(b"windows"), &sha256_hex(b"linux")).is_err());
}

/// The release channel refuses everything signed for the nightly channel, and the other way
/// round: one key signs both, and only the message's channel keeps them apart.
#[test]
fn nightly_and_release_signatures_never_pass_for_each_other() {
    const TONIGHT: &str = "nightly-2026-10-28-0c02067";
    let (nightly, sums, signature) = signed(TONIGHT);
    let keys = [nightly.public()];
    let asset = "horizon-windows-x64.zip";
    let digest = build_digest(
        Channel::Nightly,
        TONIGHT,
        asset,
        Some(sums.as_bytes()),
        Some(&signature),
        &keys,
    );
    assert_eq!(digest, Ok(sha256_hex(b"windows")));
    // Served under a release's tag: the message names the Nightly, not v0.2.81.
    for tag in ["v0.2.81", "v2026.10.28", "0.2.81"] {
        let as_release = signed_digest(tag, asset, Some(sums.as_bytes()), Some(&signature), &keys);
        assert!(as_release.is_err(), "{tag} took a nightly's signature");
    }
    // Asked for by a nightly tag on the release channel: refused before any signature is read.
    for tag in [NIGHTLY, "nightly-2026-10-27", TONIGHT] {
        let err = signed_digest(tag, asset, Some(sums.as_bytes()), Some(&signature), &keys);
        assert!(err.unwrap_err().contains("Nightly build"), "{tag}");
    }
    // A release's signed list replayed as tonight's nightly.
    let (release, sums, signature) = signed("v0.2.81");
    let keys = [release.public()];
    assert!(
        build_digest(
            Channel::Nightly,
            TONIGHT,
            asset,
            Some(sums.as_bytes()),
            Some(&signature),
            &keys
        )
        .is_err()
    );
    assert!(
        build_digest(
            Channel::Nightly,
            TONIGHT,
            asset,
            Some(sums.as_bytes()),
            None,
            &keys
        )
        .is_err()
    );
    assert!(is_nightly("nightly-2026-10-27") && !is_nightly("v0.2.81"));
}

#[test]
fn an_older_nightly_is_not_tonights() {
    let asset = "horizon-windows-x64.zip";
    // Last week's build, re-served under tonight's title: its v2 signature names last week.
    let (key, sums, old) = signed("nightly-2026-10-21-aaaaaaa");
    let keys = [key.public()];
    let tonight = build_title_version(Channel::Nightly, "Nightly 2026-10-28 · 0c02067").unwrap();
    let err = build_digest(
        Channel::Nightly,
        &tonight,
        asset,
        Some(sums.as_bytes()),
        Some(&old),
        &keys,
    );
    assert!(err.is_err(), "an older Nightly passed as {tonight}");
    // The channel signature (SHA256SUMS.sig, for launchers from before v2) is no v2 signature.
    let (key, sums, channel) = signed(NIGHTLY);
    let keys = [key.public()];
    assert!(
        build_digest(
            Channel::Nightly,
            &tonight,
            asset,
            Some(sums.as_bytes()),
            Some(&channel),
            &keys
        )
        .is_err()
    );
    // And a version that isn't a full Nightly build is refused whatever signs it.
    for version in [NIGHTLY, "nightly-2026-10-28", "v0.2.81"] {
        let (key, sums, signature) = signed(version);
        let err = build_digest(
            Channel::Nightly,
            version,
            asset,
            Some(sums.as_bytes()),
            Some(&signature),
            &[key.public()],
        );
        assert!(err.unwrap_err().contains("doesn't name"), "{version}");
    }
}

#[test]
fn a_nightlys_title_is_parsed_strictly() {
    assert_eq!(
        build_title_version(Channel::Nightly, "Nightly 2026-10-28 · 0c02067").as_deref(),
        Some("nightly-2026-10-28-0c02067")
    );
    assert_eq!(
        build_title_version(
            Channel::Nightly,
            &format!("Nightly 2026-10-28 · {}", "a".repeat(40))
        ),
        Some(format!("nightly-2026-10-28-{}", "a".repeat(40)))
    );
    for title in [
        "Something else",
        "Nightly 2026-10-28",
        "Nightly 2026-10-28 · ",
        "Nightly 2026-10-28 · 0c020",     // too short
        "Nightly 2026-10-28 · 0C02067",   // GitHub's short SHAs are lowercase
        "Nightly 2026-10-28 · 0c02067 x", // trailing words
        "Nightly 2026-1-28 · 0c02067",    // not a date
        "nightly 2026-10-28 · 0c02067",
        " Nightly 2026-10-28 · 0c02067",
        "Nightly 2026-10-28 · 0c02067/../x",
    ] {
        assert_eq!(
            build_title_version(Channel::Nightly, title),
            None,
            "{title:?}"
        );
    }
    assert!(is_build(Channel::Nightly, "nightly-2026-10-28-0c02067"));
    assert!(!is_build(Channel::Nightly, "nightly-2026-10-28"));
    assert!(!is_build(Channel::Nightly, "nightly-2026-10-28-"));
    assert!(!is_build(Channel::Nightly, "nightly-2026-10-28-0c02067-x"));
}

/// A snapshot's signature never passes for a Nightly, nor a Nightly's for a snapshot, though one
/// key signs both; the release channel refuses snapshot tags as it refuses nightly ones.
#[test]
fn snapshot_and_nightly_signatures_never_pass_for_each_other() {
    const SNAPSHOT_BUILD: &str = "snapshot-2026-11-13-0c02067";
    const NIGHTLY_BUILD: &str = "nightly-2026-11-13-0c02067";
    let asset = "horizon-windows-x64.zip";
    let digest = |channel, version: &str, signed_for: &str| {
        let (key, sums, signature) = signed(signed_for);
        build_digest(
            channel,
            version,
            asset,
            Some(sums.as_bytes()),
            Some(&signature),
            &[key.public()],
        )
    };
    use Channel::{Nightly, Snapshot};
    assert_eq!(
        digest(Snapshot, SNAPSHOT_BUILD, SNAPSHOT_BUILD),
        Ok(sha256_hex(b"windows"))
    );
    assert_eq!(
        digest(Nightly, NIGHTLY_BUILD, NIGHTLY_BUILD),
        Ok(sha256_hex(b"windows"))
    );
    assert!(digest(Nightly, NIGHTLY_BUILD, SNAPSHOT_BUILD).is_err());
    assert!(digest(Snapshot, SNAPSHOT_BUILD, NIGHTLY_BUILD).is_err());
    // A genuine build of the other channel, asked for on this one: refused before its
    // signature is read, though that signature is valid for it.
    let other = digest(Nightly, SNAPSHOT_BUILD, SNAPSHOT_BUILD).unwrap_err();
    assert!(other.contains("doesn't name a Nightly build"), "{other}");
    assert!(digest(Snapshot, NIGHTLY_BUILD, NIGHTLY_BUILD).is_err());
    assert_eq!(
        build_title_version(Nightly, "Snapshot 2026-11-13 · 0c02067"),
        None
    );
    assert_eq!(
        build_title_version(Snapshot, "Nightly 2026-11-13 · 0c02067"),
        None
    );
    assert!(
        digest(Snapshot, SNAPSHOT_BUILD, SNAPSHOT).is_err(),
        "the channel's signature"
    );
    for tag in [SNAPSHOT, SNAPSHOT_BUILD] {
        let (key, sums, signature) = signed(tag);
        let err = signed_digest(
            tag,
            asset,
            Some(sums.as_bytes()),
            Some(&signature),
            &[key.public()],
        );
        assert!(err.unwrap_err().contains("snapshot build"), "{tag}");
    }
}

#[test]
fn channels_are_read_from_their_prefix_only() {
    assert_eq!(Channel::of("nightly"), Some(Channel::Nightly));
    assert_eq!(Channel::of("nightly-2026-10-27"), Some(Channel::Nightly));
    assert_eq!(Channel::of("snapshot"), Some(Channel::Snapshot));
    assert_eq!(
        Channel::of("snapshot-2026-11-13-0c02067"),
        Some(Channel::Snapshot)
    );
    for tag in [
        "v0.2.81",
        "nightlyx",
        "snapshots-2026-11-13",
        "prebuilt-0c02067",
        "",
        "Snapshot",
    ] {
        assert_eq!(Channel::of(tag), None, "{tag:?}");
    }
    assert!(!is_nightly("snapshot-2026-11-13-0c02067"));
    assert_eq!(
        build_title_version(Channel::Snapshot, "Snapshot 2026-11-13 · 0c02067").as_deref(),
        Some("snapshot-2026-11-13-0c02067")
    );
    for title in [
        "Snapshot 2026-11-13",
        "snapshot 2026-11-13 · 0c02067",
        "Snapshot2026-11-13 · 0c02067",
        "Release 2026-11-13 · 0c02067",
        "Snapshot 2026-11-13 · 0C02067",
    ] {
        assert_eq!(
            build_title_version(Channel::Snapshot, title),
            None,
            "{title:?}"
        );
    }
    assert!(is_build(Channel::Snapshot, "snapshot-2026-11-13-0c02067"));
    for version in [
        "snapshot-2026-11-13",
        "snapshot",
        "release-2026-11-13-0c02067",
    ] {
        assert!(!is_build(Channel::Snapshot, version), "{version}");
    }
}

#[test]
fn a_nightly_is_named_by_its_date_and_commit() {
    let v = nightly_version("Nightly 2026-10-28 · 0c02067", "2026-10-28T20:40:00Z");
    assert_eq!(v, "nightly-2026-10-28-0c02067");
    assert_eq!(
        nightly_version("Something else", "2026-10-29T01:00:00Z"),
        "nightly-2026-10-29"
    );
    assert_eq!(nightly_date(&v), Some("2026-10-28"));
    assert_eq!(nightly_date("nightly-2026-10-27"), Some("2026-10-27"));
    assert_eq!(nightly_date("nightly"), None);
    assert_eq!(nightly_date("v0.2.95"), None);
    assert!(is_nightly(&v));
}

#[test]
fn tampering_is_refused() {
    let (key, sums, signature) = signed("v2026.10.19");
    let keys = [key.public()];
    // Another tag: an old signed list replayed for a new release.
    assert!(verify("v2026.10.20", sums.as_bytes(), &signature, &keys).is_err());
    // A changed checksum.
    let changed = sums.replacen(&sha256_hex(b"linux"), &sha256_hex(b"evil"), 1);
    assert!(verify("v2026.10.19", changed.as_bytes(), &signature, &keys).is_err());
    // Another key.
    let (other, _) = SigningKey::generate().unwrap();
    assert!(
        verify(
            "v2026.10.19",
            sums.as_bytes(),
            &signature,
            &[other.public()]
        )
        .is_err()
    );
    // No signature, no keys, an asset that isn't listed.
    let none = signed_digest("v2026.10.19", "a", None, None, &keys).unwrap_err();
    assert!(none.contains("not signed"), "{none}");
    assert!(verify("v2026.10.19", sums.as_bytes(), &signature, &[]).is_err());
    let missing = signed_digest(
        "v2026.10.19",
        "horizon-macos-arm64.zip",
        Some(sums.as_bytes()),
        Some(&signature),
        &keys,
    );
    assert!(missing.unwrap_err().contains("not in"));
}

#[test]
fn any_line_signed_by_any_trusted_key_passes() {
    // While rotating, releases carry one signature per key and launchers trust one or both.
    let (old, sums, old_line) = signed("v1");
    let (new, _) = SigningKey::generate().unwrap();
    let both = format!("{old_line}{}\n", new.sign("v1", sums.as_bytes()));
    assert!(verify("v1", sums.as_bytes(), &both, &[old.public()]).is_ok());
    assert!(verify("v1", sums.as_bytes(), &both, &[new.public()]).is_ok());
    assert!(verify("v1", sums.as_bytes(), "garbage\n\n", &[old.public()]).is_err());
}

#[test]
fn keys_round_trip_through_their_text_forms() {
    let (key, secret) = SigningKey::generate().unwrap();
    // PKCS#8 v1 (version 0, the seed alone): what OpenSSL 3.0 reads.
    let document = STANDARD.decode(&secret).unwrap();
    assert_eq!((document.len(), document[4]), (48, 0));
    let again = SigningKey::from_base64(&format!("{secret}\n")).unwrap();
    assert_eq!(again.public(), key.public());
    assert_eq!(
        parse_keys(&format!("# comment\n\n{}\n", key.public_base64())),
        Ok(vec![key.public()])
    );
    assert!(parse_keys("AAAA\n").is_err());
    let error = SigningKey::from_base64("bm90IGEga2V5").err().unwrap();
    assert!(!error.contains("bm90"));
}

#[test]
fn checksum_lists_read_both_sha256sum_forms() {
    let hex = sha256_hex(b"x");
    let list = parse_sums(&format!("{hex}  a.zip\n{} *b.tar.gz\n", hex.to_uppercase())).unwrap();
    assert_eq!(
        list,
        [("a.zip".into(), hex.clone()), ("b.tar.gz".into(), hex)]
    );
    assert!(parse_sums("nothex  a.zip\n").is_err());
}
