//! eula.txt (net.minecraft.server.Eula): the server runs only once the file
//! says `eula=true`; a file that is missing or unreadable is replaced with
//! the default one and counts as not agreed.
use super::java_properties::{Properties, date_string, now_millis};
use std::path::Path;

/// CommonLinks.EULA.
pub const EULA_LINK: &str = "https://aka.ms/MinecraftEULA";

/// The default file, as Properties.store(OutputStream) writes it.
pub fn default_file(millis: i64) -> String {
    file(millis, false)
}

/// The file with `eula` set: what the owner's agreement writes (horizon-server init).
pub fn file(millis: i64, agreed: bool) -> String {
    let mut p = Properties::default();
    p.put("eula", agreed.to_string());
    p.store(
        Some(&format!(
            "By changing the setting below to TRUE you are indicating your agreement to our EULA ({EULA_LINK})."
        )),
        &date_string(millis),
        true,
    )
}

/// Eula.readFile; `Err` carries the warning vanilla logs.
pub fn agreed(path: &Path) -> Result<bool, String> {
    let read = std::fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|b| Properties::load_latin1(&b).map_err(|e| e.to_string()));
    match read {
        Ok(p) => Ok(p
            .get("eula")
            .unwrap_or("false")
            .eq_ignore_ascii_case("true")),
        Err(_) => {
            let _ = std::fs::write(path, default_file(now_millis()));
            Err(format!("Failed to load {}", path.display()))
        }
    }
}
