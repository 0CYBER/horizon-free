//! The native mods horizon-mixins compiled into a workspace, read from the record it leaves there.
use std::{collections::BTreeSet, path::Path};

/// What horizon-mixins writes at the root of a workspace it changed: each injection and module it
/// applied, with its mod's `[mod] id`. The launcher's native builds have one, and so has a bundle
/// applied by hand or by a mod's own script, which has no `native-mods.json` beside its game.
pub const MIXINS_STAGE: &str = ".horizon-mixins-stage.json";

/// The ids of the mods whose injections or modules horizon-mixins applied to the workspace at
/// `root`, sorted: none for a workspace it never changed, a record that names no mod, or one that
/// does not parse.
pub fn staged_mods(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(MIXINS_STAGE)) else {
        return Vec::new();
    };
    let Ok(record) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let ids: BTreeSet<&str> = ["injections", "modules"]
        .iter()
        .filter_map(|list| record[*list].as_array())
        .flatten()
        .filter_map(|entry| entry["mod_id"].as_str())
        .filter(|id| !id.is_empty())
        .collect();
    ids.into_iter().map(str::to_owned).collect()
}

/// The hooks a mob's turn calls and waits on (the parallel mob tick, `game/creatures/mob_islands.rs`).
const MOB_TURN: &[&str] = &[
    "living_change_target",
    "living_heal",
    "living_death_message",
    "living_breathe",
    "living_drown",
    "living_fall",
    "register_goals",
    "living_incoming_damage",
    "living_incoming_damage_v2",
    "living_incoming_damage_v3",
    "living_damage_pre",
    "living_knockback",
    "living_knockback_v2",
    // Called from a mob's move: farmland it falls on, a content mod's blocks it touches.
    "trample",
    "block_step_on",
    "block_entity_inside",
    "block_fall_on",
];
/// The hooks a mob's turn calls without waiting: a trait mod hears them after the loop, but a
/// mixin into them still runs in the turn.
const MOB_TURN_QUEUED: &[&str] = &["living_jump", "living_damage_post"];

/// Whether the native mods built into the workspace at `root` can run code in a mob's turn:
/// `"hooked"` when a mixin injects into anything but a `native_*` hook point outside that turn, or
/// a trait package (`native-api/`, with its crates in `native-crates/`, as the launcher stages
/// them) has a function named for a hook the turn waits on; `"quiet"` when trait packages were
/// staged and nothing hooks; None when no trait package was staged and no mixin hooks.
pub fn mob_turn(root: &Path) -> Option<&'static str> {
    if let Ok(text) = std::fs::read_to_string(root.join(MIXINS_STAGE)) {
        let record = serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default();
        let injections = record["injections"].as_array();
        let hooked = injections.is_none_or(|list| {
            list.iter().any(|injection| {
                let target = injection["target"].as_str().unwrap_or_default();
                let function = target.rsplit("::").next().unwrap_or_default();
                function
                    .strip_prefix("native_")
                    .is_none_or(|hook| MOB_TURN.contains(&hook) || MOB_TURN_QUEUED.contains(&hook))
            })
        });
        if hooked {
            return Some("hooked");
        }
    }
    let traits = root.join("native-api");
    if !traits.is_dir() {
        return None;
    }
    let mut source = String::new();
    for dir in [traits, root.join("native-crates")] {
        read_rust(&dir, &mut source);
    }
    let words: Vec<&str> = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    let hooked = words
        .windows(2)
        .any(|w| w[0] == "fn" && MOB_TURN.contains(&w[1]));
    Some(if hooked { "hooked" } else { "quiet" })
}

/// Appends every `.rs` file under `dir` to `out`.
fn read_rust(dir: &Path, out: &mut String) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            read_rust(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            out.push('\n');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str, record: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "horizon-mixins-stage-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(record) = record {
            std::fs::write(dir.join(MIXINS_STAGE), record).unwrap();
        }
        dir
    }

    #[test]
    fn the_mods_of_every_injection_and_module_once() {
        // The shape horizon-mixins 0.6.0 writes (schema 6), trimmed to the fields read here.
        let dir = workspace(
            "mods",
            Some(
                r#"{"schema": 6, "transformer": "0.6.0", "inputs": {"Cargo.toml": "ab"},
                "injections": [
                    {"mod_id": "ironfall", "injection_id": "hud", "kind": "HEAD"},
                    {"mod_id": "veinminer", "injection_id": "break", "kind": "HEAD"},
                    {"mod_id": "ironfall", "injection_id": "tick", "kind": "HEAD"}
                ],
                "modules": [
                    {"mod_id": "ironfall", "module_id": "glue"},
                    {"mod_id": "atlas", "module_id": "maps"}
                ]}"#,
            ),
        );
        assert_eq!(staged_mods(&dir), ["atlas", "ironfall", "veinminer"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_mods_without_a_record_or_with_an_empty_or_broken_one() {
        for (name, record) in [
            ("none", None),
            (
                "empty",
                Some(r#"{"schema": 6, "injections": [], "modules": []}"#),
            ),
            ("broken", Some("{\"injections\": [")),
        ] {
            let dir = workspace(name, record);
            assert!(staged_mods(&dir).is_empty(), "{name}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn mods_that_can_run_in_a_mob_turn() {
        let record = |target: &str| {
            format!(r#"{{"injections": [{{"mod_id": "m", "target": "{target}"}}]}}"#)
        };
        for (name, target, found) in [
            (
                "hook",
                "horizon_client::game::native_hooks::Game<'_>::native_break_speed",
                None,
            ),
            (
                "turn",
                "horizon_client_lib::hunger::native_living_heal",
                Some("hooked"),
            ),
            (
                "queued",
                "horizon_client_lib::motion::native_living_jump",
                Some("hooked"),
            ),
            ("any", "horizon_client::mobs::ai::tick", Some("hooked")),
        ] {
            let dir = workspace(name, Some(&record(target)));
            assert_eq!(mob_turn(&dir), found, "{name}");
            std::fs::remove_dir_all(&dir).unwrap();
        }
        let dir = workspace("traits", None);
        assert_eq!(mob_turn(&dir), None, "nothing staged");
        let lib = dir.join("native-api/example/src");
        std::fs::create_dir_all(&lib).unwrap();
        let hooks = "impl Hooks for Example {\n    fn living_jump(&self) {}\n}\n";
        std::fs::write(lib.join("lib.rs"), hooks).unwrap();
        assert_eq!(mob_turn(&dir), Some("quiet"), "a queued hook");
        let helper = dir.join("native-crates/example/shared/src");
        std::fs::create_dir_all(&helper).unwrap();
        std::fs::write(helper.join("lib.rs"), "pub fn  living_fall() {}").unwrap();
        assert_eq!(mob_turn(&dir), Some("hooked"), "a helper crate's hook");
        std::fs::remove_dir_all(dir.join("native-crates")).unwrap();
        let moves = "impl Hooks for Example {\n    fn trample(&self) {}\n}\n";
        std::fs::write(lib.join("lib.rs"), moves).unwrap();
        assert_eq!(mob_turn(&dir), Some("hooked"), "a hook a mob's move calls");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
