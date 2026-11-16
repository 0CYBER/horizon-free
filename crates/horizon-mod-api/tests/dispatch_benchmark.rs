#![cfg(feature = "testing")]

use mod_api::testing::FakeHost;
use mod_api::*;

#[test]
#[ignore = "optimized hot-hook microbenchmark"]
fn subscriber_tick_benchmark() {
    use std::{hint::black_box, time::Instant};
    struct Quiet;
    #[mod_api::__subscriptions]
    impl Hooks for Quiet {}
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, event: &mut ClientTickPreEvent) {
            black_box(event);
        }
    }
    trait Before {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {}
    }
    impl Before for Quiet {}
    impl Before for Listener {
        fn client_tick_pre(&self, _: &mut dyn ClientHost, event: &mut ClientTickPreEvent) {
            black_box(event);
        }
    }
    struct BeforeMod {
        id: &'static str,
        hooks: Box<dyn Before>,
    }
    let mut before: Vec<_> = (0..8)
        .map(|_| BeforeMod {
            id: "quiet",
            hooks: Box::new(Quiet),
        })
        .collect();
    before.push(BeforeMod {
        id: "listener",
        hooks: Box::new(Listener),
    });
    let mut installed: Vec<_> = (0..8)
        .map(|_| ModEntry::new("quiet", Box::new(Quiet)))
        .collect();
    installed.push(ModEntry::new("listener", Box::new(Listener)));
    mod_api::__private::install(installed.into());
    let mut host = FakeHost::new();
    dispatch_client_tick_pre(&mut host); // Warm the dispatch outside the timed loop.
    let iterations = 1_000_000;
    let start = Instant::now();
    for _ in 0..iterations {
        let mut event = ClientTickPreEvent {};
        for entry in black_box(&before) {
            let _guard = mod_api::__private::enter(entry.id, "client_tick_pre");
            entry.hooks.client_tick_pre(&mut host, &mut event);
        }
    }
    let baseline = start.elapsed().as_nanos() as f64 / f64::from(iterations);
    let start = Instant::now();
    for _ in 0..iterations {
        dispatch_client_tick_pre(black_box(&mut host));
    }
    let subscribed = start.elapsed().as_nanos() as f64 / f64::from(iterations);
    println!(
        "tick dispatch: before {baseline:.2} ns, subscribed {subscribed:.2} ns per call ({iterations} iterations)"
    );
}
