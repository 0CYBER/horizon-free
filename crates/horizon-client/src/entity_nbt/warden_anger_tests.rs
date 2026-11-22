use super::*;
use crate::mobs::{Species, sensor_scan_tests::kind, warden::AngerManagement};
use std::cell::Cell;

/// A warden, with everything its save reads. No resources are needed: only
/// the species decides which fields it has.
fn warden(id: u32) -> Mob {
    let mut kind = (*kind()).clone();
    kind.id = "minecraft:warden".into();
    kind.species = Species::Warden;
    Mob::new(id, Arc::new(kind), Vec3::new(0.0, 64.0, 0.0), 0.0)
}

fn types() -> HashMap<String, Arc<MobType>> {
    let mut warden = (*kind()).clone();
    warden.id = "minecraft:warden".into();
    warden.species = Species::Warden;
    HashMap::from([("minecraft:warden".to_string(), Arc::new(warden))])
}

/// `Warden.addAdditionalSaveData` stores the anger as `AngerManagement`'s
/// codec does - one `{uuid, anger}` pair per suspect, the ones in the level
/// by each entity's UUID and then the ones that are away - and the per
/// suspect anger comes back with the mob.
#[test]
fn a_wardens_anger_round_trips_as_suspect_uuid_pairs() {
    let mut m = warden(1);
    let away = [4, 0, 0, 42];
    let mut anger = AngerManagement::from_saved([(away, 12)]);
    anger.increase(Who::Player, 95);
    anger.increase(Who::Mob(7), 41);
    m.extra.warden.as_mut().unwrap().anger = anger;
    let pig_uuid = [0, 0, 0, 7];
    let tag = mob_to_nbt(&m, [9, 9, 9, 9], None, None, |id| {
        (id == 7).then_some(pig_uuid)
    });
    // A warden is no `NeutralMob`: neither of its keys is written.
    assert!(tag.get("angry_at").is_none());
    assert!(tag.get("anger_end_time").is_none());
    let stored = tag.get("anger").and_then(|t| t.get("suspects"));
    let Some(Tag::List(pairs)) = stored else {
        panic!("the anger is saved as a suspects list");
    };
    let pair = |t: &Tag| {
        (
            uuid_from_tag(t.get("uuid")).unwrap(),
            number(t.get("anger")),
        )
    };
    assert_eq!(
        pairs.iter().map(pair).collect::<Vec<_>>(),
        // The angry suspect first (`AngerManagement.Sorter`), then the
        // other one in the level, then the one that is away.
        vec![
            (player_uuid(), Some(95)),
            (pig_uuid, Some(41)),
            (away, Some(12)),
        ]
    );

    // Loading puts every suspect in the UUID map: a warden may load long
    // before the mobs it is angry at do.
    let back = mob_from_nbt(&tag, &types(), 2, None).unwrap();
    let st = back.extra.warden.as_ref().unwrap();
    assert!(st.anger.suspects().is_empty());
    assert_eq!(
        st.anger.unloaded().collect::<Vec<_>>(),
        vec![(player_uuid(), 95), (pig_uuid, 41), (away, 12)]
    );
}

/// Anger at a suspect that is still away outlives the session: it decays
/// while nothing answers for its UUID, and the next save writes the pair
/// again, so a warden's anger at a mob in an unloaded chunk is not lost by
/// saving and loading twice.
#[test]
fn an_away_suspect_survives_another_save() {
    let away = [4, 0, 0, 42];
    let mut m = warden(1);
    m.extra.warden.as_mut().unwrap().anger = AngerManagement::from_saved([(away, 12)]);
    let tag = mob_to_nbt(&m, [9, 9, 9, 9], None, None, |_| None);
    let mut back = mob_from_nbt(&tag, &types(), 2, None).unwrap();
    let st = back.extra.warden.as_mut().unwrap();
    for _ in 0..4 {
        st.anger.tick(|_| true, |_| None);
    }
    let saved = mob_to_nbt(&back, [9, 9, 9, 9], None, None, |_| None);
    let Some(Tag::List(pairs)) = saved.get("anger").and_then(|t| t.get("suspects")) else {
        panic!("the away suspect is still saved");
    };
    assert_eq!(pairs.len(), 1);
    assert_eq!(uuid_from_tag(pairs[0].get("uuid")), Some(away));
    assert_eq!(number(pairs[0].get("anger")), Some(8));
}

/// A warden with no suspect writes the codec's default: an `anger` compound
/// with no `suspects` key, which is what vanilla's `lenientOptionalFieldOf`
/// leaves out.
#[test]
fn a_warden_with_no_anger_writes_no_suspects() {
    let m = warden(1);
    let tag = mob_to_nbt(&m, [9, 9, 9, 9], None, None, |_| None);
    assert_eq!(
        tag.get("anger"),
        Some(&Tag::compound(Vec::<(&str, Tag)>::new()))
    );
    let back = mob_from_nbt(&tag, &types(), 2, None).unwrap();
    let st = back.extra.warden.as_ref().unwrap();
    assert_eq!(st.anger, AngerManagement::default());
}

