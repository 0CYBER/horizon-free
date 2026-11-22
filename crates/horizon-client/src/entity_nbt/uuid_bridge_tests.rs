use super::*;
use crate::mobs::{Species, sensor_scan_tests::kind};

/// A wolf, whose anger is `NeutralMob`'s, so its save carries `angry_at`.
/// No resources are needed: only the species decides that.
fn wolf(id: u32) -> Mob {
    let mut kind = (*kind()).clone();
    kind.id = "minecraft:wolf".into();
    kind.species = Species::Wolf;
    let mut m = Mob::new(id, Arc::new(kind), Vec3::ZERO, 0.0);
    m.extra.anger = 400;
    m.extra.anger_end = 3000;
    m
}

/// A mob's anger at another mob is saved as that mob's UUID, which the game
/// layer answers for (`Who` itself carries none).
#[test]
fn a_mob_anger_target_is_saved_as_its_uuid() {
    let mut m = wolf(1);
    m.extra.angry_at = Some(Who::Mob(7));
    let target = [7, 0, 0, 3];
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |id| {
        (id == 7).then_some(target)
    });
    assert_eq!(tag.get("angry_at"), Some(&Tag::IntArray(target.to_vec())));
    assert_eq!(number(tag.get("anger_end_time")), Some(3000));
}

/// A target the map does not know (a mob that left the level) is not named:
/// nothing is written rather than a made-up UUID no entity answers for.
#[test]
fn an_unknown_anger_target_writes_no_angry_at() {
    let mut m = wolf(1);
    m.extra.angry_at = Some(Who::Mob(7));
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(tag.get("angry_at"), None);
    assert_eq!(number(tag.get("anger_end_time")), Some(3000));
}

/// The player is not an id: its UUID needs no map from the caller.
#[test]
fn a_player_anger_target_needs_no_map() {
    let mut m = wolf(1);
    m.extra.angry_at = Some(Who::Player);
    let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None, |_| None);
    assert_eq!(
        tag.get("angry_at"),
        Some(&Tag::IntArray(player_uuid().to_vec()))
    );
}

/// The round trip both deferred fixes need: a mob's UUID written through the
/// save seam reads back as that mob, not as another one.
#[test]
fn a_saved_mob_uuid_resolves_back_to_it() {
    let mut wolf = wolf(1);
    let pig = {
        let mut kind = (*kind()).clone();
        kind.id = "minecraft:pig".into();
        kind.species = Species::Pig;
        Mob::new(9, Arc::new(kind), Vec3::ZERO, 0.0)
    };
    let (wolf_uuid, pig_uuid) = ([1, 2, 3, 4], [9, 0, 0, 1]);
    let uuids: HashMap<u32, [i32; 4]> = HashMap::from([(1, wolf_uuid), (9, pig_uuid)]);
    wolf.extra.angry_at = Some(Who::Mob(pig.id));
    let tag = mob_to_nbt(&wolf, wolf_uuid, None, None, |id| uuids.get(&id).copied());
    let Some(Tag::IntArray(stored)) = tag.get("angry_at") else {
        panic!("anger at a mob is saved as that mob's UUID");
    };
    assert_eq!(stored, &pig_uuid.to_vec());
    let mobs = [wolf, pig];
    assert_eq!(
        who_by_uuid(&uuids, &mobs, None, pig_uuid),
        Some(Who::Mob(9))
    );
    // The other loaded mob is not mistaken for it.
    assert_eq!(
        who_by_uuid(&uuids, &mobs, None, wolf_uuid),
        Some(Who::Mob(1))
    );
    // The player is the other half of `Who`, kept apart from the id map.
    let player = [5, 5, 5, 5];
    assert_eq!(
        who_by_uuid(&uuids, &mobs, Some(player), player),
        Some(Who::Player)
    );
}

/// A UUID nothing loaded has is no entity at all: not another mob, and not
/// the projectile whose id shares the map.
#[test]
fn a_uuid_no_loaded_mob_has_resolves_to_nothing() {
    let wolf = wolf(1);
    let uuids: HashMap<u32, [i32; 4]> = HashMap::from([(1, [1, 2, 3, 4]), (1000, [5, 0, 0, 1])]);
    let mobs = [wolf];
    // Never loaded (it left, or the save is another world's).
    assert_eq!(who_by_uuid(&uuids, &mobs, None, [9, 9, 9, 9]), None);
    // Loaded, but not a mob: `Who::Mob` would mean an arrow's id.
    assert_eq!(who_by_uuid(&uuids, &mobs, None, [5, 0, 0, 1]), None);
    // Nothing loaded at all resolves to nothing either.
    assert_eq!(who_by_uuid(&uuids, &[], None, [1, 2, 3, 4]), None);
    assert_eq!(
        who_by_uuid(&uuids, &mobs, None, [1, 2, 3, 4]),
        Some(Who::Mob(1))
    );
}
