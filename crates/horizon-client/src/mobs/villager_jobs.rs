//! Villager job validation, acquisition and POI classification.
use super::*;
/// The POI a block state is (`PoiTypes`): job site blocks, bed heads, bells.
pub fn poi_at(w: &MobWorld, x: i32, y: i32, z: i32) -> Option<Poi> {
    poi_of_state(w, w.world.block(x, y, z))
}
/// Cache only vanilla classification. Mod additions are process-wide and replaced
/// on reload, so every live resource set consults the current extension table.
pub(super) fn poi_of_state(
    w: &MobWorld,
    state: horizon_world::registry::BlockStateId,
) -> Option<Poi> {
    let table = w.data.pois.get_or_init(|| {
        (0..w.registry.len())
            .map(|i| vanilla_state_poi(w, horizon_world::registry::BlockStateId(i as u32)))
            .collect()
    });
    let vanilla = table
        .get(state.0 as usize)
        .copied()
        .unwrap_or_else(|| vanilla_state_poi(w, state));
    vanilla
        .or_else(|| horizon_world::poi::extension(w.registry.state(state)?))
        .map(Poi::Vanilla)
        .or_else(|| {
            w.professions
                .workstation_pois
                .get(state.0 as usize)
                .copied()
                .flatten()
        })
}
/// Vanilla's job sites, bed heads and bells, without reloadable mod additions.
pub(super) fn vanilla_state_poi(
    w: &MobWorld,
    state: horizon_world::registry::BlockStateId,
) -> Option<PoiType> {
    let name = &*w.facts.get(state).name;
    if name == "minecraft:bell" {
        return Some(PoiType::Meeting);
    }
    let state = w.registry.state(state)?;
    let part = state.properties.get("part").map(String::as_str);
    if let Some(home) = runtime_bed_poi(name, part) {
        return Some(home);
    }
    horizon_world::poi::vanilla_poi(state).filter(|p| p.is_job())
}
pub(super) fn runtime_bed_poi(name: &str, part: Option<&str>) -> Option<PoiType> {
    // PoiTypes.BEDS contains the colored Blocks.BED collection, not STRAW_BED.
    (name.ends_with("_bed") && name != "minecraft:straw_bed" && part == Some("head"))
        .then_some(PoiType::Home)
}

/// Which POI types a predicate accepts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PoiFilter {
    /// `VillagerProfession.heldJobSite` of the current profession.
    Held,
    /// `acquirableJobSite` of the current profession.
    Acquirable,
    Home,
    Meeting,
}
pub(super) fn poi_matches(
    m: &Mob,
    f: PoiFilter,
    p: Poi,
    table: &crate::mobs::villager::ResolvedProfessions,
) -> bool {
    if matches!(f, PoiFilter::Held | PoiFilter::Acquirable)
        && table.state_index(&m.extra.profession).is_none()
    {
        return false;
    }
    let profession = table.state_index(&m.extra.profession).unwrap_or(0);
    let employed = profession != usize::from(crate::mobs::villager::NONE)
        && profession != usize::from(crate::mobs::villager::NITWIT);
    match (f, p) {
        (PoiFilter::Held, p) if p.is_job() => employed && table.accepts(profession, p, true),
        (PoiFilter::Acquirable, p) if p.is_job() => table.accepts(profession, p, false),
        (PoiFilter::Home, Poi::Vanilla(PoiType::Home))
        | (PoiFilter::Meeting, Poi::Vanilla(PoiType::Meeting)) => true,
        _ => false,
    }
}

pub(super) fn can_reset_profession(
    m: &Mob,
    table: &crate::mobs::villager::ResolvedProfessions,
) -> bool {
    table.state_index(&m.extra.profession).is_some_and(|prof| {
        prof != usize::from(crate::mobs::villager::NONE)
            && prof != usize::from(crate::mobs::villager::NITWIT)
            && crate::mobs::villager::xp(m) == 0
            && m.extra.villager_level <= 1
    })
}

fn seen_profession(
    seen: &crate::mobs::Seen,
    table: &crate::mobs::villager::ResolvedProfessions,
) -> Option<usize> {
    seen.profession.map(usize::from).or_else(|| {
        seen.custom_profession
            .as_deref()
            .and_then(|id| table.resolve(id))
    })
}

