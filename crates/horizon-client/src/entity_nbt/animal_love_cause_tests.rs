use super::*;
use crate::mobs::Species;

#[test]
fn animal_save_uses_the_parsed_love_cause_instead_of_stale_raw() {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.id = "minecraft:pig".into();
    ty.species = Species::Pig;
    let types = HashMap::from([("minecraft:pig".to_string(), kind)]);
    let raw = |cause: Option<Tag>| {
        let mut tag = Tag::compound([
            ("id", string("minecraft:pig")),
            ("Pos", doubles(&[1.5, 200.0, 4.5])),
            ("InLove", Tag::Int(123)),
            ("CustomName", string("kept")),
            ("Tags", Tag::List(vec![string("kept")])),
        ]);
        if let Some(cause) = cause {
            put(&mut tag, "LoveCause", cause);
        }
        tag
    };
    let valid = [57, 1, 2, 3];
    for cause in [
        None,
        Some(Tag::IntArray(vec![])),
        Some(Tag::IntArray(vec![57, 1, 2])),
        Some(Tag::IntArray(vec![57, 1, 2, 3, 4])),
        Some(Tag::Int(7)),
        Some(Tag::compound([("value", Tag::Int(7))])),
    ] {
        let raw = raw(cause);
        let mut mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
        assert_eq!(mob.extra.love_cause, None);
        for input in [None, Some(&raw)] {
            let saved = mob_to_nbt(&mob, [57, 0, 0, 1], input, None, |_| None);
            assert!(saved.get("LoveCause").is_none(), "invalid raw {raw:?}");
            assert_eq!(saved.get("InLove"), Some(&Tag::Int(123)));
            if input.is_some() {
                assert_eq!(saved.get("CustomName"), raw.get("CustomName"));
                assert_eq!(saved.get("Tags"), raw.get("Tags"));
            }
            mob.extra.love_cause = Some(valid);
            let saved = mob_to_nbt(&mob, [57, 0, 0, 1], input, None, |_| None);
            assert_eq!(saved.get("LoveCause"), Some(&Tag::IntArray(valid.to_vec())));
            mob.extra.love_cause = None;
        }
    }
    let loaded_raw = raw(Some(Tag::IntArray(valid.to_vec())));
    let mut mob = mob_from_nbt(&loaded_raw, &types, 2, None).unwrap();
    assert_eq!(mob.extra.love_cause, Some(valid));
    assert_eq!(
        mob_to_nbt(&mob, [57, 0, 0, 2], None, None, |_| None).get("LoveCause"),
        Some(&Tag::IntArray(valid.to_vec()))
    );
    mob.extra.love_cause = None;
    assert!(
        mob_to_nbt(&mob, [57, 0, 0, 2], Some(&loaded_raw), None, |_| None)
            .get("LoveCause")
            .is_none()
    );
    // This cleanup belongs only to Animal's writer, not generic raw data.
    Arc::make_mut(&mut mob.kind).id = "minecraft:zombie".into();
    Arc::make_mut(&mut mob.kind).species = Species::Zombie;
    assert_eq!(
        mob_to_nbt(&mob, [57, 0, 0, 2], Some(&loaded_raw), None, |_| None).get("LoveCause"),
        loaded_raw.get("LoveCause")
    );
}
