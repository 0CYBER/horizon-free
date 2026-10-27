//! Zombie villagers (`ZombieVillager`): the cure started by a golden apple on
//! a weakened one, the conversion clock that iron bars and beds speed up, and
//! the saved fields. Turning villagers into zombie villagers and back is done
//! by the game (`game/zombie_villagers.rs`), which owns entity creation.
use super::{Mob, MobEvent, MobWorld, Species, goals::Cx, villager};
use horizon_core::random::RandomSource;
use horizon_world::nbt::{Compound, Tag};

/// `VILLAGER_CONVERSION_WAIT_MIN` and the width of the random range.
pub const WAIT_MIN: i32 = 3600;
pub const WAIT_SPREAD: i32 = 2401;
/// `MAX_SPECIAL_BLOCKS_COUNT` and `SPECIAL_BLOCK_RADIUS`.
const MAX_SPECIAL_BLOCKS: i32 = 14;
const RADIUS: i32 = 4;
const SPEEDS_UP_CURING: &str = "minecraft:speeds_up_zombie_villager_curing";

/// A cure under way: ticks left and the player who started it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cure {
    pub time: i32,
    pub starter: Option<[i32; 4]>,
}

pub fn cure(m: &Mob) -> Option<&Cure> {
    villager::ext(m).and_then(|e| e.cure.as_ref())
}
/// `ZombieVillager.DATA_CONVERTING_ID` on a client: only `is_converting`
/// (the cure's shaking) reads the cure, so its time does not matter.
pub fn set_converting(m: &mut Mob, on: bool) {
    villager::ext_mut(m).cure = on.then_some(Cure {
        time: 0,
        starter: None,
    });
}
pub fn is_converting(m: &Mob) -> bool {
    m.species() == Species::ZombieVillager && cure(m).is_some()
}
/// `removeWhenFarAway`: curing ones and those with experience stay.
pub fn keeps_alive(m: &Mob) -> bool {
    m.species() == Species::ZombieVillager
        && (is_converting(m) || m.extra.trade.as_ref().is_some_and(|t| t.xp != 0))
}

/// `ZombieVillager.startConverting`: weakness gives way to strength for the
/// length of the cure and the cure sound plays (entity event 16).
pub fn start_converting(
    m: &mut Mob,
    starter: Option<[i32; 4]>,
    time: i32,
    events: &mut Vec<MobEvent>,
) {
    villager::ext_mut(m).cure = Some(Cure { time, starter });
    m.extra.effects.retain(|e| e.0 != "minecraft:weakness");
    // Strength amplifier: min(difficulty id - 1, 0), always 0 with monsters.
    m.add_effect("minecraft:strength", time, 0);
    let pitch = m.random.next_float() * 0.7 + 0.3;
    let volume = 1.0 + m.random.next_float();
    events.push(MobEvent::Sound(
        "entity.zombie_villager.cure",
        [m.position.x, m.eye_y(), m.position.z],
        volume,
        pitch,
    ));
}

/// `getConversionProgress`: 1 per tick, plus a chance per nearby iron bars or
/// bed (up to 14 of them) found on one tick in a hundred.
fn progress(m: &mut Mob, w: &MobWorld) -> i32 {
    let mut amount = 1;
    if m.random.next_float() < 0.01 {
        let mut special = 0;
        // `(int) getX()` truncates toward zero.
        let (bx, by, bz) = (
            m.position.x as i32,
            m.position.y as i32,
            m.position.z as i32,
        );
        'scan: for x in bx - RADIUS..bx + RADIUS {
            for y in by - RADIUS..by + RADIUS {
                for z in bz - RADIUS..bz + RADIUS {
                    if special >= MAX_SPECIAL_BLOCKS {
                        break 'scan;
                    }
                    if w.has_tag(SPEEDS_UP_CURING, w.block_name(x, y, z)) {
                        if m.random.next_float() < 0.3 {
                            amount += 1;
                        }
                        special += 1;
                    }
                }
            }
        }
    }
    amount
}

/// `ZombieVillager.tick`: count the cure down, finishing it at zero.
pub fn step(m: &mut Mob, cx: &mut Cx) {
    if !m.alive() || !is_converting(m) {
        return;
    }
    let amount = progress(m, cx.w);
    let Some(c) = villager::ext_mut(m).cure.as_mut() else {
        return;
    };
    c.time -= amount;
    if c.time <= 0 {
        cx.events.push(MobEvent::CureZombieVillager { mob: m.id });
    }
}

/// `ConversionTime` (-1: not converting) and `ConversionPlayer`.
fn put_cure(map: &mut Compound, cure: Option<Cure>) {
    map.insert(
        "ConversionTime".into(),
        Tag::Int(cure.map_or(-1, |c| c.time)),
    );
    if let Some(starter) = cure.and_then(|c| c.starter) {
        map.insert("ConversionPlayer".into(), Tag::IntArray(starter.to_vec()));
    }
}
fn parse_cure(tag: &Tag) -> Option<Cure> {
    let time = match tag.get("ConversionTime") {
        Some(Tag::Int(v)) => *v,
        _ => -1,
    };
    (time != -1).then(|| Cure {
        time,
        starter: crate::save::uuid_from_tag(tag.get("ConversionPlayer")),
    })
}
/// The saved fields: the cure and the gossip.
pub fn write_nbt(m: &Mob, map: &mut Compound) {
    put_cure(map, cure(m).copied());
    if let Some(e) = villager::ext(m)
        && !e.gossips.is_empty()
    {
        map.insert("Gossips".into(), e.gossips.to_nbt());
    }
}
pub fn read_nbt(m: &mut Mob, tag: &Tag) {
    let cure = parse_cure(tag);
    let e = villager::ext_mut(m);
    e.cure = cure;
    e.gossips = tag
        .get("Gossips")
        .map(crate::gossip::Gossips::from_nbt)
        .unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cure_round_trips_through_nbt() {
        let cure = Cure {
            time: 4200,
            starter: Some([1, 2, 3, 4]),
        };
        let mut map = Compound::new();
        put_cure(&mut map, Some(cure));
        assert_eq!(parse_cure(&Tag::Compound(map)), Some(cure));
    }
    #[test]
    fn no_cure_saves_minus_one() {
        let mut map = Compound::new();
        put_cure(&mut map, None);
        assert_eq!(map.get(&"ConversionTime".into()), Some(&Tag::Int(-1)));
        assert_eq!(parse_cure(&Tag::Compound(map)), None);
    }
}
