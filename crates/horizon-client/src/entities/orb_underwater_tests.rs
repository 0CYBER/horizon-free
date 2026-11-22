use super::*;

#[test]
fn matches_the_executed_java_private_method() {
    // Actual 26.3 setUnderwaterMovement calls on server-level orbs,
    // captured by test_orb_water_parity.py (orb-water-reference.json).
    // Cover both horizontal signs, falling/rising, and the vertical cap.
    let cases: [([f64; 3], [u64; 3]); 6] = [
        (
            [0.0, 0.0, 0.0],
            [0x0000000000000000, 0x3f40624de0000000, 0x0000000000000000],
        ),
        (
            [0.1, 0.01, -0.2],
            [0x3fb9581066666667, 0x3f85810625ae147b, 0xbfc9581066666667],
        ),
        (
            [-0.25, -0.03, 0.125],
            [0xbfcfae1480000000, 0xbf9e353f7c851eb8, 0x3fbfae1480000000],
        ),
        (
            [0.0, 0.059, 0.0],
            [0x0000000000000000, 0x3fae76c8b46d9168, 0x0000000000000000],
        ),
        (
            [0.5, 0.0595, -0.5],
            [0x3fdfae1480000000, 0x3faeb851e0000000, 0xbfdfae1480000000],
        ),
        (
            [-1.0, 0.2, 1.0],
            [0xbfefae1480000000, 0x3faeb851e0000000, 0x3fefae1480000000],
        ),
    ];
    let actual: Vec<_> = cases
        .iter()
        .map(|(v, _)| {
            let mut orb = ExperienceOrb {
                attachment_lifetime: Default::default(),
                fall_distance: 0.0,
                rotation: [0.0, 0.0],
                join: Default::default(),
                section_order: 0,
                no_gravity: false,
                id: 1,
                position: Vec3::ZERO,
                previous: Vec3::ZERO,
                velocity: Vec3::new(v[0], v[1], v[2]),
                value: 1,
                age: 0,
                on_ground: false,
                count: 1,
                ticks: 0,
                health: 5,
                air: 300,
                following: None,
                fire_ticks: 0,
            };
            orb.set_underwater_movement();
            [
                orb.velocity.x.to_bits(),
                orb.velocity.y.to_bits(),
                orb.velocity.z.to_bits(),
            ]
        })
        .collect();
    let expected: Vec<_> = cases.into_iter().map(|(_, bits)| bits).collect();
    assert_eq!(actual, expected);
}
