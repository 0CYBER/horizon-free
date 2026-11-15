use horizon_core::carving::{CarvingMask, CarvingMaskError};

#[test]
fn invalid_bounds_and_growth_fail_without_changing_existing_cells() {
    for (min, max) in [(0, -1), (0, 4064), (i32::MIN, i32::MAX)] {
        assert!(matches!(
            CarvingMask::new(min, max),
            Err(CarvingMaskError::InvalidBounds)
        ));
    }
    let mut mask = CarvingMask::new(-64, 319).unwrap();
    mask.carve(0, -64, 0).unwrap();
    mask.carve(15, 319, 15).unwrap();
    let previous: Vec<_> = mask.columns().collect();
    assert_eq!(mask.carve(0, -65, 0), Err(CarvingMaskError::NegativeIndex));
    assert_eq!(
        mask.carve(0, (1 << 24) - 64, 0),
        Err(CarvingMaskError::Capacity)
    );
    assert_eq!(mask.columns().collect::<Vec<_>>(), previous);
}

#[test]
fn shared_candidate_budget_counts_skipped_cells_and_stops_before_the_next_callback() {
    use horizon_core::{
        carving::{CarveError, carve_ellipsoid_with_budget},
        position::ChunkPos,
    };
    let mut mask = CarvingMask::new(0, 15).unwrap();
    let mut remaining = 2;
    let mut calls = 0;
    let result = carve_ellipsoid_with_budget(
        ChunkPos::new(0, 0),
        [8., 8., 8.],
        4.,
        4.,
        &mut mask,
        |_, _| {
            calls += 1;
            true
        },
        &mut remaining,
    );
    assert_eq!(result, Err(CarveError::IterationLimit));
    assert_eq!(calls, 2);
    assert_eq!(remaining, 0);
    assert!(mask.is_empty());
    let result = carve_ellipsoid_with_budget(
        ChunkPos::new(0, 0),
        [1000., 8., 1000.],
        1.,
        1.,
        &mut mask,
        |_, _| panic!("culled shape queried its predicate"),
        &mut remaining,
    );
    assert!(result.is_ok());
}
