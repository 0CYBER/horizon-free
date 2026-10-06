use horizon_world::worldgen::{
    height_provider::HeightProvider, value_provider::IntProvider, weighted::WeightedList,
};

#[test]
fn weighted_lists_reject_invalid_totals_and_bound_entry_allocations() {
    assert!(WeightedList::<()>::new(vec![]).is_err());
    assert!(WeightedList::new(vec![((), 0)]).is_err());
    assert!(WeightedList::new(vec![((), -1), ((), 2)]).is_err());
    assert!(WeightedList::new(vec![((), i32::MAX), ((), 1)]).is_err());
    assert!(WeightedList::new(vec![((), i32::MAX)]).is_ok());
    assert!(WeightedList::new(vec![((), 1); 8192]).is_ok());
    assert!(WeightedList::new(vec![((), 1); 8193]).is_err());
}

#[test]
fn branching_provider_decoders_share_a_total_node_budget() {
    for height in [false, true] {
        let leaf = if height { r#"{"absolute":0}"# } else { "0" };
        let entry = format!(r#"{{"data":{leaf},"weight":1}}"#);
        let branch = format!(
            r#"{{"type":"weighted_list","distribution":[{}]}}"#,
            vec![entry; 256].join(",")
        );
        let entry = format!(r#"{{"data":{branch},"weight":1}}"#);
        let tree = format!(
            r#"{{"type":"weighted_list","distribution":[{}]}}"#,
            vec![entry; 256].join(",")
        );
        // Each list is below its own limit, but the whole tree exceeds 65536 nodes.
        if height {
            assert!(HeightProvider::from_json(&tree).is_err());
        } else {
            assert!(IntProvider::from_json(&tree).is_err());
        }
    }
}

#[test]
fn optional_weighted_choices_retain_zero_entries_and_skip_empty_draws() {
    use horizon_core::random::{LegacyRandom, RandomSource};
    use horizon_world::worldgen::weighted::OptionalWeightedList;
    let mut random = LegacyRandom::new(12);
    let mut expected = random.clone();
    let empty = OptionalWeightedList::<()>::new(vec![]).unwrap();
    assert_eq!(empty.sample(&mut random).unwrap(), None);
    let zero = OptionalWeightedList::new(vec![(1, 0), (2, 0)]).unwrap();
    assert_eq!(zero.entries(), &[(1, 0), (2, 0)]);
    assert_eq!(zero.sample(&mut random).unwrap(), None);
    assert_eq!(random.next_long(), expected.next_long());
    assert!(WeightedList::new(vec![(1, 0), (2, 0)]).is_err());
    assert!(OptionalWeightedList::new(vec![(1, i32::MAX), (2, 1)]).is_err());
}
