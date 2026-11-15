//! Signed release checksums (docs/security.md, "Signed updates").
//!
//! Every release carries `SHA256SUMS` (`sha256sum`'s format, `<hex>  <file>` per line) and
//! `SHA256SUMS.sig`: one base64 Ed25519 signature per line, each over `horizon release <tag>\n`
//! followed by SHA256SUMS's bytes, so a signed list can't be passed off under another tag.
//! release.yml and launcher.yml sign with the key(s) in the `release` environment's secrets
//! (tools/ci/sign_release.sh, OpenSSL only); the launcher and `horizon-server update` check a
//! download against the public keys built in from `release-keys.txt` before installing it, and
//! refuse it otherwise.
//!
//! Releases published before signing began carry no SHA256SUMS. Their archives' SHA-256s are
//! pinned in `legacy-releases.txt` (taken from GitHub when signing began, by
//! tools/ci/legacy_releases.py), so they stay installable for a rollback without trusting
//! whatever GitHub serves for them later.

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};

pub mod native_build;

/// The checksum list's asset name.
pub const SUMS: &str = "SHA256SUMS";
/// Its signatures' asset name.
pub const SIGNATURE: &str = "SHA256SUMS.sig";
/// A Nightly's second signature file: the same checksum list signed for the Nightly's own version
/// (`horizon release nightly-<date>-<commit>`) rather than for the channel, so an older Nightly can't
/// be served as a newer one. Nightlies carry both while launchers that only read [`SIGNATURE`] are
/// in use (docs/security.md, "Nightly signatures"); current launchers and servers require this one.
pub const SIGNATURE_V2: &str = "SHA256SUMS.v2.sig";

const KEYS: &str = include_str!("../release-keys.txt");
const LEGACY: &str = include_str!("../legacy-releases.txt");

/// An Ed25519 public key.
pub type PublicKey = [u8; 32];

/// The public keys built in: those in `release-keys.txt`. A file that doesn't parse gives none,
/// so every download is refused rather than trusted (a test keeps the file valid).
pub fn trusted_keys() -> Vec<PublicKey> {
    trusted_keys_for("horizon-rust/horizon")
}

/// Repository-scoped release trust: `release-keys.txt` for horizon-rust/horizon. Any other repository has no
/// trusted keys, so its downloads fail closed.
pub fn trusted_keys_for(repository: &str) -> Vec<PublicKey> {
    parse_keys(repository_keys(repository, KEYS)).unwrap_or_default()
}

/// The environment variable that adds one more trusted SDK key: exactly one base64 Ed25519 public key, what
/// `tools/ci/sign_release.sh` writes to `ephemeral.pub` under `EPHEMERAL=yes`. A value that is not exactly one
/// key is reported on stderr and adds nothing.
pub const SDK_KEY_VAR: &str = "HORIZON_SDK_TRUSTED_KEY";

/// The keys an SDK release (`sdk-<build>`) may be signed with: [`trusted_keys_for`] and, in a build with
/// `debug_assertions` only, the key in [`SDK_KEY_VAR`]. The daily run signs its own SDK with a throwaway key per
/// run and hands the public half to a debug-assertions launcher through that variable, so its SDK is checked end
/// to end without a release key. A release build does not compile the variable's lookup at all, so no environment
/// can add trust to a shipped launcher or `cargo horizon-mod`. SDK trust only: game and launcher releases are
/// checked with [`trusted_keys_for`].
pub fn sdk_keys_for(repository: &str) -> Vec<PublicKey> {
    let mut keys = trusted_keys_for(repository);
    keys.extend(extra_sdk_keys());
    keys
}

#[cfg(debug_assertions)]
fn extra_sdk_keys() -> Vec<PublicKey> {
    let Ok(text) = std::env::var(SDK_KEY_VAR) else {
        return Vec::new();
    };
    match parse_keys(&text) {
        Ok(keys) if keys.len() == 1 => keys,
        Ok(keys) => {
            eprintln!(
                "{SDK_KEY_VAR} ignored: expected exactly one public key, found {}",
                keys.len()
            );
            Vec::new()
        }
        Err(error) => {
            eprintln!("{SDK_KEY_VAR} ignored: {error}");
            Vec::new()
        }
    }
}

