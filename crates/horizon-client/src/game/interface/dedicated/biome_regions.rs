//! `server.toml`'s `[biome_regions]`: what a dedicated server chooses for an existing world's
//! saved biome regions (docs/design/mod-api/mod-biome-regions.md), where the client asks
//! in the world list. `missing = "refuse" | "vanilla"` (default `"refuse"`) answers for the
//! regions of mods no longer installed; `adopt_installed = true` takes the installed mods'
//! regions for new chunks.

use horizon_server_files::biome_regions::{BiomeRegions as Settings, MissingRegions as Missing};

/// The section in `server.toml`'s text; the defaults without it.
fn parse(text: &str) -> Result<Settings, String> {
    let mut table = toml::from_str::<toml::Table>(text).map_err(|e| e.to_string())?;
    match table.remove("biome_regions") {
        Some(section) => section
            .try_into::<Settings>()
            .map_err(|e| format!("[biome_regions]: {e}")),
        None => Ok(Settings::default()),
    }
}

/// Records `server.toml`'s choices in `world`'s biome regions before it opens. A section that
/// does not read stops the start.
pub(super) fn choose(world: &std::path::Path) -> horizon_client::Result<()> {
    let settings = match std::fs::read_to_string("server.toml") {
        Ok(text) => parse(&text).map_err(|e| format!("server.toml: {e}"))?,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        Err(why) => return Err(format!("server.toml: {why}").into()),
    };
    horizon_client::save::biome_regions::choose(
        world,
        settings.missing == Missing::Vanilla,
        settings.adopt_installed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_mods_refuse_and_nothing_is_adopted_unless_set() {
        let default = Settings {
            missing: Missing::Refuse,
            adopt_installed: false,
        };
        assert_eq!(parse("").unwrap(), default);
        assert_eq!(parse("[combat]\nlegacy = true\n").unwrap(), default);
        assert_eq!(parse("[biome_regions]\n").unwrap(), default);
        assert_eq!(
            parse("[biome_regions]\nmissing = \"vanilla\"\nadopt_installed = true\n").unwrap(),
            Settings {
                missing: Missing::Vanilla,
                adopt_installed: true,
            }
        );
        assert!(parse("[biome_regions]\nmissing = \"skip\"\n").is_err());
        assert!(parse("[biome_regions]\nadopt = true\n").is_err());
    }
}
