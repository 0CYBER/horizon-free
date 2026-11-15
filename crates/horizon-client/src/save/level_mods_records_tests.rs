use super::*;
use crate::save::{read_dat, write_dat};

fn record(id: &str, version: &str) -> ModRecord {
    ModRecord {
        id: id.into(),
        version: version.into(),
    }
}

fn mods(list: &[(&str, &str)]) -> Tag {
    Tag::List(
        list.iter()
            .map(|(id, version)| {
                Tag::compound([
                    ("ModId", Tag::String((*id).into())),
                    ("ModVersion", Tag::String((*version).into())),
                ])
            })
            .collect(),
    )
}

/// A level.dat root saved with code mods `code`, content mods `content` and block namespaces
/// `namespaces`.
fn root(code: &[(&str, &str)], content: &[(&str, &str)], namespaces: &[&str]) -> Tag {
    Tag::compound([
        (
            "Data",
            Tag::compound([("LevelName", Tag::String("Test".into()))]),
        ),
        ("fml", Tag::compound([("LoadingModList", mods(code))])),
        (
            COMPOUND,
            Tag::compound([
                (CONTENT_MODS, mods(content)),
                (
                    BLOCK_NAMESPACES,
                    Tag::List(
                        namespaces
                            .iter()
                            .map(|n| Tag::String((*n).into()))
                            .collect(),
                    ),
                ),
            ]),
        ),
    ])
}

fn missing_mod(id: &str, version: &str, code: bool, content: bool, blocks: bool) -> MissingMod {
    MissingMod {
        id: id.into(),
        saved_version: version.into(),
        code,
        content,
        blocks,
    }
}

/// A code mod and a content mod no longer loaded, each named once with what is lost; a block
/// namespace nothing registers now joins its mod, or stands alone when no saved mod has its id.
/// A changed version, a mod loaded now in either list, and `minecraft` are no loss.
#[test]
fn missing_names_unloaded_mods_and_unregistered_block_namespaces() {
    let saved = root(
        &[
            ("minecraft", "26.3"),
            ("machines", "1.0.0"),
            ("kept", "1.0.0"),
            ("bumped", "1.0.0"),
        ],
        &[("decor", "2.1.0"), ("moved", "1.0.0")],
        &["machines", "decor", "kept", "orphan", "minecraft"],
    );
    let loaded = Loaded {
        code: vec![record("kept", "1.0.0"), record("bumped", "2.0.0")],
        content: vec![record("moved", "1.0.0")],
        namespaces: vec!["kept".into()],
    };
    assert_eq!(
        missing(&saved, &loaded),
        vec![
            missing_mod("decor", "2.1.0", false, true, true),
            missing_mod("machines", "1.0.0", true, false, true),
            missing_mod("orphan", "", false, false, true),
        ]
    );
    // Everything back: nothing is missing.
    let all = Loaded {
        code: vec![
            record("machines", "1.0.0"),
            record("kept", "1.0.0"),
            record("bumped", "2.0.0"),
        ],
        content: vec![record("decor", "2.1.0"), record("moved", "1.0.0")],
        namespaces: vec![
            "decor".into(),
            "kept".into(),
            "machines".into(),
            "orphan".into(),
        ],
    };
    assert!(missing(&saved, &all).is_empty());
}

/// A world with neither record (vanilla, or saved before Horizon kept them) has nothing missing,
/// and one with only the code list is checked on that list alone.
#[test]
fn worlds_without_the_records_are_checked_on_what_they_have() {
    let vanilla = Tag::compound([(
        "Data",
        Tag::compound([("LevelName", Tag::String("Vanilla".into()))]),
    )]);
    assert!(missing(&vanilla, &Loaded::default()).is_empty());
    let code_only = Tag::compound([(
        "fml",
        Tag::compound([("LoadingModList", mods(&[("machines", "1.0.0")]))]),
    )]);
    assert_eq!(
        missing(&code_only, &Loaded::default()),
        vec![missing_mod("machines", "1.0.0", true, false, false)]
    );
}

/// The records and the build marker round-trip through `level.dat` beside the other keys of the
/// compound; a build that does not know its commit removes the marker, and a malformed marker
/// reads as none.
#[test]
fn the_records_and_build_marker_round_trip() {
    let mut data = Tag::compound([(
        COMPOUND,
        Tag::compound([
            ("Other", Tag::Int(7)),
            (BUILD_COMMIT, Tag::String("old".into())),
        ]),
    )]);
    write_with(
        &mut data,
        Some(&[record("decor", "2.1.0")]),
        Some(&["decor".to_owned()]),
        Some("0123abcd"),
    );
    assert_eq!(
        horizon_records(&data),
        HorizonRecords {
            content_mods: Some(vec![record("decor", "2.1.0")]),
            block_namespaces: Some(vec!["decor".into()]),
            build_commit: Some("0123abcd".into()),
        }
    );
    assert_eq!(
        data.get(COMPOUND).and_then(|c| c.get("Other")),
        Some(&Tag::Int(7))
    );
    // Before startup says what is loaded, the lists stay; an unknown build drops the marker.
    write_with(&mut data, None, None, None);
    let kept = horizon_records(&data);
    assert_eq!(kept.content_mods, Some(vec![record("decor", "2.1.0")]));
    assert_eq!(kept.build_commit, None);
    let malformed = Tag::compound([(COMPOUND, Tag::compound([(BUILD_COMMIT, Tag::Int(3))]))]);
    assert_eq!(horizon_records(&malformed).build_commit, None);
    assert_eq!(
        horizon_records(&Tag::Compound(Default::default())),
        HorizonRecords::default()
    );
}

/// Checking a world reads it and writes nothing: every byte of its level.dat stays.
#[test]
fn checking_a_world_leaves_its_files_alone() {
    let dir = std::env::temp_dir().join(format!("horizon-world-guard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("level.dat");
    write_dat(&file, root(&[("machines", "1.0.0")], &[], &["machines"])).unwrap();
    let before = std::fs::read(&file).unwrap();
    assert_eq!(
        missing_in(&dir, &Loaded::default()),
        vec![missing_mod("machines", "1.0.0", true, false, true)]
    );
    assert_eq!(
        mismatches_in(&dir, &Loaded::default()).map(|m| m.len()),
        Some(1)
    );
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert!(read_dat(&file).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A resolution of a missing mod takes it off the list; an answer kept for one world is not
/// another's.
#[test]
fn resolutions_and_kept_answers_belong_to_their_mod_and_world() {
    let _answer = TEST_ANSWER.lock().unwrap_or_else(|e| e.into_inner());
    let list = vec![
        missing_mod("decor", "1", false, true, false),
        missing_mod("machines", "1", true, false, true),
    ];
    let resolved = vec![mod_api::ModMismatchResolution {
        mod_id: "machines".into(),
        resolver: "migrator".into(),
    }];
    assert_eq!(
        unresolved(list, &resolved),
        vec![missing_mod("decor", "1", false, true, false)]
    );
    let here = std::env::temp_dir().join(format!("horizon-answer-a-{}", std::process::id()));
    let there = std::env::temp_dir().join(format!("horizon-answer-b-{}", std::process::id()));
    remember_answer(&here, Vec::new(), resolved.clone());
    assert!(!is_confirmed(&here));
    confirm(&there);
    assert!(!is_confirmed(&here));
    confirm(&here);
    assert!(is_confirmed(&here) && !is_confirmed(&there));
    assert_eq!(take_answer(&there), None);
    remember_answer(&here, Vec::new(), resolved.clone());
    assert_eq!(take_answer(&here), Some((Vec::new(), resolved)));
    assert_eq!(take_answer(&here), None);
}
