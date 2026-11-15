//! Paired with native_live_holder_tests::thousand_live_block_entity_ticks.
use std::{hint::black_box, time::Instant};

#[test]
#[ignore = "measurement; needs prepared resources in HORIZON_TEST_RESOURCES"]
fn unscoped_item_entity_reads() {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.spawn_item(
            horizon_client::items::ItemStack::new("minecraft:stone", 5),
            [0.0; 3],
            [0.0; 3],
            0,
        );
        let entity = game.entity(game.dropped[0].id).unwrap();
        let item = game.item_entity(entity).unwrap();
        let mut times = Vec::with_capacity(240);
        for round in 0..260 {
            let start = Instant::now();
            for _ in 0..10_000 {
                black_box(game.item_entity_stack(black_box(item)));
            }
            if round >= 20 {
                times.push(start.elapsed().as_nanos());
            }
        }
        times.sort_unstable();
        assert_eq!(game.item_entity_stack(item).unwrap().count, 5);
        println!(
            "ITEM_READ_BENCH scope=none reads=10000 samples={} median_ns={} p95_ns={} p99_ns={}",
            times.len(),
            times[120],
            times[228],
            times[237]
        );
    });
}