pub(super) fn scan_job_competitors(m: &mut Mob, cx: &mut Bx) -> bool {
    if cx.w.professions.state_index(&m.extra.profession).is_none() {
        return false;
    }
    let mm = mem(m);
    let (Some(site), true) = (mm.job_site.value(), mm.mobs.present()) else {
        return false;
    };
    if poi_at(cx.w, site[0], site[1], site[2]).is_some() {
        let list = mem(m).mobs.value().unwrap_or_default();
        let my_xp = crate::mobs::villager::xp(m);
        let mut winner_xp = my_xp;
        let mut me_winner = true;
        for who in list {
            let Who::Mob(id) = who else { continue };
            let Some(s) = cx.w.seen(id) else { continue };
            if s.species != Species::Villager || !s.alive || s.job != Some(site) {
                continue;
            }
            if cx.w.professions.state_index(&m.extra.profession)
                != seen_profession(s, cx.w.professions)
            {
                continue;
            }
            // selectWinner(first, second): the second wins ties.
            if winner_xp > s.xp {
                // The other villager loses its site (it finds out itself).
            } else {
                if me_winner {
                    mem(m).job_site.clear();
                }
                me_winner = false;
                winner_xp = s.xp;
            }
        }
    }
    true
}

pub(super) fn yield_job_site(m: &mut Mob, cx: &mut Bx) -> bool {
    let mm = mem(m);
    let Some(site) = mm.potential_job_site.value() else {
        return false;
    };
    if mm.job_site.present() || !mm.mobs.present() {
        return false;
    }
    if m.is_baby() || m.extra.profession.index() != Some(crate::mobs::villager::NONE) {
        return false;
    }
    let Some(poi) = poi_at(cx.w, site[0], site[1], site[2]) else {
        return true;
    };
    let list = mem(m).mobs.value().unwrap_or_default();
    for who in list {
        let Who::Mob(id) = who else { continue };
        let Some(s) = cx.w.seen(id) else { continue };
        if s.species != Species::Villager || !s.alive || s.potential_job.is_some() {
            continue;
        }
        let Some(j) = cx.w.professions.job(poi) else {
            continue;
        };
        if seen_profession(s, cx.w.professions) != Some(j) {
            continue;
        }
        // A nearby villager of that profession takes it over.
        if s.job.is_none_or(|p| p == site) {
            let mm = mem(m);
            mm.walk_target.clear();
            mm.look_target.clear();
            mm.potential_job_site.clear();
            break;
        }
    }
    true
}

pub(super) fn reset_profession(m: &mut Mob, cx: &mut Bx) -> bool {
    if mem(m).job_site.present() {
        return false;
    }
    if can_reset_profession(m, cx.w.professions) {
        crate::mobs::villager::set_profession(
            m,
            cx.w.professions,
            usize::from(crate::mobs::villager::NONE),
        );
        cx.refresh = true;
        true
    } else {
        false
    }
}

pub(super) fn assign_profession(m: &mut Mob, cx: &mut Bx, r: &mut bool) -> bool {
    if cx.w.professions.state_index(&m.extra.profession).is_none() {
        return false;
    }
    let Some(p) = mem(m).potential_job_site.value() else {
        return false;
    };
    if !closer_to_center(p, m.position, 2.0) {
        return false;
    }
    let mm = mem(m);
    mm.potential_job_site.clear();
    mm.job_site.set(p);
    happy_particles(m, cx);
    *r = true;
    if m.extra.profession.index() != Some(crate::mobs::villager::NONE) {
        return true;
    }
    if let Some(j) = poi_at(cx.w, p[0], p[1], p[2]).and_then(|p| cx.w.professions.job(p)) {
        crate::mobs::villager::set_profession(m, cx.w.professions, j);
        cx.refresh = true;
    }
    true
}

pub(super) fn validate_poi(
    m: &mut Mob,
    cx: &mut Bx,
    filter: PoiFilter,
    which: &mut PoiMem,
    running: &mut bool,
) -> bool {
    let Some(p) = which.get(m) else { return false };
    if !closer_to_center(p, m.position, 16.0) {
        return false;
    }
    if !poi_exists(m, cx.w, p, filter) {
        which.clear(m);
    } else if cx.w.block_name(p[0], p[1], p[2]).ends_with("_bed")
        && cx.w.prop(p[0], p[1], p[2], "occupied") == Some("true")
        && !crate::mobs::villager::sleeping(m)
    {
        // ValidateNearbyPoi.bedIsOccupied: someone else sleeps there.
        which.clear(m);
    }
    *running = true;
    true
}
