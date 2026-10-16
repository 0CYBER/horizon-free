//! `horizon-mod.toml`, a mod's manifest, and the `.hmod` file around it.
use serde::Deserialize;
use std::{fmt, io::Read, path::Path};

/// The manifest's file name, beside `mod.wasm`.
pub const MANIFEST: &str = "horizon-mod.toml";
/// The component's file name in a `.hmod` or a mod folder.
pub const WASM: &str = "mod.wasm";
/// The API (WIT package `horizon:mods`) versions this build runs: a mod
/// built for any 0.1.x SDK.
pub const API: &str = "0.1";
/// The manifest `api` lines this build loads. 0.2 is 0.1 plus content:
/// `assets/` and `data/` in the `.hmod`, with blocks and items declared in
/// `data/<id>/horizon/` (docs/modpacks.md). Code still links API 0.1.
pub const APIS: [&str; 2] = [API, "0.2"];
/// The biggest `mod.wasm` a mod may ship.
const MAX_WASM: u64 = 32 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The API line the mod was built for: `0.1`, or `0.2` for content.
    pub api: String,
    #[serde(default)]
    pub side: Side,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Runs in the player's game only (HUD, keys, chat commands).
    #[default]
    Client,
    /// Runs where the world is simulated (rules, blocks); not run yet.
    Server,
    /// Both, and the server tells clients which version it expects.
    Both,
}

/// What a mod may do beyond writing its log. Each is a WIT interface the
/// component imports; the player sees them before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    GameRead,
    Chat,
    Hud,
    /// New blocks and items from the mod's `data/` (API 0.2): no WIT
    /// interface, the game reads them before the world loads.
    Registries,
}
impl Capability {
    pub const ALL: [Capability; 4] = [
        Capability::GameRead,
        Capability::Chat,
        Capability::Hud,
        Capability::Registries,
    ];
    /// The WIT interface it grants, in `horizon:mods`.
    pub fn interface(self) -> &'static str {
        match self {
            Capability::GameRead => "game",
            Capability::Chat => "chat",
            Capability::Hud => "hud",
            Capability::Registries => "registries",
        }
    }
    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::GameRead => "see your position, health and the time",
            Capability::Chat => "show you chat messages",
            Capability::Hud => "draw text on your screen",
            Capability::Registries => "add blocks and items to your worlds",
        }
    }
    /// The capability a component import needs, `Ok(None)` for one every
    /// mod has; `Err` for an import Horizon does not offer.
    pub fn of_import(name: &str) -> Result<Option<Capability>, String> {
        let unknown = || format!("imports {name}, which Horizon does not offer");
        let rest = name.strip_prefix("horizon:mods/").ok_or_else(unknown)?;
        let (interface, version) = rest.split_once('@').ok_or_else(unknown)?;
        if !version.starts_with("0.1.") {
            return Err(format!("needs API {version}; this Horizon runs {API}"));
        }
        if interface == "log" {
            return Ok(None);
        }
        Capability::ALL
            .into_iter()
            .filter(|c| *c != Capability::Registries)
            .find(|c| c.interface() == interface)
            .map(Some)
            .ok_or_else(unknown)
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Capability::GameRead => "game-read",
            Capability::Chat => "chat",
            Capability::Hud => "hud",
            Capability::Registries => "registries",
        };
        f.write_str(name)
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        let id_ok = (1..=64).contains(&m.id.len())
            && m.id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !id_ok {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        if !APIS.contains(&m.api.as_str()) {
            return Err(format!(
                "built for API {}; this Horizon runs {}",
                m.api,
                APIS.join(" and ")
            ));
        }
        if m.capabilities.contains(&Capability::Registries) && m.api == API {
            return Err(format!(
                "{MANIFEST}: capability registries needs api = \"0.2\""
            ));
        }
        Ok(m)
    }
    /// Checks that the component's imports need only declared capabilities.
    pub fn check_imports<'a>(
        &self,
        imports: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        for name in imports {
            if let Some(c) = Capability::of_import(name)?
                && !self.capabilities.contains(&c)
            {
                return Err(format!("uses capability {c}, missing from {MANIFEST}"));
            }
        }
        Ok(())
    }
}

