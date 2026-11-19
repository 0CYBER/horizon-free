//! A mod's calls, checked by name against the game's when it loads (#18253). Linking already
//! matched each import's name and signature hash; here each call the mod imports or answers is
//! compared with the game's in full, the run-time `Wire` layouts of the types it reaches
//! included. A call the mod doesn't know (one added after it was built) is no refusal: the
//! game never subscribes the mod to it, and a direct call answers that the mod has none.
use crate::Refusal;
use std::collections::{BTreeSet, HashMap};

/// A table of calls, as a mod answers `CALLS`: each call's key, its hash, and whether it is one
/// of the mod's exports.
pub(crate) type Table = mod_api::wire::CallsAnswer;

static GAME: std::sync::LazyLock<HashMap<&'static str, (u64, bool)>> =
    std::sync::LazyLock::new(|| {
        mod_api::call_layouts()
            .into_iter()
            .map(|(key, hash, export)| (key, (hash, export)))
            .collect()
    });

/// A hand-written guest's `horizon_hook` body that answers the game's `CALLS` question as a mod
/// built against this game whose imports are `keys` would, and runs `body` (leaving an `i64`)
/// for every other export. The table is written at 128 KiB, below the group's stack.
#[cfg(any(test, feature = "listener-fixture", feature = "runtime-fixture"))]
pub fn answering<K: AsRef<str>>(keys: &[K], body: &str) -> String {
    const AT: u64 = 128 << 10;
    let table: Table = keys
        .iter()
        .map(|key| {
            let (hash, export) = GAME[key.as_ref()];
            (key.as_ref().to_owned(), hash, export)
        })
        .collect();
    let mut bytes = mod_api::wire::Writer::new();
    bytes.put(&table);
    let mut stores = String::new();
    for (offset, byte) in bytes.bytes().iter().enumerate() {
        stores.push_str(&format!(
            "i32.const {} i32.const {byte} i32.store8 ",
            AT + offset as u64
        ));
    }
    format!(
        "local.get 0 i32.const {calls} i32.eq if (result i64) {stores} i64.const {answer} else {body} end",
        calls = mod_api::wire::CALLS as i32,
        answer = (AT << 32) | bytes.bytes().len() as u64,
    )
}

/// The wasm import a hand-written guest calls `owner::method` through (`held-Owner` for a
/// method of a handler another mod gave the game).
#[cfg(any(test, feature = "listener-fixture", feature = "runtime-fixture"))]
pub fn import_name(owner: &str, method: &str) -> Option<&'static str> {
    crate::boundary::import_name(owner, method)
}

/// For a hand-written guest that calls the game as `call $host` with the game's ids (`import_id`):
/// the imports to declare instead of `horizon.host`, each call's own and `horizon.host` for the
/// reserved ids, and a `$host` function that sends each of `calls` through its own import.
/// Answer `CALLS` with [`answering`] and the keys [`shim_keys`] gives.
#[cfg(any(test, feature = "listener-fixture", feature = "runtime-fixture"))]
pub fn host_shim(calls: &[(&str, &str)]) -> (String, String) {
    let mut imports = String::from(
        r#"(import "horizon" "host" (func $control (param i32 i32 i32 i32 i32) (result i32)))"#,
    );
    let mut function = String::from("(func $host (param i32 i32 i32 i32 i32) (result i32) ");
    for (index, (owner, method)) in calls.iter().enumerate() {
        let name = shim_name(owner, method);
        let id = crate::boundary::import_id(owner, method)
            .unwrap_or_else(|| panic!("{owner}::{method} is no import"));
        imports.push_str(&format!(
            r#"(import "horizon" "{name}" (func $call{index} (param i32 i32 i32 i32) (result i32)))"#
        ));
        function.push_str(&format!(
            "local.get 0 i32.const {id} i32.eq if local.get 1 local.get 2 local.get 3 local.get 4 call $call{index} return end "
        ));
    }
    function.push_str("local.get 0 local.get 1 local.get 2 local.get 3 local.get 4 call $control)");
    (imports, function)
}

/// A hand-written guest (`wat`) that imports `horizon.host` as `$host` and exports `horizon_hook`,
/// made to link `calls` by name and answer `CALLS` the way a mod built against this game does.
#[cfg(feature = "runtime-fixture")]
pub fn by_name(wat: &str, calls: &[(&str, &str)]) -> String {
    const HOST: &str =
        r#"(import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))"#;
    const HOOK: &str = r#"(func (export "horizon_hook")"#;
    const LAYOUT: &str = r#"(func (export "horizon_layout")"#;
    assert!(wat.matches(HOOK).count() == 1 && wat.matches(LAYOUT).count() == 1);
    let (imports, host) = host_shim(calls);
    let hook = format!(
        "(func (export \"horizon_hook\") (param i32) (result i64) {})",
        answering(&shim_keys(calls), "local.get 0 call $guest_hook")
    );
    wat.replacen(HOST, &imports, 1)
        .replacen(HOOK, "(func $guest_hook", 1)
        .replacen(LAYOUT, &format!("{host} {hook} {LAYOUT}"), 1)
}

/// The keys of `calls` in the game's table, for [`answering`].
#[cfg(any(test, feature = "listener-fixture", feature = "runtime-fixture"))]
pub fn shim_keys(calls: &[(&str, &str)]) -> Vec<&'static str> {
    calls
        .iter()
        .map(|(owner, method)| key(shim_name(owner, method)))
        .collect()
}

#[cfg(any(test, feature = "listener-fixture", feature = "runtime-fixture"))]
fn shim_name(owner: &str, method: &str) -> &'static str {
    crate::boundary::import_name(owner, method)
        .or_else(|| crate::boundary::import_name(&format!("held-{owner}"), method))
        .unwrap_or_else(|| panic!("{owner}::{method} is no import"))
}

/// Checks a mod's table against the game's: each call in `imported` (by key, `Owner::method`)
/// must match, and each export the mod has must still be the game's, unchanged. Answers the ids
/// of the game's exports the mod doesn't have, sorted.
pub(crate) fn check(
    table: &[(String, u64, bool)],
    imported: &BTreeSet<String>,
) -> Result<Vec<u32>, Refusal> {
    let mine: HashMap<&str, (u64, bool)> = table
        .iter()
        .map(|(key, hash, export)| (key.as_str(), (*hash, *export)))
        .collect();
    let refusal = |call: &str, changed| Refusal::Call {
        call: call.to_owned(),
        changed,
    };
    for call in imported {
        // Linking found the import under this name, so the game has it.
        if mine.get(call.as_str()) != GAME.get(call.as_str()) {
            return Err(refusal(call, true));
        }
    }
    // By the mod's own word for what it exports: one the game dropped, its trait's last
    // included, would never run.
    for (key, hash, _) in table.iter().filter(|(.., export)| *export) {
        match GAME.get(key.as_str()) {
            Some(&(game, true)) if game == *hash => {}
            Some(&(_, true)) => return Err(refusal(key, true)),
            _ => return Err(refusal(key, false)),
        }
    }
    let mut missing: Vec<u32> = crate::boundary::EXPORT_NAMES
        .iter()
        .filter(|(_, name)| GAME.get(name).is_some_and(|(_, export)| *export))
        .filter(|(_, name)| !mine.contains_key(name))
        .map(|(id, _)| *id)
        .collect();
    missing.sort_unstable();
    missing.dedup();
    Ok(missing)
}

/// The key an import is checked under: its name without the signature hash.
pub(crate) fn key(import: &str) -> &str {
    import.split_once('#').map_or(import, |(key, _)| key)
}
