use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::edge_backoff::{
    EdgeBackoffError, EdgeBackoffInput, EdgeBackoffWorld, back_off_from_edge,
};
struct World {
    calls: usize,
}
impl EdgeBackoffWorld for World {
    type Error = std::convert::Infallible;
    fn max_up_step(&mut self) -> Result<f32, Self::Error> {
        Ok(0.6)
    }
    fn staying_on_ground_surface(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn on_ground(&mut self) -> Result<bool, Self::Error> {
        Ok(true)
    }
    fn no_collision(&mut self, _: Aabb) -> Result<bool, Self::Error> {
        self.calls += 1;
        Ok(true)
    }
}
fn input() -> EdgeBackoffInput {
    EdgeBackoffInput {
        movement: Vec3::new(f64::INFINITY, 0.0, 0.0),
        bounds: Aabb::new(Vec3::new(-0.3, 1.0, -0.3), Vec3::new(0.3, 2.8, 0.3)),
        flying: false,
        self_or_player: true,
        fall_distance: 0.0,
        query_budget: 3,
    }
}
#[test]
fn nonconverging_motion_exhausts_budget_without_successful_displacement() {
    let mut world = World { calls: 0 };
    assert!(matches!(
        back_off_from_edge(&mut world, input()),
        Err(EdgeBackoffError::QueryBudget)
    ));
    assert_eq!(world.calls, 3);
}
#[test]
fn zero_budget_only_fails_when_a_probe_is_needed() {
    let mut world = World { calls: 0 };
    let mut request = input();
    request.query_budget = 0;
    request.flying = true;
    assert!(
        back_off_from_edge(&mut world, request)
            .unwrap()
            .x
            .is_infinite()
    );
    let mut request = input();
    request.query_budget = 0;
    assert!(matches!(
        back_off_from_edge(&mut world, request),
        Err(EdgeBackoffError::QueryBudget)
    ));
    assert_eq!(world.calls, 0);
}
