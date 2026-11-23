//! The player's server list, `servers.dat` in an instance's game folder: the game's Multiplayer
//! screen reads and writes the same file. Its format is vanilla's (`ServerList.load`/`save`,
//! `ServerData.read`/`write`): uncompressed NBT, `servers` a list of compounds with `name`, `ip`,
//! an optional `icon` (a PNG in base64), `acceptTextures`, `acceptedCodeOfConduct` and `hidden`.
//! The launcher only adds and removes entries: every other field of every entry is written back
//! as it was read.
use base64::Engine;
use horizon_platform::world::nbt::{self, Compound, JavaString, Limits, NamedTag, Tag};
use std::fs;
use std::path::Path;

const FILE: &str = "servers.dat";
/// Horizon's official survival server, named as the game lists it. Its address comes from
/// `launcher/links.json` on main (`backend::Links`), so a move to a new host needs no release.
pub const OFFICIAL_NAME: &str = "Horizon Official";
/// The official server's address until links.json is read, and when it can't be.
pub const OFFICIAL_ADDRESS: &str = horizon_telemetry::server_status::OFFICIAL_ADDRESS;
/// Addresses the official server had before: lists that still hold one are moved to the current one.
pub const OFFICIAL_PREVIOUS: &[&str] = horizon_telemetry::server_status::OFFICIAL_PREVIOUS;
/// Beside servers.dat once the official server was offered, so a player who removes it never
/// gets it back: the address offered, so a new one moves the list once. The game writes the same file.
const OFFICIAL_MARKER: &str = "horizon_official_server.txt";

#[derive(Clone, Debug, PartialEq)]
pub struct Server {
    pub name: String,
    pub ip: String,
    /// The server's picture, as the game last saw it, as PNG made small
    /// ([`crate::pictures::list_picture`]).
    pub icon: Option<Vec<u8>>,
}

/// The entries as they are in the file, the hidden ones (servers joined with Direct
/// Connection) too.
pub struct List {
    entries: Vec<Compound>,
}

fn key(k: &str) -> JavaString {
    JavaString::from(k)
}

fn text(entry: &Compound, k: &str) -> String {
    match entry.get(&key(k)) {
        Some(Tag::String(s)) => s.to_string_lossy(),
        _ => String::new(),
    }
}

/// `CompoundTag.getBooleanOr("hidden", false)`, as the game reads it: any number whose low byte
/// isn't zero.
fn hidden(entry: &Compound) -> bool {
    match entry.get(&key("hidden")) {
        Some(Tag::Byte(b)) => *b != 0,
        Some(Tag::Short(n)) => *n as i8 != 0,
        Some(Tag::Int(n)) => *n as i8 != 0,
        Some(Tag::Long(n)) => *n as i8 != 0,
        _ => false,
    }
}

fn entry(name: &str, ip: &str) -> Compound {
    [
        (key("name"), Tag::String(name.into())),
        (key("ip"), Tag::String(ip.into())),
        (key("hidden"), Tag::Byte(0)),
    ]
    .into_iter()
    .collect()
}

/// An address as the game connects to it: any case, 25565 when no port is given.
fn same_address(a: &str, b: &str) -> bool {
    let norm = |ip: &str| {
        let ip = ip.trim().to_ascii_lowercase();
        ip.strip_suffix(":25565").map(str::to_owned).unwrap_or(ip)
    };
    norm(a) == norm(b)
}

impl List {
    /// The list in `home`; empty when there is none or it can't be read, as the game does.
    pub fn load(home: &Path) -> List {
        Self::read(home).unwrap_or(List {
            entries: Vec::new(),
        })
    }

