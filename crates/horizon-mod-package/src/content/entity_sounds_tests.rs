use super::*;
use crate::{Package, content::Content};

#[test]
fn entity_sounds_admission_preserves_inherit_id_and_mute_and_rejects_bad_ids() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bird.hmod");
    let manifest = "id = \"bird\"\nname = \"Bird\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    for slot in ["ambient", "hurt", "death", "step"] {
        let file = format!(
            r#"{{"template":"minecraft:wandering_trader","sounds":{{"{slot}":"bird:bad id"}}}}"#
        );
        std::fs::write(
            &path,
            Package::write_files(
                manifest,
                &[(
                    "data/bird/horizon/entity_type/vulture.json",
                    file.as_bytes(),
                )],
            )
            .unwrap(),
        )
        .unwrap();
        let package = Package::read(&path).unwrap();
        let error = Content::read(&path, &package.manifest).expect_err("bad sound admitted");
        assert!(
            error.contains("bird:vulture") && error.contains(slot),
            "{error}"
        );
    }
    let sounds: EntitySounds =
        serde_json::from_str(r#"{"hurt":"bird:hurt","step":"none"}"#).unwrap();
    assert_eq!(sounds.ambient, None);
    assert_eq!(sounds.hurt, Some(SoundOverride::Id("bird:hurt".into())));
    assert!(matches!(sounds.step, Some(SoundOverride::Mute(_))));
    sounds.validate("bird:vulture").unwrap();
    for bad in ["hurt", "Bird:hurt", "bird:", ":hurt", "bird:a:b"] {
        let sounds = EntitySounds {
            hurt: Some(SoundOverride::Id(bad.into())),
            ..Default::default()
        };
        assert!(sounds.validate("bird:vulture").is_err(), "{bad}");
    }
}

#[test]
fn entity_sounds_thrown_type_refuses_sounds_with_exact_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bird.hmod");
    let manifest = "id = \"bird\"\nname = \"Bird\"\nversion = \"1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
    let declaration = br#"{"template":"minecraft:snowball","sounds":{"hurt":"bird:hurt"}}"#;
    std::fs::write(
        &path,
        Package::write_files(
            manifest,
            &[("data/bird/horizon/entity_type/bolt.json", declaration)],
        )
        .unwrap(),
    )
    .unwrap();
    let package = Package::read(&path).unwrap();
    let error = Content::read(&path, &package.manifest).expect_err("thrown sounds admitted");
    assert_eq!(
        error,
        "data/bird/horizon/entity_type/bolt.json: only a living type has sounds (its template is minecraft:snowball)"
    );
}
