//! Value-only eviction policy. Selection is not authorization: callers must still hold
//! the build lease and check each unit's access immediately before removing it.
use std::time::{Duration, SystemTime};

pub const DEFAULT_LIMIT_GIB: u64 = 25;
const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Protected,
    Expiring,
    Lru,
}

pub struct Unit {
    pub bytes: u64,
    pub eligible: bool,
    /// Missing stamps are unknown, not evidence that a cache is old.
    pub last_used: Option<SystemTime>,
    pub tier: Tier,
}

/// Stable inventory indices, in deletion order. The caller counts failed deletions as
/// residual; it must not retry a busy/refused unit or silently consider its bytes freed.
pub fn select(units: &[Unit], limit: Option<u64>, now: SystemTime) -> Vec<usize> {
    let mut remaining = units.iter().fold(0u64, |n, u| n.saturating_add(u.bytes));
    let mut selected = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        if unit.eligible
            && unit.tier == Tier::Expiring
            && unit
                .last_used
                .is_some_and(|used| now.duration_since(used).is_ok_and(|age| age >= WEEK))
        {
            selected.push(index);
            remaining = remaining.saturating_sub(unit.bytes);
        }
    }
    if let Some(limit) = limit.filter(|limit| remaining > *limit) {
        let target = limit / 10 * 9 + limit % 10 * 9 / 10;
        let mut lru: Vec<_> = units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.eligible && u.tier == Tier::Lru && u.last_used.is_some())
            .collect();
        lru.sort_by_key(|(_, u)| u.last_used);
        for (index, unit) in lru {
            if remaining <= target {
                break;
            }
            selected.push(index);
            remaining = remaining.saturating_sub(unit.bytes);
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unit(bytes: u64, age_days: u64, tier: Tier) -> Unit {
        Unit {
            bytes,
            eligible: true,
            last_used: Some(SystemTime::UNIX_EPOCH + Duration::from_secs((30 - age_days) * 86400)),
            tier,
        }
    }
    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(30 * 86400)
    }
    #[test]
    fn protected_and_unknown_are_never_selected() {
        let units = [
            unit(100, 20, Tier::Protected),
            Unit {
                bytes: 100,
                eligible: true,
                last_used: None,
                tier: Tier::Expiring,
            },
            Unit {
                bytes: 100,
                eligible: true,
                last_used: None,
                tier: Tier::Lru,
            },
        ];
        assert!(select(&units, Some(1), now()).is_empty());
    }
    #[test]
    fn lru_order_and_ninety_percent_watermark() {
        let units = [
            unit(10, 1, Tier::Lru),
            unit(10, 3, Tier::Lru),
            unit(10, 2, Tier::Lru),
        ];
        assert_eq!(select(&units, Some(25), now()), [1]);
        assert_eq!(select(&units, Some(22), now()), [1, 2]);
        assert!(select(&units, Some(30), now()).is_empty());
    }
    #[test]
    fn seven_day_tier_runs_without_a_limit_but_not_early() {
        let units = [
            unit(10, 6, Tier::Expiring),
            unit(10, 7, Tier::Expiring),
            unit(10, 8, Tier::Expiring),
        ];
        assert_eq!(select(&units, None, now()), [1, 2]);
    }
    #[test]
    fn ineligible_units_remain_in_the_budget() {
        let mut busy = unit(20, 20, Tier::Lru);
        busy.eligible = false;
        let units = [busy, unit(10, 2, Tier::Lru), unit(10, 1, Tier::Lru)];
        assert_eq!(select(&units, Some(25), now()), [1, 2]);
    }

    #[test]
    fn future_stamp_is_not_old() {
        let units = [Unit {
            bytes: 10,
            eligible: true,
            tier: Tier::Expiring,
            last_used: Some(now() + WEEK),
        }];
        assert!(select(&units, Some(1), now()).is_empty());
    }
}