    /// The list in `home`, empty when there is none; none when the file is there but can't be read.
    fn read(home: &Path) -> Option<List> {
        let bytes = match fs::read(home.join(FILE)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return None,
        };
        let entries = match bytes.is_empty() {
            true => Vec::new(),
            false => match nbt::read_named(&bytes, Limits::default())
                .ok()?
                .0
                .tag
                .get("servers")
            {
                Some(Tag::List(list)) => list
                    .iter()
                    .filter_map(|t| match t {
                        Tag::Compound(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            },
        };
        Some(List { entries })
    }

    /// The servers the Multiplayer screen shows, in its order.
    pub fn servers(&self) -> Vec<Server> {
        self.entries
            .iter()
            .filter(|e| !hidden(e))
            .map(server)
            .collect()
    }

    /// Whether a shown server has `address`; without making the pictures [`List::servers`] makes.
    fn lists(&self, address: &str) -> bool {
        self.entries
            .iter()
            .filter(|e| !hidden(e))
            .any(|e| same_address(&text(e, "ip"), address))
    }

    /// Adds a server at the end of the shown ones, as the game's Add Server does.
    pub fn add(&mut self, name: &str, ip: &str) -> Result<(), String> {
        let (name, ip) = (name.trim(), ip.trim());
        if ip.is_empty() {
            return Err("A server needs an address.".into());
        }
        let name = if name.is_empty() {
            "Minecraft Server"
        } else {
            name
        };
        let at = self
            .entries
            .iter()
            .rposition(|e| !hidden(e))
            .map_or(0, |i| i + 1);
        self.entries.insert(at, entry(name, ip));
        Ok(())
    }

    /// Removes the `index`th shown server.
    pub fn remove(&mut self, index: usize) -> Result<Server, String> {
        // Only the removed server's picture is made, not every one's.
        let at = (0..self.entries.len())
            .filter(|&i| !hidden(&self.entries[i]))
            .nth(index)
            .ok_or_else(|| format!("There is no server {}.", index + 1))?;
        let server = server(&self.entries[at]);
        self.entries.remove(at);
        Ok(server)
    }

    /// Removes the shown server named `name` at `ip`, the one a row showed at `index` when the
    /// player confirmed. The list may have changed since that row was drawn (the game saved,
    /// another removal), so the server is found by what it is, its position only choosing
    /// between equal entries. Whether it was there: when it is gone nothing is removed, so a
    /// row read before a change never removes another server. Makes no picture.
    pub fn remove_confirmed(&mut self, index: usize, name: &str, ip: &str) -> bool {
        let at = (0..self.entries.len())
            .filter(|&i| !hidden(&self.entries[i]))
            .enumerate()
            .filter(|&(_, i)| {
                text(&self.entries[i], "name") == name && text(&self.entries[i], "ip") == ip
            })
            .min_by_key(|&(shown, _)| shown.abs_diff(index))
            .map(|(_, i)| i);
        at.map(|i| self.entries.remove(i)).is_some()
    }

    /// The list as a servers.dat holding only the shown entries, each with all its fields: what
    /// an export shares, without the addresses the player typed in Direct Connection.
    pub fn shown_file(&self) -> Result<Vec<u8>, String> {
        // The game also reads a float or double flag (`asByte`: the low byte of its floor).
        let floating = |e: &Compound| match e.get(&key("hidden")) {
            Some(Tag::Float(bits)) => f32::from_bits(*bits).floor() as i32 & 0xff != 0,
            Some(Tag::Double(bits)) => f64::from_bits(*bits).floor() as i32 & 0xff != 0,
            _ => false,
        };
        file(self.entries.iter().filter(|e| !hidden(e) && !floating(e)))
    }

    /// Writes the list as the game does: a new file first, the old one kept as `servers.dat_old`.
    pub fn save(&self, home: &Path) -> Result<(), String> {
        let bytes = file(self.entries.iter())?;
        let (new, current) = (home.join("servers.dat_new"), home.join(FILE));
        let written = fs::create_dir_all(home)
            .and_then(|()| fs::write(&new, bytes))
            .and_then(|()| match current.exists() {
                true => fs::copy(&current, home.join("servers.dat_old")).map(|_| ()),
                false => Ok(()),
            })
            .and_then(|()| fs::rename(&new, &current));
        written.map_err(|e| format!("Could not save the server list: {e}"))
    }

    /// Points the entries for a previous official address at `address`, each keeping the player's
    /// name, picture and every other field. The shown list keeps one entry for the server: an old
    /// one goes when the list already has `address`. Whether anything changed.
    fn move_official(&mut self, address: &str, previous: &[String]) -> bool {
        let old = |ip: &str| {
            !same_address(ip, address)
                && previous
                    .iter()
                    .any(|p| !p.trim().is_empty() && same_address(ip, p))
        };
        let mut shown = self.lists(address);
        let mut changed = false;
        self.entries.retain_mut(|e| {
            if !old(&text(e, "ip")) {
                return true;
            }
            changed = true;
            if !hidden(e) {
                if shown {
                    return false;
                }
                shown = true;
            }
            e.insert(key("ip"), Tag::String(address.into()));
            true
        });
        changed
    }
}

/// `entries` as servers.dat's bytes: uncompressed NBT, `servers` their list.
fn file<'a>(entries: impl Iterator<Item = &'a Compound>) -> Result<Vec<u8>, String> {
    let root = NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "servers",
            Tag::List(entries.cloned().map(Tag::Compound).collect()),
        )]),
    };
    nbt::write_named(&root, Limits::default()).map_err(|e| e.to_string())
}

/// Puts the official server at `address` first in the list in `home`, once: not again after the
/// player removes it, and not when the list already has its address. When the server moved (the
/// address differs from the one offered last), the entries for its `previous` addresses move to
/// the new one, once, and it is not offered again to a player who removed it. A list that can't be
/// read is left alone; an empty `address` changes nothing.
pub fn add_official(home: &Path, address: &str, previous: &[String]) -> Result<(), String> {
    let address = address.trim();
    if address.is_empty() {
        return Ok(());
    }
    let marker = home.join(OFFICIAL_MARKER);
    // A marker that is there but can't be read: offered already, at an address not known.
    let offered = match fs::read_to_string(&marker) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    };
    if offered.as_deref().is_some_and(|a| same_address(a, address)) {
        return Ok(());
    }
    let mut list = List::read(home).ok_or("The server list can't be read.")?;
    let mut changed = list.move_official(address, previous);
    if offered.is_none() && !list.lists(address) {
        list.entries.insert(0, entry(OFFICIAL_NAME, address));
        changed = true;
    }
    if changed {
        list.save(home)?;
    }
    fs::write(marker, format!("{address}\n")).map_err(|e| e.to_string())
}

/// A shown entry as a [`Server`], its picture made small.
fn server(e: &Compound) -> Server {
    Server {
        name: text(e, "name"),
        ip: text(e, "ip"),
        icon: Some(text(e, "icon"))
            .filter(|s| !s.is_empty())
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .and_then(crate::pictures::list_picture),
    }
}

/// The shown server a command names: its position from 1, its name (any case) or its address.
pub fn find(list: &List, query: &str) -> Result<(usize, Server), String> {
    let servers = list.servers();
    let q = query.trim();
    q.parse::<usize>()
        .ok()
        .filter(|n| (1..=servers.len()).contains(n))
        .map(|n| n - 1)
        .or_else(|| servers.iter().position(|s| s.name.eq_ignore_ascii_case(q)))
        .or_else(|| servers.iter().position(|s| s.ip.eq_ignore_ascii_case(q)))
        .map(|i| (i, servers[i].clone()))
        .ok_or_else(|| format!("No server {q} in the list"))
}

#[cfg(test)]
#[path = "server_list/tests.rs"]
mod tests;
