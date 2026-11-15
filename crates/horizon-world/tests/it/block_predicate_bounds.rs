use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::{
    heightmap::HeightmapType,
    worldgen::{
        anchor::WorldGenerationContext,
        block_predicate::BlockPredicate,
        placement::{PlacementContext, PlacementError, PlacementModifier, PlacementQueryError},
    },
};
struct UnboundedWorld(usize);
impl PlacementContext for UnboundedWorld {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(0, 16, 8)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(i32::MIN)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(i32::MAX)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        unreachable!()
    }
    fn outside_build_height(&mut self, _: i32) -> Result<bool, PlacementQueryError> {
        self.0 += 1;
        Ok(false)
    }
}
#[test]
fn directly_constructed_scans_share_predicate_work_budget() {
    let scan = PlacementModifier::EnvironmentScan {
        upwards: true,
        target: BlockPredicate::Not(Box::new(BlockPredicate::True)),
        allowed: BlockPredicate::True,
        max_steps: i32::MAX,
    };
    let mut world = UnboundedWorld(0);
    let mut random = LegacyRandom::new(1);
    let mut expected = random.clone();
    let result = scan.modify_in(
        &mut world,
        &mut random,
        [0, i32::MAX, 0],
        |_, _| -> Result<(), ()> { panic!("false target must not emit") },
    );
    assert!(matches!(
        result,
        Err(PlacementError::World(PlacementQueryError::WorkLimit))
    ));
    assert!(world.0 > 0 && world.0 < 65536);
    assert_eq!(random.next_long(), expected.next_long());
}
#[test]
fn predicate_trees_share_decode_and_evaluation_budgets() {
    let leaf = r#"{"type":"true"}"#;
    let branch = format!(
        r#"{{"type":"all_of","predicates":[{}]}}"#,
        vec![leaf; 256].join(",")
    );
    let tree = format!(
        r#"{{"type":"all_of","predicates":[{}]}}"#,
        vec![branch; 256].join(",")
    );
    assert!(BlockPredicate::from_json(&tree).is_err());
    let tree = BlockPredicate::All(vec![
        BlockPredicate::All(vec![BlockPredicate::True; 256]);
        256
    ]);
    assert!(matches!(
        tree.test(&mut UnboundedWorld(0), [0; 3]),
        Err(PlacementQueryError::WorkLimit)
    ));
}

#[test]
fn oversized_volume_keeps_early_exit_and_bounds_total_work() {
    let mut world = UnboundedWorld(0);
    let volume = |predicate| BlockPredicate::Volume {
        min: [i32::MIN; 3],
        max: [i32::MAX; 3],
        predicate: Box::new(predicate),
    };
    assert!(
        !volume(BlockPredicate::Not(Box::new(BlockPredicate::True)))
            .test(&mut world, [0; 3])
            .unwrap()
    );
    assert!(matches!(
        volume(BlockPredicate::True).test(&mut world, [0; 3]),
        Err(PlacementQueryError::WorkLimit)
    ));
}