#[cfg(not(debug_assertions))]
fn extra_sdk_keys() -> Vec<PublicKey> {
    Vec::new()
}

fn repository_keys<'a>(repository: &str, keys: &'a str) -> &'a str {
    match repository {
        "horizon-rust/horizon" => keys,
        _ => "",
    }
}

/// The keys in a keys file: one base64 key per line; blank lines and `#` comments are skipped.
pub fn parse_keys(text: &str) -> Result<Vec<PublicKey>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let bytes = STANDARD
                .decode(line)
                .map_err(|e| format!("{line}: not base64 ({e})"))?;
            PublicKey::try_from(bytes.as_slice())
                .map_err(|_| format!("{line}: not a 32-byte Ed25519 public key"))
        })
        .collect()
}

/// What a signature covers: the tag, then the checksum list.
pub fn message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut message = format!("horizon release {tag}\n").into_bytes();
    message.extend_from_slice(sums);
    message
}

/// A checksum list for `(file name, SHA-256 hex)` pairs, as `sha256sum` writes it.
pub fn sums_file(files: &[(String, String)]) -> String {
    files
        .iter()
        .map(|(name, hex)| format!("{hex}  {name}\n"))
        .collect()
}

/// The `(file name, SHA-256 hex)` pairs of a checksum list (`<hex>  <name>` or `<hex> *<name>`).
pub fn parse_sums(text: &str) -> Result<Vec<(String, String)>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (hex, name) = line
                .split_once(' ')
                .ok_or_else(|| format!("{SUMS}: {line:?} is not `<sha256>  <file>`"))?;
            let name = name.strip_prefix([' ', '*']).unwrap_or(name);
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) || name.is_empty() {
                return Err(format!("{SUMS}: {line:?} is not `<sha256>  <file>`"));
            }
            Ok((name.to_string(), hex.to_ascii_lowercase()))
        })
        .collect()
}

/// Checks that one of `keys` signed `sums` for `tag`, and returns the list.
pub fn verify(
    tag: &str,
    sums: &[u8],
    signature: &str,
    keys: &[PublicKey],
) -> Result<Vec<(String, String)>, String> {
    if keys.is_empty() {
        return Err(
            "This build has no release key built in, so it can't check downloads: none are installed."
                .into(),
        );
    }
    let message = message(tag, sums);
    let signed = signature
        .lines()
        .filter_map(|line| STANDARD.decode(line.trim()).ok())
        .any(|signature| {
            keys.iter().any(|key| {
                UnparsedPublicKey::new(&ED25519, key)
                    .verify(&message, &signature)
                    .is_ok()
            })
        });
    if !signed {
        return Err(format!(
            "{tag}'s signature doesn't match Horizon's release key, so it was not installed. \
             The download may have been tampered with: please report it on Discord or GitHub."
        ));
    }
    let text = std::str::from_utf8(sums).map_err(|_| format!("{SUMS} is not text"))?;
    parse_sums(text)
}

/// The channel nightly builds are signed for (nightly-build.yml): their message is
/// `horizon release nightly`, which no release's tag is, so a nightly's signature never passes for a
/// release's, nor a release's for a nightly's.
pub const NIGHTLY: &str = "nightly";

/// The channel snapshots are signed for (release.yml kind snapshot, through nightly-build.yml): a
/// commit of main built to test it after merge, published as `snapshot-<date>-<commit>`. No
/// launcher lists or updates to one; a developer installs one by its tag.
pub const SNAPSHOT: &str = "snapshot";

/// The channels of builds that are not releases. Each is signed for its own name and for each
/// build's own version (`<channel>-<date>-<commit>`), so no build passes for another channel's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Nightly,
    Snapshot,
}

impl Channel {
    const ALL: [Channel; 2] = [Channel::Nightly, Channel::Snapshot];

