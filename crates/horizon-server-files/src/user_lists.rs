//! ops.json, whitelist.json, banned-players.json and banned-ips.json
//! (StoredUserList and its entries): a Java HashMap keyed by the UUID string
//! or the address, saved as a pretty-printed Gson array in that map's order.
use super::{
    json::{self, Json},
    properties::parse_uuid,
};
use horizon_core::java_hash_map::{JavaHashMap, JavaKey};
use horizon_core::random::java_string_hash;
use horizon_java_properties::{civil_from_days, days_from_civil, now_millis};
use horizon_protocol::crypto::uuid_string;
use std::{cmp::Ordering, path::PathBuf};

/// A map key: a Java String.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JKey(pub String);
impl JavaKey for JKey {
    fn java_hash(&self) -> i32 {
        java_string_hash(self.0.encode_utf16())
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.encode_utf16().cmp(other.0.encode_utf16()))
    }
    fn tie_break(&self, _: &Self) -> Ordering {
        Ordering::Less
    }
}

/// NameAndId.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameAndId {
    pub id: u128,
    pub name: String,
}
impl NameAndId {
    pub fn offline(name: &str) -> Self {
        Self {
            id: horizon_protocol::crypto::offline_uuid(name),
            name: name.to_owned(),
        }
    }
    pub(crate) fn from_json(o: &Json) -> Option<Self> {
        let uuid = o.get("uuid")?.as_string()?;
        let name = o.get("name")?.as_string()?;
        Some(Self {
            id: parse_uuid(&uuid)?,
            name,
        })
    }
    pub(crate) fn append_to(&self, o: &mut Json) {
        o.add("uuid", Json::String(uuid_string(self.id)));
        o.add("name", Json::String(self.name.clone()));
    }
}

/// BanListEntry.DATE_FORMAT ("yyyy-MM-dd HH:mm:ss Z"), written in UTC.
pub fn format_date(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}
/// The lenient SimpleDateFormat parse of that pattern: fields may overflow
/// into the next (month 13 is January of the next year).
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (date, rest) = s.split_once(' ')?;
    let (time, zone) = rest.trim_start().split_once(' ')?;
    let num = |p: &str| -> Option<i64> {
        let p = p.trim();
        if p.is_empty()
            || !p
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b'-' || b == b'+')
        {
            return None;
        }
        p.parse().ok()
    };
    let d: Vec<&str> = date.splitn(3, '-').collect();
    let t: Vec<&str> = time.splitn(3, ':').collect();
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    let (y, mo, da) = (num(d[0])?, num(d[1])?, num(d[2])?);
    let (h, mi, se) = (num(t[0])?, num(t[1])?, num(t[2])?);
    let offset = zone_offset(zone.trim_start())?;
    // Lenient month and day arithmetic, checked: the file is edited by hand, and a value past what
    // milliseconds since 1970 hold is no date (None, as an unparsable one) rather than a wrapped one.
    let months = y.checked_mul(12)?.checked_add(mo.checked_sub(1)?)?;
    let (year, month) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    // Far beyond any i64 of milliseconds, and small enough for days_from_civil's own arithmetic.
    if !(-300_000_000..=300_000_000).contains(&year) {
        return None;
    }
    let days = days_from_civil(year, month as u32, 1)
        .checked_add(da)?
        .checked_sub(1)?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(h.checked_mul(3600)?)?
        .checked_add(mi.checked_mul(60)?)?
        .checked_add(se)?
        .checked_sub(offset * 60)?;
    seconds.checked_mul(1000)
}

/// The `Z` field of that lenient parse, in minutes east of UTC: `+hhmm` / `-hhmm` (exactly four digits,
/// hours to 23, minutes to 59), `GMT` alone or `GMT+h:mm` / `GMT-hh:mm`, or `UTC`, the names in any case.
/// What follows the zone is left unread, as Java's date parser leaves it. Other zone names
/// (`EST`...) are not read here.
fn zone_offset(zone: &str) -> Option<i64> {
    let digits = |text: &str| text.bytes().all(|b| b.is_ascii_digit());
    let minutes_east = |sign: &str, hours: &str, minutes: &str| -> Option<i64> {
        let (h, m) = (hours.parse::<i64>().ok()?, minutes.parse::<i64>().ok()?);
        (h <= 23 && m <= 59).then_some(if sign == "-" {
            -(h * 60 + m)
        } else {
            h * 60 + m
        })
    };
    let name = zone.get(..3).unwrap_or_default();
    if name.eq_ignore_ascii_case("utc") {
        return Some(0);
    }
    if name.eq_ignore_ascii_case("gmt") {
        let rest = &zone[3..];
        let Some(sign @ ("+" | "-")) = rest.get(..1) else {
            return Some(0);
        };
        // `h:mm` or `hh:mm`.
        let (hours, after) = rest[1..].split_once(':')?;
        let minutes = after.get(..2)?;
        if !matches!(hours.len(), 1 | 2) || !digits(hours) || !digits(minutes) {
            return None;
        }
        return minutes_east(sign, hours, minutes);
    }
    let sign @ ("+" | "-") = zone.get(..1)? else {
        return None;
    };
    let hhmm = zone.get(1..5)?;
    if !digits(hhmm) {
        return None;
    }
    minutes_east(sign, &hhmm[..2], &hhmm[2..])
}