/// A suspect whose anger is below zero, or whose UUID is not four ints,
/// does not decode: the whole list is dropped, as
/// `lenientOptionalFieldOf` does with a value that fails.
#[test]
fn a_suspects_list_that_does_not_decode_is_ignored() {
    let pair = |uuid: Tag, anger: i32| Tag::compound([("uuid", uuid), ("anger", Tag::Int(anger))]);
    let anger = |pairs: Vec<Tag>| Tag::compound([("suspects", Tag::List(pairs))]);
    let four = || Tag::IntArray(vec![1, 2, 3, 4]);
    assert_eq!(
        warden_anger_from_nbt(&anger(vec![pair(four(), 30)])),
        vec![([1, 2, 3, 4], 30)]
    );
    // `ExtraCodecs.NON_NEGATIVE_INT` refuses a negative anger.
    assert!(warden_anger_from_nbt(&anger(vec![pair(four(), -1)])).is_empty());
    // `UUIDUtil.CODEC` needs exactly four ints.
    assert!(warden_anger_from_nbt(&anger(vec![pair(Tag::IntArray(vec![1, 2, 3]), 30)])).is_empty());
    // One bad pair takes the good ones with it.
    assert!(warden_anger_from_nbt(&anger(vec![pair(four(), 30), pair(four(), -7)])).is_empty());
    // No list at all (and an `anger` that is no compound) is no suspect.
    assert!(warden_anger_from_nbt(&Tag::compound(Vec::<(&str, Tag)>::new())).is_empty());
    assert!(warden_anger_from_nbt(&Tag::Int(1)).is_empty());
}

/// An away suspect calms down by one an anger tick, like the ones in the
/// level, and is forgotten at one point - and its UUID keeps the anger until
/// it does.
#[test]
fn an_away_suspect_calms_down_one_a_tick() {
    let mut a = AngerManagement::from_saved([([1, 0, 0, 1], 30), ([2, 0, 0, 2], 1)]);
    for _ in 0..5 {
        a.tick(|_| true, |_| None);
    }
    // Five ticks, five points: the one at one point was dropped on the first.
    assert_eq!(a.unloaded().collect::<Vec<_>>(), vec![([1, 0, 0, 1], 25)]);
    for _ in 0..24 {
        a.tick(|_| true, |_| None);
    }
    assert_eq!(a.unloaded().collect::<Vec<_>>(), vec![([1, 0, 0, 1], 1)]);
    a.tick(|_| true, |_| None);
    assert!(a.unloaded().next().is_none());
    // Nothing in the level sees that anger while it is away.
    assert!(a.suspects().is_empty());
    assert_eq!(a.active_anger(None), 0);
}

/// The reconversion: an away suspect's UUID becomes a live `Who` once the
/// mob is loaded, on the first anger tick that is due a lookup
/// (`conversionDelay` counts two of them).
#[test]
fn an_away_suspect_rejoins_once_its_mob_is_loaded() {
    let uuid = [7, 0, 0, 3];
    let mut a = AngerManagement::from_saved([(uuid, 60)]);
    // The mob is not in the level for the first lookups.
    let loaded = Cell::new(false);
    let by_uuid = |u: [i32; 4]| (loaded.get() && u == uuid).then_some(Who::Mob(3));
    // A fresh anger converts on its first tick, then every second one.
    a.tick(|_| true, by_uuid);
    assert_eq!(a.unloaded().collect::<Vec<_>>(), vec![(uuid, 59)]);
    assert!(a.suspects().is_empty());
    // Now the mob loads. This tick is no lookup one: nothing happens yet.
    loaded.set(true);
    a.tick(|_| true, by_uuid);
    assert_eq!(a.unloaded().collect::<Vec<_>>(), vec![(uuid, 58)]);
    assert!(a.suspects().is_empty());
    // The lookup is due: it rejoins with the anger it kept.
    a.tick(|_| true, by_uuid);
    assert!(a.unloaded().next().is_none());
    assert_eq!(a.suspects(), &[Who::Mob(3)]);
    // 60, less one for each of the three ticks: the two it was away for and
    // the one it rejoined on, which calms it like any suspect in the level.
    assert_eq!(a.anger_of(Who::Mob(3)), 57);
    assert_eq!(a.active_anger(None), 57);
}

/// Anger gained at a suspect that has not rejoined yet: vanilla's
/// `increaseAnger` takes the anger kept under its UUID into the new total
/// (min(150, 35) + 20 = 55), so the lookup adds it rather than dropping it or
/// letting it overwrite the new points; one anger tick then calms it to 54.
#[test]
fn a_suspect_that_has_not_rejoined_keeps_both_angers() {
    let mut a = AngerManagement::from_saved([([3, 0, 0, 3], 20)]);
    assert_eq!(a.increase(Who::Mob(3), 35), 35);
    a.tick(|_| true, |_| Some(Who::Mob(3)));
    assert_eq!(a.anger_of(Who::Mob(3)), 54);
    assert!(a.unloaded().next().is_none());
}