    /// The channel's tag, which its channel signature is for: [`NIGHTLY`] or [`SNAPSHOT`].
    pub const fn tag(self) -> &'static str {
        match self {
            Channel::Nightly => NIGHTLY,
            Channel::Snapshot => SNAPSHOT,
        }
    }

    /// How its release titles start (`Nightly 2026-10-28 · 0c02067`).
    const fn title(self) -> &'static str {
        match self {
            Channel::Nightly => "Nightly",
            Channel::Snapshot => "Snapshot",
        }
    }

    /// The channel of a tag or version: the channel's own tag, or one that starts `<tag>-`.
    pub fn of(tag: &str) -> Option<Channel> {
        Self::ALL.into_iter().find(|channel| {
            tag.strip_prefix(channel.tag())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        })
    }
}

/// Whether `tag` is a nightly's: `nightly`, or a past one kept as `nightly-<date>`.
pub fn is_nightly(tag: &str) -> bool {
    Channel::of(tag) == Some(Channel::Nightly)
}

/// The version a nightly is listed as, from its release title ("Nightly 2026-10-28 · 0c02067"):
/// `nightly-2026-10-28-0c02067`. `published` (GitHub's `published_at`) dates a title without one.
/// For listing only: what installs is checked against [`build_title_version`], which has no
/// fallback, since nothing signs `published`.
pub fn nightly_version(title: &str, published: &str) -> String {
    let mut words = title
        .strip_prefix("Nightly ")
        .unwrap_or_default()
        .split(" · ");
    let date = words
        .next()
        .filter(|d| is_date(d))
        .unwrap_or_else(|| published.get(..10).unwrap_or(published));
    let commit = words
        .next()
        .filter(|c| !c.is_empty() && c.chars().all(|c| c.is_ascii_hexdigit()));
    match commit {
        Some(commit) => format!("{NIGHTLY}-{date}-{commit}"),
        None => format!("{NIGHTLY}-{date}"),
    }
}

/// The date of a nightly's version or tag (`nightly-2026-10-28-0c02067`, `nightly-2026-10-28`):
/// nightlies follow every release and each other by it.
pub fn nightly_date(version: &str) -> Option<&str> {
    let date = version.strip_prefix("nightly-")?.get(..10)?;
    is_date(date).then_some(date)
}

fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// The prefix of a mod SDK's release tag: `sdk-<build id>`, the game's full commit sha
/// (design: the SDK is published as its own signed release per build).
pub const SDK_PREFIX: &str = "sdk-";

/// Whether `tag` names a mod SDK's release (`sdk-…`), whatever follows the prefix.
pub fn is_sdk(tag: &str) -> bool {
    tag.starts_with(SDK_PREFIX)
}

/// The signed SHA-256 of `asset` in release `tag`, or why the download must be refused.
/// `sums` and `signature` are the release's SHA256SUMS and SHA256SUMS.sig, None when it has none.
/// A nightly or a snapshot is never a release: their tags are refused here, whatever their
/// signature ([`build_digest`] checks those). Nor is a mod SDK: its files are never game files
/// ([`sdk_digest`] checks those).
pub fn signed_digest(
    tag: &str,
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    match Channel::of(tag) {
        Some(Channel::Nightly) => {
            return Err(format!(
                "{tag} is a Nightly build: only a launcher with Nightly turned on installs it."
            ));
        }
        Some(Channel::Snapshot) => {
            return Err(format!(
                "{tag} is a snapshot build: only `horizon-launcher install {tag}` installs it."
            ));
        }
        None => {}
    }
    if is_sdk(tag) {
        return Err(format!(
            "{tag} is a mod SDK, not a game release: it is never installed as the game."
        ));
    }
    digest_in(tag, tag, asset, sums, signature, keys)
}

/// The signed SHA-256 of `asset` in the mod SDK of game build `build` (a full 40-character commit
/// sha): `sums` must be signed for exactly `sdk-<build>`, so neither a game release's list nor
/// another build's SDK passes for it.
pub fn sdk_digest(
    build: &str,
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    if build.len() != 40
        || !build
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!(
            "{build:?} is not a game build's full commit, so its SDK can't be checked."
        ));
    }
    let tag = format!("{SDK_PREFIX}{build}");
    digest_in(&tag, &tag, asset, sums, signature, keys)
}

