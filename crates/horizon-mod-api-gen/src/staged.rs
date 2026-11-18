//! The native mods horizon-mods compiled into a workspace, read from the record it leaves there.
use std::{collections::BTreeSet, path::Path};

/// What horizon-mods writes at the root of a workspace it changed: each injection and module it
/// applied, with its mod's `[mod] id`. The launcher's native builds have one, and so has a bundle
/// applied by hand or by a mod's own script, which has no `native-mods.json` beside its game.
pub const MODS_STAGE: &str = ".horizon-mods-stage.json";

/// The ids of the mods whose injections or modules horizon-mods applied to the workspace at
/// `root`, sorted: none for a workspace it never changed, a record that names no mod, or one that
/// does not parse.
pub fn staged_mods(root: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join(MODS_STAGE)) else {
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
/// Names are those of the declarations in `horizon-mod-vocab/src/hooks/`, without the `native_`
/// prefix: an injection names the game function it runs in, and `native_<name>` is that function for
/// a hook point, whatever `version` its declaration carries.
const MOB_TURN: &[&str] = &[
    "living_change_target",
    "living_heal",
    "living_death_message",
    "living_breathe",
    "living_drown",
    "living_fall",
    "living_incoming_damage",
    "living_damage_pre",
    "living_knockback",
    // Called from a mob's move: farmland it falls on, a content mod's blocks it touches, the block
    // it lands on, the one it bounces off, and the ones its sweep crosses.
    "trample",
    "block_step_on",
    "fall_on",
    "bounce_on",
    "entity_inside",
    // Called from that move's own physics: the surface under it, the speeds it travels with, the
    // fluid that pushes it, the ladder it climbs, and the sound its step makes.
    "friction",
    "block_speed_factor",
    "block_jump_factor",
    "is_ladder",
    "makes_open_trapdoor_above_climbable",
    "living_speed",
    "living_jump_power",
    "living_flying_speed",
    "fluid_flow",
    "sound_type",
    // Called from an enderman's or a shulker's own teleport, in its AI step.
    "ender_entity_teleport",
    // Asked while a mob ticks: how it types the path it finds, what may see it, and what it may
    // break.
    "block_path_type",
    "adjacent_block_path_type",
    "living_visibility",
    "enderman_anger",
    "can_entity_destroy",
    "entity_mob_griefing",
];
/// The hooks a mob's turn calls without waiting: a trait mod hears them after the loop, but a
/// mixin into them still runs in the turn.
const MOB_TURN_QUEUED: &[&str] = &["living_jump", "living_damage_post"];

/// The path prefixes of the modules a mob's turn never runs, so that a mixin into one of them
/// leaves the parallel tick on. A target is outside the turn when it names a module inside one of
/// these prefixes and something more within it: the module and its descendants, never a sibling
/// whose name starts the same way, and never a record too broken to name a module, which stays in
/// the turn.
const OUTSIDE_A_MOB_TURN: &[&str] = &[
    // The client's drawing: the frame's own passes, HUD, widgets and shared screen drawing, and the
    // settings and options screens. Drawn while the window renders, after its tick.
    "horizon_client_render::gfx",
    "horizon_client_render::hud",
    "horizon_client_render::options_ui",
    "horizon_client_render::screen_draw",
    "horizon_client_render::ui",
    // The wire: codecs, framing, encryption and the packets. A mob's news leaves the turn as an
    // event, and the level sends it after the loop.
    "horizon_protocol",
    // The rest of `horizon_client_render` is left out on purpose: the turn reads its `world`, moves
    // through its `collisions`, draws the level's random for its `fast_hash` maps and `spans`, and
    // every part hook's forwarder reads its `api_context`.
];

/// Whether the vanilla game function `target` — a mixin target, `crate::module::Type::method` — is
/// one a mob's turn can run.
fn in_a_mob_turn(target: &str) -> bool {
    let function = target.rsplit("::").next().unwrap_or_default();
    match function.strip_prefix("native_") {
        // A hook point: only the hooks the turn calls itself, waiting on them or not.
        Some(hook) => MOB_TURN.contains(&hook) || MOB_TURN_QUEUED.contains(&hook),
        // Any other function of the game: it runs in the turn unless its module never does.
        None => !OUTSIDE_A_MOB_TURN.iter().any(|prefix| {
            target
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix("::"))
                .is_some_and(|module| !module.is_empty())
        }),
    }
}

