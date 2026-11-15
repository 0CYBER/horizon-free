//! A dedicated server's check before it opens a world saved with mods that are not loaded now
//! (arismoko/horizon#3856): opening it would remove those mods' blocks, and what they hold, for good. The server
//! refuses, one log line per mod and a last one naming the switch, unless `--allowMissingMods` or
//! `server.toml`'s `[missing_mods] allow = true` says to open it anyway (off by default).

use super::{Game, Result, log};
use horizon_client::save::level_mods;

/// An intentional refusal to open a world without its saved mods, not a crash.
#[derive(Debug)]
struct MissingModsRefusal(pub String);

impl std::fmt::Display for MissingModsRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MissingModsRefusal {}

/// Whether this error is the already-explained, intentional world refusal.
pub(crate) fn is_missing_mods_refusal(error: &(dyn std::error::Error + 'static)) -> bool {
    error.is::<MissingModsRefusal>()
}

/// `server.toml`'s `[missing_mods]`.
#[derive(Debug, Default, PartialEq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    /// Open a world whose saved mods are missing, as `--allowMissingMods` does.
    #[serde(default)]
    allow: bool,
}

/// The section in `server.toml`'s text; the default (refuse) without it.
fn parse(text: &str) -> std::result::Result<Settings, String> {
    let mut table = toml::from_str::<toml::Table>(text).map_err(|e| e.to_string())?;
    match table.remove("missing_mods") {
        Some(section) => section
            .try_into::<Settings>()
            .map_err(|e| format!("[missing_mods]: {e}")),
        None => Ok(Settings::default()),
    }
}

/// Whether `server.toml` allows opening a world with missing mods. A section that does not read
/// stops the start.
fn allowed_by_server_toml() -> Result<bool> {
    match std::fs::read_to_string("server.toml") {
        Ok(text) => Ok(parse(&text).map_err(|e| format!("server.toml: {e}"))?.allow),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(why) => Err(format!("server.toml: {why}").into()),
    }
}

/// `f` with English as the language when none is set: a dedicated server's log is English, as
/// vanilla's is.
pub(super) fn in_english<R>(f: impl FnOnce() -> R) -> R {
    use horizon_client::lang;
    if lang::current().is_some() {
        return f();
    }
    lang::scoped(lang::Language::load(&[], "en_us"), f).0
}

impl Game<'_> {
    /// Refuses to open the world in `dir` while a mod it was saved with is missing and no
    /// `mod_mismatch` listener resolved it, unless `flag` (`--allowMissingMods`) or `server.toml`
    /// allows it; then the same list is a warning and the world opens.
    pub(super) fn check_missing_mods(&mut self, dir: &std::path::Path, flag: bool) -> Result<()> {
        let allowed = flag || allowed_by_server_toml()?;
        let missing = self.ask_missing_mods(dir);
        if missing.is_empty() {
            return Ok(());
        }
        if allowed {
            let (lines, allowed) = in_english(|| {
                let lines: Vec<String> = missing.iter().map(level_mods::describe).collect();
                let allowed = horizon_client::lang::tr(
                    "horizon.missing_mods.server.allowed",
                    &[&level_mods::ALLOW_FLAG],
                );
                (lines, allowed)
            });
            for line in &lines {
                log("WARN", line);
            }
            log("WARN", &allowed);
            return Ok(());
        }
        level_mods::forget_answer();
        let mut lines = in_english(|| level_mods::refusal_lines(&missing));
        let last = lines.pop().unwrap_or_default();
        let heading = in_english(|| horizon_client::lang::get("horizon.missing_mods.title"));
        log("ERROR", &heading);
        for line in &lines {
            log("ERROR", line);
        }
        for line in last.lines() {
            log("ERROR", line);
        }
        Err(MissingModsRefusal(last).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_mods_refuse_unless_allowed() {
        assert_eq!(parse("").unwrap(), Settings { allow: false });
        assert_eq!(
            parse("[combat]\nlegacy = true\n").unwrap(),
            Settings { allow: false }
        );
        assert_eq!(
            parse("[missing_mods]\n").unwrap(),
            Settings { allow: false }
        );
        assert_eq!(
            parse("[missing_mods]\nallow = true\n").unwrap(),
            Settings { allow: true }
        );
        assert!(parse("[missing_mods]\nallow = \"yes\"\n").is_err());
        assert!(parse("[missing_mods]\nopen = true\n").is_err());
    }

    /// Without the switch the server refuses, naming the switch, and changes nothing in the
    /// world; with it the world opens.
    #[test]
    #[ignore = "daily"]
    fn check_missing_mods_refuses_unless_allowed_and_writes_nothing() {
        use horizon_core::nbt::Tag;
        let dir = std::env::temp_dir().join(format!(
            "horizon-dedicated-missing-mod-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let gone = Tag::compound([
            ("ModId", Tag::String("gone".into())),
            ("ModVersion", Tag::String("1.0.0".into())),
        ]);
        horizon_client::save::write_dat(
            &dir.join("level.dat"),
            Tag::compound([(
                "fml",
                Tag::compound([("LoadingModList", Tag::List(vec![gone]))]),
            )]),
        )
        .unwrap();
        let before = std::fs::read(dir.join("level.dat")).unwrap();
        crate::game::native_entity_refs::tests::with_game_role(
            std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap()),
            crate::game::execution_role::ExecutionRole::DedicatedServer,
            |game| {
                let refused = game
                    .check_missing_mods(&dir, false)
                    .unwrap_err()
                    .to_string();
                assert!(refused.contains(level_mods::ALLOW_FLAG), "{refused}");
                assert!(game.check_missing_mods(&dir, true).is_ok());
            },
        );
        assert_eq!(std::fs::read(dir.join("level.dat")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