/// The version a build of `channel` names in its GitHub title, strictly: on the Nightly channel
/// `Nightly 2026-10-28 · 0c02067` (as nightly-build.yml writes it) gives
/// `nightly-2026-10-28-0c02067`, on the snapshot channel `Snapshot 2026-11-13 · 0c02067` gives
/// `snapshot-2026-11-13-0c02067`. Anything else is None, the other channel's title included: the
/// caller says which channel it asked for, never the title. A build is verified against the
/// version it installs as, so a title that doesn't name its date and commit is refused rather than
/// dated by when GitHub says it was published.
pub fn build_title_version(channel: Channel, title: &str) -> Option<String> {
    let (date, commit) = title
        .strip_prefix(channel.title())?
        .strip_prefix(' ')?
        .split_once(" · ")?;
    (is_date(date) && is_commit(commit)).then(|| format!("{}-{date}-{commit}", channel.tag()))
}

fn is_commit(text: &str) -> bool {
    (7..=40).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `version` is a full version of a build of `channel`, `<channel>-<date>-<commit>`, the
/// form its [`SIGNATURE_V2`] is signed for.
pub fn is_build(channel: Channel, version: &str) -> bool {
    version
        .strip_prefix(channel.tag())
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.split_at_checked(10))
        .is_some_and(|(date, commit)| {
            is_date(date) && commit.strip_prefix('-').is_some_and(is_commit)
        })
}

/// The signed SHA-256 of `asset` in the build of `channel` (the channel the caller asked for) that
/// installs as `version` (`<channel>-<date>-<commit>`, from [`build_title_version`]): `signature`
/// is its [`SIGNATURE_V2`], which must sign `sums` for exactly that version. A version of the other
/// channel is refused before any signature is read; a release's list, a channel signature
/// (`nightly`, `snapshot`) and another build's are refused by the signature, so an older Nightly
/// can't be passed off as tonight's, nor a snapshot as a Nightly.
pub fn build_digest(
    channel: Channel,
    version: &str,
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    if !is_build(channel, version) {
        return Err(format!(
            "{version:?} doesn't name a {} build's date and commit, so it can't be checked: not installed.",
            channel.title()
        ));
    }
    let Some(signature) = signature else {
        return Err(format!(
            "{version} has no {SIGNATURE_V2} (its signature for this exact build): it was published \
             before Nightlies were signed for their own version, so it can't be checked and was not \
             installed. Pick a Nightly published since then; `horizon-server rollback` still goes back \
             to a build a server already has."
        ));
    };
    digest_in(version, version, asset, sums, Some(signature), keys)
}

/// `asset`'s SHA-256 in `sums`, once one of `keys` is found to have signed them for `signed_for`;
/// `name` says which build in messages.
fn digest_in(
    signed_for: &str,
    name: &str,
    asset: &str,
    sums: Option<&[u8]>,
    signature: Option<&str>,
    keys: &[PublicKey],
) -> Result<String, String> {
    let (Some(sums), Some(signature)) = (sums, signature) else {
        return Err(format!(
            "{name} is not signed, and Horizon installs only signed releases: pick a newer version."
        ));
    };
    verify(signed_for, sums, signature, keys)?
        .into_iter()
        .find(|(n, _)| n == asset)
        .map(|(_, hex)| hex)
        .ok_or_else(|| {
            format!("{asset} is not in {name}'s signed checksums, so it was not installed.")
        })
}

/// The pinned SHA-256 of `asset` in a release from before signing (legacy-releases.txt). Callers
/// use it instead of the release's files when there is one: [`signed_digest`] otherwise.
pub fn legacy_digest(tag: &str, asset: &str) -> Option<&'static str> {
    legacy_digest_for("horizon-rust/horizon", tag, asset)
}

/// Historical pins belong to horizon-rust/horizon only: no other repository has unsigned game-release history.
pub fn legacy_digest_for(repository: &str, tag: &str, asset: &str) -> Option<&'static str> {
    if repository != "horizon-rust/horizon" {
        return None;
    }
    legacy_entries(LEGACY).find_map(|(t, a, hex)| (t == tag && a == asset).then_some(hex))
}

/// The `<tag> <asset> <sha256>` lines of a legacy releases file; blank lines and `#` comments skipped.
fn legacy_entries(text: &str) -> impl Iterator<Item = (&str, &str, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?, parts.next()?))
        })
}

