//! Profession-keyed additions to the vanilla secondary POI sensor.
use super::*;

#[horizon_mod_api_macros::replaceable]
pub(super) fn sense_secondary(m: &mut Mob, w: &MobWorld) {
    let index = w.professions.state_index(&m.extra.profession);
    let custom = index
        .and_then(|index| w.professions.entry(index))
        .filter(|_| index.is_some_and(|i| i >= 15));
    let farmer = super::super::villager::is_farmer(m);
    // Tags are resolved with the world's item generation. Borrow its membership
    // set once; the scan performs only constant-time block membership checks.
    let additions = index
        .and_then(|index| TAGS.get(index))
        .and_then(|tag| w.block_tags.get(*tag));
    if !farmer
        && additions.is_none_or(|set| set.is_empty())
        && custom.is_none_or(|p| p.secondary_blocks.is_empty())
    {
        mem(m).secondary_job_site.clear();
        return;
    }
    let c = block_of(m.position);
    let mut sites = Vec::new();
    for x in -4..=4 {
        for y in -2..=2 {
            for z in -4..=4 {
                #[cfg(test)]
                SCAN_READS.with(|reads| reads.set(reads.get() + 1));
                let p = [c[0] + x, c[1] + y, c[2] + z];
                if farmer && farmland(w, p)
                    || additions.is_some_and(|set| set.contains(w.block_name(p[0], p[1], p[2])))
                    || custom.is_some_and(|entry| {
                        entry
                            .secondary_blocks
                            .contains(w.block_name(p[0], p[1], p[2]))
                    })
                {
                    sites.push(p);
                }
            }
        }
    }
    mem(m).secondary_job_site.set_list(sites);
}

const TAGS: [&str; 15] = [
    "horizon:villager_secondary_poi/none",
    "horizon:villager_secondary_poi/armorer",
    "horizon:villager_secondary_poi/butcher",
    "horizon:villager_secondary_poi/cartographer",
    "horizon:villager_secondary_poi/cleric",
    "horizon:villager_secondary_poi/farmer",
    "horizon:villager_secondary_poi/fisherman",
    "horizon:villager_secondary_poi/fletcher",
    "horizon:villager_secondary_poi/leatherworker",
    "horizon:villager_secondary_poi/librarian",
    "horizon:villager_secondary_poi/mason",
    "horizon:villager_secondary_poi/nitwit",
    "horizon:villager_secondary_poi/shepherd",
    "horizon:villager_secondary_poi/toolsmith",
    "horizon:villager_secondary_poi/weaponsmith",
];

#[cfg(test)]
thread_local! { static SCAN_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
#[path = "villager_secondary_tests.rs"]
mod tests;