/// One list entry kind.
pub trait Entry: Clone {
    type User: Clone;
    fn user(&self) -> &Self::User;
    fn key(user: &Self::User) -> String;
    fn from_json(o: &Json) -> Option<Self>;
    fn serialize(&self, o: &mut Json);
    fn expired(&self, _now: i64) -> bool {
        false
    }
    /// `infos.equals(previous)` in StoredUserList.add: only ban entries
    /// override equals; the others compare by identity, so a new entry is
    /// never equal to the stored one.
    fn same(&self, _other: &Self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpEntry {
    pub user: NameAndId,
    pub level: i32,
    pub bypasses_player_limit: bool,
}
impl Entry for OpEntry {
    type User = NameAndId;
    fn user(&self) -> &NameAndId {
        &self.user
    }
    fn key(user: &NameAndId) -> String {
        uuid_string(user.id)
    }
    fn from_json(o: &Json) -> Option<Self> {
        let user = NameAndId::from_json(o)?;
        let level = o
            .get("level")
            .and_then(json_int)
            .map_or(0, |l| l.clamp(0, 4));
        let bypasses_player_limit = o.get("bypassesPlayerLimit").is_some_and(json_bool);
        Some(Self {
            user,
            level,
            bypasses_player_limit,
        })
    }
    fn serialize(&self, o: &mut Json) {
        self.user.append_to(o);
        o.add("level", Json::Number(self.level.to_string()));
        o.add(
            "bypassesPlayerLimit",
            Json::Bool(self.bypasses_player_limit),
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhiteEntry {
    pub user: NameAndId,
}
impl Entry for WhiteEntry {
    type User = NameAndId;
    fn user(&self) -> &NameAndId {
        &self.user
    }
    fn key(user: &NameAndId) -> String {
        uuid_string(user.id)
    }
    fn from_json(o: &Json) -> Option<Self> {
        Some(Self {
            user: NameAndId::from_json(o)?,
        })
    }
    fn serialize(&self, o: &mut Json) {
        self.user.append_to(o);
    }
}

/// BanListEntry for players (NameAndId) or addresses (String).
#[derive(Clone, Debug)]
pub struct BanEntry<T> {
    pub user: T,
    pub created: i64,
    pub source: String,
    pub expires: Option<i64>,
    pub reason: Option<String>,
}
impl<T: PartialEq> PartialEq for BanEntry<T> {
    fn eq(&self, o: &Self) -> bool {
        self.source == o.source
            && self.expires == o.expires
            && self.reason == o.reason
            && self.user == o.user
    }
}
impl<T> BanEntry<T> {
    pub fn new(
        user: T,
        source: Option<String>,
        expires: Option<i64>,
        reason: Option<String>,
    ) -> Self {
        Self {
            user,
            created: now_millis(),
            source: source.unwrap_or_else(|| "(Unknown)".into()),
            expires,
            reason,
        }
    }
    fn fields(user: T, o: &Json) -> Self {
        let date = |k: &str| {
            o.get(k)
                .and_then(Json::as_string)
                .and_then(|s| parse_date(&s))
        };
        Self {
            user,
            created: date("created").unwrap_or_else(now_millis),
            source: o
                .get("source")
                .and_then(Json::as_string)
                .unwrap_or_else(|| "(Unknown)".into()),
            expires: date("expires"),
            reason: o.get("reason").and_then(Json::as_string),
        }
    }
    fn serialize_ban(&self, o: &mut Json) {
        o.add("created", Json::String(format_date(self.created)));
        o.add("source", Json::String(self.source.clone()));
        o.add(
            "expires",
            Json::String(self.expires.map_or_else(|| "forever".into(), format_date)),
        );
        o.add(
            "reason",
            self.reason.clone().map_or(Json::Null, Json::String),
        );
    }
}
impl Entry for BanEntry<NameAndId> {
    type User = NameAndId;
    fn same(&self, other: &Self) -> bool {
        self == other
    }
    fn user(&self) -> &NameAndId {
        &self.user
    }
    fn key(user: &NameAndId) -> String {
        uuid_string(user.id)
    }
    fn from_json(o: &Json) -> Option<Self> {
        Some(Self::fields(NameAndId::from_json(o)?, o))
    }
    fn serialize(&self, o: &mut Json) {
        self.user.append_to(o);
        self.serialize_ban(o);
    }
    fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e < now)
    }
}
impl Entry for BanEntry<String> {
    type User = String;
    fn same(&self, other: &Self) -> bool {
        self == other
    }
    fn user(&self) -> &String {
        &self.user
    }
    fn key(user: &String) -> String {
        user.clone()
    }
    fn from_json(o: &Json) -> Option<Self> {
        Some(Self::fields(o.get("ip")?.as_string()?, o))
    }
    fn serialize(&self, o: &mut Json) {
        o.add("ip", Json::String(self.user.clone()));
        self.serialize_ban(o);
    }
    fn expired(&self, now: i64) -> bool {
        self.expires.is_some_and(|e| e < now)
    }
}

/// Gson's int and boolean views for primitives.
fn json_int(v: &Json) -> Option<i32> {
    match v {
        Json::Number(n) => n
            .parse::<i64>()
            .ok()
            .map(|v| v as i32)
            .or_else(|| n.parse::<f64>().ok().map(|f| f as i32)),
        Json::String(s) => s.trim().parse::<i32>().ok(),
        _ => None,
    }
}
fn json_bool(v: &Json) -> bool {
    match v {
        Json::Bool(b) => *b,
        Json::String(s) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// What happened to a list, for the one that watches it: what vanilla's `StoredUserList` tells its
/// `NotificationService` (`playerOped`/`playerDeoped`, `playerAddedToAllowlist`/`playerRemovedFromAllowlist`,
/// `playerBanned`/`playerUnbanned`, `ipBanned`/`ipUnbanned`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change<E> {
    /// An entry was added (or replaced by a different one).
    Added(E),
    /// An entry was removed; a `clear` says it once for each entry, in the map's order, before it empties.
    Removed(E),
}

/// StoredUserList.
pub struct UserList<E: Entry> {
    pub file: PathBuf,
    map: JavaHashMap<JKey, E>,
    watcher: Option<Box<dyn Fn(Change<E>) + Send>>,
}
impl<E: Entry> UserList<E> {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self {
            file: file.into(),
            map: JavaHashMap::default(),
            watcher: None,
        }
    }
    /// Has `watcher` told of every change from now on (`add`, `remove` and `clear`, not a `load`, and not the
    /// expired entries `get` drops, as vanilla's don't).
    pub fn watch(&mut self, watcher: impl Fn(Change<E>) + Send + 'static) {
        self.watcher = Some(Box::new(watcher));
    }
    fn told(&self, change: Change<E>) {
        if let Some(watcher) = &self.watcher {
            watcher(change);
        }
    }
    /// The entries in the map's iteration order.
    pub fn entries(&self) -> Vec<&E> {
        self.map.entries().map(|(_, v)| v).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub fn contains(&self, user: &E::User) -> bool {
        self.map.get(&JKey(E::key(user))).is_some()
    }
    /// StoredUserList.get: expired entries are dropped first.
    pub fn get(&mut self, user: &E::User) -> Option<&E> {
        let now = now_millis();
        let expired: Vec<JKey> = self
            .map
            .entries()
            .filter(|(_, e)| e.expired(now))
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired {
            self.map.remove(&k);
        }
        self.map.get(&JKey(E::key(user)))
    }
    /// Returns false when an equal entry is already there; saves otherwise.
    pub fn add(&mut self, entry: E) -> bool {
        let key = JKey(E::key(entry.user()));
        if self.map.get(&key).is_some_and(|old| entry.same(old)) {
            return false;
        }
        self.map.put(key, entry.clone());
        if let Err(e) = self.save() {
            log::warn!("Could not save the list after adding a user. {e}");
        }
        self.told(Change::Added(entry));
        true
    }
    pub fn remove(&mut self, user: &E::User) -> bool {
        let Some(entry) = self.map.remove(&JKey(E::key(user))) else {
            return false;
        };
        if let Err(e) = self.save() {
            log::warn!("Could not save the list after removing a user. {e}");
        }
        self.told(Change::Removed(entry));
        true
    }
    /// StoredUserList.clear: no entry left; saves.
    pub fn clear(&mut self) {
        let entries: Vec<E> = self.map.entries().map(|(_, e)| e.clone()).collect();
        for entry in entries {
            self.told(Change::Removed(entry));
        }
        self.map.clear();
        if let Err(e) = self.save() {
            log::warn!("Could not save the list after removing a user. {e}");
        }
    }
    pub fn to_json(&self) -> String {
        let items = self
            .map
            .entries()
            .map(|(_, e)| {
                let mut o = Json::Object(Vec::new());
                e.serialize(&mut o);
                o
            })
            .collect();
        Json::Array(items).pretty()
    }
    pub fn save(&self) -> std::io::Result<()> {
        // An existing list is written in place, so its owner and access controls stay; a new one is
        // created whole.
        crate::durable::write_in_place(&self.file, self.to_json().as_bytes(), |bytes| {
            Self::whole(&read_utf8(bytes))
        })
    }
    /// StoredUserList.load: a missing file leaves the list as it is; a path that exists but cannot
    /// be read is an error, as vanilla's reader throws before it clears the map.
    pub fn load(&mut self) -> Result<(), String> {
        match read_saved(&self.file, |bytes| Self::whole(&read_utf8(bytes))) {
            Ok((text, copy)) => {
                if let Some(copy) = copy {
                    log::warn!(
                        "{} was cut off by an interrupted save; loading its complete copy {}",
                        self.file.display(),
                        copy.display()
                    );
                }
                self.load_str(&text)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", self.file.display())),
        }
    }
    /// `text` is a list every entry of which [`Self::load_str`] reads, none skipped.
    fn whole(text: &str) -> bool {
        matches!(json::parse(text, false), Ok(Json::Array(items)) if items
            .iter()
            .all(|item| matches!(item, Json::Object(_)) && E::from_json(item).is_some()))
    }
    pub fn load_str(&mut self, text: &str) -> Result<(), String> {
        self.map = JavaHashMap::default();
        if text.trim().is_empty() {
            return Ok(());
        }
        let Json::Array(items) = json::parse(text, false)? else {
            return Err("Expected a JSON array".into());
        };
        for item in items {
            if !matches!(item, Json::Object(_)) {
                return Err("Expected entry to be an object".into());
            }
            if let Some(e) = E::from_json(&item) {
                self.map.put(JKey(E::key(e.user())), e);
            }
        }
        Ok(())
    }
}

pub type OpList = UserList<OpEntry>;
pub type WhiteList = UserList<WhiteEntry>;
pub type BanList = UserList<BanEntry<NameAndId>>;
pub type IpBanList = UserList<BanEntry<String>>;

/// A list or cache file as vanilla reads it, through Guava's UTF-8 reader: a byte
/// sequence that is not UTF-8 becomes U+FFFD, grouped as Java's decoder groups it (`java_utf8`), and the
/// rest is kept.
pub(crate) fn read_utf8(bytes: &[u8]) -> String {
    horizon_protocol::buf::java_utf8(bytes)
}

/// A list or cache file's bytes, read under its lock (`durable::lock`), so a save of it under way
/// is waited for and never seen half written.
pub fn read_locked(file: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let _lock = crate::durable::lock(file)?;
    std::fs::read(file)
}

/// A list or cache file's text to load ([`read_utf8`]), and the copy it was read from instead, under
/// `durable::lock`. A file that is a JSON array is read, and copies left by earlier saves are removed.
/// Otherwise the copy of the save that cut this file off (`durable::cut_off_copy`: the file is the
/// start of its bytes, and `valid` takes it whole, every entry included; every copy is looked at,
/// newest first) is read, and that save is finished (the file gets the copy, the copy goes). A folder
/// or copy that cannot be read is an error. Anything else reads the file as it is.
pub(crate) fn read_saved(
    file: &std::path::Path,
    valid: impl Fn(&[u8]) -> bool,
) -> std::io::Result<(String, Option<std::path::PathBuf>)> {
    let lock = crate::durable::lock(file)?;
    let raw = std::fs::read(file)?;
    let text = read_utf8(&raw);
    // Unlocked, another save may be under way: its copy is neither read nor removed.
    if !lock.held() {
        return Ok((text, None));
    }
    if matches!(json::parse(&text, false), Ok(Json::Array(_))) {
        // The file is whole, so no copy holds anything to recover: a copy left in place (the folder
        // or a copy could not be listed or removed) is only ever read against a cut-off file.
        for copy in crate::durable::kept_copies(file).unwrap_or_default() {
            let _ = crate::durable::remove(&copy);
        }
        return Ok((text, None));
    }
    // A folder or copy that cannot be read is an error, never "no copy": the list stays as it is.
    let copies = crate::durable::kept_copies(file)?;
    let Some((copy, saved)) = crate::durable::cut_off_copy(&raw, &copies, valid)? else {
        return Ok((text, None));
    };
    if let Err(e) = crate::durable::finish_cut_off_save(file, &copy, &saved) {
        log::warn!(
            "{} could not be rewritten from {}: {e}",
            file.display(),
            copy.display()
        );
    }
    Ok((read_utf8(&saved), Some(copy)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn a_failed_save_leaves_the_previous_list() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::durable::scratch_dir("lists-save");
        let file = dir.join("ops.json");
        let old = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":3}]"#;
        std::fs::write(&file, old).unwrap();
        let mut ops = OpList::new(&file);
        ops.load_str("[]").unwrap();
        // A list that cannot be written is left as it was (nothing is truncated first).
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400)).unwrap();
        let probe = std::fs::OpenOptions::new().write(true).open(&file).is_ok();
        let result = ops.save();
        // Root can write anywhere, so the failure can't be produced.
        if !probe {
            assert!(result.is_err());
            assert_eq!(std::fs::read_to_string(&file).unwrap(), old);
        }
        // The list and its lock file, no copy.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A list the server may write but not read, with no copy beside it, is saved as on main.
    #[cfg(unix)]
    #[test]
    fn a_write_only_list_is_saved() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::durable::scratch_dir("lists-writeonly");
        let file = dir.join("ops.json");
        std::fs::write(&file, "[]").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o200)).unwrap();
        // Root reads anything, so a write-only list can't be produced.
        let write_only = std::fs::read(&file).is_err();
        let mut ops = OpList::new(&file);
        ops.load_str(r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#).unwrap();
        let result = ops.save();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        if write_only {
            result.unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), ops.to_json());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A list the server may write in a folder it may not create files in is still saved, as on main,
    /// only without the complete copy.
    #[cfg(unix)]
    #[test]
    fn a_list_in_a_folder_closed_to_new_files_is_still_saved() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::durable::scratch_dir("lists-closed");
        let file = dir.join("ops.json");
        std::fs::write(&file, "[]").unwrap();
        let notch = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#;
        let mut ops = OpList::new(&file);
        ops.load_str(notch).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root can create files anywhere, so the closed folder can't be produced.
        let closed = std::fs::write(dir.join("probe"), "").is_err();
        let result = ops.save();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        if closed {
            result.unwrap();
            let mut loaded = OpList::new(&file);
            loaded.load().unwrap();
            assert_eq!(loaded.to_json(), ops.to_json());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A save cut off while the list was rewritten in place: the next load takes the complete copy
    /// that save made first and finishes the save, so the list is whole and the copy gone.
    #[test]
    fn an_interrupted_save_is_loaded_from_its_complete_copy() {
        let dir = crate::durable::scratch_dir("lists-cut");
        let file = dir.join("ops.json");
        let saved = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#;
        let copy = crate::durable::cut_off_write_in_place(&file, saved.as_bytes());
        let mut whole = OpList::new(&file);
        whole.load_str(saved).unwrap();
        let mut ops = OpList::new(&file);
        ops.load().unwrap();
        assert_eq!(ops.to_json(), whole.to_json());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
        assert!(!copy.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A load that comes while a save holds the list's lock waits for it: it neither removes the
    /// save's copy nor reads the list before the save is done.
    #[test]
    fn a_load_waits_for_a_save_in_progress_and_leaves_its_copy() {
        let dir = crate::durable::scratch_dir("lists-locked");
        let file = dir.join("ops.json");
        let notch = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#;
        // A save that has written its copy and not yet touched the list, which is whole.
        let held = crate::durable::lock(&file).unwrap();
        let copy = crate::durable::cut_off_write_in_place(&file, notch.as_bytes());
        std::fs::write(&file, "[]").unwrap();
        let reader = std::thread::spawn({
            let file = file.clone();
            move || {
                let mut ops = OpList::new(&file);
                ops.load().unwrap();
                ops.entries().len()
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(copy.exists());
        assert!(!reader.is_finished());
        // The save finishes and lets go.
        std::fs::write(&file, notch).unwrap();
        std::fs::remove_file(&copy).unwrap();
        drop(held);
        assert_eq!(reader.join().unwrap(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A copy an older save left is never loaded over a list a later save cut off, and the next save
    /// removes it before it touches the list.
    #[test]
    fn an_older_saves_copy_is_never_loaded() {
        let dir = crate::durable::scratch_dir("lists-older");
        let file = dir.join("ops.json");
        let mut older = OpList::new(&file);
        older.load_str(r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#).unwrap();
        let stale = crate::durable::cut_off_write_in_place(&file, older.to_json().as_bytes());
        let mut newer = OpList::new(&file);
        newer.load_str(r#"[{"uuid":"853c80ef-3c37-49fd-aa49-938b674adae6","name":"jeb_","level":4,"bypassesPlayerLimit":false}]"#).unwrap();
        let bytes = newer.to_json();
        std::fs::write(&file, &bytes[..bytes.len() * 3 / 4]).unwrap();
        let mut ops = OpList::new(&file);
        assert!(ops.load().is_err());
        assert!(ops.entries().is_empty());
        assert!(stale.exists());
        newer.save().unwrap();
        assert!(!stale.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A save that failed while it rewrote the list left the cut-off list and its copy; a later save
    /// made its own copy durable and stopped before it removed the first. The load still takes the
    /// copy the list was cut from, not the newer one.
    #[test]
    fn a_cut_off_list_is_recovered_past_a_later_saves_copy() {
        let dir = crate::durable::scratch_dir("lists-two");
        let file = dir.join("ops.json");
        let mut first = OpList::new(&file);
        first.load_str(r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false},{"uuid":"853c80ef-3c37-49fd-aa49-938b674adae6","name":"jeb_","level":3,"bypassesPlayerLimit":false}]"#).unwrap();
        let mut later = OpList::new(&file);
        later.load_str(r#"[{"uuid":"853c80ef-3c37-49fd-aa49-938b674adae6","name":"jeb_","level":3,"bypassesPlayerLimit":false}]"#).unwrap();
        let first_bytes = first.to_json();
        crate::durable::cut_off_write_in_place(&file, first_bytes.as_bytes());
        let newer = crate::durable::cut_off_write_in_place(&file, later.to_json().as_bytes());
        std::fs::OpenOptions::new()
            .write(true)
            .open(&newer)
            .unwrap()
            .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .unwrap();
        // The later save never touched the list: it is still cut from the first save.
        std::fs::write(&file, &first_bytes[..first_bytes.len() / 2]).unwrap();
        let mut ops = OpList::new(&file);
        ops.load().unwrap();
        assert_eq!(ops.to_json(), first_bytes);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), first_bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// In a folder whose files cannot be listed (write and search, no read), the copies beside a
    /// cut-off list cannot be told: a save fails and leaves the list and its copy as they are, rather
    /// than staging a copy of its own and removing nothing, or the wrong thing.
    #[cfg(unix)]
    #[test]
    fn a_save_in_a_folder_that_cannot_be_listed_leaves_the_list_and_its_copy() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::durable::scratch_dir("lists-unlisted");
        let file = dir.join("ops.json");
        let saved = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#;
        let copy = crate::durable::cut_off_write_in_place(&file, saved.as_bytes());
        let mut ops = OpList::new(&file);
        ops.load_str("[]").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o300)).unwrap();
        // Root lists anything, so the folder can't be produced.
        let unlisted = std::fs::read_dir(&dir).is_err();
        let result = ops.save();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if unlisted {
            assert!(result.is_err());
            assert_eq!(
                std::fs::read(&file).unwrap(),
                &saved.as_bytes()[..saved.len() / 2]
            );
            assert_eq!(std::fs::read_to_string(&copy).unwrap(), saved);
            // The list, its lock file and the one copy: none staged.
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A copy that cannot be read beside a list cut off to nothing may be the one it was cut from:
    /// the load is an error rather than an empty list, and the save that follows a failed load (as a
    /// dedicated server's start does) fails too, so the copy stays.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_copy_is_an_error_not_a_missing_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::durable::scratch_dir("lists-unreadable-copy");
        let file = dir.join("ops.json");
        let saved = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#;
        let copy = crate::durable::cut_off_write_in_place(&file, saved.as_bytes());
        std::fs::write(&file, "").unwrap();
        std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything, so the copy can't be made unreadable.
        let unreadable = std::fs::read(&copy).is_err();
        let mut ops = OpList::new(&file);
        let loaded = ops.load();
        let saved_after = ops.save();
        std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o600)).unwrap();
        if unreadable {
            assert!(loaded.is_err());
            assert!(saved_after.is_err());
            assert_eq!(std::fs::read_to_string(&copy).unwrap(), saved);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A copy holding an entry the list would skip is not loaded.
    #[test]
    fn a_copy_with_an_invalid_entry_is_not_loaded() {
        let dir = crate::durable::scratch_dir("lists-invalid");
        let file = dir.join("ops.json");
        let saved = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false},{"uuid":"not a uuid","name":"jeb_","level":4,"bypassesPlayerLimit":false}]"#;
        let copy = crate::durable::cut_off_write_in_place(&file, saved.as_bytes());
        let mut ops = OpList::new(&file);
        assert!(ops.load().is_err());
        assert!(ops.entries().is_empty());
        assert!(copy.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The default list paths are bare file names: the first save creates the list, the next one
    /// rewrites it, and it loads back.
    #[test]
    fn a_list_at_a_bare_file_name_saves_and_loads() {
        let name = crate::durable::claim_bare_name("lists-bare");
        struct Cleanup<'a>(&'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0);
                let _ = std::fs::remove_file(format!(".{}.lock", self.0));
            }
        }
        let _cleanup = Cleanup(&name);
        let notch = r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":4,"bypassesPlayerLimit":false}]"#;
        let mut ops = OpList::new(&name);
        ops.load_str(notch).unwrap();
        ops.save().unwrap();
        ops.save().unwrap();
        let mut loaded = OpList::new(&name);
        loaded.load().unwrap();
        assert_eq!(loaded.to_json(), ops.to_json());
    }

    #[test]
    fn saves_like_gson() {
        let mut ops = OpList::new("/nonexistent/ops.json");
        ops.load_str(
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Notch","level":3}]"#,
        )
        .unwrap();
        assert_eq!(
            ops.to_json(),
            "[\n  {\n    \"uuid\": \"069a79f4-44e9-4726-a5be-fca90e38aaf5\",\n    \"name\": \"Notch\",\n    \"level\": 3,\n    \"bypassesPlayerLimit\": false\n  }\n]"
        );
        assert_eq!(
            parse_date("2026-10-12 10:00:00 +0200"),
            parse_date("2026-10-12 08:00:00 +0000")
        );
        assert_eq!(
            format_date(parse_date("2026-10-12 08:00:00 +0000").unwrap()),
            "2026-10-12 08:00:00 +0000"
        );
        assert_eq!(parse_date("forever"), None);
    }

    /// `SimpleDateFormat("yyyy-MM-dd HH:mm:ss Z", Locale.ROOT).parse`, as `BanListEntry` and
    /// `CachedUserNameToIdResolver` read their dates. Every value is what vanilla's own format gave for
    /// "2026-10-12 08:00:00 <zone>" (a Java run on OpenJDK 25), None where it threw `ParseException`.
    #[test]
    fn parse_date_matches_simple_date_format() {
        for (zone, millis) in [
            ("+0000", Some(1791792000000)),
            ("-0000", Some(1791792000000)),
            ("+0200", Some(1791784800000)),
            ("-1230", Some(1791837000000)),
            ("+2359", Some(1791705660000)),
            ("-2359", Some(1791878340000)),
            ("+0059", Some(1791788460000)),
            ("+2400", None),
            ("+0075", None),
            ("+020", None),
            ("+02:00", None),
            ("+02 00", None),
            ("+02000", Some(1791784800000)),
            ("+0200 extra", Some(1791784800000)),
            ("+0200abc", Some(1791784800000)),
            ("UTC", Some(1791792000000)),
            ("utc", Some(1791792000000)),
            ("UTCabc", Some(1791792000000)),
            ("UTC+02:00", Some(1791792000000)),
            ("GMT", Some(1791792000000)),
            ("gmt", Some(1791792000000)),
            ("GMTx", Some(1791792000000)),
            ("GMT +02:00", Some(1791792000000)),
            ("GMT+02:00", Some(1791784800000)),
            ("gmt+02:00", Some(1791784800000)),
            ("GMT+2:30", Some(1791783000000)),
            ("GMT-5:30", Some(1791811800000)),
            ("GMT+23:59", Some(1791705660000)),
            ("GMT-23:59", Some(1791878340000)),
            ("GMT-00:00", Some(1791792000000)),
            ("GMT+12:00x", Some(1791748800000)),
            ("GMT+02:005", Some(1791784800000)),
            ("GMT+24:00", None),
            ("GMT+2:60", None),
            ("GMT+2", None),
            ("GMT+02", None),
            ("GMT+0200", None),
            ("GMT+2:3", None),
            ("GMT+002:00", None),
            ("GMT+", None),
            ("GMT+x", None),
            ("Z", None),
            ("UT", None),
            ("CET", None),
        ] {
            assert_eq!(
                parse_date(&format!("2026-10-12 08:00:00 {zone}")),
                millis,
                "{zone:?}"
            );
        }
        // The zone comes after any run of spaces, as the date and the time do.
        assert_eq!(
            parse_date("2026-10-12 08:00:00  +0200"),
            Some(1791784800000)
        );
        assert_eq!(parse_date("2026-10-12 08:00:00"), None);
    }

    #[test]
    fn dates_too_far_out_are_none_not_wrapped() {
        // Lenient overflow still works within range: month 13 is January of the next year.
        assert_eq!(
            parse_date("2026-13-01 00:00:00 +0000"),
            parse_date("2027-01-01 00:00:00 +0000")
        );
        for text in [
            "9223372036854775807-01-01 00:00:00 +0000",
            "2026-9223372036854775807-01 00:00:00 +0000",
            "2026-01-9223372036854775807 00:00:00 +0000",
            "2026-01-01 9223372036854775807:00:00 +0000",
            "2026-01-01 00:00:9223372036854775807 +0000",
            "-9223372036854775808-01-01 00:00:00 +0000",
            "300000001-01-01 00:00:00 +0000",
            "2026--9223372036854775808-01 00:00:00 +0000",
        ] {
            assert_eq!(parse_date(text), None, "{text}");
        }
        // The far end of what milliseconds hold still parses (year 292 million).
        assert!(parse_date("292000000-01-01 00:00:00 +0000").is_some());
    }

    #[test]
    fn a_watcher_hears_what_vanillas_notification_service_does() {
        use std::sync::{Arc, Mutex};
        let dir = std::env::temp_dir().join(format!("horizon-lists-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mut ops = OpList::new(dir.join("ops.json"));
        let sink = heard.clone();
        ops.watch(move |change| sink.lock().unwrap().push(change));
        let op = |name: &str, level| OpEntry {
            user: NameAndId::offline(name),
            level,
            bypasses_player_limit: false,
        };
        assert!(ops.add(op("Kim", 4)));
        assert!(ops.add(op("Lee", 2)));
        // A removal that finds nothing says nothing; one that finds an entry says which.
        assert!(!ops.remove(&NameAndId::offline("Nobody")));
        assert!(ops.remove(&NameAndId::offline("Kim")));
        // A clear says it for each entry that is left, and the list is empty after.
        ops.clear();
        assert!(ops.is_empty());
        assert_eq!(
            *heard.lock().unwrap(),
            [
                Change::Added(op("Kim", 4)),
                Change::Added(op("Lee", 2)),
                Change::Removed(op("Kim", 4)),
                Change::Removed(op("Lee", 2)),
            ]
        );
        // A ban equal to the stored one is not added again, so nothing is said.
        let mut bans = BanList::new(dir.join("banned-players.json"));
        let said = Arc::new(Mutex::new(0));
        let counter = said.clone();
        bans.watch(move |_| *counter.lock().unwrap() += 1);
        let ban = BanEntry::new(NameAndId::offline("Kim"), None, None, None);
        assert!(bans.add(ban.clone()));
        assert!(!bans.add(ban));
        assert_eq!(*said.lock().unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `StoredUserList.load` reads through Guava's UTF-8 reader: a byte that is not
    /// UTF-8 becomes U+FFFD (vanilla's reader gave `0043 0061 0066 fffd` for `Caf\xE9`), so the entry is
    /// kept, and the next save writes it back with the others instead of losing the file.
    #[test]
    fn a_byte_that_is_not_utf8_is_replaced_not_a_missing_file() {
        let dir = std::env::temp_dir().join(format!("horizon-lists-latin1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("whitelist.json");
        let mut bytes = br#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Caf"#.to_vec();
        bytes.extend(b"\xE9\"}]");
        std::fs::write(&file, &bytes).unwrap();
        let mut white = WhiteList::new(&file);
        white.load().unwrap();
        let names: Vec<String> = white
            .entries()
            .iter()
            .map(|e| e.user.name.clone())
            .collect();
        assert_eq!(names, ["Caf\u{fffd}"]);
        assert!(white.add(WhiteEntry {
            user: NameAndId::offline("Kim")
        }));
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(
            saved.contains("Caf\u{fffd}") && saved.contains("Kim"),
            "{saved}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Java's reader gives one U+FFFD for an encoded UTF-16 surrogate (`ED A0 80`, and its truncated
    /// prefixes), where `String::from_utf8_lossy` gives two or three; both are read, not taken as missing.
    #[test]
    fn an_encoded_surrogate_is_one_replacement_as_java_reads_it() {
        let dir = std::env::temp_dir().join(format!("horizon-lists-cesu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("ops.json");
        for (bytes, expected) in [
            (&b"\xED\xA0\x80"[..], "\u{fffd}"),
            (&b"\xED\xA0"[..], "\u{fffd}"),
            (&b"\xED\xBFA"[..], "\u{fffd}A"),
            (&b"Caf\xE9"[..], "Caf\u{fffd}"),
        ] {
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(
                read_utf8(&std::fs::read(&file).unwrap()),
                expected,
                "{bytes:x?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `StoredUserList.load` only treats a missing path as a no-op: a path that exists but cannot
    /// be read (here a directory in place of the JSON file) throws before the map is touched.
    #[test]
    fn a_list_that_cannot_be_read_is_an_error_and_keeps_its_entries() {
        let dir =
            std::env::temp_dir().join(format!("horizon-lists-unreadable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("whitelist.json");
        std::fs::write(
            &file,
            r#"[{"uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Alex"}]"#,
        )
        .unwrap();
        let mut white = WhiteList::new(&file);
        white.load().unwrap();
        assert_eq!(white.entries().len(), 1);

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        let error = white.load().unwrap_err();
        assert!(error.contains("whitelist.json"), "{error}");
        assert_eq!(white.entries().len(), 1, "the entries survive a read error");

        let mut missing = WhiteList::new(dir.join("missing.json"));
        missing.load().unwrap();
        assert!(missing.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cleared_list_saves_empty() {
        let dir = std::env::temp_dir().join(format!("horizon-lists-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut white = WhiteList::new(dir.join("whitelist.json"));
        assert!(white.add(WhiteEntry {
            user: NameAndId::offline("Kim")
        }));
        white.clear();
        assert!(white.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.join("whitelist.json")).unwrap(),
            "[]"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
