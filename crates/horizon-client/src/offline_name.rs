//! The offline player name, when neither `--name`, `HORIZON_PLAYER_NAME` nor a
//! signed-in account gives one. Vanilla's client defaults `--username` to
//! `"Player" + System.currentTimeMillis() % 1000` (net.minecraft.client.main.Main)
//! and the official launcher always passes the account's name, so two players
//! rarely share one. Horizon used plain `Player` for everyone, which is also
//! the LAN host's own profile: every guest got the host's offline UUID and was
//! refused with "You logged in from another location". The generated name is
//! kept in `horizon-player.txt` next to `options.txt` so the same player gets
//! the same offline UUID (and inventory on a friend's world) every time.
use std::{fs, path::Path};

pub const FILE: &str = "horizon-player.txt";
const KEY: &str = "name:";

/// StringUtil.isValidPlayerName, plus not empty (the login screen sends at least one character).
pub fn is_valid(name: &str) -> bool {
    crate::lan::conn::valid_name(name)
}

/// `Player` and five digits (vanilla's default has three: one shared name in a
/// thousand pairs is too many for a game that keeps it).
pub fn generate(seed: u64) -> String {
    format!("Player{:05}", seed % 100_000)
}

/// The name kept in `dir`, created on first use. A file that can't be written
/// still yields a name for this run.
pub fn load_or_create(dir: &Path) -> String {
    let path = dir.join(FILE);
    if let Some(name) = fs::read_to_string(&path).ok().and_then(|text| parse(&text)) {
        return name;
    }
    let name = generate(seed());
    if let Err(e) = fs::write(&path, format!("{KEY}{name}\n")) {
        log::warn!(
            "Horizon: could not keep the player name in {}: {e}",
            path.display()
        );
    }
    name
}

fn parse(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim_end_matches('\r').strip_prefix(KEY))
        .filter(|name| is_valid(name))
        .map(str::to_owned)
}

fn seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    // Two players starting the game in the same millisecond still differ.
    let mut x = nanos ^ u64::from(std::process::id()).rotate_left(32);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^ (x >> 33)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "horizon-offline-name-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn generated_names_are_valid_and_never_the_hosts_profile() {
        for seed in [0, 7, 99_999, 100_000, u64::MAX] {
            let name = generate(seed);
            assert!(is_valid(&name), "{name}");
            // The LAN host's own profile (game/chat.rs PLAYER_NAME).
            assert_ne!(name, "Player");
            assert_eq!(name.len(), 11);
        }
        assert_eq!(generate(42), "Player00042");
    }

    #[test]
    fn the_name_is_kept_and_reused() {
        let d = dir("kept");
        let first = load_or_create(&d);
        assert_eq!(
            fs::read_to_string(d.join(FILE)).unwrap(),
            format!("name:{first}\n")
        );
        assert_eq!(load_or_create(&d), first);
        // An edited name is used as it is.
        fs::write(d.join(FILE), "name:Alex_2\r\n").unwrap();
        assert_eq!(load_or_create(&d), "Alex_2");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn invalid_names_are_replaced() {
        for text in [
            "",
            "name:",
            "name:has space",
            "name:ThisNameIsTooLong17",
            "name:été",
        ] {
            let d = dir("invalid");
            fs::write(d.join(FILE), text).unwrap();
            let name = load_or_create(&d);
            assert!(
                name.starts_with("Player") && is_valid(&name),
                "{text:?} -> {name}"
            );
            assert_eq!(load_or_create(&d), name);
            let _ = fs::remove_dir_all(&d);
        }
    }
}
