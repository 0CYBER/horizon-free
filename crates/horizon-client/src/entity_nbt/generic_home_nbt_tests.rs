use super::*;
use crate::mobs::Species;

fn mob(species: Species, id: &str) -> Mob {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let ty = Arc::make_mut(&mut kind);
    ty.species = species;
    ty.id = id.into();
    Mob::new(1, kind, Vec3::new(3.5, 201.0, 0.5), 0.0)
}
fn stale() -> Tag {
    Tag::compound([
        ("home_radius", Tag::Int(99)),
        ("home_pos", Tag::IntArray(vec![90, 91, 92])),
        ("audit:unrelated", Tag::Int(7)),
    ])
}

#[test]
fn spawned_trader_home_restricts_and_survives_save() {
    let mut m = mob(Species::WanderingTrader, "minecraft:wandering_trader");
    crate::mobs::trader::start_visit(&mut m, [4, 201, 0]);
    m.position = Vec3::new(19.5, 201.0, 0.5);
    assert!(crate::mobs::trader::within_home(&m));
    m.position = Vec3::new(20.5, 201.0, 0.5);
    assert!(!crate::mobs::trader::within_home(&m));
    let saved = mob_to_nbt(&m, [0; 4], None, None, |_| None);
    assert_eq!(saved.get("home_radius"), Some(&Tag::Int(16)));
    assert_eq!(saved.get("home_pos"), Some(&Tag::IntArray(vec![4, 201, 0])));
    let mut loaded = mob(Species::WanderingTrader, "minecraft:wandering_trader");
    loaded.position = m.position;
    read_mob_home(&mut loaded, &saved);
    assert!(!crate::mobs::trader::within_home(&loaded));
}

#[test]
fn generic_home_output_uses_model_and_removes_stale_raw_fields() {
    let mut m = mob(Species::Pig, "minecraft:pig");
    let raw = stale();
    for raw in [None, Some(&raw)] {
        for radius in [0, 5, -2, -1] {
            m.ai.home = Some(([4, 5, 6], radius));
            let saved = mob_to_nbt(&m, [0; 4], raw, None, |_| None);
            if radius == -1 {
                assert!(saved.get("home_radius").is_none());
                assert!(saved.get("home_pos").is_none());
            } else {
                assert_eq!(saved.get("home_radius"), Some(&Tag::Int(radius)));
                assert_eq!(saved.get("home_pos"), Some(&Tag::IntArray(vec![4, 5, 6])));
            }
        }
        m.ai.home = None;
        let saved = mob_to_nbt(&m, [0; 4], raw, None, |_| None);
        assert!(saved.get("home_radius").is_none());
        assert!(saved.get("home_pos").is_none());
        if raw.is_some() {
            assert_eq!(saved.get("audit:unrelated"), Some(&Tag::Int(7)));
        }
    }
}

#[test]
fn negative_home_updates_retain_position_without_replacing_runtime_ai() {
    let mut m = mob(Species::Pig, "minecraft:pig");
    m.ai.home = Some(([4, 5, 6], 5));
    m.ai.jump = true;
    m.ai.speed = 0.37;
    m.ai.no_action_time = 29;
    for radius in [-2, -1, -2] {
        read_mob_home(&mut m, &Tag::compound([("home_radius", Tag::Int(radius))]));
        assert_eq!(m.ai.home, (radius != -1).then_some(([4, 5, 6], radius)));
        assert_eq!(m.ai.home_position(), [4, 5, 6]);
        assert!(m.ai.jump);
        assert_eq!((m.ai.speed, m.ai.no_action_time), (0.37, 29));
    }
    // Runtime setters still assign ai.home directly. clearHome must retain
    // that latest live position, rather than a previous load's position.
    m.ai.home = Some(([7, 8, 9], 5));
    m.ai.clear_home();
    assert!(m.ai.home.is_none());
    read_mob_home(&mut m, &Tag::compound([("home_radius", Tag::Int(-2))]));
    assert_eq!(m.ai.home, Some(([7, 8, 9], -2)));
}

#[test]
fn fresh_home_loads_apply_radius_and_position_codec_defaults() {
    let m = mob(Species::Pig, "minecraft:pig");
    let types = [(m.kind.id.clone(), m.kind.clone())].into();
    for radius in [0, 5, -1, -2] {
        for pos in [
            None,
            Some(Tag::IntArray(vec![4, 5, 6])),
            Some(Tag::IntArray(vec![4, 5])),
        ] {
            let mut raw = Tag::compound([
                ("id", string("minecraft:pig")),
                ("Pos", doubles(&[3.5, 201.0, 0.5])),
                ("home_radius", Tag::Int(radius)),
            ]);
            if let Some(pos) = pos.clone() {
                put(&mut raw, "home_pos", pos);
            }
            let loaded = mob_from_nbt(&raw, &types, 1, None).unwrap();
            let expected_pos = if radius >= 0 && pos == Some(Tag::IntArray(vec![4, 5, 6])) {
                [4, 5, 6]
            } else {
                [0; 3]
            };
            assert_eq!(
                loaded.ai.home,
                (radius != -1).then_some((expected_pos, radius))
            );
        }
    }
    let mut loaded = m;
    read_mob_home(
        &mut loaded,
        &Tag::compound([
            ("home_radius", Tag::Float((-2.5_f32).to_bits())),
            ("home_pos", Tag::IntArray(vec![4, 5, 6])),
        ]),
    );
    assert_eq!(loaded.ai.home, Some(([0; 3], -3)));
    read_mob_home(
        &mut loaded,
        &Tag::compound([
            ("home_radius", Tag::Int(5)),
            ("home_pos", doubles(&[-2.5, 5.5, 6.5])),
        ]),
    );
    assert_eq!(loaded.ai.home, Some(([-2, 5, 6], 5)));
}

#[test]
fn turtle_and_creaking_keep_their_own_home_position_writer() {
    let raw = stale();
    for (species, id) in [
        (Species::Turtle, "minecraft:turtle"),
        (Species::Creaking, "minecraft:creaking"),
    ] {
        let mut m = mob(species, id);
        m.extra.home = Some([4, 5, 6]);
        for raw in [None, Some(&raw)] {
            for home in [None, Some(([7, 8, 9], 5))] {
                m.ai.home = home;
                let saved = mob_to_nbt(&m, [0; 4], raw, None, |_| None);
                assert_eq!(saved.get("home_pos"), Some(&Tag::IntArray(vec![4, 5, 6])));
                assert_eq!(
                    saved.get("home_radius"),
                    home.map(|(_, r)| Tag::Int(r)).as_ref()
                );
            }
        }
    }
}
