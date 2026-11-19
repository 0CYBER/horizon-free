//! The automatic sizes of the long-lived pools the client starts at launch or
//! when a world opens, in one table: Battery Mode halves each of them through it,
//! instead of the streaming pool alone. Far terrain's pools
//! (`crates/horizon-client-render/src/lod.rs`'s sampling and meshing,
//! `crates/horizon-client/src/vanillagen/lod.rs`'s Far decoration) are not in the
//! table: they start only with far terrain on and keep formulas measured there.

use std::sync::OnceLock;

/// One of the long-lived pools the client starts at launch or when a world opens.
#[derive(Clone, Copy, Debug)]
pub enum Pool {
    /// Chunk generation, light and meshing (`crate::jobs`).
    Jobs,
    /// The helper threads of the parallel tick.
    TickHelper,
    /// The workers that decode the chunks a server sends.
    ChunkDecoder,
    /// Save compression.
    SaveCompression,
    /// The network threads of a LAN host.
    Net,
    /// The LAN warm-up threads that encode chunks for guests.
    ChunkEncode,
}

/// The automatic size `pool` takes on a machine of `cores` cores; under Battery
/// Mode half of it, rounded up, and a pool that takes none (a single core's
/// warm-up threads) still takes none.
pub fn sizing(battery: bool, cores: usize, pool: Pool) -> usize {
    let auto = match pool {
        Pool::Jobs => cores.saturating_sub(1).max(1),
        Pool::TickHelper => cores.clamp(1, 8).saturating_sub(1).max(1),
        Pool::ChunkDecoder => cores.clamp(1, 4),
        Pool::SaveCompression => cores.div_ceil(2).max(1),
        Pool::Net => cores.div_ceil(2),
        Pool::ChunkEncode => cores.saturating_sub(1),
    };
    if battery { auto.div_ceil(2) } else { auto }
}

/// Battery Mode as the pools were sized with it: the first answer fixes it, since
/// the pools start once and a later call changes nothing they already took.
pub fn set_battery(battery: bool) {
    let _ = BATTERY.set(battery);
}

/// Battery Mode's answer for this process: the one [`set_battery`] fixed, and false
/// before anything fixed it (a process that starts no client pool, or a test).
pub fn battery() -> bool {
    BATTERY.get().copied().unwrap_or(false)
}

static BATTERY: OnceLock<bool> = OnceLock::new();

/// [`sizing`] with this process's cores and `battery` as the caller knows it.
pub fn for_pool_with(battery: bool, pool: Pool) -> usize {
    let cores = match pool {
        Pool::Jobs => crate::usable_cores(),
        _ => crate::worker_count(),
    };
    sizing(battery, cores, pool)
}

/// [`for_pool_with`] under this process's Battery Mode ([`battery`]).
pub fn for_pool(pool: Pool) -> usize {
    for_pool_with(battery(), pool)
}

#[cfg(test)]
mod tests {
    use super::{Pool, battery, set_battery, sizing};

    /// Every pool, in the order the table below lists them.
    const ALL: [Pool; 6] = [
        Pool::Jobs,
        Pool::TickHelper,
        Pool::ChunkDecoder,
        Pool::SaveCompression,
        Pool::Net,
        Pool::ChunkEncode,
    ];

    /// Each core count gives each pool the size written here, plain and under
    /// Battery Mode: the numbers are the pools' own ones, not worked out again.
    #[test]
    fn the_table_sizes_every_pool_and_halves_it() {
        let table: [(usize, [usize; 6], [usize; 6]); 4] = [
            (2, [1, 1, 2, 1, 1, 1], [1, 1, 1, 1, 1, 1]),
            (4, [3, 3, 4, 2, 2, 3], [2, 2, 2, 1, 1, 2]),
            (8, [7, 7, 4, 4, 4, 7], [4, 4, 2, 2, 2, 4]),
            (16, [15, 7, 4, 8, 8, 15], [8, 4, 2, 4, 4, 8]),
        ];
        for (cores, plain, halved) in table {
            for (i, pool) in ALL.into_iter().enumerate() {
                assert_eq!(
                    sizing(false, cores, pool),
                    plain[i],
                    "{cores} cores, {pool:?}"
                );
                assert_eq!(
                    sizing(true, cores, pool),
                    halved[i],
                    "{cores} cores, {pool:?}, battery"
                );
            }
        }
    }

    /// On a 4-core machine the six pools add up to 17 threads, and to 10 under
    /// Battery Mode: the mode takes more than a third of them away.
    #[test]
    fn a_four_core_machines_pools_add_up() {
        let sum = |battery_mode: bool| {
            ALL.into_iter()
                .map(|pool| sizing(battery_mode, 4, pool))
                .sum::<usize>()
        };
        assert_eq!(sum(false), 17);
        assert_eq!(sum(true), 10);
    }

    /// The first answer wins, so a pool started before the setting changed keeps
    /// the size it was counted at. No other test in this crate reads [`battery`].
    #[test]
    fn the_battery_setting_is_fixed_the_first_time() {
        set_battery(true);
        assert!(battery());
        set_battery(false);
        assert!(battery());
    }

    /// A plain Jobs pool is the streaming pool's own automatic count, so the table
    /// changed nothing for it while Battery Mode is off.
    #[test]
    fn jobs_matches_the_streaming_pools_own_count() {
        assert_eq!(
            sizing(false, crate::usable_cores(), Pool::Jobs),
            crate::default_workers()
        );
    }
}
