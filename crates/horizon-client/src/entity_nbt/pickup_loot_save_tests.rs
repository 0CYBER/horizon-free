use super::*;
use crate::mobs::Species;

#[test]
fn pickup_save_uses_live_state_instead_of_raw_and_preserves_overrides() {
    for (species, id) in [
        (Species::Pig, "minecraft:pig"),
        (Species::Allay, "minecraft:allay"),
        (Species::SulfurCube, "minecraft:sulfur_cube"),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = id.into();
        ty.species = species;
        let types = HashMap::from([(id.to_string(), kind)]);
        for supplied in [None, Some(false), Some(true)] {
            let mut raw = Tag::compound([
                ("id", string(id)),
                ("Pos", doubles(&[4.5, 200.0, 4.5])),
                ("audit:unrelated", Tag::Int(7)),
            ]);
            if let Some(on) = supplied {
                put(&mut raw, "CanPickUpLoot", Tag::Byte(i8::from(on)));
            }
            let mut mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
            for live in [false, true] {
                mob.extra.pick_up_loot = live;
                mob.extra
                    .set_main_hand(live.then(|| "minecraft:stone".into()));
                mob.extra.set_body(live.then(|| "minecraft:stone".into()));
                let expected = if species == Species::SulfurCube {
                    !live
                } else {
                    live
                };
                for input in [None, Some(&raw)] {
                    let saved = mob_to_nbt(&mob, [46, 0, 0, 1], input, None, |_| None);
                    assert_eq!(
                        saved.get("CanPickUpLoot"),
                        Some(&Tag::Byte(i8::from(expected))),
                        "{id} supplied={supplied:?} live={live} raw={}",
                        input.is_some()
                    );
                    if input.is_some() {
                        assert_eq!(saved.get("audit:unrelated"), Some(&Tag::Int(7)));
                    }
                }
            }
        }
    }
}
