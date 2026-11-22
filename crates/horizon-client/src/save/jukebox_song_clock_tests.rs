use super::*;
use crate::containers::{BlockEntity, Jukebox};

fn input(clock: Option<i64>, with_record: bool) -> Tag {
    let record = if with_record {
        "RecordItem:{id:\"minecraft:music_disc_cat\",count:1,components:{\"minecraft:custom_data\":{h032:1}}},"
    } else {
        ""
    };
    let clock = clock.map_or(String::new(), |n| format!("ticks_since_song_started:{n}L,"));
    nbt::parse_snbt(&format!(
        "{{id:\"minecraft:jukebox\",x:4,y:200,z:4,{record}{clock}}}"
    ))
    .unwrap()
    .0
}
fn record_kept(tag: &Tag) {
    let record = tag.get("RecordItem").expect("actual modeled record");
    assert_eq!(
        record.get("id"),
        Some(&Tag::String("minecraft:music_disc_cat".into()))
    );
    assert_eq!(record.get("count"), Some(&Tag::Int(1)));
    assert_eq!(
        record
            .get("components")
            .and_then(|c| c.get("minecraft:custom_data"))
            .and_then(|c| c.get("h032")),
        Some(&Tag::Int(1))
    );
}
#[test]
fn model_save_preserves_song_clock_presence_without_raw() {
    let mut failures = Vec::new();
    let mut check = |label: &str, tag: &Tag, expected: Option<i64>| {
        let got = tag.get("ticks_since_song_started").cloned();
        let want = expected.map(Tag::Long);
        if got != want {
            failures.push(format!("{label}: {got:?}, expected {want:?}"));
        }
    };
    for (label, clock, with_record) in [
        ("empty", None, false),
        ("record-only", None, true),
        ("loaded-zero", Some(0), true),
        ("loaded-41", Some(41), true),
    ] {
        let (pos, model) =
            decode_block_entity(&input(clock, with_record)).expect("actual load path");
        let BlockEntity::Jukebox(j) = &model else {
            panic!("wrong modeled type")
        };
        assert_eq!(j.record().is_some(), with_record);
        assert_eq!(j.ticks, clock.unwrap_or(0));
        // `setSongWithoutPlaying`: a saved clock, unstarted; else no song.
        assert_eq!(j.playing, clock.is_some());
        assert_eq!(j.loaded_clock, clock.is_some());
        let saved = block_entity_encode::encode(pos, &model, "minecraft:jukebox");
        let command = block_entity_data(pos, &model, "minecraft:jukebox", None);
        check(label, &saved, clock);
        check(label, &command, clock);
        if with_record {
            record_kept(&saved);
            record_kept(&command);
        } else {
            assert!(saved.get("RecordItem").is_none());
        }
        if clock.is_some() {
            let mut detached = model.clone();
            let BlockEntity::Jukebox(j) = &mut detached else {
                unreachable!()
            };
            j.playing = false;
            // Decoded clock state, not this separate legacy flag, owns saving.
            let saved = block_entity_encode::encode(pos, &detached, "minecraft:jukebox");
            check("clock independent of legacy playing", &saved, clock);
            record_kept(&saved);
        }
    }
    let empty = BlockEntity::Jukebox(Jukebox::default());
    check(
        "default modeled/no raw",
        &block_entity_encode::encode((4, 200, 4), &empty, "minecraft:jukebox"),
        None,
    );
    let raw = input(Some(41), true);
    let (pos, loaded_model) = decode_block_entity(&raw).unwrap();
    let loaded = block_entity_encode::encode(pos, &loaded_model, "minecraft:jukebox");
    let BlockEntity::Jukebox(j) = loaded_model else {
        unreachable!()
    };
    let current = BlockEntity::Jukebox(Jukebox {
        items: j.items,
        playing: true,
        ..Default::default()
    });
    let current = block_entity_encode::encode(pos, &current, "minecraft:jukebox");
    check("modeled record-only/no raw", &current, None);
    record_kept(&current);
    // The chunk writer uses this real overlay after encoding its model.
    // A stale input clock must not resurrect the removed modeled field.
    let merged = merge_block_entity(&raw, &loaded, &current);
    check("contradictory raw clock", &merged, None);
    record_kept(&merged);
    let canonical = crate::block_entity_nbt::canonical(&merged).unwrap();
    check(
        "chunk canonical after actual model overlay",
        &canonical,
        None,
    );
    record_kept(&canonical);
    assert!(
        failures.is_empty(),
        "modeled song clock presence mismatches: {failures:?}"
    );
}
