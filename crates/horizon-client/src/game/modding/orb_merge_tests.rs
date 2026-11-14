//! The experience orb's saved form (docs: `entity_nbt::orb_to_nbt`) and the exact-total
//! write boundary that keeps a merge's result reloadable.
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::entities::ExperienceOrb;
use horizon_client::entity_nbt::{orb_to_nbt, orbs_from_nbt};
use horizon_core::geometry::Vec3;

fn orb(id: u32, value: i32, count: u32) -> ExperienceOrb {
    let at = Vec3::new(0.5, 70.0, 0.5);
    ExperienceOrb {
        attachment_lifetime: Default::default(),
        fall_distance: 0.0,
        rotation: [0.0, 0.0],
        join: Default::default(),
        id,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        value,
        age: 0,
        on_ground: true,
        count,
        ticks: 0,
        health: 5,
        air: 300,
        no_gravity: true,
        following: None,
    }
}

/// Every pair `set_experience_orb` accepts reloads unchanged: `Value` is a short and
/// `Count` an int in the save.
#[test]
fn every_accepted_orb_value_and_count_survives_save_and_reload() {
    for (value, count) in [(11, 1), (32767, i32::MAX as u32), (-32768, 1)] {
        let back = orbs_from_nbt(&orb_to_nbt(&orb(1, value, count), [0; 4]));
        assert_eq!((back[0].value, back[0].count), (value, count));
    }
}

/// A write outside the save's range is refused whole: no value changed, no count changed.
#[test]
#[ignore = "daily"]
fn set_experience_orb_refuses_a_write_a_save_cannot_hold() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.orbs.push(orb(7, 3, 2));
        let dimension = game.dim.id().to_owned();
        assert!(!game.set_experience_orb(&dimension, 7, Some(40_000), None, None));
        assert!(!game.set_experience_orb(&dimension, 7, None, Some(i32::MAX as u32 + 1), None));
        assert!(!game.set_experience_orb(&dimension, 7, None, Some(0), None));
        assert_eq!((game.orbs[0].value, game.orbs[0].count), (3, 2));
        assert!(game.set_experience_orb(&dimension, 7, Some(11), Some(1), None));
        assert_eq!((game.orbs[0].value, game.orbs[0].count), (11, 1));
    });
}