/// Whether a downloaded file's SHA-256 (`got`) is the signed one.
pub fn check(asset: &str, expected: &str, got: &str) -> Result<(), String> {
    if expected.eq_ignore_ascii_case(got) {
        Ok(())
    } else {
        Err(format!(
            "{asset} doesn't match its signed checksum (corrupted or tampered with), so it was \
             not installed. Try again."
        ))
    }
}

/// SHA-256 over data that arrives in pieces (a download).
pub struct Sha256(ring::digest::Context);

impl Default for Sha256 {
    fn default() -> Self {
        Self(ring::digest::Context::new(&ring::digest::SHA256))
    }
}

impl Sha256 {
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    pub fn hex(self) -> String {
        self.0
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// The SHA-256 of `bytes`, in hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::default();
    hash.update(bytes);
    hash.hex()
}

/// A private signing key: `keygen` makes them, and tests sign with them (releases are signed by
/// OpenSSL in tools/ci/sign_release.sh, from the same PKCS#8 document).
pub struct SigningKey(Ed25519KeyPair);

impl SigningKey {
    /// A new key, and its PKCS#8 document in base64 (what the secret holds).
    ///
    /// The document is PKCS#8 v1 (RFC 8410: the 32-byte seed alone), which every OpenSSL 3 reads;
    /// ring makes v2 (with the public key too), which OpenSSL 3.0, Ubuntu's, refuses.
    pub fn generate() -> Result<(Self, String), String> {
        const V2_PREFIX: [u8; 16] = [
            0x30, 0x51, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        const V1_PREFIX: [u8; 16] = [
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        let v2 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .map_err(|_| "no randomness to make a key with".to_string())?;
        let seed = v2
            .as_ref()
            .strip_prefix(&V2_PREFIX)
            .and_then(|rest| rest.get(..32))
            .ok_or("ring's key document has an unexpected layout")?;
        let v1 = [&V1_PREFIX[..], seed].concat();
        let key = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&v1)
            .map_err(|_| "the new key doesn't parse".to_string())?;
        let same = Ed25519KeyPair::from_pkcs8(v2.as_ref())
            .is_ok_and(|k| k.public_key().as_ref() == key.public_key().as_ref());
        if !same {
            return Err("the new key's two forms disagree".into());
        }
        Ok((Self(key), STANDARD.encode(&v1)))
    }

    /// The key a secret holds (base64 PKCS#8). The error never quotes the secret.
    pub fn from_base64(text: &str) -> Result<Self, String> {
        let document = STANDARD
            .decode(text.trim())
            .map_err(|_| "the signing key is not base64".to_string())?;
        Ed25519KeyPair::from_pkcs8_maybe_unchecked(&document)
            .map(Self)
            .map_err(|_| "the signing key is not an Ed25519 PKCS#8 key".to_string())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey::try_from(self.0.public_key().as_ref()).expect("Ed25519 public keys are 32 bytes")
    }

    /// The public key in base64, as release-keys.txt lists it.
    pub fn public_base64(&self) -> String {
        STANDARD.encode(self.public())
    }

    /// A signature line for SHA256SUMS.sig.
    pub fn sign(&self, tag: &str, sums: &[u8]) -> String {
        self.sign_bytes(&message(tag, sums))
    }

    /// A base64 signature line over `message` as it is (a prebuilt native build's
    /// [`native_build::message`]).
    pub fn sign_bytes(&self, message: &[u8]) -> String {
        STANDARD.encode(self.0.sign(message).as_ref())
    }
}

#[cfg(test)]
mod tests {
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
        let sdk = |signature: &str| {
            sdk_digest(build, asset, Some(sums.as_bytes()), Some(signature), &keys)
        };
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
            let as_release =
                signed_digest(tag, asset, Some(sums.as_bytes()), Some(&signature), &keys);
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
        let tonight =
            build_title_version(Channel::Nightly, "Nightly 2026-10-28 · 0c02067").unwrap();
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
        let list =
            parse_sums(&format!("{hex}  a.zip\n{} *b.tar.gz\n", hex.to_uppercase())).unwrap();
        assert_eq!(
            list,
            [("a.zip".into(), hex.clone()), ("b.tar.gz".into(), hex)]
        );
        assert!(parse_sums("nothex  a.zip\n").is_err());
    }
}