/// A mod as found on disk: its manifest and component bytes.
pub struct Package {
    pub manifest: Manifest,
    /// None for a content-only mod (capability `registries`, no code).
    pub wasm: Option<Vec<u8>>,
}
impl Package {
    /// A `.hmod` (a zip holding `horizon-mod.toml`, `mod.wasm` and, for
    /// content, `assets/` and `data/`) or a folder holding the same.
    pub fn read(path: &Path) -> Result<Self, String> {
        let (toml, wasm) = if path.is_dir() {
            let toml = std::fs::read_to_string(path.join(MANIFEST));
            let wasm = path.join(WASM);
            let wasm = match wasm.exists() {
                true => Some(std::fs::read(wasm).map_err(|e| format!("{WASM}: {e}"))?),
                false => None,
            };
            (toml.map_err(|e| format!("{MANIFEST}: {e}"))?, wasm)
        } else {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            let mut entry = |name: &str| -> Result<Option<Vec<u8>>, String> {
                let f = match zip.by_name(name) {
                    Ok(f) => f,
                    Err(zip::result::ZipError::FileNotFound) => return Ok(None),
                    Err(e) => return Err(format!("{name}: {e}")),
                };
                let mut out = Vec::new();
                f.take(MAX_WASM + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| e.to_string())?;
                Ok(Some(out))
            };
            let toml = entry(MANIFEST)?.ok_or_else(|| format!("{MANIFEST}: missing"))?;
            let toml = String::from_utf8(toml).map_err(|_| format!("{MANIFEST}: not UTF-8"))?;
            (toml, entry(WASM)?)
        };
        if wasm.as_ref().is_some_and(|w| w.len() as u64 > MAX_WASM) {
            return Err(format!("{WASM} is over {} MiB", MAX_WASM >> 20));
        }
        let manifest = Manifest::parse(&toml)?;
        if wasm.is_none() && !manifest.capabilities.contains(&Capability::Registries) {
            return Err(format!(
                "{WASM}: missing (only a content mod, with capability registries, may have no code)"
            ));
        }
        Ok(Package { manifest, wasm })
    }
    /// The `.hmod` bytes for `manifest_text` and a component.
    pub fn write(manifest_text: &str, wasm: &[u8]) -> Result<Vec<u8>, String> {
        Self::write_files(manifest_text, Some(wasm), &[])
    }
    /// The `.hmod` bytes for `manifest_text`, a component if the mod has
    /// code, and content `files` (`assets/...`, `data/...`) by path.
    pub fn write_files(
        manifest_text: &str,
        wasm: Option<&[u8]>,
        files: &[(&str, &[u8])],
    ) -> Result<Vec<u8>, String> {
        use std::io::Write;
        Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        if let Some(wasm) = wasm {
            put(WASM, wasm)?;
        }
        for (name, bytes) in files {
            crate::content::check_path(name)?;
            put(name, bytes)?;
        }
        Ok(zip.finish().map_err(err)?.into_inner())
    }
}

/// Turns a `wasm32-unknown-unknown` cdylib built with the SDK into the
/// component the game loads.
#[cfg(feature = "pack")]
pub fn componentize(core: &[u8]) -> Result<Vec<u8>, String> {
    wit_component::ComponentEncoder::default()
        .module(core)
        .and_then(|e| e.validate(true).encode())
        .map_err(|e| format!("{e:#}"))
}

/// The names a component imports: `horizon:mods/game@0.1.0`, ...
#[cfg(feature = "pack")]
pub fn component_imports(component: &[u8]) -> Result<Vec<String>, String> {
    use wit_parser::{WorldItem, WorldKey};
    let wit_component::DecodedWasm::Component(resolve, world) =
        wit_component::decode(component).map_err(|e| format!("{e:#}"))?
    else {
        return Err("a WIT package, not a component".into());
    };
    let names = resolve.worlds[world]
        .imports
        .iter()
        .map(|(key, item)| match (key, item) {
            (WorldKey::Interface(id), _) | (_, WorldItem::Interface { id, .. }) => resolve
                .id_of(*id)
                .unwrap_or_else(|| resolve.name_world_key(key)),
            _ => resolve.name_world_key(key),
        });
    Ok(names.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
id = "coords_hud"
name = "Coordinates HUD"
version = "0.1.0"
api = "0.1"
capabilities = ["game-read", "hud"]
"#;

    #[test]
    fn parses_and_checks_imports() {
        let m = Manifest::parse(EXAMPLE).unwrap();
        assert_eq!(m.side, Side::Client);
        assert_eq!(m.capabilities, [Capability::GameRead, Capability::Hud]);
        let ok = [
            "horizon:mods/log@0.1.0",
            "horizon:mods/game@0.1.0",
            "horizon:mods/hud@0.1.2",
        ];
        assert_eq!(m.check_imports(ok), Ok(()));
        let chat = m.check_imports(["horizon:mods/chat@0.1.0"]);
        assert_eq!(
            chat,
            Err("uses capability chat, missing from horizon-mod.toml".into())
        );
        assert!(m.check_imports(["wasi:filesystem/types@0.2.0"]).is_err());
        assert!(m.check_imports(["horizon:mods/game@0.2.0"]).is_err());
    }

    #[test]
    fn refuses_bad_ids_and_apis() {
        let bad_id = EXAMPLE.replace("coords_hud", "Coords HUD");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        let bad_api = EXAMPLE.replace("api = \"0.1\"", "api = \"0.3\"");
        assert!(Manifest::parse(&bad_api).unwrap_err().contains("API 0.3"));
        let unknown = format!("{EXAMPLE}network = true\n");
        assert!(Manifest::parse(&unknown).is_err());
    }

    #[test]
    fn hmod_round_trips() {
        let dir = std::env::temp_dir().join(format!("horizon-mods-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("coords_hud.hmod");
        std::fs::write(&path, Package::write(EXAMPLE, b"\0asm").unwrap()).unwrap();
        let p = Package::read(&path).unwrap();
        assert_eq!(p.manifest.id, "coords_hud");
        assert_eq!(p.wasm.as_deref(), Some(&b"\0asm"[..]));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
