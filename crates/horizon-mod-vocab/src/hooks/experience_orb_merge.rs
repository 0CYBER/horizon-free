//! The `experience_orb_merge` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0130

pub trait Hooks {
    /// The experience orb `into` (`into_value` points, `into_count` orbs) is about to
    /// take `from` (`from_value` points, `from_count` orbs) in, after
    /// `experience_orb_can_merge` said yes (`from` is 0 and `from_count` 1 for a new orb
    /// that is spawning). Set `canceled` to stop it: both orbs stay, and a spawning orb
    /// tries the next orb in reach, then spawns on its own. Left alone, `into` adds
    /// `from`'s count, as vanilla does. A mod that writes `into`'s value or count with
    /// `set_experience_orb` here makes that write the merged orb: `from` is taken in
    /// without adding its count, so `into_value = 11, into_count = 1` pays 11 once. The
    /// merge is refused, and `into` put back as it was (value, count and age), when `from`
    /// has changed or left reach by the time the merge applies. A merge whose count would
    /// pass `i32::MAX` (what a save holds) is refused the same way. A write the listener made
    /// to `from` itself is not undone. Server-side.
    #[notes = r#"## Experience orb merge

- **`experience_orb_merge(dimension, into, from, into_value, into_count, from_value, from_count, canceled)`** follows a yes from `experience_orb_can_merge` and may cancel the merge: neither orb is absorbed, and a spawning orb (`from` 0, `from_count` 1) tries the next orb in reach, then spawns on its own. `into_value` is signed and reported as the orb holds it, so a negative orb is not clamped to the 0 points it pays.
- Left alone, `into` adds `from`'s count. A mod that writes `into` with `set_experience_orb` here makes that write the merged orb: `from` is taken in without adding its count, so `into_value = 11, into_count = 1` pays 11 once.
- The merge is refused, and `into` put back as it was (value, count and age), when the mod cancels, when `from` changed or left reach during the hook, or when the count would pass `i32::MAX`, what a save holds. Neither orb is absorbed; a write the listener made to `from` itself is not undone.
- `set_experience_orb` writes a `value` within -32768..=32767 (a save keeps it as a short) and a `count` from 1 to `i32::MAX`; it refuses anything else with nothing written, so an accepted write reloads unchanged.
- Version 2 adds `into_value`, `into_count` and `from_count`; rebuild listeners against this SDK."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        version = 2,
        neoforge = "none (vanilla ExperienceOrb.tryMergeToExisting; NeoForge adds no event or extension for it)"
    )]
    fn experience_orb_merge(
        dimension: &str,
        into: u32,
        from: u32,
        into_value: i32,
        into_count: u32,
        from_value: i32,
        from_count: u32,
        canceled: &mut bool,
    );
}