/// Whether the native mods built into the workspace at `root` can run code in a mob's turn:
/// `"hooked"` when a mixin injects into a function that turn runs ([`in_a_mob_turn`]: a hook point
/// it calls, or any other function of a module it can reach), or a trait package (`native-api/`,
/// with its crates in `native-crates/`, as the launcher stages them) has a function named for a
/// hook the turn waits on; `"quiet"` when trait packages were staged and nothing hooks; None when
/// no trait package was staged and no mixin hooks.
pub fn mob_turn(root: &Path) -> Option<&'static str> {
    // A record without a usable target conservatively serializes mob turns: only a known hook or a
    // known module outside the turn leaves parallelism enabled.
    if let Ok(text) = std::fs::read_to_string(root.join(MODS_STAGE)) {
        let record = serde_json::from_str::<serde_json::Value>(&text).unwrap_or_default();
        let injections = record["injections"].as_array();
        let hooked = injections.is_none_or(|list| {
            list.iter()
                .any(|injection| in_a_mob_turn(injection["target"].as_str().unwrap_or_default()))
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
        let dir =
            std::env::temp_dir().join(format!("horizon-mods-stage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(record) = record {
            std::fs::write(dir.join(MODS_STAGE), record).unwrap();
        }
        dir
    }

    #[test]
    fn the_mods_of_every_injection_and_module_once() {
        // The shape horizon-mods 0.6.0 writes (schema 6), trimmed to the fields read here.
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
            (
                "move",
                "horizon_client_lib::motion::native_friction",
                Some("hooked"),
            ),
            // A block part is asked inside the mob's own move, on landing and on the sweep.
            (
                "landing",
                "horizon_client::game::native_hooks::block_landing::HitLevel<'_>::native_fall_on",
                Some("hooked"),
            ),
            // The questions its AI asks while it ticks: the path's block types among them.
            (
                "path",
                "horizon_client::game::native_hooks::Game<'_>::native_block_path_type",
                Some("hooked"),
            ),
            ("any", "horizon_client::mobs::ai::tick", Some("hooked")),
            // A screen is drawn in the window's frame, after its tick: hooking one leaves the
            // mobs' turns parallel.
            (
                "screen",
                "horizon_client_render::options_ui::OptionsUi::draw",
                None,
            ),
            // A packet's codec runs when the level sends it, not in a mob's turn.
            ("wire", "horizon_protocol::frame::decompress", None),
            // A sibling whose name starts with an outside prefix's (`hud_kit`, beside `hud`) is
            // its own module, so hooking it keeps the tick serial.
            (
                "hud neighbour",
                "horizon_client_render::hud_kit::Something::draw",
                Some("hooked"),
            ),
            // A record too broken to name a module under an outside prefix is no evidence that
            // anything is outside the turn: it stays in, as every unknown target does.
            ("broken wire", "horizon_protocol::", Some("hooked")),
            // The rest of the render crate is a module a mob's turn does reach, so a mixin into
            // it keeps the tick serial like any other unknown one.
            (
                "render world",
                "horizon_client_render::world::World::block",
                Some("hooked"),
            ),
            // A module of a crate the list does not name at all.
            (
                "unknown crate",
                "horizon_world::level::tick::BlockTicks::tick",
                Some("hooked"),
            ),
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
        let parts = "impl Hooks for Example {\n    fn entity_inside(&self) {}\n}\n";
        std::fs::write(lib.join("lib.rs"), parts).unwrap();
        assert_eq!(
            mob_turn(&dir),
            Some("hooked"),
            "a block part a mob's move asks"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
